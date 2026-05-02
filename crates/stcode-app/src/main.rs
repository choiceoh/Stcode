use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, Entity, IntoElement, MouseButton,
    ParentElement, Render, ScrollHandle, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

mod chat_input;
mod selectable_text;
mod theme;

use chat_input::ChatInput;
use selectable_text::SelectableText;
use stcode_codex::bridge::{Bridge, UiCommand, UiEvent};

enum Screen {
    Welcome,
    Chat {
        project: PathBuf,
        messages: Vec<ChatItem>,
        thread_started: bool,
        turn_in_flight: bool,
        input: Entity<ChatInput>,
    },
}

/// 채팅 영역의 한 항목. Message(사용자/agent/system) / Tool 카드 두 종류.
enum ChatItem {
    Message {
        who: Speaker,
        text: Entity<SelectableText>,
        /// Agent 메시지의 reasoning(별도 회색 영역). None이면 표시 안 함.
        reasoning: Option<Entity<SelectableText>>,
    },
    Tool {
        item_id: String,
        kind: stcode_codex::bridge::ToolKind,
        title: String,
        output: Entity<SelectableText>,
        status: ToolStatus,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolStatus {
    InProgress,
    CompletedOk,
    CompletedFail,
}

impl ChatItem {
    fn message(who: Speaker, text: impl Into<gpui::SharedString>, cx: &mut Context<MainView>) -> Self {
        let s = text.into();
        let color = color_for(who);
        let entity = cx.new(|cx| SelectableText::new(s, color, cx));
        Self::Message {
            who,
            text: entity,
            reasoning: None,
        }
    }

    fn tool(
        item_id: String,
        kind: stcode_codex::bridge::ToolKind,
        title: String,
        cx: &mut Context<MainView>,
    ) -> Self {
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

struct MainView {
    screen: Screen,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>,
    messages_scroll: ScrollHandle,
}

impl MainView {
    fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>) -> Self {
        Self {
            screen: Screen::Welcome,
            cmd_tx,
            messages_scroll: ScrollHandle::new(),
        }
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        // 중요: GPUI listener 안에서 sync rfd::pick_folder를 호출하면 macOS modal이
        // 키보드 레이아웃 등 시스템 알림을 발생시켜 GPUI App을 재진입(borrow_mut) 하면서
        // RefCell double-borrow panic. 반드시 cx.spawn으로 분리해야 한다.
        cx.spawn(async move |this, cx| {
            let handle = rfd::AsyncFileDialog::new()
                .set_title("프로젝트 폴더 선택")
                .pick_folder()
                .await;
            let Some(handle) = handle else { return };
            let path = handle.path().to_path_buf();
            tracing::info!("프로젝트 폴더 선택: {}", path.display());
            let _ = this.update(cx, |this, cx| {
                let _ = this.cmd_tx.send(UiCommand::StartProject { path: path.clone() });
                let intro = ChatItem::message(Speaker::System, "세션을 여는 중…", cx);
                let input = cx.new(|cx| {
                    ChatInput::new("무엇을 만들까요?", theme::TOKENS.fg, theme::TOKENS.muted, cx)
                });
                this.screen = Screen::Chat {
                    project: path,
                    messages: vec![intro],
                    thread_started: false,
                    turn_in_flight: false,
                    input,
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn send_user_input(&mut self, cx: &mut Context<Self>) {
        let Screen::Chat {
            thread_started,
            turn_in_flight,
            input,
            ..
        } = &self.screen
        else {
            return;
        };
        if !*thread_started || *turn_in_flight {
            return;
        }
        let text = input.read(cx).content().to_string();
        if text.trim().is_empty() {
            return;
        }
        let input_entity = input.clone();
        input_entity.update(cx, |this, cx| this.clear(cx));

        let user_msg = ChatItem::message(Speaker::User, text.clone(), cx);
        let agent_msg = ChatItem::message(Speaker::Agent, "", cx);
        if let Screen::Chat {
            messages,
            turn_in_flight,
            ..
        } = &mut self.screen
        {
            messages.push(user_msg);
            messages.push(agent_msg);
            *turn_in_flight = true;
        }
        let _ = self.cmd_tx.send(UiCommand::SendText(text));
        cx.notify();
    }

    fn handle_event(&mut self, ev: UiEvent, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat { .. }) {
            return;
        }
        match ev {
            UiEvent::Started { thread_id } => {
                let intro = ChatItem::message(
                    Speaker::System,
                    format!("세션 시작됨 ({thread_id})"),
                    cx,
                );
                if let Screen::Chat {
                    messages,
                    thread_started,
                    ..
                } = &mut self.screen
                {
                    *thread_started = true;
                    messages.clear();
                    messages.push(intro);
                }
            }
            UiEvent::AgentDelta(text) => {
                let mut new_msg = None;
                if let Screen::Chat { messages, .. } = &mut self.screen {
                    match last_agent_message_text(messages) {
                        Some(entity) => {
                            entity.update(cx, |this, cx| this.append(&text, cx));
                        }
                        None => new_msg = Some(()),
                    }
                }
                if new_msg.is_some() {
                    let m = ChatItem::message(Speaker::Agent, text, cx);
                    if let Screen::Chat { messages, .. } = &mut self.screen {
                        messages.push(m);
                    }
                }
            }
            UiEvent::ReasoningDelta(text) => {
                let mut create_new = None;
                if let Screen::Chat { messages, .. } = &mut self.screen {
                    if let Some(reasoning_entity) = ensure_agent_reasoning(messages) {
                        reasoning_entity.update(cx, |this, cx| this.append(&text, cx));
                    } else {
                        create_new = Some(text);
                    }
                }
                if let Some(text) = create_new {
                    // 마지막이 Agent Message가 아닐 때 — agent 빈 메시지 만들고 reasoning 시작.
                    let mut agent_msg = ChatItem::message(Speaker::Agent, "", cx);
                    if let ChatItem::Message { reasoning, .. } = &mut agent_msg {
                        let r = cx.new(|cx| SelectableText::new(text, theme::TOKENS.muted, cx));
                        *reasoning = Some(r);
                    }
                    if let Screen::Chat { messages, .. } = &mut self.screen {
                        messages.push(agent_msg);
                    }
                }
            }
            UiEvent::ToolStarted {
                item_id,
                kind,
                title,
            } => {
                let card = ChatItem::tool(item_id, kind, title, cx);
                if let Screen::Chat { messages, .. } = &mut self.screen {
                    messages.push(card);
                }
            }
            UiEvent::ToolOutput { item_id, delta } => {
                if let Screen::Chat { messages, .. } = &mut self.screen {
                    if let Some(out) = find_tool_output(messages, &item_id) {
                        out.update(cx, |this, cx| this.append(&delta, cx));
                    }
                }
            }
            UiEvent::ToolCompleted {
                item_id,
                ok,
                summary,
            } => {
                if let Screen::Chat { messages, .. } = &mut self.screen {
                    if let Some(item) = messages.iter_mut().rev().find(|m| match m {
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
                }
            }
            UiEvent::TurnDone { ok, error_text } => {
                let err_msg = if !ok {
                    Some(ChatItem::message(
                        Speaker::System,
                        format!("⚠ turn 실패: {}", error_text.unwrap_or_default()),
                        cx,
                    ))
                } else {
                    None
                };
                if let Screen::Chat {
                    messages,
                    turn_in_flight,
                    ..
                } = &mut self.screen
                {
                    *turn_in_flight = false;
                    if let Some(m) = err_msg {
                        messages.push(m);
                    }
                }
            }
            UiEvent::Error(text) => {
                let m = ChatItem::message(Speaker::System, format!("⚠ {text}"), cx);
                if let Screen::Chat {
                    messages,
                    turn_in_flight,
                    ..
                } = &mut self.screen
                {
                    *turn_in_flight = false;
                    messages.push(m);
                }
            }
        }
        // 새 메시지/델타 후 자동 scroll to bottom
        self.messages_scroll.scroll_to_bottom();
        cx.notify();
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &theme::TOKENS;
        let body = match &self.screen {
            Screen::Welcome => render_welcome(t, cx),
            Screen::Chat {
                project,
                messages,
                thread_started,
                turn_in_flight,
                input,
            } => render_chat(
                t,
                project,
                messages,
                *thread_started,
                *turn_in_flight,
                input.clone(),
                self.messages_scroll.clone(),
                cx,
            ),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
            // ChatInput Submit 액션 — 어디서 발생하든 잡아서 send.
            .on_action(cx.listener(|this, _: &chat_input::Submit, _, cx| this.send_user_input(cx)))
            .child(body)
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

fn render_chat(
    t: &theme::Tokens,
    project: &PathBuf,
    messages: &[ChatItem],
    thread_started: bool,
    turn_in_flight: bool,
    input: Entity<ChatInput>,
    scroll: ScrollHandle,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let project_label = project
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.to_string_lossy().into_owned());

    let send_enabled = thread_started && !turn_in_flight;
    let send_label = if turn_in_flight { "⏳ 응답 중…" } else { "↵ 보내기" };
    let send_color = if send_enabled { t.accent } else { 0x555566 };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .h_10()
                .px_4()
                .items_center()
                .bg(rgb(t.surface))
                .border_b_1()
                .border_color(rgb(0x383848))
                .child(div().text_sm().child(format!("📁 {project_label}"))),
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
                .track_scroll(&scroll)
                .children(messages.iter().map(|m| render_chat_item(t, m))),
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
                .border_color(rgb(0x383848))
                .child(
                    div()
                        .flex_1()
                        .px_3()
                        .py_2()
                        .bg(rgb(t.bg))
                        .border_1()
                        .border_color(rgb(0x383848))
                        .rounded_md()
                        .child(input),
                )
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(send_color))
                        .text_color(rgb(0x111122))
                        .rounded_md()
                        .when(send_enabled, |d| {
                            d.cursor_pointer()
                                .hover(|d| d.bg(rgb(0xa0c0ff)))
                        })
                        .child(send_label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.send_user_input(cx)),
                        ),
                ),
        )
}

fn render_chat_item(t: &theme::Tokens, item: &ChatItem) -> gpui::Div {
    match item {
        ChatItem::Message {
            who,
            text,
            reasoning,
        } => render_message(t, *who, text.clone(), reasoning.clone()),
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
    body = body.child(
        div()
            .px_3()
            .py_2()
            .bg(rgb(bubble_bg))
            .rounded_md()
            .child(text),
    );
    div()
        .flex()
        .gap_2()
        .items_start()
        .child(div().w_6().mt_1().child(icon))
        .child(body)
}

fn render_tool_card(
    t: &theme::Tokens,
    kind: stcode_codex::bridge::ToolKind,
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

fn ensure_agent_reasoning(messages: &mut Vec<ChatItem>) -> Option<Entity<SelectableText>> {
    // 마지막이 Agent Message면 그 reasoning entity 반환 (없으면 None — 호출자가 만들어 push)
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
        // 빈 entity 만들 수 없음 — 호출자가 cx로 새로 만들어야. None 반환해서 시그널.
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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,stcode=debug".into()),
        )
        .init();

    // tokio 런타임 + codex 세션을 별도 스레드에서. cmd/evt 채널만 노출.
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

        // evt_rx → MainView 펌프. GPUI executor 위에서 await — tokio mpsc는 Send/Sync 안전.
        cx.spawn(async move |cx| {
            while let Some(ev) = evt_rx.recv().await {
                let _ = main_view_handle
                    .update(cx, |this, _window, cx| this.handle_event(ev, cx));
            }
        })
        .detach();
    });
}
