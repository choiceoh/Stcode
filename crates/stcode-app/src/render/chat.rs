use super::*;

/// 현재 active 세션의 main panel. active 가 None이면 placeholder.
pub(super) fn render_active_main(
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
            .gap_2()
            .bg(rgb(t.bg))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(t.fg))
                    .child("프로젝트를 열어 작업을 시작하세요"),
            )
            .text_color(rgb(t.muted))
            .child("왼쪽의 새 작업을 누르면 세션별 작업공간과 작업 기록을 자동으로 준비합니다.");
    };
    let Some(s) = ws.sessions.get(&sid) else {
        return div().flex_1();
    };
    render_chat_main(t, s, workspace_stats(ws), chips_model, cx)
}

fn render_chat_main(
    t: &theme::Tokens,
    s: &SessionUiState,
    stats: WorkspaceStats,
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

    let status_label = session_status_label(s);

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
                .gap_3()
                .bg(rgb(t.bg))
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(t.fg))
                                .child(project_label),
                        )
                        .child(status_pill(t, status_label)),
                )
                .child(workspace_load_pill(t, stats))
                .child(top_bar_controls(t, chips_model, cx))
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
                        .child(render_session_canvas(t, s))
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

fn render_session_canvas(t: &theme::Tokens, s: &SessionUiState) -> impl IntoElement {
    let has_dialogue = s.messages.iter().any(|item| {
        matches!(
            item,
            ChatItem::Message {
                who: Speaker::User | Speaker::Agent,
                ..
            } | ChatItem::Tool { .. }
        )
    });

    let content = div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .max_w(px(760.))
        .px_4()
        .py_4();

    div()
        .id("messages")
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .overflow_y_scroll()
        .track_scroll(&s.scroll)
        .child(if has_dialogue {
            content.children(s.messages.iter().map(|m| render_chat_item(t, m)))
        } else {
            content.child(render_active_session_board(t, s))
        })
}

fn render_active_session_board(t: &theme::Tokens, s: &SessionUiState) -> gpui::Div {
    let project_label = s
        .project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.project.to_string_lossy().into_owned());
    let status = session_status_label(s);

    div()
        .flex()
        .flex_col()
        .gap_3()
        .w_full()
        .child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(t.fg))
                                .child(project_label),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(t.muted))
                                .child(s.project.to_string_lossy().into_owned()),
                        ),
                )
                .child(status_pill(t, status)),
        )
        .child(render_session_facts(t, s))
        .child(render_live_activity_panel(t, s))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(panel_title(t, "상태 기록"))
                .children(s.messages.iter().map(|m| render_chat_item(t, m))),
        )
}

fn render_session_facts(t: &theme::Tokens, s: &SessionUiState) -> gpui::Div {
    let workspace_mode = match s.workspace_mode {
        Some(WorkspaceMode::Isolated) => "원본 보호",
        Some(WorkspaceMode::Direct) => "직접 작업",
        None => "준비 중",
    };
    let runtime_state = s
        .workspace
        .as_ref()
        .map(|w| {
            if w.branch.is_some() {
                "세션 전용 공간".to_string()
            } else {
                "선택한 폴더".to_string()
            }
        })
        .unwrap_or_else(|| "확인 중".to_string());
    let record_state = s
        .workspace
        .as_ref()
        .and_then(|w| w.branch.as_ref())
        .map(|_| "자동 기록 연결됨".to_string())
        .unwrap_or_else(|| {
            if s.thread_started {
                "기본 기록".to_string()
            } else {
                "준비 중".to_string()
            }
        });
    let thread_state = s
        .thread_id
        .as_ref()
        .map(|id| format!("연결됨 {}", short_id(id)))
        .unwrap_or_else(|| "연결 중".to_string());

    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .child(panel_title(t, "세션 상태"))
        .child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .bg(rgb(0xf8f8f9))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_md()
                .child(session_fact(t, "작업공간", workspace_mode, s.thread_started))
                .child(session_fact(
                    t,
                    "실행 위치",
                    runtime_state.as_str(),
                    s.workspace.is_some(),
                ))
                .child(session_fact(
                    t,
                    "작업 기록",
                    record_state.as_str(),
                    s.workspace.is_some(),
                ))
                .child(session_fact(t, "에이전트", thread_state.as_str(), s.thread_started)),
        )
}

