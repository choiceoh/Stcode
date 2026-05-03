use std::path::Path;

use gpui::{
    App, Context, Entity, FontWeight, MouseButton, MouseDownEvent, ParentElement, Styled, Window,
    div, prelude::*, px, rgb, rgba,
};

use crate::app_state::{
    ChatItem, MessageSegment, PendingApproval, SessionSummary, SessionUiState, SettingsDraft,
    Speaker, ToolStatus, WorkspaceState, WorkspaceStats,
};
use crate::chat_input::ChatInput;
use crate::selectable_text::SelectableText;
use crate::{MainView, theme};
use stcode_codex::bridge::{ApprovalDecision, SessionId, ToolKind, WorkspaceMode};
use stcode_vibe::{AgentModelRole, Settings, settings};

pub(crate) fn session_started_message(workspace_mode: WorkspaceMode) -> &'static str {
    match workspace_mode {
        WorkspaceMode::Isolated => {
            "작업공간 준비됨\n원본 폴더는 그대로 두고 이 세션 전용 공간에서 진행해요.\n에이전트 연결됨"
        }
        WorkspaceMode::Direct => "작업공간 준비됨\n에이전트 연결됨",
    }
}

pub(crate) fn model_route_label(settings: &Settings) -> String {
    let main = settings.model_for_role(AgentModelRole::Main);
    let sub = settings.model_for_role(AgentModelRole::Sub);
    if main == sub {
        "조율/작업 모델".into()
    } else {
        "조율 모델 · 작업 모델".into()
    }
}

mod chat;
mod modal;
mod sidebar;

pub(crate) use modal::{render_approval_modal, render_notice_modal, render_settings_modal};
pub(crate) use sidebar::{render_welcome, render_workspace};

fn status_pill(t: &theme::Tokens, label: &'static str) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .bg(rgb(0xf4f4f6))
        .rounded_lg()
        .text_xs()
        .text_color(rgb(t.muted))
        .child(label)
}

fn top_bar_controls(t: &theme::Tokens, chips_model: &str, cx: &mut Context<MainView>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(model_selector_chip(
            t,
            chips_model,
            cx.listener(|this, _, _, cx| this.open_settings(cx)),
        ))
}

fn model_selector_chip(
    t: &theme::Tokens,
    label: &str,
    on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .bg(rgb(0xf8f8f9))
        .border_1()
        .border_color(rgb(t.border))
        .rounded_lg()
        .text_xs()
        .text_color(rgb(t.fg))
        .cursor_pointer()
        .hover(|d| d.border_color(rgb(0xc4c4ca)).bg(rgb(0xf4f4f6)))
        .child(label.to_string())
        .child(div().text_color(rgb(t.muted)).child("⌄"))
        .on_mouse_down(MouseButton::Left, on_click)
}

fn send_circle(label: &'static str, color: u32, enabled: bool) -> gpui::Div {
    div()
        .size(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(color))
        .text_color(rgb(0xffffff))
        .rounded_full()
        .when(enabled, |d| {
            d.cursor_pointer().hover(|d| d.bg(rgb(0x6f6f75)))
        })
        .child(label)
}

fn composer_status_text(t: &theme::Tokens, label: &str) -> gpui::Div {
    div()
        .text_xs()
        .text_color(rgb(t.muted))
        .child(label.to_string())
}

pub(crate) const LONG_REASONING_CHARS: usize = 4_000;

pub(crate) fn turn_status_label(
    thread_started: bool,
    turn_in_flight: bool,
    interrupt_requested: bool,
    reasoning_chars: usize,
    answer_chars: usize,
) -> &'static str {
    if !thread_started {
        "세션 여는 중…"
    } else if !turn_in_flight {
        "대기"
    } else if interrupt_requested {
        "중단 요청 중"
    } else if answer_chars > 0 {
        "답변 작성 중"
    } else if reasoning_chars >= LONG_REASONING_CHARS {
        "생각이 길어지는 중"
    } else if reasoning_chars > 0 {
        "생각 중"
    } else {
        "응답 기다리는 중"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_label_tracks_reasoning_before_answer() {
        assert_eq!(
            turn_status_label(false, false, false, 0, 0),
            "세션 여는 중…"
        );
        assert_eq!(turn_status_label(true, false, false, 0, 0), "대기");
        assert_eq!(
            turn_status_label(true, true, false, 0, 0),
            "응답 기다리는 중"
        );
        assert_eq!(turn_status_label(true, true, false, 10, 0), "생각 중");
        assert_eq!(
            turn_status_label(true, true, false, LONG_REASONING_CHARS, 0),
            "생각이 길어지는 중"
        );
        assert_eq!(
            turn_status_label(true, true, false, 10_000, 1),
            "답변 작성 중"
        );
        assert_eq!(
            turn_status_label(true, true, true, 10_000, 0),
            "중단 요청 중"
        );
    }

    #[test]
    fn session_started_message_is_user_facing() {
        let isolated = session_started_message(WorkspaceMode::Isolated);
        assert!(isolated.contains("작업공간 준비됨"));
        assert!(isolated.contains("원본 폴더는 그대로"));
        assert!(!isolated.contains("branch"));
        assert!(!isolated.contains("worktree"));

        let direct = session_started_message(WorkspaceMode::Direct);
        assert_eq!(direct, "작업공간 준비됨\n에이전트 연결됨");
    }

    #[test]
    fn model_route_label_shows_main_and_sub_roles() {
        let same = Settings {
            provider: "local-vllm".into(),
            model: "qwen".into(),
            main_model: "qwen".into(),
            sub_model: "qwen".into(),
            recent_projects: Vec::new(),
        };
        assert_eq!(model_route_label(&same), "조율/작업 모델");

        let split = Settings {
            provider: "local-vllm".into(),
            model: "planner".into(),
            main_model: "planner".into(),
            sub_model: "worker".into(),
            recent_projects: Vec::new(),
        };
        assert_eq!(model_route_label(&split), "조율 모델 · 작업 모델");
    }
}
