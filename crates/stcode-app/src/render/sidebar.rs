use super::*;

pub(crate) fn render_welcome(
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
        "+",
        "새 작업",
        cx.listener(|this, _, _, cx| this.open_folder(cx)),
    )
    .bg(rgb(t.sidebar_active));
    let settings_btn = sidebar_settings_row(
        t,
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(236.))
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
                .child(new_session_btn),
        )
        .child(render_recent_project_section(t, recent_projects, cx))
        .child(div().flex_1())
        .child(render_sidebar_footer(t, settings_btn))
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
                .h(px(46.))
                .px_4()
                .items_center()
                .border_b_1()
                .border_color(rgb(t.border))
                .child(top_bar_title(t, "새 작업"))
                .child(div().flex_1())
                .child(top_bar_controls(t, chips_model, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .items_center()
                .justify_start()
                .px_5()
                .pt(px(40.))
                .pb_4()
                .child(render_welcome_deck(t, recent_projects, cx)),
        )
}

fn render_welcome_deck(
    t: &theme::Tokens,
    recent_projects: &[String],
    cx: &mut Context<MainView>,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .h_full()
        .max_w(px(940.))
        .child(
            div().flex().items_end().gap_4().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(t.fg))
                            .child("작업 보드"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(t.muted))
                            .child("대기 중인 작업 없음"),
                    ),
            ),
        )
        .child(render_agent_capacity_strip(t))
        .when(!recent_projects.is_empty(), |d| {
            d.child(render_recent_projects_panel(t, recent_projects, cx))
        })
        .child(div().flex_1())
        .child(welcome_composer(t, cx))
}

fn render_agent_capacity_strip(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .px_3()
        .py_2()
        .bg(rgb(t.surface))
        .rounded_md()
        .border_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(t.fg))
                        .child("대기열"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.muted))
                        .child("열린 작업 없음"),
                ),
        )
        .child(welcome_signal(t, "동시 작업", "0 / 10"))
        .child(welcome_signal(t, "기록 정리", "자동"))
        .child(welcome_signal(t, "모델 역할", "분리"))
}

fn welcome_signal(t: &theme::Tokens, label: &'static str, value: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap_1()
        .min_w(px(64.))
        .child(div().text_xs().text_color(rgb(t.muted)).child(label))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(t.fg))
                .child(value),
        )
}

fn welcome_composer(t: &theme::Tokens, cx: &mut Context<MainView>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .max_w(px(940.))
        .min_h(px(68.))
        .px_3()
        .py_2()
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.border_color(rgb(0xc4c4ca)))
        .child(
            div()
                .text_sm()
                .text_color(rgb(t.muted))
                .child("작업할 프로젝트를 선택하세요"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(composer_status_text(t, "프로젝트 선택"))
                .child(div().flex_1())
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
        .flex_1()
        .gap_1()
        .w_full()
        .max_w(px(940.))
        .child(section_label("최근 프로젝트"))
        .children(
            recent_projects
                .iter()
                .take(4)
                .cloned()
                .map(|path| render_recent_project_row(t, path, true, cx)),
        )
}

pub(crate) fn render_workspace(
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
        .child(super::chat::render_active_main(t, ws, chips_model, cx))
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
        "+",
        "새 작업",
        cx.listener(|this, _, _, cx| this.open_folder(cx)),
    );
    let settings_btn = sidebar_settings_row(
        t,
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(236.))
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
                .child(new_session_btn),
        )
        .child(render_recent_project_section(t, recent_projects, cx))
        .child(
            div()
                .px_4()
                .pt_3()
                .pb_1()
                .text_xs()
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
        .child(render_sidebar_footer(t, settings_btn))
}

fn sidebar_brand(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .h(px(46.))
        .px_3()
        .items_center()
        .gap_2()
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(t.fg))
        .child("Stcode")
}

fn top_bar_title(t: &theme::Tokens, title: &'static str) -> gpui::Div {
    div().flex().items_center().gap_2().child(
        div()
            .text_size(px(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(t.fg))
            .child(title),
    )
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
        .mx_2()
        .px_2()
        .py_1()
        .text_color(rgb(t.fg))
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .w(px(18.))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(icon),
        )
        .child(div().flex_1().text_size(px(14.)).child(label))
        .on_mouse_down(MouseButton::Left, on_click)
}

