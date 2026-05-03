use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{
    App, Bounds, Context, Entity, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    ScrollHandle, SharedString, Styled, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, rgba, size,
};
use gpui_platform::application;

mod chat_input;
mod selectable_text;
mod theme;

use chat_input::ChatInput;
use selectable_text::{InlineKind, InlineSpan, SelectableText};
use stcode_codex::bridge::{
    ApprovalDecision, Bridge, SessionId, ToolKind, UiCommand, UiEvent, WorkspaceMode,
};
use stcode_vibe::{AgentModelRole, Settings, friendly_translate, settings};

// ─── 화면 / 상태 ──────────────────────────────────────────

enum Screen {
    Welcome,
    Workspace(WorkspaceState),
}

/// 워크스페이스 — 사이드바 + 활성 세션. v1 핵심 워크플로우 "병렬 멀티 에이전트"를
/// 받아 안기 위한 구조.
struct WorkspaceState {
    sessions: HashMap<SessionId, SessionUiState>,
    /// 사이드바 표시 순서 — 세션 추가된 순.
    order: Vec<SessionId>,
    /// 현재 사이드바에서 선택된 세션. None은 모든 세션이 닫힌 상태.
    active: Option<SessionId>,
    /// 다음 세션 id 발급용 카운터.
    next_id: u32,
}

struct SessionUiState {
    project: PathBuf,
    messages: Vec<ChatItem>,
    thread_started: bool,
    turn_in_flight: bool,
    interrupt_requested: bool,
    turn_reasoning_chars: usize,
    turn_answer_chars: usize,
    input: Entity<ChatInput>,
    last_commit: Option<LastCommit>,
    /// active 가 아닌 세션에서 새 message/델타가 와서 unread 표식.
    has_unread: bool,
    /// 메시지 영역 별 ScrollHandle — 세션마다 따로 스크롤 위치 유지.
    scroll: ScrollHandle,
}

#[derive(Default)]
struct SessionSummary {
    user_turns: usize,
    agent_messages: usize,
    tools_running: usize,
    tools_ok: usize,
    tools_failed: usize,
}

impl SessionUiState {
    fn new(project: PathBuf, cx: &mut Context<MainView>) -> Self {
        let intro = ChatItem::message(Speaker::System, "세션을 여는 중…", cx);
        let input = cx.new(|cx| {
            ChatInput::new(
                "후속 변경 사항을 부탁하세요",
                theme::TOKENS.fg,
                theme::TOKENS.muted,
                cx,
            )
        });
        Self {
            project,
            messages: vec![intro],
            thread_started: false,
            turn_in_flight: false,
            interrupt_requested: false,
            turn_reasoning_chars: 0,
            turn_answer_chars: 0,
            input,
            last_commit: None,
            has_unread: false,
            scroll: ScrollHandle::new(),
        }
    }
}

#[derive(Clone)]
struct LastCommit {
    /// commit 메시지 첫 줄 (사용자에게 보여줌).
    summary: String,
    /// 되돌릴 수 있는지(첫 commit이 아닌지).
    revertible: bool,
}

/// 채팅 영역의 한 항목. Message(사용자/agent/system) / Tool 카드 두 종류.
enum ChatItem {
    Message {
        who: Speaker,
        /// Streaming 중인 raw 텍스트 — turn 끝나기 전엔 이걸 그대로 렌더.
        text: Entity<SelectableText>,
        /// Agent 메시지의 reasoning(별도 회색 영역). None이면 표시 안 함.
        reasoning: Option<Entity<SelectableText>>,
        /// turn이 끝나면 markdown 파싱해서 채움. Some이면 segments를 렌더 — text 무시.
        segments: Option<Vec<MessageSegment>>,
    },
    Tool {
        item_id: String,
        kind: ToolKind,
        title: String,
        output: Entity<SelectableText>,
        status: ToolStatus,
    },
}

