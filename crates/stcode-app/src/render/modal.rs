use super::*;

pub(crate) fn render_approval_modal(
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

pub(crate) fn render_notice_modal(
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

pub(crate) fn render_settings_modal(
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
