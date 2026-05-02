//! GPUI ↔ tokio(codex) 브리지.
//!
//! GPUI는 자체 executor를 쓰지만 codex 클라이언트는 tokio 런타임을 요구한다.
//! 별도 스레드에서 tokio Runtime을 돌리고 양방향 mpsc 채널로 통신한다.
//!
//! M1.1 범위: 단일 세션, hardcoded provider/model. 설정 화면은 이후.

use std::path::PathBuf;

use stcode_codex::{
    ApprovalPolicy, ClientInfo, SandboxMode, SpawnOptions, ThreadEvent, ThreadSession,
    ThreadStartParams, TurnStatus,
};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum UiCommand {
    /// 폴더로 새 thread 시작.
    StartProject { path: PathBuf },
    /// 사용자 텍스트를 turn으로 보냄.
    SendText(String),
    /// 정리 후 tokio 스레드 종료.
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    /// thread/start 성공.
    Started { thread_id: String },
    /// agentMessage delta — 본문에 한 글자~여러 글자.
    AgentDelta(String),
    /// turn 종료. ok=true면 정상 완료, false면 실패 + error_text.
    TurnDone {
        ok: bool,
        error_text: Option<String>,
    },
    /// 일반 에러 (세션 시작 실패 등).
    Error(String),
}

/// 브리지 핸들. cmd_tx로 명령 보내고 evt_rx로 이벤트 받는다.
pub struct Bridge {
    pub cmd_tx: mpsc::UnboundedSender<UiCommand>,
    pub evt_rx: mpsc::UnboundedReceiver<UiEvent>,
}

impl Bridge {
    /// 별도 OS 스레드에서 tokio 런타임을 띄우고 핸들러 루프 시작.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = evt_tx.send(UiEvent::Error(format!("tokio rt 생성 실패: {e}")));
                    return;
                }
            };
            rt.block_on(handler_loop(cmd_rx, evt_tx));
        });

        Bridge { cmd_tx, evt_rx }
    }
}

async fn handler_loop(
    mut cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    evt_tx: mpsc::UnboundedSender<UiEvent>,
) {
    let mut session: Option<ThreadSession> = None;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            UiCommand::StartProject { path } => {
                if let Some(prev) = session.take() {
                    prev.shutdown().await;
                }
                match start_session(&path).await {
                    Ok(s) => {
                        let _ = evt_tx.send(UiEvent::Started {
                            thread_id: s.thread_id.clone(),
                        });
                        session = Some(s);
                    }
                    Err(e) => {
                        let _ = evt_tx.send(UiEvent::Error(format!("세션 시작 실패: {e}")));
                    }
                }
            }
            UiCommand::SendText(text) => {
                let Some(s) = session.as_mut() else {
                    let _ = evt_tx.send(UiEvent::Error("세션이 시작되지 않았어요".into()));
                    continue;
                };
                if let Err(e) = s.send_user_text(text).await {
                    tracing::warn!("turn 시작 실패: {e}");
                    let _ = evt_tx.send(UiEvent::Error(format!("turn 시작 실패: {e}")));
                    continue;
                }
                pump_turn(s, &evt_tx).await;
            }
            UiCommand::Shutdown => break,
        }
    }

    if let Some(s) = session {
        s.shutdown().await;
    }
}

async fn pump_turn(s: &mut ThreadSession, evt_tx: &mpsc::UnboundedSender<UiEvent>) {
    while let Some(ev) = s.next_event().await {
        match ev {
            ThreadEvent::AgentMessageDelta(d) => {
                let _ = evt_tx.send(UiEvent::AgentDelta(d.delta));
            }
            ThreadEvent::TurnCompleted { turn } => {
                let ok = matches!(turn.status, Some(TurnStatus::Completed));
                let error_text = turn.error.map(|e| e.to_string());
                if !ok {
                    tracing::warn!("turn 실패: {:?}", error_text);
                }
                let _ = evt_tx.send(UiEvent::TurnDone { ok, error_text });
                return;
            }
            _ => {}
        }
    }
    let _ = evt_tx.send(UiEvent::Error("codex 연결 끊김".into()));
}

async fn start_session(path: &PathBuf) -> anyhow::Result<ThreadSession> {
    let mut opts = SpawnOptions::with_provider_model("local-vllm", "qwen3.6-35b-a3b");
    // codex fork(STCODE_VLLM_COMPAT=1)가 outbound input 평탄화 + reasoning→
    // OutputTextDelta를 직접 처리. proxy 불필요. base_url은 사용자 config.toml의
    // local-vllm 그대로 사용 (100.105.145.6:8000).
    opts = opts
        .with_env("STCODE_VLLM_COMPAT", "1")
        // 사용자 config.toml은 xhigh — reasoning model이 무한 사고만 하고 message
        // 안 출력하는 케이스를 막는다. fork 패치로 reasoning이 message로 노출되니
        // 더 이상 필수는 아니지만, 토큰 절약 위해 유지.
        .push("model_reasoning_effort", "minimal")
        // codex는 wire_api=Responses 시 WebSocket을 우선 시도하지만 vLLM은 ws 미지원.
        // 또 우리 fork 패치는 HTTP path(endpoint/responses.rs)에만 있어 ws path는
        // 변환 안 됨. provider config에서 ws 비활성화 필요.
        .push("model_providers.local-vllm.supports_websockets", "false");
    if std::env::var_os("VLLM_API_KEY").is_none() {
        opts = opts.with_env("VLLM_API_KEY", "dummy");
    }
    ThreadSession::start_with(
        ClientInfo::default(),
        ThreadStartParams {
            cwd: Some(path.to_string_lossy().into_owned()),
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox: Some(SandboxMode::ReadOnly),
            ..Default::default()
        },
        opts,
    )
    .await
}