/// Markdown 파싱된 한 조각. block-level + 일부 inline(code/bold/link).
enum MessageSegment {
    /// 일반 텍스트 paragraph (줄바꿈 포함).
    Paragraph(Entity<SelectableText>),
    /// `# heading` `## heading` `### heading`. level=1..3.
    Heading {
        level: u8,
        body: Entity<SelectableText>,
    },
    /// `- item` 또는 `* item`. body는 bullet 제외한 본문.
    ListItem { body: Entity<SelectableText> },
    /// fenced code block. ```language\n...\n``` 의 안쪽 내용. mono font + 다른 bg.
    Code {
        body: Entity<SelectableText>,
        language: Option<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolStatus {
    InProgress,
    CompletedOk,
    CompletedFail,
}

impl ChatItem {
    fn message(who: Speaker, text: impl Into<SharedString>, cx: &mut Context<MainView>) -> Self {
        let s = text.into();
        let color = color_for(who);
        let entity = cx.new(|cx| SelectableText::new(s, color, cx));
        Self::Message {
            who,
            text: entity,
            reasoning: None,
            segments: None,
        }
    }

    fn tool(item_id: String, kind: ToolKind, title: String, cx: &mut Context<MainView>) -> Self {
        let output = cx.new(|cx| SelectableText::new("", theme::TOKENS.muted, cx));
        Self::Tool {
            item_id,
            kind,
            title,
            output,
            status: ToolStatus::InProgress,
        }
    }
}

fn color_for(who: Speaker) -> u32 {
    match who {
        Speaker::User | Speaker::Agent => theme::TOKENS.fg,
        Speaker::System => theme::TOKENS.muted,
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Speaker {
    User,
    Agent,
    System,
}

/// 진행 중인 승인 요청. v1 자동 모드에선 거의 안 뜨지만 인프라는 남김.
struct PendingApproval {
    session_id: SessionId,
    request_id: i64,
    kind: ToolKind,
    friendly_title: String,
    raw_detail: String,
}

/// 설정 모달이 떠 있을 때의 임시 입력 상태.
struct SettingsDraft {
    provider: Entity<ChatInput>,
    main_model: Entity<ChatInput>,
    sub_model: Entity<ChatInput>,
    /// 저장 후 잠깐 보여줄 안내 (Some(text), 자동 사라짐 없음 — 닫을 때 None).
    notice: Option<String>,
}

// ─── MainView ────────────────────────────────────────────

struct MainView {
    screen: Screen,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>,
    pending_approval: Option<PendingApproval>,
    /// 영구 사용자 설정. 새 세션 시작 시 provider/model로 사용.
    settings: Settings,
    /// 설정 모달이 열려 있으면 Some.
    settings_draft: Option<SettingsDraft>,
    /// 세션이 이미 닫힌 뒤에도 보여야 하는 짧은 친화적 알림.
    notice: Option<String>,
}

impl MainView {
    fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>) -> Self {
        Self {
            screen: Screen::Welcome,
            cmd_tx,
            pending_approval: None,
            settings: settings::load(),
            settings_draft: None,
            notice: None,
        }
    }

    fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let provider_text = self.settings.provider.clone();
        let main_model_text = self.settings.main_model.clone();
        let sub_model_text = self.settings.sub_model.clone();
        let provider = cx.new(|cx| {
            let mut ci = ChatInput::new("provider 이름", theme::TOKENS.fg, theme::TOKENS.muted, cx);
            ci.set_content(provider_text, cx);
            ci
        });
        let main_model = cx.new(|cx| {
            let mut ci = ChatInput::new("조율 모델", theme::TOKENS.fg, theme::TOKENS.muted, cx);
            ci.set_content(main_model_text, cx);
            ci
        });
        let sub_model = cx.new(|cx| {
            let mut ci = ChatInput::new("작업 모델", theme::TOKENS.fg, theme::TOKENS.muted, cx);
            ci.set_content(sub_model_text, cx);
            ci
        });
        self.settings_draft = Some(SettingsDraft {
            provider,
            main_model,
            sub_model,
            notice: None,
        });
        cx.notify();
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_draft = None;
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.settings_draft.as_ref() else {
            return;
        };
        let provider = d.provider.read(cx).content().trim().to_string();
        let main_model = d.main_model.read(cx).content().trim().to_string();
        let sub_model = d.sub_model.read(cx).content().trim().to_string();
        if provider.is_empty() || main_model.is_empty() || sub_model.is_empty() {
            if let Some(d) = self.settings_draft.as_mut() {
                d.notice = Some("provider, 조율 모델, 작업 모델을 모두 입력해 주세요".into());
            }
            cx.notify();
            return;
        }
        let new_settings = Settings {
            provider,
            model: main_model.clone(),
            main_model,
            sub_model,
            recent_projects: self.settings.recent_projects.clone(),
        };
        match settings::save(&new_settings) {
            Ok(()) => {
                self.settings = new_settings;
                self.settings_draft = None;
            }
            Err(e) => {
                if let Some(d) = self.settings_draft.as_mut() {
                    d.notice = Some(format!("저장 실패: {e}"));
                }
            }
        }
        cx.notify();
    }

    /// 폴더 다이얼로그 → 새 세션 추가. Welcome이면 Workspace로 전환.
    fn open_folder(&mut self, cx: &mut Context<Self>) {
        // GPUI listener 안에서 sync rfd::pick_folder 부르면 macOS modal이 시스템
        // 알림으로 GPUI App을 재진입(borrow_mut) → RefCell double-borrow panic.
        // cx.spawn으로 분리 필수.
        cx.spawn(async move |this, cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("프로젝트 폴더 선택")
                .pick_folder()
                .await;
            let Some(handle) = handle else { return };
            let path = handle.path().to_path_buf();
            tracing::info!("프로젝트 폴더 선택: {}", path.display());
            let _ = this.update(cx, |this, cx| {
                this.add_new_session(path, cx);
            });
        })
        .detach();
    }

    fn open_recent_project(&mut self, path: String, cx: &mut Context<Self>) {
        let path = PathBuf::from(path);
        if !path.is_dir() {
            self.notice = Some(format!(
                "최근 프로젝트를 찾을 수 없어요\n{}",
                path.to_string_lossy()
            ));
            cx.notify();
            return;
        }
        self.add_new_session(path, cx);
    }

    fn add_new_session(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.remember_recent_project(&path);
        // Welcome → Workspace 전환 또는 기존 Workspace에 추가.
        if matches!(self.screen, Screen::Welcome) {
            self.screen = Screen::Workspace(WorkspaceState {
                sessions: HashMap::new(),
                order: Vec::new(),
                active: None,
                next_id: 0,
            });
        }
        let Screen::Workspace(ws) = &mut self.screen else {
            return;
        };
        ws.next_id += 1;
        let session_id: SessionId = format!("s{}", ws.next_id);
        let state = SessionUiState::new(path.clone(), cx);
        ws.sessions.insert(session_id.clone(), state);
        ws.order.push(session_id.clone());
        ws.active = Some(session_id.clone());
        let _ = self.cmd_tx.send(UiCommand::NewSession {
            session_id,
            path,
            provider: self.settings.provider.clone(),
            main_model: self
                .settings
                .model_for_role(AgentModelRole::Main)
                .to_string(),
            sub_model: self
                .settings
                .model_for_role(AgentModelRole::Sub)
                .to_string(),
        });
        cx.notify();
    }

    fn remember_recent_project(&mut self, path: &Path) {
        self.settings.remember_recent_project(path);
        if let Err(e) = settings::save(&self.settings) {
            tracing::warn!("최근 프로젝트 저장 실패: {e}");
        }
    }

    fn set_active(&mut self, sid: SessionId, cx: &mut Context<Self>) {
        if let Screen::Workspace(ws) = &mut self.screen {
            if ws.sessions.contains_key(&sid) {
                ws.active = Some(sid.clone());
                if let Some(s) = ws.sessions.get_mut(&sid) {
                    s.has_unread = false;
                }
                cx.notify();
            }
        }
    }

    fn close_session(&mut self, sid: SessionId, cx: &mut Context<Self>) {
        if let Screen::Workspace(ws) = &mut self.screen {
            ws.sessions.remove(&sid);
            ws.order.retain(|s| s != &sid);
            if ws.active.as_ref() == Some(&sid) {
                ws.active = ws.order.last().cloned();
            }
            // 모두 닫혔으면 Welcome 복귀.
            if ws.order.is_empty() {
                self.screen = Screen::Welcome;
            }
        }
        let _ = self
            .cmd_tx
            .send(UiCommand::CloseSession { session_id: sid });
        cx.notify();
    }

    fn answer_approval(&mut self, decision: ApprovalDecision, cx: &mut Context<Self>) {
        let Some(p) = self.pending_approval.take() else {
            return;
        };
        let _ = self.cmd_tx.send(UiCommand::ApprovalDecision {
            session_id: p.session_id,
            request_id: p.request_id,
            decision,
        });
        cx.notify();
    }

    fn revert_active(&mut self, cx: &mut Context<Self>) {
        let Screen::Workspace(ws) = &mut self.screen else {
            return;
        };
        let Some(sid) = ws.active.clone() else { return };
        let Some(s) = ws.sessions.get_mut(&sid) else {
            return;
        };
        if !s.last_commit.as_ref().is_some_and(|c| c.revertible) {
            return;
        }
        s.last_commit = None;
        let _ = self
            .cmd_tx
            .send(UiCommand::RevertLastTurn { session_id: sid });
        cx.notify();
    }

    fn interrupt_active(&mut self, cx: &mut Context<Self>) {
        let Screen::Workspace(ws) = &mut self.screen else {
            return;
        };
        let Some(sid) = ws.active.clone() else { return };
        let Some(s) = ws.sessions.get_mut(&sid) else {
            return;
        };
        if !s.turn_in_flight || s.interrupt_requested {
            return;
        }
        s.interrupt_requested = true;
        s.messages.push(ChatItem::message(
            Speaker::System,
            "중단 요청을 보냈어요",
            cx,
        ));
        s.scroll.scroll_to_bottom();
        let _ = self
            .cmd_tx
            .send(UiCommand::InterruptTurn { session_id: sid });
        cx.notify();
    }

    fn send_user_input(&mut self, cx: &mut Context<Self>) {
        let Screen::Workspace(ws) = &self.screen else {
            return;
        };
        let Some(sid) = ws.active.clone() else { return };
        let Some(s) = ws.sessions.get(&sid) else {
            return;
        };
        if !s.thread_started || s.turn_in_flight {
            return;
        }
        let text = s.input.read(cx).content().to_string();
        if text.trim().is_empty() {
            return;
        }
        let input_entity = s.input.clone();
        input_entity.update(cx, |this, cx| this.clear(cx));

        let user_msg = ChatItem::message(Speaker::User, text.clone(), cx);
        let agent_msg = ChatItem::message(Speaker::Agent, "", cx);
        if let Screen::Workspace(ws) = &mut self.screen {
            if let Some(s) = ws.sessions.get_mut(&sid) {
                s.messages.push(user_msg);
                s.messages.push(agent_msg);
                s.turn_in_flight = true;
                s.interrupt_requested = false;
                s.turn_reasoning_chars = 0;
                s.turn_answer_chars = 0;
                s.scroll.scroll_to_bottom();
            }
        }
        let _ = self.cmd_tx.send(UiCommand::SendText {
            session_id: sid,
            text,
        });
        cx.notify();
    }

    /// 모든 UiEvent → 적절한 SessionUiState로 라우팅.
    /// active 가 아닌 세션에 도착한 메시지/델타는 has_unread 표식.
    fn handle_event(&mut self, ev: UiEvent, cx: &mut Context<Self>) {
        match ev {
            UiEvent::SessionStarted {
                session_id,
                project: _,
                thread_id: _,
                workspace_mode,
            } => {
                let intro =
                    ChatItem::message(Speaker::System, session_started_message(workspace_mode), cx);
                self.with_session(&session_id, |s| {
                    s.thread_started = true;
                    s.messages.clear();
                    s.messages.push(intro);
                });
            }
            UiEvent::SessionFailed { session_id, error } => {
                let friendly = friendly_translate(&error);
                let m =
                    ChatItem::message(Speaker::System, format!("세션 시작 실패\n{friendly}"), cx);
                self.with_session(&session_id, |s| s.messages.push(m));
            }
            UiEvent::SessionClosed { session_id } => {
                // 사이드바에서 close_session으로 이미 정리됨 — 이벤트는 확인용.
                tracing::info!("[{session_id}] 세션 닫힘");
            }
            UiEvent::WorkspaceCleanup {
                session_id: _,
                message,
            } => {
                self.notice = Some(message);
            }
            UiEvent::AgentDelta { session_id, text } => {
                let mut new_msg = None;
                self.with_session(&session_id, |s| {
                    s.turn_answer_chars = s.turn_answer_chars.saturating_add(text.chars().count());
                    match last_agent_message_text(&mut s.messages) {
                        Some(entity) => {
                            entity.update(cx, |this, cx| this.append(&text, cx));
                        }
                        None => new_msg = Some(text.clone()),
                    }
                    s.scroll.scroll_to_bottom();
                });
                if let Some(text) = new_msg {
                    let m = ChatItem::message(Speaker::Agent, text, cx);
                    self.with_session(&session_id, |s| s.messages.push(m));
                }
                self.mark_unread_if_inactive(&session_id);
            }
            UiEvent::ReasoningDelta { session_id, text } => {
                let mut create_new = None;
                self.with_session(&session_id, |s| {
                    s.turn_reasoning_chars =
                        s.turn_reasoning_chars.saturating_add(text.chars().count());
                    if let Some(reasoning_entity) = ensure_agent_reasoning(&mut s.messages) {
                        reasoning_entity.update(cx, |this, cx| this.append(&text, cx));
                    } else {
                        create_new = Some(text.clone());
                    }
                    s.scroll.scroll_to_bottom();
                });
                if let Some(text) = create_new {
                    let mut agent_msg = ChatItem::message(Speaker::Agent, "", cx);
                    if let ChatItem::Message { reasoning, .. } = &mut agent_msg {
                        let r = cx.new(|cx| SelectableText::new(text, theme::TOKENS.muted, cx));
                        *reasoning = Some(r);
                    }
                    self.with_session(&session_id, |s| s.messages.push(agent_msg));
                }
                self.mark_unread_if_inactive(&session_id);
            }
            UiEvent::ToolStarted {
                session_id,
                item_id,
                kind,
                title,
            } => {
                let card = ChatItem::tool(item_id, kind, title, cx);
                self.with_session(&session_id, |s| {
                    s.messages.push(card);
                    s.scroll.scroll_to_bottom();
                });
                self.mark_unread_if_inactive(&session_id);
            }
            UiEvent::ToolOutput {
                session_id,
                item_id,
                delta,
            } => {
                self.with_session(&session_id, |s| {
                    if let Some(out) = find_tool_output(&mut s.messages, &item_id) {
                        out.update(cx, |this, cx| this.append(&delta, cx));
                    }
                });
            }
            UiEvent::ToolCompleted {
                session_id,
                item_id,
                ok,
                summary,
            } => {
                self.with_session(&session_id, |s| {
                    if let Some(item) = s.messages.iter_mut().rev().find(|m| match m {
                        ChatItem::Tool { item_id: id, .. } => id == &item_id,
                        _ => false,
                    }) {
                        if let ChatItem::Tool { status, output, .. } = item {
                            *status = if ok {
                                ToolStatus::CompletedOk
                            } else {
                                ToolStatus::CompletedFail
                            };
                            if let Some(s) = summary {
                                let entity = output.clone();
                                entity.update(cx, |this, cx| this.set_content(s, cx));
                            }
                        }
                    }
                });
            }
            UiEvent::ApprovalRequested {
                session_id,
                request_id,
                kind,
                friendly_title,
                raw_detail,
            } => {
                self.pending_approval = Some(PendingApproval {
                    session_id,
                    request_id,
                    kind,
                    friendly_title,
                    raw_detail,
                });
            }
            UiEvent::TurnDone {
                session_id,
                ok,
                error_text,
            } => {
                let err_msg = if !ok {
                    let raw = error_text.unwrap_or_default();
                    let friendly = friendly_translate(&raw);
                    Some(ChatItem::message(
                        Speaker::System,
                        format!("turn 실패\n{friendly}"),
                        cx,
                    ))
                } else {
                    None
                };
                if let Screen::Workspace(ws) = &mut self.screen {
                    if let Some(s) = ws.sessions.get_mut(&session_id) {
                        s.turn_in_flight = false;
                        s.interrupt_requested = false;
                        s.turn_reasoning_chars = 0;
                        s.turn_answer_chars = 0;
                        if let Some(m) = err_msg {
                            s.messages.push(m);
                        }
                        // turn 성공 시: 마지막 agent message에 markdown 파싱해서 segments 채움.
                        // streaming 끝나서 raw text 완성된 시점이라 안전.
                        if ok {
                            finalize_last_agent_message_markdown(s, cx);
                        }
                    }
                }
                self.mark_unread_if_inactive(&session_id);
            }
            UiEvent::TurnCommitted {
                session_id,
                commit_oid,
                summary,
                revert_to,
            } => {
                let short = commit_oid.chars().take(7).collect::<String>();
                let m = ChatItem::message(
                    Speaker::System,
                    format!("💾 변경 저장됨 ({short}) — {summary}"),
                    cx,
                );
                self.with_session(&session_id, |s| {
                    s.messages.push(m);
                    s.last_commit = Some(LastCommit {
                        summary: summary.clone(),
                        revertible: revert_to.is_some(),
                    });
                });
            }
            UiEvent::Reverted {
                session_id,
                ok,
                error_text,
            } => {
                let m = if ok {
                    ChatItem::message(Speaker::System, "↶ 마지막 변경을 되돌렸어요", cx)
                } else {
                    let raw = error_text.unwrap_or_default();
                    let friendly = friendly_translate(&raw);
                    ChatItem::message(Speaker::System, format!("되돌리기 실패\n{friendly}"), cx)
                };
                self.with_session(&session_id, |s| s.messages.push(m));
            }
            UiEvent::Error(text) => {
                // 글로벌 에러 — active 세션에 표시. 세션이 없으면 무시.
                let friendly = friendly_translate(&text);
                let m = ChatItem::message(Speaker::System, friendly, cx);
                if let Screen::Workspace(ws) = &mut self.screen {
                    if let Some(sid) = ws.active.clone() {
                        if let Some(s) = ws.sessions.get_mut(&sid) {
                            s.turn_in_flight = false;
                            s.messages.push(m);
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    /// 도우미: SessionId로 SessionUiState 가져와 클로저 적용.
    fn with_session(&mut self, sid: &SessionId, f: impl FnOnce(&mut SessionUiState)) {
        if let Screen::Workspace(ws) = &mut self.screen {
            if let Some(s) = ws.sessions.get_mut(sid) {
                f(s);
            }
        }
    }

    /// 활성 세션이 아닐 때만 has_unread 표식.
    fn mark_unread_if_inactive(&mut self, sid: &SessionId) {
        if let Screen::Workspace(ws) = &mut self.screen {
            if ws.active.as_ref() != Some(sid) {
                if let Some(s) = ws.sessions.get_mut(sid) {
                    s.has_unread = true;
                }
            }
        }
    }
}

// ─── Render ──────────────────────────────────────────────

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &theme::TOKENS;
        let display_model = "5.5 매우 높음";
        let body = match &self.screen {
            Screen::Welcome => render_welcome(t, display_model, &self.settings.recent_projects, cx),
            Screen::Workspace(ws) => {
                render_workspace(t, ws, display_model, &self.settings.recent_projects, cx)
            }
        };
        let approval_modal = self
            .pending_approval
            .as_ref()
            .map(|p| render_approval_modal(t, p, cx));
        let settings_modal = self
            .settings_draft
            .as_ref()
            .map(|d| render_settings_modal(t, d, cx));
        let notice_modal = self
            .notice
            .as_ref()
            .map(|n| render_notice_modal(t, n.clone(), cx));
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
            .on_action(cx.listener(|this, _: &chat_input::Submit, _, cx| this.send_user_input(cx)))
            .child(body)
            .when_some(approval_modal, |d, m| d.child(m))
            .when_some(settings_modal, |d, m| d.child(m))
            .when_some(notice_modal, |d, m| d.child(m))
    }
}

fn session_started_message(workspace_mode: WorkspaceMode) -> &'static str {
    match workspace_mode {
        WorkspaceMode::Isolated => {
            "작업공간 준비됨\n원본 폴더는 그대로 두고 이 세션 전용 공간에서 진행해요.\n에이전트 연결됨"
        }
        WorkspaceMode::Direct => "작업공간 준비됨\n에이전트 연결됨",
    }
}

#[cfg(test)]
fn model_route_label(settings: &Settings) -> String {
    let main = settings.model_for_role(AgentModelRole::Main);
    let sub = settings.model_for_role(AgentModelRole::Sub);
    if main == sub {
        format!("조율/작업 {main}")
    } else {
        format!("조율 {main} · 작업 {sub}")
    }
}

fn render_welcome(
    t: &theme::Tokens,
    chips_model: &str,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .size_full()
        .bg(rgb(t.bg))
        .child(render_welcome_sidebar(t, recent_projects, cx))
        .child(render_welcome_main(t, chips_model, recent_projects, cx))
}

fn render_welcome_sidebar(
    t: &theme::Tokens,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let new_session_btn = sidebar_action_row(
        t,
        "✎",
        "새 작업",
        cx.listener(|this, _, _, cx| this.open_folder(cx)),
    )
    .bg(rgb(t.sidebar_active));
    let settings_btn = sidebar_action_row(
        t,
        "⚙",
        "설정",
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(300.))
        .h_full()
        .bg(rgb(t.sidebar))
        .border_r_1()
        .border_color(rgb(t.border))
        .child(sidebar_brand(t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .pb_4()
                .child(new_session_btn)
                .child(sidebar_nav_row(t, "⌕", "검색", false))
                .child(sidebar_nav_row(t, "◇", "플러그인", false))
                .child(sidebar_nav_row(t, "○", "자동화", false)),
        )
        .child(render_recent_project_section(t, recent_projects, cx))
        .child(div().flex_1())
        .child(settings_btn)
}

fn render_welcome_main(
    t: &theme::Tokens,
    chips_model: &str,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .bg(rgb(t.bg))
        .child(
            div()
                .flex()
                .h(px(58.))
                .px_6()
                .items_center()
                .border_b_1()
                .border_color(rgb(t.border))
                .child(top_bar_title(t, "새 작업"))
                .child(div().flex_1())
                .child(top_bar_controls(t, chips_model)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_end()
                .gap_6()
                .px_8()
                .pb_8()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .mb_6()
                        .child(
                            div()
                                .text_2xl()
                                .text_color(rgb(t.fg))
                                .child("무엇을 만들까요?"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(t.muted))
                                .child("새 작업 준비됨"),
                        ),
                )
                .when(!recent_projects.is_empty(), |d| {
                    d.child(render_recent_projects_panel(t, recent_projects, cx))
                })
                .child(welcome_composer(t, chips_model, cx))
                .child(
                    div()
                        .w_full()
                        .max_w(px(980.))
                        .text_sm()
                        .text_color(rgb(t.fg))
                        .child("작업 트리"),
                ),
        )
}

fn welcome_composer(t: &theme::Tokens, chips_model: &str, cx: &mut Context<MainView>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .max_w(px(980.))
        .min_h(px(126.))
        .px_4()
        .py_3()
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_2xl()
        .shadow_lg()
        .cursor_pointer()
        .hover(|d| d.border_color(rgb(0xc8c8cc)))
        .child(
            div()
                .text_lg()
                .text_color(rgb(t.muted))
                .child("작업할 프로젝트를 선택하세요"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(composer_icon_button(t, "+"))
                .child(permission_chip(t))
                .child(div().flex_1())
                .child(chip_owned(t, chips_model.to_string()))
                .child(composer_icon_button(t, "⌕"))
                .child(send_circle("↑", 0x8a8a8f, true)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.open_folder(cx)),
        )
}

fn render_recent_projects_panel(
    t: &theme::Tokens,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .max_w(px(820.))
        .child(section_label("최근 프로젝트"))
        .children(
            recent_projects
                .iter()
                .take(4)
                .cloned()
                .map(|path| render_recent_project_row(t, path, true, cx)),
        )
}

fn render_workspace(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    chips_model: &str,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .size_full()
        .child(render_sidebar(t, ws, recent_projects, cx))
        .child(render_active_main(t, ws, chips_model, cx))
}

/// 좌측 사이드바 — dynamic 세션 list. 클릭으로 active 전환. status icon 동기화.
fn render_sidebar(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let items: Vec<gpui::Div> = ws
        .order
        .iter()
        .filter_map(|sid| {
            let s = ws.sessions.get(sid)?;
            Some(render_session_item(
                t,
                sid.clone(),
                s,
                ws.active.as_ref() == Some(sid),
                cx,
            ))
        })
        .collect();

    let new_session_btn = sidebar_action_row(
        t,
        "✎",
        "새 작업",
        cx.listener(|this, _, _, cx| this.open_folder(cx)),
    );
    let settings_btn = sidebar_action_row(
        t,
        "⚙",
        "설정",
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(300.))
        .h_full()
        .bg(rgb(t.sidebar))
        .border_r_1()
        .border_color(rgb(t.border))
        .child(sidebar_brand(t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .pb_4()
                .child(new_session_btn)
                .child(sidebar_nav_row(t, "⌕", "검색", false))
                .child(sidebar_nav_row(t, "◇", "플러그인", false))
                .child(sidebar_nav_row(t, "○", "자동화", false)),
        )
        .child(render_recent_project_section(t, recent_projects, cx))
        .child(
            div()
                .px_5()
                .pt_4()
                .pb_2()
                .text_sm()
                .text_color(rgb(0xa0a0a6))
                .child("작업 세션"),
        )
        .child(
            div()
                .id("session-list")
                .flex()
                .flex_col()
                .flex_1()
                .gap_1()
                .px_2()
                .overflow_y_scroll()
                .children(items),
        )
        .child(settings_btn)
}

fn sidebar_brand(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .h(px(58.))
        .px_5()
        .items_center()
        .gap_2()
        .text_lg()
        .text_color(rgb(t.fg))
        .child("Stcode")
}

fn top_bar_title(t: &theme::Tokens, title: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_lg().text_color(rgb(t.fg)).child(title))
        .child(toolbar_icon_button(t, "…"))
}

fn sidebar_action_row(
    t: &theme::Tokens,
    icon: &'static str,
    label: &'static str,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .flex()
        .gap_2()
        .items_center()
        .mx_3()
        .px_3()
        .py_2()
        .text_color(rgb(t.fg))
        .rounded_lg()
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(div().w_5().text_color(rgb(t.muted)).child(icon))
        .child(div().flex_1().child(label))
        .on_mouse_down(MouseButton::Left, on_click)
}

fn sidebar_nav_row(
    t: &theme::Tokens,
    icon: &'static str,
    label: &'static str,
    active: bool,
) -> gpui::Div {
    let bg = if active { t.sidebar_active } else { t.sidebar };
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_3()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(rgb(bg))
        .text_color(rgb(t.muted))
        .child(div().w_5().child(icon))
        .child(div().flex_1().child(label))
}

fn render_recent_project_section(
    t: &theme::Tokens,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let rows: Vec<gpui::Div> = recent_projects
        .iter()
        .take(5)
        .cloned()
        .map(|path| render_recent_project_row(t, path, false, cx))
        .collect();
    let body = if rows.is_empty() {
        div()
            .mx_3()
            .px_3()
            .py_2()
            .text_sm()
            .text_color(rgb(t.muted))
            .child("최근 프로젝트 없음")
    } else {
        div().flex().flex_col().gap_1().children(rows)
    };

    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_4()
        .child(section_label("프로젝트"))
        .child(body)
}

fn section_label(label: &'static str) -> gpui::Div {
    div()
        .px_5()
        .pt_4()
        .pb_2()
        .text_sm()
        .text_color(rgb(0xa0a0a6))
        .child(label)
}

fn render_recent_project_row(
    t: &theme::Tokens,
    path: String,
    roomy: bool,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let label = project_display_name(&path);
    let hint = project_parent_hint(&path);
    let path_for_click = path.clone();
    let row = div()
        .flex()
        .items_center()
        .gap_2()
        .rounded_lg()
        .cursor_pointer()
        .bg(rgb(if roomy { t.surface } else { t.sidebar }))
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(div().w_5().text_color(rgb(t.muted)).child("□"))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(div().text_color(rgb(t.fg)).child(label))
                .child(div().text_xs().text_color(rgb(t.muted)).child(hint)),
        )
        .child(div().text_color(rgb(t.muted)).child("↗"))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| this.open_recent_project(path_for_click.clone(), cx)),
        );

    if roomy {
        row.px_4()
            .py_3()
            .border_1()
            .border_color(rgb(t.border))
            .shadow_sm()
    } else {
        row.mx_3().px_3().py_2()
    }
}

fn project_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn project_parent_hint(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn render_session_item(
    t: &theme::Tokens,
    sid: SessionId,
    s: &SessionUiState,
    active: bool,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let project_label = s
        .project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.project.to_string_lossy().into_owned());
    let status_icon = if !s.thread_started {
        "○"
    } else if s.turn_in_flight {
        "⏳"
    } else if s.has_unread {
        "●"
    } else {
        "✓"
    };
    let bg = if active { t.sidebar_active } else { t.sidebar };
    let sid_for_click = sid.clone();
    let sid_for_close = sid.clone();
    let mut row = div()
        .flex()
        .gap_2()
        .items_center()
        .mx_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(rgb(bg))
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .w_5()
                .text_color(rgb(if s.turn_in_flight { t.accent } else { t.muted }))
                .child(status_icon),
        )
        .child(div().flex_1().overflow_hidden().child(project_label))
        .child(
            div()
                .w_5()
                .text_xs()
                .text_color(rgb(t.muted))
                .cursor_pointer()
                .hover(|d| d.text_color(rgb(0xb43b3b)))
                .child("×")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _ev: &MouseDownEvent, _, cx| {
                        this.close_session(sid_for_close.clone(), cx);
                    }),
                ),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| this.set_active(sid_for_click.clone(), cx)),
        );
    if active {
        row = row.text_color(rgb(0x111114));
    }
    row
}