fn session_fact(t: &theme::Tokens, label: &'static str, value: &str, active: bool) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .child(
            div()
                .size(px(6.))
                .rounded_full()
                .bg(rgb(if active { t.accent } else { 0xc7c7cc })),
        )
        .child(
            div()
                .w(px(78.))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if active { t.fg } else { t.muted }))
                .child(value.to_string()),
        )
}

fn render_live_activity_panel(t: &theme::Tokens, s: &SessionUiState) -> gpui::Div {
    let summary = session_summary(s);
    let tool_state = format!(
        "진행 {} · 완료 {} · 실패 {}",
        summary.tools_running, summary.tools_ok, summary.tools_failed
    );
    let text_state = if s.turn_in_flight {
        format!(
            "생각 {}자 · 답변 {}자",
            s.turn_reasoning_chars, s.turn_answer_chars
        )
    } else {
        format!(
            "요청 {} · 응답 {}",
            summary.user_turns, summary.agent_messages
        )
    };
    let save_state = s
        .last_commit
        .as_ref()
        .map(|c| c.summary.clone())
        .unwrap_or_else(|| "아직 저장된 변경 없음".to_string());

    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .child(panel_title(t, "실시간 활동"))
        .child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .rounded_md()
                .bg(rgb(0xf8f8f9))
                .child(activity_row(
                    t,
                    "현재 상태",
                    session_status_label(s),
                    s.turn_in_flight,
                ))
                .child(activity_row(
                    t,
                    "도구 사용",
                    tool_state.as_str(),
                    summary.tools_running > 0,
                ))
                .child(activity_row(
                    t,
                    "응답 흐름",
                    text_state.as_str(),
                    s.turn_in_flight,
                ))
                .child(activity_row(
                    t,
                    "변경 저장",
                    save_state.as_str(),
                    s.last_commit.is_some(),
                )),
        )
}

fn activity_row(t: &theme::Tokens, label: &'static str, value: &str, active: bool) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .child(
            div()
                .size(px(6.))
                .rounded_full()
                .bg(rgb(if active { t.accent } else { 0xc7c7cc })),
        )
        .child(
            div()
                .w(px(72.))
                .text_xs()
                .text_color(rgb(t.muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_xs()
                .text_color(rgb(t.fg))
                .child(value.to_string()),
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
    div().flex().justify_center().px_4().pb_4().child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .max_w(px(760.))
            .min_h(px(76.))
            .px_3()
            .py_2()
            .bg(rgb(t.surface))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_md()
            .child(div().flex_1().child(s.input.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(composer_status_text(t, "자동 실행"))
                    .child(div().flex_1())
                    .child(composer_status_text(t, chips_model))
                    .child(
                        send_circle(send_label, send_color, send_enabled).when(
                            send_enabled,
                            |d| {
                                d.on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| this.send_user_input(cx)),
                                )
                            },
                        ),
                    ),
            ),
    )
}

fn render_session_overview(t: &theme::Tokens, s: &SessionUiState) -> gpui::Div {
    let summary = session_summary(s);
    let workspace_state = match s.workspace_mode {
        Some(WorkspaceMode::Isolated) => "원본 보호",
        Some(WorkspaceMode::Direct) => "직접 작업",
        None if s.session_failed => "실패",
        None => "준비 중",
    };
    let agent_state = if s.interrupt_requested {
        "중단 요청"
    } else if s.session_failed {
        "시작 실패"
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
        .w(px(224.))
        .h_full()
        .px_3()
        .py_3()
        .gap_3()
        .bg(rgb(0xf9f9fa))
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
                    "작업 기록",
                    if s.workspace
                        .as_ref()
                        .and_then(|w| w.branch.as_ref())
                        .is_some()
                    {
                        "자동"
                    } else {
                        "기본"
                    },
                ))
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
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(t.muted))
        .child(label)
}

fn timeline_row(
    t: &theme::Tokens,
    _step: &'static str,
    label: &'static str,
    state: &str,
    active: bool,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_1()
        .py_1()
        .child(
            div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(if active { t.accent } else { 0xc7c7cc })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(div().text_xs().text_color(rgb(t.fg)).child(label))
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
        .items_center()
        .flex_1()
        .gap_2()
        .px_2()
        .py_2()
        .rounded_md()
        .bg(rgb(0xffffff))
        .child(div().text_xs().text_color(rgb(t.muted)).child(label))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(t.fg))
                .child(value),
        )
}

