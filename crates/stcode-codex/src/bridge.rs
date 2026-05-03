//! GPUI ↔ tokio(codex) 브리지 — 멀티 세션 버전.
//!
//! 사용자 명시한 핵심 워크플로우는 "병렬 멀티 에이전트 바이브코딩". 즉 동시에 여러
//! 세션을 띄워 백그라운드 진행도 가능해야 한다.
//!
//! 구조:
//! - 각 세션은 자체 tokio task. `tokio::select! { session_cmd_rx, session.next_event() }`
//!   로 자기 명령과 codex 이벤트를 동시 polling.
//! - handler_loop은 외부 UiCommand 와 unified `(SessionId, ThreadEvent)` 채널만 처리.
//! - inbound approval request 는 자동 Accept (자동 모드 정책). UiEvent로 끌어올리지 않음.
//! - git auto-commit / revert 는 handler_loop 측에서 ProjectState 들고 처리.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{
    ApprovalPolicy, ClientInfo, SandboxMode, SpawnOptions, ThreadEvent, ThreadSession,
    ThreadStartParams, TurnStatus, method,
};
use stcode_vibe::SessionWorktree;
use tokio::sync::mpsc;

/// 클라이언트가 부여하는 세션 식별자. UI가 자체 counter로 발급.
pub type SessionId = String;