/// 현재 active 세션의 main panel. active 가 None이면 placeholder.
fn render_active_main(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    chips_model: &str,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let Some(sid) = ws.active.clone() else {
        return div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(t.bg))
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(t.fg))
                    .child("프로젝트를 열어 작업을 시작하세요"),
            )
            .text_color(rgb(t.muted))
            .child("왼쪽의 새 작업을 누르면 세션별 작업공간과 브랜치를 자동으로 준비합니다.");
    };
    let Some(s) = ws.sessions.get(&sid) else {
        return div().flex_1();
    };
    render_chat_main(t, s, chips_model, cx)
}

fn render_chat_main(
    t: &theme::Tokens,
    s: &SessionUiState,
    chips_model: &str,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let send_enabled = s.thread_started && !s.turn_in_flight;
    let send_label = if s.turn_in_flight { "…" } else { "↑" };
    let send_color = if send_enabled { 0x8a8a8f } else { 0xd1d1d6 };
    let project_label = s
        .project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.project.to_string_lossy().into_owned());

    let revert_btn = s.last_commit.as_ref().filter(|c| c.revertible).map(|c| {
        let tooltip = format!("되돌리기 · {}", c.summary);
        div()
            .px_3()
            .py_2()
            .bg(rgb(0xf1f1f3))
            .text_color(rgb(t.fg))
            .text_xs()
            .rounded_lg()
            .cursor_pointer()
            .child(tooltip)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.revert_active(cx)),
            )
    });

    let interrupt_btn = s.turn_in_flight.then(|| {
        let requested = s.interrupt_requested;
        let label = if requested {
            "중단 요청됨"
        } else {
            "중단"
        };
        let bg = if requested { 0xf1f1f3 } else { 0xffeee6 };
        let fg = if requested { t.muted } else { t.accent };
        div()
            .px_3()
            .py_2()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .text_xs()
            .rounded_lg()
            .child(label)
            .when(!requested, |d| {
                d.cursor_pointer()
                    .hover(|d| d.bg(rgb(0xffe0d0)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.interrupt_active(cx)),
                    )
            })
    });

    let status_label = turn_status_label(
        s.thread_started,
        s.turn_in_flight,
        s.interrupt_requested,
        s.turn_reasoning_chars,
        s.turn_answer_chars,
    );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .bg(rgb(t.bg))
        .child(
            div()
                .flex()
                .h(px(58.))
                .px_6()
                .items_center()
                .gap_3()
                .bg(rgb(0xfbfbfc))
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().text_lg().text_color(rgb(t.fg)).child(project_label))
                        .child(toolbar_icon_button(t, "…"))
                        .child(status_pill(t, status_label)),
                )
                .child(top_bar_controls(t, chips_model))
                .when_some(interrupt_btn, |d, b| d.child(b))
                .when_some(revert_btn, |d, b| d.child(b)),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .h_full()
                        .child(
                            div()
                                .id("messages")
                                .flex()
                                .flex_col()
                                .flex_1()
                                .items_center()
                                .overflow_y_scroll()
                                .track_scroll(&s.scroll)
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .w_full()
                                        .max_w(px(1040.))
                                        .px_8()
                                        .py_8()
                                        .children(
                                            s.messages.iter().map(|m| render_chat_item(t, m)),
                                        ),
                                ),
                        )
                        .child(render_composer(
                            t,
                            s,
                            chips_model,
                            send_label,
                            send_color,
                            send_enabled,
                            cx,
                        )),
                )
                .child(render_session_overview(t, s)),
        )
}

