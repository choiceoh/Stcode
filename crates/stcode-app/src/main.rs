use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    App, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

mod app_state;
mod chat_input;
mod markdown;
mod render;
mod selectable_text;
mod theme;

use app_state::{
    ChatItem, LastCommit, PendingApproval, Screen, SessionUiState, SettingsDraft, Speaker,
    ToolStatus, WorkspaceState, ensure_agent_reasoning, find_tool_output, last_agent_message_text,
};
use chat_input::ChatInput;
use selectable_text::SelectableText;
use stcode_codex::bridge::{ApprovalDecision, Bridge, SessionId, UiCommand, UiEvent};
use stcode_vibe::{AgentModelRole, Settings, friendly_translate, settings};

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
                session_prefix: session_run_prefix(),
                next_id: 0,
            });
        }
        let Screen::Workspace(ws) = &mut self.screen else {
            return;
        };
        ws.next_id += 1;
        let session_id = next_session_id(&ws.session_prefix, ws.next_id);
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
                thread_id,
                workspace_mode,
                workspace,
            } => {
                let intro = ChatItem::message(
                    Speaker::System,
                    render::session_started_message(workspace_mode),
                    cx,
                );
                self.with_session(&session_id, |s| {
                    s.thread_started = true;
                    s.session_failed = false;
                    s.thread_id = Some(thread_id);
                    s.workspace_mode = Some(workspace_mode);
                    s.workspace = Some(workspace);
                    s.messages.clear();
                    s.messages.push(intro);
                });
            }
            UiEvent::SessionFailed { session_id, error } => {
                let friendly = friendly_translate(&error);
                let m =
                    ChatItem::message(Speaker::System, format!("세션 시작 실패\n{friendly}"), cx);
                self.with_session(&session_id, |s| {
                    s.session_failed = true;
                    s.messages.push(m);
                });
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
                            markdown::finalize_last_agent_message_markdown(s, cx);
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

fn session_run_prefix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("s{:x}-{:x}", process::id(), millis)
}

fn next_session_id(prefix: &str, next_id: u32) -> SessionId {
    format!("{prefix}-{next_id}")
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &theme::TOKENS;
        let display_model = render::model_route_label(&self.settings);
        let body = match &self.screen {
            Screen::Welcome => {
                render::render_welcome(t, &display_model, &self.settings.recent_projects, cx)
            }
            Screen::Workspace(ws) => {
                render::render_workspace(t, ws, &display_model, &self.settings.recent_projects, cx)
            }
        };
        let approval_modal = self
            .pending_approval
            .as_ref()
            .map(|p| render::render_approval_modal(t, p, cx));
        let settings_modal = self
            .settings_draft
            .as_ref()
            .map(|d| render::render_settings_modal(t, d, cx));
        let notice_modal = self
            .notice
            .as_ref()
            .map(|n| render::render_notice_modal(t, n.clone(), cx));
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
            .text_size(px(14.))
            .line_height(px(20.))
            .on_action(cx.listener(|this, _: &chat_input::Submit, _, cx| this.send_user_input(cx)))
            .child(body)
            .when_some(approval_modal, |d, m| d.child(m))
            .when_some(settings_modal, |d, m| d.child(m))
            .when_some(notice_modal, |d, m| d.child(m))
    }
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
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
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
