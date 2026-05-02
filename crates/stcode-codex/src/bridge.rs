//! GPUI ↔ tokio(codex) 브리지.
//!
//! GPUI는 자체 executor를 쓰지만 codex 클라이언트는 tokio 런타임을 요구한다.
//! 별도 스레드에서 tokio Runtime을 돌리고 양방향 mpsc 채널로 통신한다.
//!
//! M1.1 범위: 단일 세션, hardcoded provider/model. 설정 화면은 이후.

use std::path::PathBuf;

use crate::{
    method, ApprovalPolicy, ClientInfo, SandboxMode, SpawnOptions, ThreadEvent, ThreadSession,
    ThreadStartParams, TurnStatus,
};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum UiCommand {
    /// 폴더로 새 thread 시작.
    StartProject { path: PathBuf },
    /// 사용자 텍스트를 turn으로 보냄.
    SendText(String),
    /// 승인 요청에 응답. (자동 모드에선 bridge가 자체 처리하므로 거의 호출되지 않음)
    ApprovalDecision {
        request_id: i64,
        decision: ApprovalDecision,
    },
    /// 마지막 turn 변경을 hard reset으로 되돌린다.
    RevertLastTurn,
    /// 정리 후 tokio 스레드 종료.
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AcceptOnce,
    AcceptForSession,
    Decline,
}

impl ApprovalDecision {
    fn as_wire(self) -> &'static str {
        match self {
            Self::AcceptOnce => "accept",
            Self::AcceptForSession => "acceptForSession",
            Self::Decline => "decline",
        }
    }
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    /// thread/start 성공.
    Started { thread_id: String },
    /// agentMessage delta — 본문에 한 글자~여러 글자.
    AgentDelta(String),
    /// reasoning text delta (qwen 등 reasoning model의 thinking).
    ReasoningDelta(String),
    /// 도구 카드 시작 — commandExecution / fileChange 등.
    ToolStarted {
        item_id: String,
        kind: ToolKind,
        title: String,
    },
    /// 도구 출력 incremental — commandExecution stdout/stderr.
    ToolOutput {
        item_id: String,
        delta: String,
    },
    /// 도구 종료 — exit code 또는 success/fail.
    ToolCompleted {
        item_id: String,
        ok: bool,
        summary: Option<String>,
    },
    /// 승인 요청 — 친화적 표현 + raw 디테일을 같이 줘서 모달이 골라서 보여줌.
    ApprovalRequested {
        request_id: i64,
        kind: ToolKind,
        /// "터미널 명령을 실행해도 될까요?" 같은 자연어 제목.
        friendly_title: String,
        /// 실제 명령/경로. 작은 글씨로 보조 표시.
        raw_detail: String,
    },
    /// turn 종료. ok=true면 정상 완료, false면 실패 + error_text.
    TurnDone {
        ok: bool,
        error_text: Option<String>,
    },
    /// turn이 working tree에 변경을 만들어 자동 커밋됨.
    TurnCommitted {
        commit_oid: String,
        summary: String,
        /// 되돌리기 시 reset 대상. None이면 첫 commit (되돌리기 불가).
        revert_to: Option<String>,
    },
    /// 되돌리기 결과.
    Reverted {
        ok: bool,
        error_text: Option<String>,
    },
    /// 일반 에러 (세션 시작 실패 등).
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    CommandExecution,
    FileChange,
    McpToolCall,
    WebSearch,
    Other,
}