fn render_composer(
    t: &theme::Tokens,
    s: &SessionUiState,
    chips_model: &str,
    send_label: &'static str,
    send_color: u32,
    send_enabled: bool,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div().flex().justify_center().px_8().pb_8().child(
        div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .max_w(px(980.))
            .min_h(px(126.))
            .px_4()
            .py_3()
            .bg(rgb(t.surface))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_2xl()
            .shadow_lg()
            .child(div().flex_1().text_lg().child(s.input.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(composer_icon_button(t, "+"))
                    .child(permission_chip(t))
                    .child(div().flex_1())
                    .child(chip_owned(t, chips_model.to_string()))
                    .child(composer_icon_button(t, "⌕"))
                    .child(
                        send_circle(send_label, send_color, send_enabled).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.send_user_input(cx)),
                        ),
                    ),
            ),
    )
}

fn render_session_overview(t: &theme::Tokens, s: &SessionUiState) -> gpui::Div {
    let summary = session_summary(s);
    let workspace_state = if s.thread_started {
        "준비됨"
    } else {
        "준비 중"
    };
    let agent_state = if s.interrupt_requested {
        "중단 요청"
    } else if s.turn_in_flight {
        "진행 중"
    } else if s.thread_started {
        "대기"
    } else {
        "연결 중"
    };
    let save_state = s
        .last_commit
        .as_ref()
        .map(|commit| commit.summary.clone())
        .unwrap_or_else(|| "아직 저장 없음".to_string());
    let tool_total = summary.tools_running + summary.tools_ok + summary.tools_failed;

    div()
        .flex()
        .flex_col()
        .w(px(292.))
        .h_full()
        .px_4()
        .py_5()
        .gap_5()
        .bg(rgb(0xfbfbfc))
        .border_l_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(panel_title(t, "작업 트리"))
                .child(timeline_row(
                    t,
                    "1",
                    "작업공간",
                    workspace_state,
                    s.thread_started,
                ))
                .child(timeline_row(
                    t,
                    "2",
                    "에이전트",
                    agent_state,
                    s.turn_in_flight,
                ))
                .child(timeline_row(
                    t,
                    "3",
                    "변경 저장",
                    save_state.as_str(),
                    s.last_commit.is_some(),
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(panel_title(t, "세션 요약"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(metric_tile(t, "요청", summary.user_turns.to_string()))
                        .child(metric_tile(t, "응답", summary.agent_messages.to_string())),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(metric_tile(t, "도구", tool_total.to_string()))
                        .child(metric_tile(t, "실패", summary.tools_failed.to_string())),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(panel_title(t, "안전망"))
                .child(safety_row(t, "원본 폴더", "보호"))
                .child(safety_row(t, "작업공간", "자동"))
                .child(safety_row(
                    t,
                    "되돌리기",
                    if s.last_commit.as_ref().is_some_and(|c| c.revertible) {
                        "가능"
                    } else {
                        "대기"
                    },
                )),
        )
}

fn panel_title(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div().text_sm().text_color(rgb(t.muted)).child(label)
}

fn timeline_row(
    t: &theme::Tokens,
    step: &'static str,
    label: &'static str,
    state: &str,
    active: bool,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .bg(rgb(if active { 0xffffff } else { 0xf1f1f3 }))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_lg()
        .child(
            div()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(if active { t.accent } else { 0xd8d8dc }))
                .text_color(rgb(0xffffff))
                .text_xs()
                .child(step),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(div().text_sm().text_color(rgb(t.fg)).child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child(state.to_string()),
                ),
        )
}

fn metric_tile(t: &theme::Tokens, label: &'static str, value: String) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap_1()
        .px_3()
        .py_3()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_lg()
        .child(div().text_xs().text_color(rgb(t.muted)).child(label))
        .child(div().text_lg().text_color(rgb(t.fg)).child(value))
}

fn safety_row(t: &theme::Tokens, label: &'static str, state: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(rgb(0xf1f1f3))
        .child(div().flex_1().text_sm().text_color(rgb(t.fg)).child(label))
        .child(div().text_xs().text_color(rgb(t.muted)).child(state))
}

fn session_summary(s: &SessionUiState) -> SessionSummary {
    let mut summary = SessionSummary::default();
    for item in &s.messages {
        match item {
            ChatItem::Message { who, .. } => match who {
                Speaker::User => summary.user_turns += 1,
                Speaker::Agent => summary.agent_messages += 1,
                Speaker::System => {}
            },
            ChatItem::Tool { status, .. } => match status {
                ToolStatus::InProgress => summary.tools_running += 1,
                ToolStatus::CompletedOk => summary.tools_ok += 1,
                ToolStatus::CompletedFail => summary.tools_failed += 1,
            },
        }
    }
    summary
}

fn status_pill(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div()
        .px_3()
        .py_1()
        .bg(rgb(0xf1f1f3))
        .rounded_lg()
        .text_xs()
        .text_color(rgb(t.muted))
        .child(label)
}

fn top_bar_controls(t: &theme::Tokens, chips_model: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(toolbar_icon_button(t, "▷"))
        .child(model_selector_chip(t, chips_model))
        .child(toolbar_icon_button(t, "▯"))
}

fn model_selector_chip(t: &theme::Tokens, label: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_xl()
        .text_xs()
        .text_color(rgb(t.fg))
        .shadow_sm()
        .child(
            div()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(0x1f1f23))
                .text_color(rgb(0xffffff))
                .child("⌘"),
        )
        .child(label.to_string())
        .child(div().text_color(rgb(t.muted)).child("⌄"))
}

fn toolbar_icon_button(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div()
        .size(px(34.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .text_color(rgb(t.muted))
        .hover(|d| d.bg(rgb(0xf1f1f3)).text_color(rgb(t.fg)))
        .child(label)
}

fn composer_icon_button(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .text_lg()
        .text_color(rgb(t.muted))
        .rounded_full()
        .hover(|d| d.bg(rgb(0xf1f1f3)).text_color(rgb(t.fg)))
        .child(label)
}

fn send_circle(label: &'static str, color: u32, enabled: bool) -> gpui::Div {
    div()
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(color))
        .text_color(rgb(0xffffff))
        .rounded_full()
        .when(enabled, |d| {
            d.cursor_pointer().hover(|d| d.bg(rgb(0x6f6f75)))
        })
        .child(label)
}

fn permission_chip(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_lg()
        .text_sm()
        .text_color(rgb(t.accent))
        .child("전체 권한")
        .child("⌄")
}

fn chip_owned(t: &theme::Tokens, label: String) -> gpui::Div {
    div()
        .px_3()
        .py_1()
        .bg(rgb(0xf1f1f3))
        .text_xs()
        .text_color(rgb(t.muted))
        .rounded_lg()
        .child(label)
}

const LONG_REASONING_CHARS: usize = 4_000;

fn turn_status_label(
    thread_started: bool,
    turn_in_flight: bool,
    interrupt_requested: bool,
    reasoning_chars: usize,
    answer_chars: usize,
) -> &'static str {
    if !thread_started {
        "세션 여는 중…"
    } else if !turn_in_flight {
        "대기"
    } else if interrupt_requested {
        "중단 요청 중"
    } else if answer_chars > 0 {
        "답변 작성 중"
    } else if reasoning_chars >= LONG_REASONING_CHARS {
        "생각이 길어지는 중"
    } else if reasoning_chars > 0 {
        "생각 중"
    } else {
        "응답 기다리는 중"
    }
}

fn render_chat_item(t: &theme::Tokens, item: &ChatItem) -> gpui::Div {
    match item {
        ChatItem::Message {
            who,
            text,
            reasoning,
            segments,
        } => render_message(
            t,
            *who,
            text.clone(),
            reasoning.clone(),
            segments.as_deref(),
        ),
        ChatItem::Tool {
            kind,
            title,
            output,
            status,
            ..
        } => render_tool_card(t, *kind, title, output.clone(), *status),
    }
}

fn render_message(
    t: &theme::Tokens,
    who: Speaker,
    text: Entity<SelectableText>,
    reasoning: Option<Entity<SelectableText>>,
    segments: Option<&[MessageSegment]>,
) -> gpui::Div {
    let (icon, bubble_bg) = match who {
        Speaker::User => ("나", 0xfff3eb),
        Speaker::Agent => ("AI", t.surface),
        Speaker::System => ("상태", 0xf1f1f3),
    };
    let mut body = div().flex_1().flex().flex_col().gap_2();
    if let Some(r) = reasoning {
        body = body.child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(0xf1f3f5))
                .rounded_lg()
                .border_l_2()
                .border_color(rgb(0xc7ccd3))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(div().mb_1().child("생각"))
                .child(r),
        );
    }
    // segments가 채워졌으면(turn 끝나서 markdown 파싱됨) segments를 렌더, 아니면
    // streaming 중이라 raw text 그대로.
    if let Some(segs) = segments {
        for seg in segs {
            body = body.child(render_segment(t, seg, bubble_bg));
        }
    } else {
        body = body.child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(bubble_bg))
                .rounded_md()
                .child(text),
        );
    }
    div()
        .flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(38.))
                .mt_1()
                .text_xs()
                .text_color(rgb(t.muted))
                .child(icon),
        )
        .child(body)
}