fn safety_row(t: &theme::Tokens, label: &'static str, state: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .child(div().flex_1().text_xs().text_color(rgb(t.fg)).child(label))
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

fn workspace_stats(ws: &WorkspaceState) -> WorkspaceStats {
    let mut stats = WorkspaceStats {
        total: ws.order.len(),
        ..WorkspaceStats::default()
    };
    for sid in &ws.order {
        let Some(s) = ws.sessions.get(sid) else {
            continue;
        };
        if s.turn_in_flight {
            stats.running += 1;
        }
        if s.session_failed {
            stats.failed += 1;
        }
    }
    stats
}

fn workspace_load_pill(t: &theme::Tokens, stats: WorkspaceStats) -> gpui::Div {
    let color = if stats.failed > 0 {
        0xb43b3b
    } else if stats.running > 0 {
        t.accent
    } else {
        t.muted
    };
    let label = format!("작업 {} / 10 · 진행 {}", stats.total, stats.running);
    div()
        .px_2()
        .py_1()
        .bg(rgb(0xf4f4f6))
        .rounded_lg()
        .text_xs()
        .text_color(rgb(color))
        .child(label)
}

fn session_status_label(s: &SessionUiState) -> &'static str {
    if s.session_failed {
        "시작 실패"
    } else {
        turn_status_label(
            s.thread_started,
            s.turn_in_flight,
            s.interrupt_requested,
            s.turn_reasoning_chars,
            s.turn_answer_chars,
        )
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(7).collect()
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
    if who == Speaker::System {
        return render_status_message(t, text);
    }
    if who == Speaker::User {
        return render_user_message(t, text);
    }

    let mut body = div().flex_1().flex().flex_col().gap_2();
    if let Some(r) = reasoning {
        body = body.child(
            div()
                .px_2()
                .py_1()
                .bg(rgb(0xf4f5f6))
                .rounded_md()
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
            body = body.child(render_segment(t, seg));
        }
    } else {
        body = body.child(div().py_1().text_color(rgb(t.fg)).child(text));
    }
    div().flex().gap_2().items_start().child(body)
}

fn render_status_message(t: &theme::Tokens, text: Entity<SelectableText>) -> gpui::Div {
    div()
        .flex()
        .items_start()
        .gap_2()
        .py_1()
        .text_xs()
        .text_color(rgb(t.muted))
        .child(
            div()
                .mt(px(7.))
                .size(px(6.))
                .rounded_full()
                .bg(rgb(0xb7b7bd)),
        )
        .child(div().flex_1().child(text))
}

fn render_user_message(t: &theme::Tokens, text: Entity<SelectableText>) -> gpui::Div {
    div().flex().justify_end().child(
        div()
            .max_w(px(520.))
            .px_3()
            .py_2()
            .bg(rgb(0xf0f0f2))
            .rounded_lg()
            .text_color(rgb(t.fg))
            .child(text),
    )
}

fn render_segment(t: &theme::Tokens, seg: &MessageSegment) -> gpui::Div {
    match seg {
        MessageSegment::Paragraph(entity) => {
            div().py_1().text_color(rgb(t.fg)).child(entity.clone())
        }
        MessageSegment::Heading { level, body } => {
            let base = div()
                .pt_2()
                .pb_1()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(t.fg));
            match level {
                1 => base.text_lg(),
                2 => base.text_size(px(16.)),
                _ => base.text_size(px(15.)),
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
            let mut card = div()
                .flex()
                .flex_col()
                .gap_1()
                .px_3()
                .py_2()
                .bg(rgb(0xf4f4f5))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_md()
                .text_size(px(13.));
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
        .gap_2()
        .items_center()
        .py_1()
        .text_xs()
        .text_color(rgb(t.muted))
        .child(div().w(px(18.)).text_color(rgb(status_color)).child(icon))
        .child(div().text_color(rgb(status_color)).child(status_label))
        .child(
            div()
                .flex_1()
                .text_color(rgb(t.fg))
                .child(title.to_string()),
        )
        .child(div().max_w(px(280.)).overflow_hidden().child(output))
}

// ─── 모달 (자동 모드에선 거의 안 뜸 — 인프라만 남김) ──────
