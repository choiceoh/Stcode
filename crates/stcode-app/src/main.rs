use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, Entity, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Render, ScrollHandle, SharedString, Styled, Window, WindowBounds,
    WindowOptions,
};
use gpui_platform::application;

mod chat_input;
mod selectable_text;
mod theme;

use chat_input::ChatInput;
use selectable_text::SelectableText;
use stcode_codex::bridge::{ApprovalDecision, Bridge, SessionId, ToolKind, UiCommand, UiEvent};
use stcode_vibe::friendly_translate;

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
    input: Entity<ChatInput>,
    last_commit: Option<LastCommit>,
    /// active 가 아닌 세션에서 새 message/델타가 와서 unread 표식.
    has_unread: bool,
    /// 메시지 영역 별 ScrollHandle — 세션마다 따로 스크롤 위치 유지.
    scroll: ScrollHandle,
}

impl SessionUiState {
    fn new(project: PathBuf, cx: &mut Context<MainView>) -> Self {
        let intro = ChatItem::message(Speaker::System, "세션을 여는 중…", cx);
        let input =
            cx.new(|cx| ChatInput::new("무엇을 만들까요?", theme::TOKENS.fg, theme::TOKENS.muted, cx));
        Self {
            project,
            messages: vec![intro],
            thread_started: false,
            turn_in_flight: false,
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

/// Markdown 파싱된 한 조각. line/block-level만. inline (코드/볼드/링크) 은 다음.
enum MessageSegment {
    /// 일반 텍스트 paragraph (줄바꿈 포함).
    Paragraph(Entity<SelectableText>),
    /// `# heading` `## heading` `### heading`. level=1..3.
    Heading {
        level: u8,
        body: Entity<SelectableText>,
    },
    /// `- item` 또는 `* item`. body는 bullet 제외한 본문.
    ListItem {
        body: Entity<SelectableText>,
    },
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

// ─── MainView ────────────────────────────────────────────

struct MainView {
    screen: Screen,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>,
    pending_approval: Option<PendingApproval>,
}

impl MainView {
    fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>) -> Self {
        Self {
            screen: Screen::Welcome,
            cmd_tx,
            pending_approval: None,
        }
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

    fn add_new_session(&mut self, path: PathBuf, cx: &mut Context<Self>) {
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
        let _ = self.cmd_tx.send(UiCommand::NewSession { session_id, path });
        cx.notify();
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
        let _ = self.cmd_tx.send(UiCommand::CloseSession { session_id: sid });
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
        let _ = self.cmd_tx.send(UiCommand::RevertLastTurn { session_id: sid });
        cx.notify();
    }

    fn send_user_input(&mut self, cx: &mut Context<Self>) {
        let Screen::Workspace(ws) = &self.screen else {
            return;
        };
        let Some(sid) = ws.active.clone() else { return };
        let Some(s) = ws.sessions.get(&sid) else { return };
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
                thread_id,
            } => {
                let intro =
                    ChatItem::message(Speaker::System, format!("세션 시작 ({thread_id})"), cx);
                self.with_session(&session_id, |s| {
                    s.thread_started = true;
                    s.messages.clear();
                    s.messages.push(intro);
                });
            }
            UiEvent::SessionFailed { session_id, error } => {
                let friendly = friendly_translate(&error);
                let m = ChatItem::message(
                    Speaker::System,
                    format!("세션 시작 실패\n{friendly}"),
                    cx,
                );
                self.with_session(&session_id, |s| s.messages.push(m));
            }
            UiEvent::SessionClosed { session_id } => {
                // 사이드바에서 close_session으로 이미 정리됨 — 이벤트는 확인용.
                tracing::info!("[{session_id}] 세션 닫힘");
            }
            UiEvent::AgentDelta { session_id, text } => {
                let mut new_msg = None;
                self.with_session(&session_id, |s| {
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
                    ChatItem::message(
                        Speaker::System,
                        format!("되돌리기 실패\n{friendly}"),
                        cx,
                    )
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
        let body = match &self.screen {
            Screen::Welcome => render_welcome(t, cx),
            Screen::Workspace(ws) => render_workspace(t, ws, cx),
        };
        let modal = self
            .pending_approval
            .as_ref()
            .map(|p| render_approval_modal(t, p, cx));
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
            .on_action(cx.listener(|this, _: &chat_input::Submit, _, cx| this.send_user_input(cx)))
            .child(body)
            .when_some(modal, |d, m| d.child(m))
    }
}

fn render_welcome(t: &theme::Tokens, cx: &mut Context<MainView>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap_8()
        .child(div().text_2xl().child("🚀 Stcode"))
        .child(
            div()
                .text_sm()
                .text_color(rgb(t.muted))
                .child("자연어로 시키면 코드를 만들어드려요"),
        )
        .child(
            div()
                .px_8()
                .py_4()
                .bg(rgb(t.surface))
                .rounded_md()
                .border_1()
                .border_color(rgb(t.accent))
                .cursor_pointer()
                .child("📁  폴더 열기")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.open_folder(cx)),
                ),
        )
}

fn render_workspace(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .size_full()
        .child(render_sidebar(t, ws, cx))
        .child(render_active_main(t, ws, cx))
}

/// 좌측 사이드바 — dynamic 세션 list. 클릭으로 active 전환. status icon 동기화.
fn render_sidebar(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let items: Vec<gpui::Div> = ws
        .order
        .iter()
        .filter_map(|sid| {
            let s = ws.sessions.get(sid)?;
            Some(render_session_item(t, sid.clone(), s, ws.active.as_ref() == Some(sid), cx))
        })
        .collect();

    let new_session_btn = div()
        .flex()
        .gap_2()
        .items_center()
        .px_3()
        .py_2()
        .text_sm()
        .text_color(rgb(t.muted))
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)).text_color(rgb(t.fg)))
        .child(div().w_4().text_xs().child("+"))
        .child(div().flex_1().child("새 세션"))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.open_folder(cx)),
        );

    div()
        .flex()
        .flex_col()
        .w(px(220.))
        .h_full()
        .bg(rgb(t.sidebar))
        .border_r_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .flex()
                .h_10()
                .px_4()
                .items_center()
                .border_b_1()
                .border_color(rgb(t.border))
                .text_sm()
                .text_color(rgb(t.muted))
                .child("🚀 Stcode"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .py_2()
                .children(items)
                .child(new_session_btn),
        )
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
        .px_3()
        .py_2()
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .child(
            div()
                .w_4()
                .text_xs()
                .text_color(rgb(t.muted))
                .child(status_icon),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .child(format!("📁 {project_label}")),
        )
        .child(
            div()
                .px_1()
                .text_xs()
                .text_color(rgb(t.muted))
                .cursor_pointer()
                .hover(|d| d.text_color(rgb(0xe0a0a0)))
                .child("✕")
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
        row = row.border_l_2().border_color(rgb(t.accent));
    }
    row
}

