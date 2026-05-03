use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{AppContext, Context, Entity, ScrollHandle, SharedString};

use crate::MainView;
use crate::chat_input::ChatInput;
use crate::selectable_text::SelectableText;
use crate::theme;
use stcode_codex::bridge::{SessionId, SessionWorkspaceInfo, ToolKind, WorkspaceMode};

pub(crate) enum Screen {
    Welcome,
    Workspace(WorkspaceState),
}

/// 워크스페이스 — 사이드바 + 활성 세션. v1 핵심 워크플로우 "병렬 멀티 에이전트"를
/// 받아 안기 위한 구조.
pub(crate) struct WorkspaceState {
    pub(crate) sessions: HashMap<SessionId, SessionUiState>,
    /// 사이드바 표시 순서 — 세션 추가된 순.
    pub(crate) order: Vec<SessionId>,
    /// 현재 사이드바에서 선택된 세션. None은 모든 세션이 닫힌 상태.
    pub(crate) active: Option<SessionId>,
    /// 앱 실행마다 달라지는 내부 세션 id 접두어. 반복 실행 시 작업공간 이름 충돌을 막는다.
    pub(crate) session_prefix: String,
    /// 다음 세션 id 발급용 카운터.
    pub(crate) next_id: u32,
}

pub(crate) struct SessionUiState {
    pub(crate) project: PathBuf,
    pub(crate) thread_id: Option<String>,
    pub(crate) workspace_mode: Option<WorkspaceMode>,
    pub(crate) workspace: Option<SessionWorkspaceInfo>,
    pub(crate) messages: Vec<ChatItem>,
    pub(crate) thread_started: bool,
    pub(crate) session_failed: bool,
    pub(crate) turn_in_flight: bool,
    pub(crate) interrupt_requested: bool,
    pub(crate) turn_reasoning_chars: usize,
    pub(crate) turn_answer_chars: usize,
    pub(crate) input: Entity<ChatInput>,
    pub(crate) last_commit: Option<LastCommit>,
    /// active 가 아닌 세션에서 새 message/델타가 와서 unread 표식.
    pub(crate) has_unread: bool,
    /// 메시지 영역 별 ScrollHandle — 세션마다 따로 스크롤 위치 유지.
    pub(crate) scroll: ScrollHandle,
}

#[derive(Default)]
pub(crate) struct SessionSummary {
    pub(crate) user_turns: usize,
    pub(crate) agent_messages: usize,
    pub(crate) tools_running: usize,
    pub(crate) tools_ok: usize,
    pub(crate) tools_failed: usize,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct WorkspaceStats {
    pub(crate) total: usize,
    pub(crate) running: usize,
    pub(crate) failed: usize,
}

impl SessionUiState {
    pub(crate) fn new(project: PathBuf, cx: &mut Context<MainView>) -> Self {
        let intro = ChatItem::message(Speaker::System, "세션을 여는 중…", cx);
        let input = cx.new(|cx| {
            ChatInput::new(
                "후속 변경 사항을 부탁하세요",
                theme::TOKENS.fg,
                theme::TOKENS.muted,
                cx,
            )
        });
        Self {
            project,
            thread_id: None,
            workspace_mode: None,
            workspace: None,
            messages: vec![intro],
            thread_started: false,
            session_failed: false,
            turn_in_flight: false,
            interrupt_requested: false,
            turn_reasoning_chars: 0,
            turn_answer_chars: 0,
            input,
            last_commit: None,
            has_unread: false,
            scroll: ScrollHandle::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct LastCommit {
    /// commit 메시지 첫 줄 (사용자에게 보여줌).
    pub(crate) summary: String,
    /// 되돌릴 수 있는지(첫 commit이 아닌지).
    pub(crate) revertible: bool,
}

/// 채팅 영역의 한 항목. Message(사용자/agent/system) / Tool 카드 두 종류.
pub(crate) enum ChatItem {
    Message {
        who: Speaker,
        /// Streaming 중인 raw 텍스트 — turn 끝나기 전엔 이걸 그대로 렌더.
        text: Entity<SelectableText>,
        /// Agent 메시지의 reasoning(별도 회색 영역). None이면 표시 안 함.
        reasoning: Option<Entity<SelectableText>>,
        /// turn이 끝나면 markdown 파싱해서 채움. Some이면 segments를 렌더 — text 무시.
        segments: Option<Vec<MessageSegment>>,
    },
    Tool {
        item_id: String,
        kind: ToolKind,
        title: String,
        output: Entity<SelectableText>,
        status: ToolStatus,
    },
}

/// Markdown 파싱된 한 조각. block-level + 일부 inline(code/bold/link).
pub(crate) enum MessageSegment {
    /// 일반 텍스트 paragraph (줄바꿈 포함).
    Paragraph(Entity<SelectableText>),
    /// `# heading` `## heading` `### heading`. level=1..3.
    Heading {
        level: u8,
        body: Entity<SelectableText>,
    },
    /// `- item` 또는 `* item`. body는 bullet 제외한 본문.
    ListItem { body: Entity<SelectableText> },
    /// fenced code block. ```language\n...\n``` 의 안쪽 내용. mono font + 다른 bg.
    Code {
        body: Entity<SelectableText>,
        language: Option<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    InProgress,
    CompletedOk,
    CompletedFail,
}

impl ChatItem {
    pub(crate) fn message(
        who: Speaker,
        text: impl Into<SharedString>,
        cx: &mut Context<MainView>,
    ) -> Self {
        let s = text.into();
        let color = color_for(who);
        let entity = cx.new(|cx| SelectableText::new(s, color, cx));
        Self::Message {
            who,
            text: entity,
            reasoning: None,
            segments: None,
        }
    }

    pub(crate) fn tool(
        item_id: String,
        kind: ToolKind,
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
pub(crate) enum Speaker {
    User,
    Agent,
    System,
}

/// 진행 중인 승인 요청. v1 자동 모드에선 거의 안 뜨지만 인프라는 남김.
pub(crate) struct PendingApproval {
    pub(crate) session_id: SessionId,
    pub(crate) request_id: i64,
    pub(crate) kind: ToolKind,
    pub(crate) friendly_title: String,
    pub(crate) raw_detail: String,
}

/// 설정 모달이 떠 있을 때의 임시 입력 상태.
pub(crate) struct SettingsDraft {
    pub(crate) provider: Entity<ChatInput>,
    pub(crate) main_model: Entity<ChatInput>,
    pub(crate) sub_model: Entity<ChatInput>,
    /// 저장 후 잠깐 보여줄 안내 (Some(text), 자동 사라짐 없음 — 닫을 때 None).
    pub(crate) notice: Option<String>,
}

pub(crate) fn last_agent_message_text(messages: &mut [ChatItem]) -> Option<Entity<SelectableText>> {
    messages.iter_mut().rev().find_map(|m| match m {
        ChatItem::Message {
            who: Speaker::Agent,
            text,
            ..
        } => Some(text.clone()),
        _ => None,
    })
}

pub(crate) fn ensure_agent_reasoning(messages: &mut [ChatItem]) -> Option<Entity<SelectableText>> {
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
        return None;
    }
    None
}

pub(crate) fn find_tool_output(
    messages: &mut [ChatItem],
    item_id: &str,
) -> Option<Entity<SelectableText>> {
    messages.iter_mut().rev().find_map(|m| match m {
        ChatItem::Tool {
            item_id: id,
            output,
            ..
        } if id == item_id => Some(output.clone()),
        _ => None,
    })
}
