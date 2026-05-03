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
    let settings_btn = sidebar_action_row(
        t,
        "",
        "설정",
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(248.))
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
        .child(render_sidebar_safety_section(t))
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
                .h(px(50.))
                .px_4()
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
                .gap_3()
                .px_5()
                .pb_5()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .mb_3()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::MEDIUM)
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
                .child(welcome_composer(t, chips_model, cx)),
        )
}

fn welcome_composer(t: &theme::Tokens, chips_model: &str, cx: &mut Context<MainView>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .max_w(px(800.))
        .min_h(px(84.))
        .px_4()
        .py_2()
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_lg()
        .cursor_pointer()
        .hover(|d| d.border_color(rgb(0xc8c8cc)))
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
                .child(composer_icon_button(t, "+"))
                .child(permission_chip(t))
                .child(div().flex_1())
                .child(chip_owned(t, chips_model.to_string()))
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
        .max_w(px(760.))
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
    let settings_btn = sidebar_action_row(
        t,
        "",
        "설정",
        cx.listener(|this, _, _, cx| this.open_settings(cx)),
    );

    div()
        .flex()
        .flex_col()
        .w(px(248.))
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
        .child(render_sidebar_safety_section(t))
        .child(settings_btn)
}

fn sidebar_brand(t: &theme::Tokens) -> gpui::Div {
    div()
        .flex()
        .h(px(50.))
        .px_4()
        .items_center()
        .gap_2()
        .text_size(px(15.))
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
        .rounded_lg()
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
        .pb_3()
        .child(section_label("작업 안전망"))
        .child(sidebar_status_row(t, "원본 폴더", "보호"))
        .child(sidebar_status_row(t, "작업공간", "자동"))
        .child(sidebar_status_row(t, "브랜치 정리", "자동"))
}

fn sidebar_status_row(t: &theme::Tokens, label: &'static str, value: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_2()
        .px_2()
        .py_1()
        .rounded_lg()
        .bg(rgb(0xf6f6f7))
        .child(div().flex_1().text_xs().text_color(rgb(t.fg)).child(label))
        .child(div().text_xs().text_color(rgb(t.muted)).child(value))
}

fn section_label(label: &'static str) -> gpui::Div {
    div()
        .px_4()
        .pt_3()
        .pb_1()
        .text_xs()
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
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_md()
                .border_1()
                .border_color(rgb(t.muted)),
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
        row.px_4().py_2().border_1().border_color(rgb(t.border))
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
        .px_2()
        .py_1()
        .rounded_lg()
        .bg(rgb(bg))
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .hover(|d| d.bg(rgb(t.sidebar_active)))
        .child(
            div()
                .w(px(16.))
                .text_xs()
                .text_color(rgb(if s.turn_in_flight { t.accent } else { t.muted }))
                .child(status_icon),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .child(project_label),
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