fn render_segment(t: &theme::Tokens, seg: &MessageSegment, bubble_bg: u32) -> gpui::Div {
    match seg {
        MessageSegment::Paragraph(entity) => div()
            .px_3()
            .py_2()
            .bg(rgb(bubble_bg))
            .rounded_lg()
            .child(entity.clone()),
        MessageSegment::Heading { level, body } => {
            // H1=2xl, H2=xl, H3=lg. 색은 약간 더 밝게 강조.
            let base = div().px_3().py_2().text_color(rgb(t.fg));
            match level {
                1 => base.text_2xl(),
                2 => base.text_xl(),
                _ => base.text_lg(),
            }
            .child(body.clone())
        }
        MessageSegment::ListItem { body } => div()
            .flex()
            .gap_2()
            .px_3()
            .py_1()
            .child(div().w_4().text_color(rgb(t.muted)).child("•"))
            .child(div().flex_1().child(body.clone())),
        MessageSegment::Code { body, language } => {
            // 코드 블록은 읽기 전용 요약 카드처럼 차분하게 보인다.
            let mut card = div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(rgb(0xf1f1f3))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_lg();
            if let Some(lang) = language {
                if !lang.is_empty() {
                    card = card.child(div().text_xs().text_color(rgb(t.muted)).child(lang.clone()));
                }
            }
            card.font_family("Menlo")
                .text_color(rgb(t.fg))
                .child(body.clone())
        }
    }
}