/// 현재 active 세션의 main panel. active 가 None이면 placeholder.
fn render_active_main(
    t: &theme::Tokens,
    ws: &WorkspaceState,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let Some(sid) = ws.active.clone() else {
        return div()
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(t.muted))
            .child("열린 세션이 없어요. 사이드바의 + 새 세션을 눌러주세요.");
    };
    let Some(s) = ws.sessions.get(&sid) else {
        return div().flex_1();
    };
    render_chat_main(t, s, cx)
}

fn render_chat_main(
    t: &theme::Tokens,
    s: &SessionUiState,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let send_enabled = s.thread_started && !s.turn_in_flight;
    let send_label = if s.turn_in_flight {
        "⏳ 응답 중…"
    } else {
        "↵ 보내기"
    };
    let send_color = if send_enabled { t.accent } else { 0x555566 };

    let revert_btn = s.last_commit.as_ref().filter(|c| c.revertible).map(|c| {
        let tooltip = format!("↶ 되돌리기 — {}", c.summary);
        div()
            .px_3()
            .py_1()
            .bg(rgb(0x2a2030))
            .text_color(rgb(0xe0c0d0))
            .text_xs()
            .rounded_md()
            .cursor_pointer()
            .child(tooltip)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.revert_active(cx)),
            )
    });

    let status_label = if !s.thread_started {
        "세션 여는 중…"
    } else if s.turn_in_flight {
        "응답 중"
    } else {
        "대기"
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .h_full()
        .child(
            div()
                .flex()
                .h_10()
                .px_4()
                .items_center()
                .gap_3()
                .bg(rgb(t.surface))
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(rgb(t.muted))
                        .child(status_label),
                )
                .when_some(revert_btn, |d, b| d.child(b)),
        )
        .child(
            div()
                .id("messages")
                .flex()
                .flex_col()
                .flex_1()
                .gap_3()
                .p_4()
                .overflow_y_scroll()
                .track_scroll(&s.scroll)
                .children(s.messages.iter().map(|m| render_chat_item(t, m))),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .px_4()
                .py_2()
                .bg(rgb(t.surface))
                .border_t_1()
                .border_color(rgb(t.border))
                .child(chip(t, "🤖 qwen3.6-35b-a3b"))
                .child(chip(t, "⚡ 자동 모드"))
                .child(chip(t, "📂 작업 폴더 자유")),
        )
        .child(
            div()
                .flex()
                .min_h_16()
                .px_4()
                .py_3()
                .items_start()
                .gap_3()
                .bg(rgb(t.surface))
                .border_t_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .flex_1()
                        .px_3()
                        .py_2()
                        .bg(rgb(t.bg))
                        .border_1()
                        .border_color(rgb(t.border))
                        .rounded_md()
                        .child(s.input.clone()),
                )
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(send_color))
                        .text_color(rgb(0x111122))
                        .rounded_md()
                        .when(send_enabled, |d| {
                            d.cursor_pointer().hover(|d| d.bg(rgb(0xa0c0ff)))
                        })
                        .child(send_label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.send_user_input(cx)),
                        ),
                ),
        )
}

