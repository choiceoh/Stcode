use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, IntoElement, MouseButton,
    ParentElement, Render, Styled, Window, WindowBounds, WindowOptions,
};

mod bridge;
mod theme;

use bridge::{Bridge, UiCommand, UiEvent};

const DEMO_PROMPT: &str = "한 줄로만 답해. 너 누구야?";

enum Screen {
    Welcome,
    Chat {
        project: PathBuf,
        messages: Vec<ChatMessage>,
        thread_started: bool,
        turn_in_flight: bool,
    },
}

struct ChatMessage {
    who: Speaker,
    text: String,
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
}

impl MainView {
    fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<UiCommand>) -> Self {
        Self {
            screen: Screen::Welcome,
            cmd_tx,
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
                this.screen = Screen::Chat {
                    project: path,
                    messages: vec![ChatMessage {
                        who: Speaker::System,
                        text: "세션을 여는 중…".into(),
                    }],
                    thread_started: false,
                    turn_in_flight: false,
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn send_demo(&mut self, cx: &mut Context<Self>) {
        let Screen::Chat { messages, turn_in_flight, thread_started, .. } = &mut self.screen
        else {
            return;
        };
        if !*thread_started || *turn_in_flight {
            return;
        }
        messages.push(ChatMessage {
            who: Speaker::User,
            text: DEMO_PROMPT.into(),
        });
        messages.push(ChatMessage {
            who: Speaker::Agent,
            text: String::new(),
        });
        *turn_in_flight = true;
        let _ = self.cmd_tx.send(UiCommand::SendText(DEMO_PROMPT.into()));
        cx.notify();
    }

    fn handle_event(&mut self, ev: UiEvent, cx: &mut Context<Self>) {
        let Screen::Chat {
            messages,
            thread_started,
            turn_in_flight,
            ..
        } = &mut self.screen
        else {
            return;
        };
        match ev {
            UiEvent::Started { thread_id } => {
                *thread_started = true;
                messages.clear();
                messages.push(ChatMessage {
                    who: Speaker::System,
                    text: format!("세션 시작됨 ({thread_id})"),
                });
            }
            UiEvent::AgentDelta(text) => {
                if let Some(last) = messages.last_mut() {
                    if last.who == Speaker::Agent {
                        last.text.push_str(&text);
                    } else {
                        messages.push(ChatMessage {
                            who: Speaker::Agent,
                            text,
                        });
                    }
                }
            }
            UiEvent::TurnDone { ok, error_text } => {
                *turn_in_flight = false;
                if !ok {
                    messages.push(ChatMessage {
                        who: Speaker::System,
                        text: format!("⚠ turn 실패: {}", error_text.unwrap_or_default()),
                    });
                }
            }
            UiEvent::Error(text) => {
                *turn_in_flight = false;
                messages.push(ChatMessage {
                    who: Speaker::System,
                    text: format!("⚠ {text}"),
                });
            }
        }
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
                ..
            } => render_chat(t, project, messages, *thread_started, *turn_in_flight, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
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
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let project_label = project
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.to_string_lossy().into_owned());

    let send_enabled = thread_started && !turn_in_flight;
    let send_label = if turn_in_flight { "⏳ 응답 중…" } else { "🐾 데모 보내기" };
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
                .flex()
                .flex_col()
                .flex_1()
                .gap_3()
                .p_4()
                .overflow_hidden()
                .children(messages.iter().map(|m| render_message(t, m))),
        )
        .child(
            div()
                .flex()
                .h_16()
                .px_4()
                .items_center()
                .gap_3()
                .bg(rgb(t.surface))
                .border_t_1()
                .border_color(rgb(0x383848))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(rgb(t.muted))
                        .child(format!("(M1.1: 데모 프롬프트 hardcoded — \"{DEMO_PROMPT}\")")),
                )
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .bg(rgb(send_color))
                        .text_color(rgb(0x111122))
                        .rounded_md()
                        .when(send_enabled, |d| d.cursor_pointer())
                        .child(send_label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.send_demo(cx)),
                        ),
                ),
        )
}

fn render_message(t: &theme::Tokens, m: &ChatMessage) -> gpui::Div {
    let (icon, color) = match m.who {
        Speaker::User => ("🧑", t.fg),
        Speaker::Agent => ("🤖", t.fg),
        Speaker::System => ("ℹ", t.muted),
    };
    div()
        .flex()
        .gap_2()
        .text_color(rgb(color))
        .child(div().w_6().child(icon))
        .child(
            div()
                .flex_1()
                .child(if m.text.is_empty() { "▏".into() } else { m.text.clone() }),
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

    Application::new().run(move |cx: &mut App| {
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