fn render_tool_card(
    t: &theme::Tokens,
    kind: ToolKind,
    title: &str,
    output: Entity<SelectableText>,
    status: ToolStatus,
) -> gpui::Div {
    let icon = kind.icon();
    let (status_label, status_color) = match status {
        ToolStatus::InProgress => ("진행", 0x8a8a91),
        ToolStatus::CompletedOk => ("완료", 0x2f8f55),
        ToolStatus::CompletedFail => ("실패", 0xb43b3b),
    };
    div()
        .flex()
        .gap_3()
        .items_start()
        .child(div().w(px(38.)).mt_1().text_color(rgb(t.muted)).child(icon))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(rgb(0xf7f7f8))
                .rounded_lg()
                .border_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(status_color))
                                .child(status_label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(t.fg))
                                .child(title.to_string()),
                        ),
                )
                .child(div().text_xs().text_color(rgb(t.muted)).child(output)),
        )
}

// ─── 모달 (자동 모드에선 거의 안 뜸 — 인프라만 남김) ──────

fn render_approval_modal(
    t: &theme::Tokens,
    p: &PendingApproval,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let icon = p.kind.icon();
    let detail = p.raw_detail.clone();
    let show_detail = !detail.is_empty();

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000040))
        .on_mouse_down(MouseButton::Left, |_, _, _| {})
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(480.))
                .p_6()
                .bg(rgb(t.surface))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_2xl()
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(div().text_2xl().child(icon))
                        .child(div().flex_1().text_lg().child(p.friendly_title.clone())),
                )
                .when(show_detail, |d| {
                    d.child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(t.bg))
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(t.border))
                            .text_sm()
                            .text_color(rgb(t.muted))
                            .child(detail),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child("Stcode가 도구를 쓰려고 해요. 안전해 보이면 허락해 주세요."),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(approval_button(
                            "거절",
                            0x4a3a3a,
                            0xe0a0a0,
                            cx.listener(|this, _, _, cx| {
                                this.answer_approval(ApprovalDecision::Decline, cx)
                            }),
                        ))
                        .child(approval_button(
                            "한 번만 허락",
                            t.surface,
                            t.fg,
                            cx.listener(|this, _, _, cx| {
                                this.answer_approval(ApprovalDecision::AcceptOnce, cx)
                            }),
                        ))
                        .child(approval_button(
                            "이번 세션 내내 허락",
                            t.accent,
                            0x111122,
                            cx.listener(|this, _, _, cx| {
                                this.answer_approval(ApprovalDecision::AcceptForSession, cx)
                            }),
                        )),
                ),
        )
}

fn approval_button(
    label: &'static str,
    bg_color: u32,
    fg_color: u32,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .px_3()
        .py_2()
        .bg(rgb(bg_color))
        .text_color(rgb(fg_color))
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .child(label)
        .on_mouse_down(MouseButton::Left, on_click)
}

fn render_notice_modal(
    t: &theme::Tokens,
    message: String,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000040))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.dismiss_notice(cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w(px(360.))
                .p_5()
                .bg(rgb(t.surface))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_md()
                .on_mouse_down(MouseButton::Left, |_, _, _| {})
                .child(div().text_lg().child("작업공간"))
                .child(div().text_sm().text_color(rgb(t.muted)).child(message))
                .child(div().flex().justify_end().child(approval_button(
                    "확인",
                    t.accent,
                    0x111122,
                    cx.listener(|this, _, _, cx| this.dismiss_notice(cx)),
                ))),
        )
}

// ─── 설정 모달 ───────────────────────────────────────────

