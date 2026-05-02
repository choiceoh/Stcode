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
        messages: Vec<ChatMessage>,
        thread_started: bool,
        turn_in_flight: bool,
        input: Entity<ChatInput>,
    },
}

struct ChatMessage {
    who: Speaker,
    text: Entity<SelectableText>,
}

impl ChatMessage {
    fn new(who: Speaker, text: impl Into<gpui::SharedString>, cx: &mut Context<MainView>) -> Self {
        let color = color_for(who);
        let s = text.into();
        let entity = cx.new(|cx| SelectableText::new(s, color, cx));
        Self { who, text: entity }
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
                let intro = ChatMessage::new(Speaker::System, "세션을 여는 중…", cx);
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

        let user_msg = ChatMessage::new(Speaker::User, text.clone(), cx);
        let agent_msg = ChatMessage::new(Speaker::Agent, "", cx);
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
                let intro = ChatMessage::new(
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
                    match messages.last() {
                        Some(last) if last.who == Speaker::Agent => {
                            let entity = last.text.clone();
                            entity.update(cx, |this, cx| this.append(&text, cx));
                        }
                        _ => new_msg = Some(()),
                    }
                }
                if new_msg.is_some() {
                    let m = ChatMessage::new(Speaker::Agent, text, cx);
                    if let Screen::Chat { messages, .. } = &mut self.screen {
                        messages.push(m);
                    }
                }
            }
            UiEvent::TurnDone { ok, error_text } => {
                let err_msg = if !ok {
                    Some(ChatMessage::new(
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
                let m = ChatMessage::new(Speaker::System, format!("⚠ {text}"), cx);
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
    messages: &[ChatMessage],
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
                .children(messages.iter().map(|m| render_message(t, m))),
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

fn render_message(t: &theme::Tokens, m: &ChatMessage) -> gpui::Div {
    let (icon, bubble_bg, _is_user) = match m.who {
        Speaker::User => ("🧑", 0x2a3050, true),
        Speaker::Agent => ("🤖", t.surface, false),
        Speaker::System => ("ℹ", 0x252535, false),
    };
    div()
        .flex()
        .gap_2()
        .items_start()
        .child(div().w_6().mt_1().child(icon))
        .child(
            div()
                .flex_1()
                .px_3()
                .py_2()
                .bg(rgb(bubble_bg))
                .rounded_md()
                .child(m.text.clone()),
        )
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