fn chip(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .bg(rgb(t.bg))
        .text_xs()
        .text_color(rgb(t.muted))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_md()
        .child(label)
}

fn render_chat_item(t: &theme::Tokens, item: &ChatItem) -> gpui::Div {
    match item {
        ChatItem::Message {
            who,
            text,
            reasoning,
            segments,
        } => render_message(t, *who, text.clone(), reasoning.clone(), segments.as_deref()),
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
        Speaker::User => ("🧑", 0x2a3050),
        Speaker::Agent => ("🤖", t.surface),
        Speaker::System => ("ℹ", 0x252535),
    };
    let mut body = div().flex_1().flex().flex_col().gap_2();
    if let Some(r) = reasoning {
        body = body.child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(0x202028))
                .rounded_md()
                .border_l_2()
                .border_color(rgb(0x556677))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(div().mb_1().child("💭 사고"))
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
        .gap_2()
        .items_start()
        .child(div().w_6().mt_1().child(icon))
        .child(body)
}

fn render_segment(t: &theme::Tokens, seg: &MessageSegment, bubble_bg: u32) -> gpui::Div {
    match seg {
        MessageSegment::Paragraph(entity) => div()
            .px_3()
            .py_2()
            .bg(rgb(bubble_bg))
            .rounded_md()
            .child(entity.clone()),
        MessageSegment::Heading { level, body } => {
            // H1=2xl, H2=xl, H3=lg. 색은 약간 더 밝게 강조.
            let base = div()
                .px_3()
                .py_2()
                .text_color(rgb(0xffffff));
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
            // 코드 블록: mono font + 어두운 bg + 옅은 라벨 (language). language는 부모
            // div에 .font_family로 걸면 SelectableText가 자동 상속.
            let mut card = div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(rgb(0x10141a))
                .border_1()
                .border_color(rgb(0x2a3a4a))
                .rounded_md();
            if let Some(lang) = language {
                if !lang.is_empty() {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(rgb(t.muted))
                            .child(lang.clone()),
                    );
                }
            }
            card.font_family("Menlo")
                .text_color(rgb(0xc0d8e8))
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
        ToolStatus::InProgress => ("⏳", 0xb0b0c0),
        ToolStatus::CompletedOk => ("✅", 0x60d090),
        ToolStatus::CompletedFail => ("❌", 0xd07070),
    };
    div()
        .flex()
        .gap_2()
        .items_start()
        .child(div().w_6().mt_1().child(icon))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(rgb(0x1a1f2a))
                .rounded_md()
                .border_1()
                .border_color(rgb(0x303848))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_color(rgb(status_color)).child(status_label))
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(t.fg))
                                .child(title.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child(output),
                ),
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
        .bg(rgb(0x10101a))
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
                .border_color(rgb(t.accent))
                .rounded_md()
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(div().text_2xl().child(icon))
                        .child(
                            div()
                                .flex_1()
                                .text_lg()
                                .child(p.friendly_title.clone()),
                        ),
                )
                .when(show_detail, |d| {
                    d.child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(t.bg))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x303848))
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
    let ChatItem::Message {
        text, segments, ..
    } = item
    else {
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
    if raw.contains("```") {
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
    Heading { level: u8, body: String },
    ListItem(String),
    Code { body: String, language: Option<String> },
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

/// RawSegment 들에서 SelectableText entity를 만들어 MessageSegment 로 변환.
fn parse_markdown_segments(raw: &str, cx: &mut Context<MainView>) -> Vec<MessageSegment> {
    parse_markdown_raw(raw)
        .into_iter()
        .map(|raw_seg| match raw_seg {
            RawSegment::Paragraph(text) => {
                let entity = cx.new(|cx| SelectableText::new(text, theme::TOKENS.fg, cx));
                MessageSegment::Paragraph(entity)
            }
            RawSegment::Heading { level, body } => {
                let entity = cx.new(|cx| SelectableText::new(body, theme::TOKENS.fg, cx));
                MessageSegment::Heading { level, body: entity }
            }
            RawSegment::ListItem(body) => {
                let entity = cx.new(|cx| SelectableText::new(body, theme::TOKENS.fg, cx));
                MessageSegment::ListItem { body: entity }
            }
            RawSegment::Code { body, language } => {
                let entity = cx.new(|cx| SelectableText::new(body, 0xc0d8e8, cx));
                MessageSegment::Code { body: entity, language }
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
                RawSegment::Heading { level: 1, body: "h1".into() },
                RawSegment::Heading { level: 2, body: "h2".into() },
                RawSegment::Heading { level: 3, body: "h3".into() },
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
                RawSegment::Heading { level: 1, body: "안녕".into() },
                RawSegment::Paragraph("첫 단락.".into()),
                RawSegment::ListItem("항목 1".into()),
                RawSegment::ListItem("항목 2".into()),
                RawSegment::Code { body: "fn main() {}".into(), language: Some("rs".into()) },
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
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
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