fn render_settings_modal(
    t: &theme::Tokens,
    d: &SettingsDraft,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let path_hint = settings::settings_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(경로 알 수 없음)".into());
    let notice = d.notice.clone();

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000040))
        .on_mouse_down(MouseButton::Left, |_, _, _| {})
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(520.))
                .p_6()
                .bg(rgb(t.surface))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_2xl()
                .shadow_lg()
                .child(div().text_lg().child("설정"))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child("새 세션부터 적용돼요. 진행 중인 세션엔 영향 없음."),
                )
                .child(setting_field(
                    t,
                    "Provider (codex config.toml에 정의된 이름)",
                    d.provider.clone(),
                ))
                .child(setting_field(t, "조율 모델", d.main_model.clone()))
                .child(setting_field(t, "작업 모델", d.sub_model.clone()))
                .when_some(notice, |dv, n| {
                    dv.child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x40242c))
                            .text_color(rgb(0xe0a0a0))
                            .text_sm()
                            .rounded_md()
                            .child(n),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child(format!("저장 위치: {path_hint}")),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(approval_button(
                            "취소",
                            t.surface,
                            t.fg,
                            cx.listener(|this, _, _, cx| this.close_settings(cx)),
                        ))
                        .child(approval_button(
                            "저장",
                            t.accent,
                            0x111122,
                            cx.listener(|this, _, _, cx| this.save_settings(cx)),
                        )),
                ),
        )
}

fn setting_field(t: &theme::Tokens, label: &'static str, input: Entity<ChatInput>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(rgb(t.muted)).child(label))
        .child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(t.bg))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_md()
                .child(input),
        )
}

// ─── Markdown ───────────────────────────────────────────

/// 세션의 마지막 agent message에 markdown 파싱한 segments를 채운다.
/// 이미 채워졌거나 ``` 코드블록이 없으면 no-op.
fn finalize_last_agent_message_markdown(s: &mut SessionUiState, cx: &mut Context<MainView>) {
    let Some(item) = s.messages.iter_mut().rev().find(|m| {
        matches!(
            m,
            ChatItem::Message {
                who: Speaker::Agent,
                ..
            }
        )
    }) else {
        return;
    };
    let ChatItem::Message { text, segments, .. } = item else {
        return;
    };
    if segments.is_some() {
        return;
    }
    // raw text 추출 — read는 immut borrow, scope 끝내고 cx.new로 mut.
    let raw = text.read(cx).content().to_string();
    if !has_markdown_markers(&raw) {
        return;
    }
    let parsed = parse_markdown_segments(&raw, cx);
    *segments = Some(parsed);
}

/// 마크다운 marker가 하나라도 있으면 true. 파싱 비용 절약용 빠른 체크.
fn has_markdown_markers(raw: &str) -> bool {
    if raw.contains("```") || raw.contains('`') || raw.contains("**") || raw.contains("](") {
        return true;
    }
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("# ")
            || t.starts_with("## ")
            || t.starts_with("### ")
            || t.starts_with("- ")
            || t.starts_with("* ")
        {
            return true;
        }
    }
    false
}

/// 구조적 파싱 결과 — entity 만들기 전 단계. 단위 테스트 가능.
#[derive(Debug, PartialEq, Eq)]
enum RawSegment {
    Paragraph(String),
    Heading {
        level: u8,
        body: String,
    },
    ListItem(String),
    Code {
        body: String,
        language: Option<String>,
    },
}

/// `raw`를 RawSegment로 자른다 (entity 없음 → 테스트 가능).
/// 절차:
/// 1. ``` fence 토글 — fence 안은 통째로 Code.
/// 2. non-code 본문은 line 단위:
///    - `# `/`## `/`### ` → Heading
///    - `- `/`* ` (들여쓰기 0~3) → ListItem
///    - 빈 줄 → paragraph 끊기
///    - 그 외 → paragraph buffer 누적
/// inline (코드/볼드) 는 다음 단계.
fn parse_markdown_raw(raw: &str) -> Vec<RawSegment> {
    let mut segments: Vec<RawSegment> = Vec::new();
    let mut text_buf = String::new();
    let mut code_buf = String::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;

    let flush_text = |buf: &mut String, segs: &mut Vec<RawSegment>| {
        if buf.trim().is_empty() {
            buf.clear();
            return;
        }
        let text = buf.trim_end_matches('\n').to_string();
        segs.push(RawSegment::Paragraph(text));
        buf.clear();
    };

    let push_code = |buf: &mut String, lang: &mut Option<String>, segs: &mut Vec<RawSegment>| {
        let body = buf.trim_end_matches('\n').to_string();
        segs.push(RawSegment::Code {
            body,
            language: lang.take(),
        });
        buf.clear();
    };

    for line in raw.split_inclusive('\n') {
        let stripped_nl = line.trim_end_matches('\n').trim_end_matches('\r');

        if let Some(rest) = stripped_nl.strip_prefix("```") {
            if in_code {
                push_code(&mut code_buf, &mut code_lang, &mut segments);
                in_code = false;
            } else {
                flush_text(&mut text_buf, &mut segments);
                let lang = rest.trim().to_string();
                code_lang = if lang.is_empty() { None } else { Some(lang) };
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_buf.push_str(line);
            continue;
        }

        if let Some(level) = heading_level(stripped_nl) {
            flush_text(&mut text_buf, &mut segments);
            let body = stripped_nl[(level as usize + 1).min(stripped_nl.len())..]
                .trim_start()
                .to_string();
            segments.push(RawSegment::Heading { level, body });
            continue;
        }

        if let Some(item) = list_item_body(stripped_nl) {
            flush_text(&mut text_buf, &mut segments);
            segments.push(RawSegment::ListItem(item));
            continue;
        }

        if stripped_nl.trim().is_empty() {
            flush_text(&mut text_buf, &mut segments);
            continue;
        }

        text_buf.push_str(line);
    }

    if in_code {
        push_code(&mut code_buf, &mut code_lang, &mut segments);
    } else {
        flush_text(&mut text_buf, &mut segments);
    }

    segments
}

#[derive(Debug, PartialEq, Eq)]
struct InlineParse {
    text: String,
    spans: Vec<InlineSpan>,
}

fn parse_inline_markdown(raw: &str) -> InlineParse {
    let mut text = String::with_capacity(raw.len());
    let mut spans = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let rest = &raw[i..];
        if let Some(after_tick) = rest.strip_prefix('`') {
            if let Some(end) = after_tick.find('`') {
                let body = &after_tick[..end];
                if !body.is_empty() {
                    let start = text.len();
                    text.push_str(body);
                    spans.push(InlineSpan {
                        range: start..text.len(),
                        kind: InlineKind::Code,
                    });
                    i += 1 + end + 1;
                    continue;
                }
            }
        }
        if let Some(after_open) = rest.strip_prefix("**") {
            if let Some(end) = after_open.find("**") {
                let body = &after_open[..end];
                if !body.is_empty() {
                    let start = text.len();
                    text.push_str(body);
                    spans.push(InlineSpan {
                        range: start..text.len(),
                        kind: InlineKind::Bold,
                    });
                    i += 2 + end + 2;
                    continue;
                }
            }
        }
        if let Some(after_bracket) = rest.strip_prefix('[') {
            if let Some(label_end) = after_bracket.find("](") {
                let label = &after_bracket[..label_end];
                let after_url_open = &after_bracket[label_end + 2..];
                if let Some(url_end) = after_url_open.find(')') {
                    let url = &after_url_open[..url_end];
                    if !label.is_empty() && !url.is_empty() {
                        let start = text.len();
                        text.push_str(label);
                        text.push_str(" (");
                        text.push_str(url);
                        text.push(')');
                        spans.push(InlineSpan {
                            range: start..text.len(),
                            kind: InlineKind::Link,
                        });
                        i += 1 + label_end + 2 + url_end + 1;
                        continue;
                    }
                }
            }
        }

        let ch = rest.chars().next().expect("non-empty rest");
        text.push(ch);
        i += ch.len_utf8();
    }
    InlineParse { text, spans }
}

fn selectable_from_markdown_inline(
    raw: String,
    color: u32,
    cx: &mut Context<MainView>,
) -> Entity<SelectableText> {
    let parsed = parse_inline_markdown(&raw);
    if parsed.spans.is_empty() && parsed.text == raw {
        cx.new(|cx| SelectableText::new(raw, color, cx))
    } else {
        cx.new(|cx| SelectableText::new_inline(parsed.text, color, parsed.spans, cx))
    }
}

