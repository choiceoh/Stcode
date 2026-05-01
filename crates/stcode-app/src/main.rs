use gpui::{
    App, Application, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};

mod theme;

struct MainView;

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = &theme::TOKENS;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.fg))
            .p_8()
            .gap_4()
            .child(div().text_2xl().child("Stcode"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(t.muted))
                    .child("바이브코더용 코딩 에이전트 — M0 scaffolding"),
            )
            .child(
                div()
                    .mt_8()
                    .p_4()
                    .bg(rgb(t.surface))
                    .rounded_md()
                    .child("창이 떴다면 GPUI 의존성은 정상입니다."),
            )
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,stcode=debug".into()),
        )
        .init();

    // M0: codex 핸드셰이크는 별도 thread에서 실행해 콘솔에 결과를 찍는다.
    // GUI와 결합은 M1에서.
    std::thread::spawn(|| {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("tokio runtime 생성 실패: {e}");
                return;
            }
        };
        rt.block_on(async {
            match stcode_codex::probe_initialize().await {
                Ok(info) => tracing::info!("codex initialize OK: {info:?}"),
                Err(e) => tracing::warn!("codex initialize 실패 (M0에선 정상일 수 있음): {e}"),
            }
        });
    });

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MainView),
        )
        .expect("윈도우 생성 실패");
        cx.activate(true);
    });
}