#[derive(Debug)]
pub enum UiCommand {
    /// 새 세션 추가. id는 클라이언트가 발급해서 같이 보냄 — 응답 매칭 단순.
    /// provider/model은 사용자 설정에서 — 새 세션마다 적용.
    NewSession {
        session_id: SessionId,
        path: PathBuf,
        provider: String,
        model: String,
    },
    /// 특정 세션에 사용자 텍스트 전달.
    SendText { session_id: SessionId, text: String },
    /// (희소) 승인 요청 응답 — 자동 모드에선 거의 호출 안 됨.
    ApprovalDecision {
        session_id: SessionId,
        request_id: i64,
        decision: ApprovalDecision,
    },
    /// 마지막 turn 변경을 hard reset으로 되돌린다.
    RevertLastTurn { session_id: SessionId },
    /// 진행 중인 turn을 중단한다.
    InterruptTurn { session_id: SessionId },
    /// 특정 세션 종료.
    CloseSession { session_id: SessionId },
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
    /// 세션 시작 성공.
    SessionStarted {
        session_id: SessionId,
        project: PathBuf,
        thread_id: String,
    },
    /// 세션 시작 실패.
    SessionFailed {
        session_id: SessionId,
        error: String,
    },
    /// 세션 종료(사용자 닫음 또는 codex 끊김).
    SessionClosed { session_id: SessionId },
    /// agentMessage delta.
    AgentDelta { session_id: SessionId, text: String },
    /// reasoning text delta.
    ReasoningDelta { session_id: SessionId, text: String },
    /// 도구 카드 시작.
    ToolStarted {
        session_id: SessionId,
        item_id: String,
        kind: ToolKind,
        title: String,
    },
    /// 도구 출력 incremental.
    ToolOutput {
        session_id: SessionId,
        item_id: String,
        delta: String,
    },
    /// 도구 종료.
    ToolCompleted {
        session_id: SessionId,
        item_id: String,
        ok: bool,
        summary: Option<String>,
    },
    /// (희소) 승인 요청. 자동 모드에선 발생 안 함 — 인프라만 남김.
    #[allow(dead_code)]
    ApprovalRequested {
        session_id: SessionId,
        request_id: i64,
        kind: ToolKind,
        friendly_title: String,
        raw_detail: String,
    },
    /// turn 종료.
    TurnDone {
        session_id: SessionId,
        ok: bool,
        error_text: Option<String>,
    },
    /// turn이 working tree에 변경을 만들어 자동 커밋됨.
    TurnCommitted {
        session_id: SessionId,
        commit_oid: String,
        summary: String,
        revert_to: Option<String>,
    },
    /// 되돌리기 결과.
    Reverted {
        session_id: SessionId,
        ok: bool,
        error_text: Option<String>,
    },
    /// 세션과 묶지 못하는 글로벌 에러.
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
            let rt = match tokio::runtime::Builder::new_multi_thread()
                // 멀티 세션 — 각 세션이 자체 task. multi_thread로 진짜 병렬 polling.
                .worker_threads(2)
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

// ─── 세션별 상태 ──────────────────────────────────────────

/// handler_loop가 세션별로 들고 있는 정보.
struct ManagedSession {
    /// 실제 codex가 작업하는 경로. git repo면 세션용 worktree, 빈/non-git 폴더면 원본 path.
    path: PathBuf,
    /// 세션 전용 worktree. None이면 원본 path에서 직접 작업.
    worktree: Option<SessionWorktree>,
    /// 가장 최근 SendText 시점의 prompt — turn 종료 후 commit 메시지로 사용.
    pending_prompt: Option<String>,
    /// turn 시작 직전 HEAD oid (revert 대상).
    pending_snapshot: Option<Option<String>>,
    /// 가장 최근 commit의 revert_to oid — RevertLastTurn 의 타깃.
    last_revert_to: Option<String>,
    /// session_task로 명령 보내는 채널. send_user_text/respond/shutdown.
    cmd_tx: mpsc::UnboundedSender<SessionInternalCmd>,
}

#[derive(Debug, PartialEq, Eq)]
enum SessionInternalCmd {
    SendText(String),
    RespondApproval(i64, ApprovalDecision),
    InterruptTurn,
    Shutdown { interrupt_turn: bool },
}

enum SessionTaskEvent {
    Thread(ThreadEvent),
    Exited,
}

fn request_session_shutdown(s: &ManagedSession, interrupt_turn: bool) {
    let _ = s
        .cmd_tx
        .send(SessionInternalCmd::Shutdown { interrupt_turn });
}

// ─── handler_loop ────────────────────────────────────────

async fn handler_loop(
    mut cmd_rx: mpsc::UnboundedReceiver<UiCommand>,
    evt_tx: mpsc::UnboundedSender<UiEvent>,
) {
    let mut sessions: HashMap<SessionId, ManagedSession> = HashMap::new();
    // 모든 세션 task가 이리로 (sid, event) 를 던진다.
    let (unified_evt_tx, mut unified_evt_rx) =
        mpsc::unbounded_channel::<(SessionId, SessionTaskEvent)>();

    loop {
        tokio::select! {
            biased;
            cmd_opt = cmd_rx.recv() => {
                let Some(cmd) = cmd_opt else { break; };
                match cmd {
                    UiCommand::NewSession { session_id, path, provider, model } => {
                        let prepared = match prepare_session_workspace(&session_id, &path) {
                            Ok(prepared) => prepared,
                            Err(e) => {
                                let _ = evt_tx.send(UiEvent::SessionFailed {
                                    session_id,
                                    error: format!("작업공간 준비 실패: {e}"),
                                });
                                continue;
                            }
                        };
                        match start_session(&prepared.runtime_path, &provider, &model).await {
                            Ok(thread_session) => {
                                let thread_id = thread_session.thread_id.clone();
                                let (s_cmd_tx, s_cmd_rx) = mpsc::unbounded_channel();
                                let session_id_for_task = session_id.clone();
                                let unified_tx = unified_evt_tx.clone();
                                tokio::spawn(async move {
                                    session_task(
                                        thread_session,
                                        s_cmd_rx,
                                        unified_tx,
                                        session_id_for_task,
                                    )
                                    .await;
                                });
                                sessions.insert(
                                    session_id.clone(),
                                    ManagedSession {
                                        path: prepared.runtime_path,
                                        worktree: prepared.worktree,
                                        pending_prompt: None,
                                        pending_snapshot: None,
                                        last_revert_to: None,
                                        cmd_tx: s_cmd_tx,
                                    },
                                );
                                let _ = evt_tx.send(UiEvent::SessionStarted {
                                    session_id,
                                    project: path,
                                    thread_id,
                                });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(UiEvent::SessionFailed {
                                    session_id,
                                    error: e.to_string(),
                                });
                            }
                        }
                    }
                    UiCommand::SendText { session_id, text } => {
                        let Some(s) = sessions.get_mut(&session_id) else {
                            let _ = evt_tx.send(UiEvent::Error(format!(
                                "알 수 없는 세션: {session_id}"
                            )));
                            continue;
                        };
                        // turn 시작 직전 HEAD snapshot.
                        let snapshot = stcode_vibe::current_head(&s.path)
                            .map_err(|e| tracing::warn!("[{session_id}] HEAD snapshot 실패: {e}"))
                            .ok()
                            .flatten();
                        s.pending_snapshot = Some(snapshot);
                        s.pending_prompt = Some(text.clone());
                        let _ = s.cmd_tx.send(SessionInternalCmd::SendText(text));
                    }
                    UiCommand::ApprovalDecision {
                        session_id,
                        request_id,
                        decision,
                    } => {
                        if let Some(s) = sessions.get(&session_id) {
                            let _ = s
                                .cmd_tx
                                .send(SessionInternalCmd::RespondApproval(request_id, decision));
                        }
                    }
                    UiCommand::RevertLastTurn { session_id } => {
                        let Some(s) = sessions.get_mut(&session_id) else {
                            let _ = evt_tx.send(UiEvent::Reverted {
                                session_id,
                                ok: false,
                                error_text: Some("알 수 없는 세션".into()),
                            });
                            continue;
                        };
                        let target = s.last_revert_to.clone();
                        match stcode_vibe::revert_to(&s.path, target.as_deref()) {
                            Ok(()) => {
                                s.last_revert_to = None;
                                let _ = evt_tx.send(UiEvent::Reverted {
                                    session_id,
                                    ok: true,
                                    error_text: None,
                                });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(UiEvent::Reverted {
                                    session_id,
                                    ok: false,
                                    error_text: Some(e.to_string()),
                                });
                            }
                        }
                    }
                    UiCommand::InterruptTurn { session_id } => {
                        if let Some(s) = sessions.get(&session_id) {
                            let _ = s.cmd_tx.send(SessionInternalCmd::InterruptTurn);
                        } else {
                            let _ = evt_tx.send(UiEvent::Error(format!(
                                "알 수 없는 세션: {session_id}"
                            )));
                        }
                    }
                    UiCommand::CloseSession { session_id } => {
                        if let Some(s) = sessions.get(&session_id) {
                            request_session_shutdown(&s, true);
                        }
                    }
                    UiCommand::Shutdown => break,
                }
            }
            event = unified_evt_rx.recv() => {
                let Some((sid, ev)) = event else { continue; };
                match ev {
                    SessionTaskEvent::Thread(ev) => {
                        handle_thread_event(sid, ev, &evt_tx, &mut sessions).await;
                    }
                    SessionTaskEvent::Exited => {
                        if let Some(s) = sessions.remove(&sid) {
                            cleanup_session_workspace(&sid, &s, &evt_tx);
                        }
                        let _ = evt_tx.send(UiEvent::SessionClosed { session_id: sid });
                    }
                }
            }
        }
    }

    // 정리: 모든 세션에 Shutdown 신호.
    for (_, s) in sessions.drain() {
        request_session_shutdown(&s, true);
    }
}

// ─── 세션 task ───────────────────────────────────────────

/// 세션 1개 owned. 자기 명령 + codex 이벤트를 select 로 동시 polling.
/// inbound approval request는 자체 자동 Accept (자동 모드 정책).
async fn session_task(
    mut session: ThreadSession,
    mut cmd_rx: mpsc::UnboundedReceiver<SessionInternalCmd>,
    evt_tx: mpsc::UnboundedSender<(SessionId, SessionTaskEvent)>,
    session_id: SessionId,
) {
    loop {
        tokio::select! {
            biased;
            cmd_opt = cmd_rx.recv() => {
                let Some(cmd) = cmd_opt else { break; };
                match cmd {
                    SessionInternalCmd::SendText(text) => {
                        if let Err(e) = session.send_user_text(text).await {
                            tracing::warn!("[{session_id}] turn 시작 실패: {e}");
                        }
                    }
                    SessionInternalCmd::RespondApproval(id, decision) => {
                        let payload = serde_json::json!({ "decision": decision.as_wire() });
                        if let Err(e) = session.respond_request(id, &payload).await {
                            tracing::warn!("[{session_id}] approval 응답 실패: {e}");
                        }
                    }
                    SessionInternalCmd::InterruptTurn => {
                        if let Err(e) = session.interrupt().await {
                            tracing::warn!("[{session_id}] turn 중단 실패: {e}");
                        }
                    }
                    SessionInternalCmd::Shutdown { interrupt_turn } => {
                        if interrupt_turn {
                            if let Err(e) = session.interrupt().await {
                                tracing::debug!("[{session_id}] shutdown 전 turn 중단 실패: {e}");
                            }
                        }
                        break;
                    }
                }
            }
            ev_opt = session.next_event() => {
                let Some(ev) = ev_opt else { break; };
                // 자동 모드: 승인 inbound는 자체 처리, UI로 끌어올리지 않음.
                if let ThreadEvent::InboundRequest { id, ref method, .. } = ev {
                    if method == method::REQ_COMMAND_APPROVAL
                        || method == method::REQ_FILE_CHANGE_APPROVAL
                    {
                        let payload = serde_json::json!({
                            "decision": ApprovalDecision::AcceptForSession.as_wire(),
                        });
                        if let Err(e) = session.respond_request(id, &payload).await {
                            tracing::warn!("[{session_id}] 자동 승인 실패: {e}");
                        }
                        continue;
                    } else {
                        tracing::warn!(
                            "[{session_id}] 미처리 inbound request: {method}"
                        );
                        continue;
                    }
                }
                if evt_tx
                    .send((session_id.clone(), SessionTaskEvent::Thread(ev)))
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    session.shutdown().await;
    let _ = evt_tx.send((session_id, SessionTaskEvent::Exited));
}

// ─── handle_thread_event ────────────────────────────────

/// (SessionId, ThreadEvent) → UiEvent 변환 + git 후처리.
async fn handle_thread_event(
    sid: SessionId,
    ev: ThreadEvent,
    evt_tx: &mpsc::UnboundedSender<UiEvent>,
    sessions: &mut HashMap<SessionId, ManagedSession>,
) {
    match ev {
        ThreadEvent::AgentMessageDelta(d) => {
            let _ = evt_tx.send(UiEvent::AgentDelta {
                session_id: sid,
                text: d.delta,
            });
        }
        ThreadEvent::ReasoningDelta(d) => {
            let _ = evt_tx.send(UiEvent::ReasoningDelta {
                session_id: sid,
                text: d.delta,
            });
        }
        ThreadEvent::CommandOutputDelta(d) => {
            let _ = evt_tx.send(UiEvent::ToolOutput {
                session_id: sid,
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
                return;
            }
            let title = item_card_title(&kind, &params);
            let _ = evt_tx.send(UiEvent::ToolStarted {
                session_id: sid,
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
                session_id: sid,
                item_id,
                ok,
                summary,
            });
        }
        ThreadEvent::TurnCompleted { turn } => {
            let ok = matches!(turn.status, Some(TurnStatus::Completed));
            let error_text = turn.error.map(|e| e.to_string());
            if !ok {
                tracing::warn!("[{sid}] turn 실패: {:?}", error_text);
            }
            // git auto-commit
            if ok {
                if let Some(s) = sessions.get_mut(&sid) {
                    let prompt = s.pending_prompt.take().unwrap_or_default();
                    let snapshot = s.pending_snapshot.take().flatten();
                    match stcode_vibe::auto_commit_turn(&s.path, &prompt, snapshot.as_deref()) {
                        Ok(Some(c)) => {
                            s.last_revert_to = c.revert_to.clone();
                            let _ = evt_tx.send(UiEvent::TurnCommitted {
                                session_id: sid.clone(),
                                commit_oid: c.commit_oid,
                                summary: c.summary,
                                revert_to: c.revert_to,
                            });
                        }
                        Ok(None) => {
                            // 변경 없음 — skip
                        }
                        Err(e) => {
                            tracing::warn!("[{sid}] 자동 commit 실패: {e}");
                            let _ = evt_tx.send(UiEvent::Error(format!("자동 저장 실패: {e}")));
                        }
                    }
                }
            }
            let _ = evt_tx.send(UiEvent::TurnDone {
                session_id: sid,
                ok,
                error_text,
            });
        }
        // InboundRequest는 session_task에서 처리되어 여기 안 옴.
        _ => {}
    }
}

// ─── 헬퍼 ────────────────────────────────────────────────

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
            let summary = if ok {
                Some("완료".into())
            } else {
                Some(format!("실패 (코드 {})", exit.unwrap_or(-1)))
            };
            (ok, summary)
        }
        ToolKind::FileChange => (true, Some("적용됨".into())),
        _ => (true, None),
    }
}

struct PreparedSessionWorkspace {
    runtime_path: PathBuf,
    worktree: Option<SessionWorktree>,
}

fn prepare_session_workspace(
    session_id: &SessionId,
    source_path: &PathBuf,
) -> anyhow::Result<PreparedSessionWorkspace> {
    let worktrees_root = stcode_vibe::default_worktrees_root()
        .ok_or_else(|| anyhow::anyhow!("worktree 저장 위치를 찾을 수 없어요"))?;
    prepare_session_workspace_with_root(session_id, source_path, &worktrees_root)
}

fn prepare_session_workspace_with_root(
    session_id: &SessionId,
    source_path: &PathBuf,
    worktrees_root: &Path,
) -> anyhow::Result<PreparedSessionWorkspace> {
    stcode_vibe::ensure_repo(source_path)?;
    if stcode_vibe::current_head(source_path)?.is_none() {
        return Ok(PreparedSessionWorkspace {
            runtime_path: source_path.clone(),
            worktree: None,
        });
    }

    let worktree = stcode_vibe::prepare_session_worktree(source_path, session_id, worktrees_root)?;
    tracing::info!(
        "[{session_id}] 세션 작업공간 준비: {}",
        worktree.worktree_path.display()
    );
    Ok(PreparedSessionWorkspace {
        runtime_path: worktree.worktree_path.clone(),
        worktree: Some(worktree),
    })
}

fn cleanup_session_workspace(
    session_id: &SessionId,
    session: &ManagedSession,
    evt_tx: &mpsc::UnboundedSender<UiEvent>,
) {
    let Some(worktree) = &session.worktree else {
        return;
    };
    match stcode_vibe::cleanup_session_worktree(worktree) {
        Ok(cleanup) => {
            tracing::info!(
                "[{session_id}] 세션 작업공간 정리: removed_worktree={} deleted_branch={} kept={:?}",
                cleanup.removed_worktree,
                cleanup.deleted_branch,
                cleanup.kept_branch_reason
            );
        }
        Err(e) => {
            tracing::warn!("[{session_id}] 세션 작업공간 정리 실패: {e}");
            let _ = evt_tx.send(UiEvent::Error(format!("작업공간 정리 실패: {e}")));
        }
    }
}

async fn start_session(
    path: &PathBuf,
    provider: &str,
    model: &str,
) -> anyhow::Result<ThreadSession> {
    let mut opts = SpawnOptions::with_provider_model(provider, model);
    opts = opts
        .with_env("STCODE_VLLM_COMPAT", "1")
        .push("model_reasoning_effort", "minimal");
    // vLLM은 ws 미지원 — provider가 vLLM 계열일 때만 ws 끄기.
    if provider.contains("vllm") {
        opts = opts.push(
            format!("model_providers.{}.supports_websockets", provider),
            "false",
        );
    }
    if std::env::var_os("VLLM_API_KEY").is_none() {
        opts = opts.with_env("VLLM_API_KEY", "dummy");
    }
    ThreadSession::start_with(
        ClientInfo::default(),
        ThreadStartParams {
            cwd: Some(path.to_string_lossy().into_owned()),
            // 자동 에이전트: 권한 묻지 않음. 안전망은 stcode-vibe git auto-commit + 되돌리기.
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox: Some(SandboxMode::DangerFullAccess),
            ..Default::default()
        },
        opts,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command spawn");
        assert!(
            output.status.success(),
            "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
            repo.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(path: &Path) {
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.name", "Stcode Test"]);
        git(path, &["config", "user.email", "stcode-test@example.local"]);
        fs::write(path.join("README.md"), "# test\n").expect("write readme");
        git(path, &["add", "README.md"]);
        git(path, &["commit", "-m", "initial"]);
    }

    fn managed_session_for_test(
        cmd_tx: mpsc::UnboundedSender<SessionInternalCmd>,
    ) -> ManagedSession {
        ManagedSession {
            path: PathBuf::from("/tmp/stcode-test"),
            worktree: None,
            pending_prompt: None,
            pending_snapshot: None,
            last_revert_to: None,
            cmd_tx,
        }
    }

    #[test]
    fn prepare_session_workspace_uses_worktree_for_committed_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).expect("repo dir");
        init_repo(&repo);
        let worktrees_root = tmp.path().join("worktrees");

        let prepared = prepare_session_workspace_with_root(&"s1".into(), &repo, &worktrees_root)
            .expect("prepare workspace");

        let worktree = prepared.worktree.expect("session worktree");
        assert_eq!(prepared.runtime_path, worktree.worktree_path);
        assert!(prepared.runtime_path.exists());
        assert_ne!(
            prepared.runtime_path,
            repo.canonicalize().expect("canonical repo")
        );
        assert_eq!(worktree.branch, "stcode/s1");

        stcode_vibe::cleanup_session_worktree(&worktree).expect("cleanup worktree");
    }

    #[test]
    fn prepare_session_workspace_keeps_empty_repo_on_source_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("empty");
        fs::create_dir(&source).expect("source dir");
        let worktrees_root = tmp.path().join("worktrees");

        let prepared =
            prepare_session_workspace_with_root(&"s-empty".into(), &source, &worktrees_root)
                .expect("prepare workspace");

        assert_eq!(prepared.runtime_path, source);
        assert!(prepared.worktree.is_none());
        assert!(prepared.runtime_path.join(".git").exists());
    }

    #[test]
    fn shutdown_request_can_ask_session_task_to_interrupt_turn_first() {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let session = managed_session_for_test(cmd_tx);

        request_session_shutdown(&session, true);

        match cmd_rx.try_recv() {
            Ok(SessionInternalCmd::Shutdown { interrupt_turn }) => assert!(interrupt_turn),
            other => panic!("expected interrupting shutdown, got {other:?}"),
        }
    }

    #[test]
    fn shutdown_request_can_skip_interrupt_when_requested() {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
        let session = managed_session_for_test(cmd_tx);

        request_session_shutdown(&session, false);

        match cmd_rx.try_recv() {
            Ok(SessionInternalCmd::Shutdown { interrupt_turn }) => assert!(!interrupt_turn),
            other => panic!("expected plain shutdown, got {other:?}"),
        }
    }
}