/// RawSegment 들에서 SelectableText entity를 만들어 MessageSegment 로 변환.
fn parse_markdown_segments(raw: &str, cx: &mut Context<MainView>) -> Vec<MessageSegment> {
    parse_markdown_raw(raw)
        .into_iter()
        .map(|raw_seg| match raw_seg {
            RawSegment::Paragraph(text) => {
                let entity = selectable_from_markdown_inline(text, theme::TOKENS.fg, cx);
                MessageSegment::Paragraph(entity)
            }
            RawSegment::Heading { level, body } => {
                let entity = selectable_from_markdown_inline(body, theme::TOKENS.fg, cx);
                MessageSegment::Heading {
                    level,
                    body: entity,
                }
            }
            RawSegment::ListItem(body) => {
                let entity = selectable_from_markdown_inline(body, theme::TOKENS.fg, cx);
                MessageSegment::ListItem { body: entity }
            }
            RawSegment::Code { body, language } => {
                let entity = cx.new(|cx| SelectableText::new(body, theme::TOKENS.fg, cx));
                MessageSegment::Code {
                    body: entity,
                    language,
                }
            }
        })
        .collect()
}

/// `# `/`## `/`### ` 만 인식. 더 깊은 헤딩은 무시.
fn heading_level(line: &str) -> Option<u8> {
    if line.starts_with("### ") {
        Some(3)
    } else if line.starts_with("## ") {
        Some(2)
    } else if line.starts_with("# ") {
        Some(1)
    } else {
        None
    }
}

/// 들여쓰기 0~3 + `- ` 또는 `* ` 로 시작하면 list item.
fn list_item_body(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some(rest.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return Some(rest.to_string());
    }
    None
}

#[cfg(test)]
mod markdown_tests {
    use super::*;

    #[test]
    fn status_label_tracks_reasoning_before_answer() {
        assert_eq!(
            turn_status_label(false, false, false, 0, 0),
            "세션 여는 중…"
        );
        assert_eq!(turn_status_label(true, false, false, 0, 0), "대기");
        assert_eq!(
            turn_status_label(true, true, false, 0, 0),
            "응답 기다리는 중"
        );
        assert_eq!(turn_status_label(true, true, false, 10, 0), "생각 중");
        assert_eq!(
            turn_status_label(true, true, false, LONG_REASONING_CHARS, 0),
            "생각이 길어지는 중"
        );
        assert_eq!(
            turn_status_label(true, true, false, 10_000, 1),
            "답변 작성 중"
        );
        assert_eq!(
            turn_status_label(true, true, true, 10_000, 0),
            "중단 요청 중"
        );
    }

    #[test]
    fn session_started_message_is_user_facing() {
        let isolated = session_started_message(WorkspaceMode::Isolated);
        assert!(isolated.contains("작업공간 준비됨"));
        assert!(isolated.contains("원본 폴더는 그대로"));
        assert!(!isolated.contains("branch"));
        assert!(!isolated.contains("worktree"));

        let direct = session_started_message(WorkspaceMode::Direct);
        assert_eq!(direct, "작업공간 준비됨\n에이전트 연결됨");
    }

    #[test]
    fn model_route_label_shows_main_and_sub_roles() {
        let same = Settings {
            provider: "local-vllm".into(),
            model: "qwen".into(),
            main_model: "qwen".into(),
            sub_model: "qwen".into(),
            recent_projects: Vec::new(),
        };
        assert_eq!(model_route_label(&same), "조율/작업 qwen");

        let split = Settings {
            provider: "local-vllm".into(),
            model: "planner".into(),
            main_model: "planner".into(),
            sub_model: "worker".into(),
            recent_projects: Vec::new(),
        };
        assert_eq!(model_route_label(&split), "조율 planner · 작업 worker");
    }

    #[test]
    fn inline_code_strips_markers_and_records_span() {
        let parsed = parse_inline_markdown("use `cargo test` now");

        assert_eq!(parsed.text, "use cargo test now");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 4..14,
                kind: InlineKind::Code,
            }]
        );
    }

    #[test]
    fn inline_bold_strips_markers_and_records_span() {
        let parsed = parse_inline_markdown("this is **important**");

        assert_eq!(parsed.text, "this is important");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 8..17,
                kind: InlineKind::Bold,
            }]
        );
    }

    #[test]
    fn inline_link_keeps_url_visible_and_records_span() {
        let parsed = parse_inline_markdown("see [docs](https://example.test)");

        assert_eq!(parsed.text, "see docs (https://example.test)");
        assert_eq!(
            parsed.spans,
            vec![InlineSpan {
                range: 4..31,
                kind: InlineKind::Link,
            }]
        );
    }

    #[test]
    fn unmatched_inline_markers_stay_literal() {
        let parsed = parse_inline_markdown("bad `code and **bold");

        assert_eq!(parsed.text, "bad `code and **bold");
        assert!(parsed.spans.is_empty());
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_markdown_raw(""), vec![]);
    }

    #[test]
    fn plain_paragraph() {
        let segs = parse_markdown_raw("hello world");
        assert_eq!(segs, vec![RawSegment::Paragraph("hello world".into())]);
    }

    #[test]
    fn fenced_code_with_lang() {
        let segs = parse_markdown_raw("```python\nprint('hi')\n```");
        assert_eq!(
            segs,
            vec![RawSegment::Code {
                body: "print('hi')".into(),
                language: Some("python".into())
            }]
        );
    }

    #[test]
    fn heading_levels() {
        let segs = parse_markdown_raw("# h1\n## h2\n### h3");
        assert_eq!(
            segs,
            vec![
                RawSegment::Heading {
                    level: 1,
                    body: "h1".into()
                },
                RawSegment::Heading {
                    level: 2,
                    body: "h2".into()
                },
                RawSegment::Heading {
                    level: 3,
                    body: "h3".into()
                },
            ]
        );
    }

    #[test]
    fn list_dash_and_star() {
        let segs = parse_markdown_raw("- one\n* two\n  - nested");
        assert_eq!(
            segs,
            vec![
                RawSegment::ListItem("one".into()),
                RawSegment::ListItem("two".into()),
                RawSegment::ListItem("nested".into()),
            ]
        );
    }

    #[test]
    fn paragraph_break_on_blank_line() {
        let segs = parse_markdown_raw("para 1\n\npara 2");
        assert_eq!(
            segs,
            vec![
                RawSegment::Paragraph("para 1".into()),
                RawSegment::Paragraph("para 2".into()),
            ]
        );
    }

    #[test]
    fn mixed_kitchen_sink() {
        let raw = "# 안녕\n\n첫 단락.\n\n- 항목 1\n- 항목 2\n\n```rs\nfn main() {}\n```\n\n끝.";
        let segs = parse_markdown_raw(raw);
        assert_eq!(
            segs,
            vec![
                RawSegment::Heading {
                    level: 1,
                    body: "안녕".into()
                },
                RawSegment::Paragraph("첫 단락.".into()),
                RawSegment::ListItem("항목 1".into()),
                RawSegment::ListItem("항목 2".into()),
                RawSegment::Code {
                    body: "fn main() {}".into(),
                    language: Some("rs".into())
                },
                RawSegment::Paragraph("끝.".into()),
            ]
        );
    }

    #[test]
    fn unclosed_code_block_treated_as_code() {
        let segs = parse_markdown_raw("```\nhello\nworld");
        assert_eq!(
            segs,
            vec![RawSegment::Code {
                body: "hello\nworld".into(),
                language: None
            }]
        );
    }
}

// ─── ChatItem 헬퍼 ──────────────────────────────────────

fn last_agent_message_text(messages: &mut [ChatItem]) -> Option<Entity<SelectableText>> {
    messages.iter_mut().rev().find_map(|m| match m {
        ChatItem::Message {
            who: Speaker::Agent,
            text,
            ..
        } => Some(text.clone()),
        _ => None,
    })
}

fn ensure_agent_reasoning(messages: &mut [ChatItem]) -> Option<Entity<SelectableText>> {
    let last = messages.last_mut()?;
    if let ChatItem::Message {
        who: Speaker::Agent,
        reasoning,
        ..
    } = last
    {
        if reasoning.is_some() {
            return reasoning.clone();
        }
        return None;
    }
    None
}

fn find_tool_output(messages: &mut [ChatItem], item_id: &str) -> Option<Entity<SelectableText>> {
    messages.iter_mut().rev().find_map(|m| match m {
        ChatItem::Tool {
            item_id: id,
            output,
            ..
        } if id == item_id => Some(output.clone()),
        _ => None,
    })
}

// ─── main ────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,stcode=debug".into()),
        )
        .init();

    let Bridge { cmd_tx, mut evt_rx } = Bridge::spawn();

    application().run(move |cx: &mut App| {
        selectable_text::init(cx);
        chat_input::init(cx);
        let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
        let main_view_handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| MainView::new(cmd_tx)),
            )
            .expect("윈도우 생성 실패");
        cx.activate(true);

        cx.spawn(async move |cx| {
            while let Some(ev) = evt_rx.recv().await {
                let _ = main_view_handle.update(cx, |this, _window, cx| this.handle_event(ev, cx));
            }
        })
        .detach();
    });
}