impl ToolKind {
    pub fn from_item_type(t: &str) -> Self {
        match t {
            "commandExecution" => Self::CommandExecution,
            "fileChange" => Self::FileChange,
            "mcpToolCall" => Self::McpToolCall,
            "webSearch" => Self::WebSearch,
            _ => Self::Other,
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::CommandExecution => "⚙",
            Self::FileChange => "📄",
            Self::McpToolCall => "🔌",
            Self::WebSearch => "🌐",
            Self::Other => "•",
        }
    }
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

/// handler_loop 가 들고 있는 project 단위 상태.
struct ProjectState {
    path: PathBuf,
    /// 가장 최근 SendText 시점의 prompt — turn 종료 후 commit 메시지로 사용.
    pending_prompt: Option<String>,
    /// pending_prompt 와 짝. turn 시작 직전 HEAD oid (revert 대상).
    pending_snapshot: Option<Option<String>>,
    /// 가장 최근 commit의 revert_to oid — RevertLastTurn 의 타깃.
    last_revert_to: Option<String>,
}

async fn handler_loop(
    mut cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    evt_tx: mpsc::UnboundedSender<UiEvent>,
) {
    let mut session: Option<ThreadSession> = None;
    let mut project: Option<ProjectState> = None;

    loop {
        // 두 갈래를 동시에 await. 세션이 없으면 next_event 쪽은 영구 pending.
        // 이렇게 해야 turn 진행 중에도 ApprovalDecision/RevertLastTurn 명령이 막히지 않는다.
        tokio::select! {
            biased;
            cmd_opt = cmd_rx.recv() => {
                let Some(cmd) = cmd_opt else { break; };
                match cmd {
                    UiCommand::StartProject { path } => {
                        if let Some(prev) = session.take() {
                            prev.shutdown().await;
                        }
                        // git repo 자동 init (이미 있으면 no-op).
                        if let Err(e) = stcode_vibe::ensure_repo(&path) {
                            tracing::warn!("git 초기화 실패: {e}");
                            // 치명적이지 않음 — codex는 그래도 돌아간다.
                        }
                        match start_session(&path).await {
                            Ok(s) => {
                                let _ = evt_tx.send(UiEvent::Started {
                                    thread_id: s.thread_id.clone(),
                                });
                                session = Some(s);
                                project = Some(ProjectState {
                                    path,
                                    pending_prompt: None,
                                    pending_snapshot: None,
                                    last_revert_to: None,
                                });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(UiEvent::Error(format!("세션 시작 실패: {e}")));
                                project = None;
                            }
                        }
                    }
                    UiCommand::SendText(text) => {
                        let Some(s) = session.as_mut() else {
                            let _ = evt_tx.send(UiEvent::Error("세션이 시작되지 않았어요".into()));
                            continue;
                        };
                        // turn 시작 직전 HEAD를 snapshot. 이게 RevertLastTurn 의 타깃.
                        if let Some(p) = project.as_mut() {
                            let snapshot = stcode_vibe::current_head(&p.path)
                                .map_err(|e| tracing::warn!("HEAD snapshot 실패: {e}"))
                                .ok()
                                .flatten();
                            p.pending_snapshot = Some(snapshot);
                            p.pending_prompt = Some(text.clone());
                        }
                        if let Err(e) = s.send_user_text(text).await {
                            tracing::warn!("turn 시작 실패: {e}");
                            let _ = evt_tx.send(UiEvent::Error(format!("turn 시작 실패: {e}")));
                        }
                    }
                    UiCommand::ApprovalDecision { request_id, decision } => {
                        let Some(s) = session.as_ref() else { continue; };
                        let payload = serde_json::json!({ "decision": decision.as_wire() });
                        if let Err(e) = s.respond_request(request_id, &payload).await {
                            tracing::warn!("승인 응답 실패: {e}");
                            let _ = evt_tx.send(UiEvent::Error(format!("승인 응답 실패: {e}")));
                        }
                    }
                    UiCommand::RevertLastTurn => {
                        let Some(p) = project.as_mut() else {
                            let _ = evt_tx.send(UiEvent::Reverted {
                                ok: false,
                                error_text: Some("세션이 없어요".into()),
                            });
                            continue;
                        };
                        let target = p.last_revert_to.clone();
                        match stcode_vibe::revert_to(&p.path, target.as_deref()) {
                            Ok(()) => {
                                p.last_revert_to = None;
                                let _ = evt_tx.send(UiEvent::Reverted { ok: true, error_text: None });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(UiEvent::Reverted {
                                    ok: false,
                                    error_text: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    UiCommand::Shutdown => break,
                }
            }
            ev_opt = next_event_or_pending(&mut session) => {
                match ev_opt {
                    Some(ev) => {
                        handle_thread_event(ev, &evt_tx, session.as_ref(), project.as_mut()).await;
                    }
                    None => {
                        // session 끝남 (codex 종료)
                        let _ = evt_tx.send(UiEvent::Error("codex 연결 끊김".into()));
                        session = None;
                        project = None;
                    }
                }
            }
        }
    }

    if let Some(s) = session {
        s.shutdown().await;
    }
}

/// 세션이 None이면 영구 pending — select에서 다른 갈래만 polling 되도록.
async fn next_event_or_pending(session: &mut Option<ThreadSession>) -> Option<ThreadEvent> {
    match session.as_mut() {
        Some(s) => s.next_event().await,
        None => std::future::pending().await,
    }
}

/// 자동 모드: 모든 inbound 승인 요청은 즉시 Accept로 응답. UiEvent로 끌어올리지 않음.
/// 자동 commit은 session/project 둘 다 살아있을 때만.
async fn handle_thread_event(
    ev: ThreadEvent,
    evt_tx: &mpsc::UnboundedSender<UiEvent>,
    session: Option<&ThreadSession>,
    project: Option<&mut ProjectState>,
) {
    match ev {
        ThreadEvent::AgentMessageDelta(d) => {
            let _ = evt_tx.send(UiEvent::AgentDelta(d.delta));
        }
        ThreadEvent::ReasoningDelta(d) => {
            let _ = evt_tx.send(UiEvent::ReasoningDelta(d.delta));
        }
        ThreadEvent::CommandOutputDelta(d) => {
            let _ = evt_tx.send(UiEvent::ToolOutput {
                item_id: d.item_id,
                delta: d.delta,
            });
        }
        ThreadEvent::ItemStarted {
            item_id,
            item_type,
            params,
        } => {
            let kind = ToolKind::from_item_type(&item_type);
            if matches!(kind, ToolKind::Other) {
                // userMessage / agentMessage / reasoning 등은 무시
                return;
            }
            let title = item_card_title(&kind, &params);
            let _ = evt_tx.send(UiEvent::ToolStarted {
                item_id,
                kind,
                title,
            });
        }
        ThreadEvent::ItemCompleted {
            item_id,
            item_type,
            params,
        } => {
            let kind = ToolKind::from_item_type(&item_type);
            if matches!(kind, ToolKind::Other) {
                return;
            }
            let (ok, summary) = item_card_completion(&kind, &params);
            let _ = evt_tx.send(UiEvent::ToolCompleted {
                item_id,
                ok,
                summary,
            });
        }
        ThreadEvent::TurnCompleted { turn } => {
            let ok = matches!(turn.status, Some(TurnStatus::Completed));
            let error_text = turn.error.map(|e| e.to_string());
            if !ok {
                tracing::warn!("turn 실패: {:?}", error_text);
            }
            // turn 종료 후 변경된 게 있으면 자동 commit. 실패해도 turn 자체는 끝났으니 흐름 유지.
            if ok {
                if let Some(p) = project {
                    let prompt = p.pending_prompt.take().unwrap_or_default();
                    let snapshot = p.pending_snapshot.take().flatten();
                    match stcode_vibe::auto_commit_turn(&p.path, &prompt, snapshot.as_deref()) {
                        Ok(Some(c)) => {
                            p.last_revert_to = c.revert_to.clone();
                            let _ = evt_tx.send(UiEvent::TurnCommitted {
                                commit_oid: c.commit_oid,
                                summary: c.summary,
                                revert_to: c.revert_to,
                            });
                        }
                        Ok(None) => {
                            // 변경 없음 — 조용히 skip.
                        }
                        Err(e) => {
                            tracing::warn!("자동 commit 실패: {e}");
                            let _ = evt_tx.send(UiEvent::Error(format!(
                                "자동 저장 실패: {e}"
                            )));
                        }
                    }
                }
            }
            let _ = evt_tx.send(UiEvent::TurnDone { ok, error_text });
        }
        ThreadEvent::InboundRequest { id, method, params: _ } => {
            // 자동 모드: 모든 승인 요청 자동 Accept. UiEvent::ApprovalRequested 로
            // 끌어올리지 않음 — 모달은 더 이상 안 뜬다.
            if method == method::REQ_COMMAND_APPROVAL || method == method::REQ_FILE_CHANGE_APPROVAL
            {
                let payload = serde_json::json!({
                    "decision": ApprovalDecision::AcceptForSession.as_wire(),
                });
                if let Some(s) = session {
                    if let Err(e) = s.respond_request(id, &payload).await {
                        tracing::warn!("자동 승인 실패: {e}");
                    }
                }
            } else {
                tracing::warn!("미처리 inbound request: {method}");
            }
        }
        _ => {}
    }
}

/// 승인 요청 params에서 사용자에게 보여줄 친화적 제목 + raw 디테일을 뽑는다.
/// codex `CommandExecutionRequestApprovalParams` / `FileChangeRequestApprovalParams`는
/// 평탄한 구조 (item wrapper 없음). v1 자동 모드에선 사용 안 함 — 미래 "민감한 작업만
/// 묻기" 옵션 도입 시 재사용.
#[allow(dead_code)]
fn approval_text(kind: &ToolKind, params: &serde_json::Value) -> (String, String) {
    let reason = params.get("reason").and_then(|v| v.as_str());
    match kind {
        ToolKind::CommandExecution => {
            let cmd = params
                .get("command")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| "(알 수 없는 명령)".into());
            let title = if let Some(r) = reason {
                format!("터미널 명령을 실행해도 될까요? ({r})")
            } else {
                "터미널 명령을 실행해도 될까요?".into()
            };
            (title, cmd)
        }
        ToolKind::FileChange => {
            // codex FileChangeRequestApprovalParams엔 path가 안 담김 — item_id로 별도 Tool Card 연결.
            // 디테일 줄은 reason 또는 빈 문자열.
            let detail = reason.map(String::from).unwrap_or_default();
            ("파일 변경을 적용해도 될까요?".into(), detail)
        }
        _ => ("이 작업을 진행해도 될까요?".into(), String::new()),
    }
}

fn item_card_title(kind: &ToolKind, params: &serde_json::Value) -> String {
    let item = params.get("item");
    match kind {
        ToolKind::CommandExecution => item
            .and_then(|i| i.get("command"))
            .and_then(|v| v.as_str())
            .unwrap_or("(command)")
            .to_string(),
        ToolKind::FileChange => item
            .and_then(|i| i.get("path"))
            .or_else(|| item.and_then(|i| i.get("filePath")))
            .and_then(|v| v.as_str())
            .unwrap_or("(file)")
            .to_string(),
        ToolKind::McpToolCall => item
            .and_then(|i| i.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("(tool)")
            .to_string(),
        ToolKind::WebSearch => item
            .and_then(|i| i.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("(search)")
            .to_string(),
        ToolKind::Other => "—".into(),
    }
}

fn item_card_completion(kind: &ToolKind, params: &serde_json::Value) -> (bool, Option<String>) {
    let item = match params.get("item") {
        Some(it) => it,
        None => return (true, None),
    };
    match kind {
        ToolKind::CommandExecution => {
            let exit = item
                .get("exitCode")
                .or_else(|| item.get("exit_code"))
                .and_then(|v| v.as_i64());
            let ok = exit.map(|c| c == 0).unwrap_or(true);
            // 바이브 코더용: raw 출력 대신 결과 한 줄.
            let summary = if ok {
                Some("완료".into())
            } else {
                Some(format!("실패 (코드 {})", exit.unwrap_or(-1)))
            };
            (ok, summary)
        }
        ToolKind::FileChange => {
            // diff/raw summary 노출 X — "수정됨" 정도만.
            (true, Some("적용됨".into()))
        }
        _ => (true, None),
    }
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
            // 자동 에이전트: 권한 묻지 않음. 안전망은 stcode-vibe git auto-commit + 되돌리기.
            approval_policy: Some(ApprovalPolicy::Never),
            // sandbox 완전 풀기 — 자동 모드 의도. 위험성은 git 되돌리기로 회수.
            sandbox: Some(SandboxMode::DangerFullAccess),
            ..Default::default()
        },
        opts,
    )
    .await
}