fn render_sidebar_footer(t: &theme::Tokens, settings_btn: gpui::Div) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .pt_2()
        .pb_3()
        .border_t_1()
        .border_color(rgb(t.border))
        .child(render_sidebar_safety_section(t))
        .child(settings_btn)
}

fn sidebar_settings_row(
    t: &theme::Tokens,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_2()
        .px_2()
        .py_2()
        .rounded_md()
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(rgb(0x8f8f96))
                .child(div().size(px(4.)).rounded_full().bg(rgb(0x8f8f96))),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child("설정"),
        )
        .on_mouse_down(MouseButton::Left, on_click)
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
            .mx_2()
            .px_2()
            .py_1()
            .text_xs()
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

fn render_sidebar_safety_section(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .pb_2()
        .child(section_label("작업 안전망"))
        .child(sidebar_status_row(t, "원본 폴더", "보호"))
        .child(sidebar_status_row(t, "작업공간", "자동"))
        .child(sidebar_status_row(t, "기록 정리", "자동"))
}

fn sidebar_status_row(t: &theme::Tokens, label: &'static str, value: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_2()
        .px_2()
        .py_1()
        .child(div().flex_1().text_xs().text_color(rgb(t.fg)).child(label))
        .child(
            div()
                .px_1()
                .rounded_sm()
                .bg(rgb(0xf0f0f2))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(value),
        )
}

fn section_label(label: &'static str) -> gpui::Div {
    div()
        .px_4()
        .pt_2()
        .pb_1()
        .text_xs()
        .text_color(rgb(0x8f8f96))
        .child(label)
}

fn render_recent_project_row(
    t: &theme::Tokens,
    path: String,
    roomy: bool,
    cx: &mut Context<MainView>,
) -> gpui::Div {
    let label = project_display_name(&path);
    let hint = if roomy {
        project_parent_hint(&path)
    } else {
        project_sidebar_hint(&path)
    };
    let path_for_click = path.clone();
    let row = div()
        .flex()
        .items_center()
        .gap_2()
        .rounded_lg()
        .cursor_pointer()
        .bg(rgb(if roomy { t.surface } else { t.sidebar }))
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_md()
                .border_1()
                .border_color(rgb(0x7f7f87)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(t.fg))
                        .child(label),
                )
                .child(div().text_xs().text_color(rgb(t.muted)).child(hint)),
        )
        .child(div().text_xs().text_color(rgb(t.muted)).child("열기"))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| this.open_recent_project(path_for_click.clone(), cx)),
        );

    if roomy {
        row.px_3().py_2().border_1().border_color(rgb(t.border))
    } else {
        row.mx_2().px_2().py_1()
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

fn project_sidebar_hint(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| project_parent_hint(path))
}

fn session_sidebar_subtitle(s: &SessionUiState) -> String {
    if s.session_failed {
        "시작 실패".into()
    } else if !s.thread_started {
        "작업공간 준비 중".into()
    } else if s.turn_in_flight {
        super::turn_status_label(
            s.thread_started,
            s.turn_in_flight,
            s.interrupt_requested,
            s.turn_reasoning_chars,
            s.turn_answer_chars,
        )
        .into()
    } else if s.has_unread {
        "새 응답 도착".into()
    } else if s.last_commit.is_some() {
        "변경 저장됨".into()
    } else {
        "대기".into()
    }
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
    let subtitle = session_sidebar_subtitle(s);
    let status_color = if s.session_failed {
        0xb43b3b
    } else if !s.thread_started {
        0xb8b8be
    } else if s.turn_in_flight {
        t.accent
    } else if s.has_unread {
        0x2f8f55
    } else {
        0x8c8c94
    };
    let bg = if active { t.sidebar_active } else { t.sidebar };
    let sid_for_click = sid.clone();
    let sid_for_close = sid.clone();
    let mut row = div()
        .flex()
        .gap_2()
        .items_center()
        .mx_2()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(bg))
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .w(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .child(div().size(px(7.)).rounded_full().bg(rgb(status_color))),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(project_label),
                )
                .child(div().text_xs().text_color(rgb(t.muted)).child(subtitle)),
        )
        .child(
            div()
                .w(px(16.))
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
