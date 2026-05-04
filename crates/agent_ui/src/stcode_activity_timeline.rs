use acp_thread::{AcpThread, AgentThreadEntry, ThreadStatus, ToolCall, ToolCallStatus};
use agent_client_protocol as acp;
use gpui::{Entity, IntoElement, RenderOnce};
use ui::{Icon, IconName, Label, LabelSize, prelude::*};
use util::truncate_and_trailoff;

const MAX_TIMELINE_ENTRIES: usize = 4;
const MAX_ENTRY_LABEL_CHARS: usize = 72;

#[derive(IntoElement)]
pub(crate) struct StcodeActivityTimeline {
    thread: Option<Entity<AcpThread>>,
}

impl StcodeActivityTimeline {
    pub(crate) fn new(thread: Option<Entity<AcpThread>>) -> Self {
        Self { thread }
    }
}

impl RenderOnce for StcodeActivityTimeline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let snapshot = match self.thread {
            Some(thread) => {
                let thread = thread.read(cx);
                ActivitySnapshot::from_thread(thread, cx)
            }
            None => ActivitySnapshot::empty(),
        };

        v_flex()
            .id("stcode-activity-timeline")
            .flex_none()
            .gap_1()
            .px_3()
            .py_2()
            .bg(cx.theme().colors().panel_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_2()
                            .child(
                                Icon::new(snapshot.icon)
                                    .size(IconSize::Small)
                                    .color(snapshot.tone.color()),
                            )
                            .child(
                                Label::new("Workspace Activity")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                Label::new(snapshot.status)
                                    .size(LabelSize::Small)
                                    .color(snapshot.tone.color()),
                            )
                            .child(
                                Label::new(snapshot.detail)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    ),
            )
            .children(snapshot.entries.into_iter().map(render_activity_entry))
    }
}

fn render_activity_entry(entry: ActivityEntry) -> impl IntoElement {
    h_flex()
        .id(("stcode-activity-entry", entry.id))
        .w_full()
        .min_w_0()
        .gap_2()
        .pl_1()
        .child(
            Icon::new(entry.icon)
                .size(IconSize::XSmall)
                .color(entry.tone.color()),
        )
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .gap_2()
                .child(
                    Label::new(entry.label)
                        .size(LabelSize::XSmall)
                        .color(Color::Default)
                        .truncate(),
                )
                .child(
                    Label::new(entry.status)
                        .size(LabelSize::XSmall)
                        .color(entry.tone.color()),
                ),
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivitySnapshot {
    status: &'static str,
    detail: &'static str,
    icon: IconName,
    tone: ActivityTone,
    entries: Vec<ActivityEntry>,
}

impl ActivitySnapshot {
    fn empty() -> Self {
        Self {
            status: "Ready",
            detail: "No workspace activity yet",
            icon: IconName::Circle,
            tone: ActivityTone::Idle,
            entries: Vec::new(),
        }
    }

    fn from_thread(thread: &AcpThread, cx: &App) -> Self {
        let entries = thread
            .entries()
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(entry_index, entry)| {
                ActivityEntry::from_thread_entry(entry_index, entry, cx)
            })
            .take(MAX_TIMELINE_ENTRIES)
            .collect::<Vec<_>>();

        let last_tool_tone = entries
            .iter()
            .find(|entry| entry.is_tool)
            .map(|entry| entry.tone);

        let summary = summarize_thread_state(
            !thread.entries().is_empty(),
            thread.is_waiting_for_confirmation(),
            thread.status() == ThreadStatus::Generating,
            thread.has_in_progress_tool_calls(),
            thread.had_error(),
            last_tool_tone,
        );

        Self {
            status: summary.status,
            detail: summary.detail,
            icon: summary.icon,
            tone: summary.tone,
            entries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivityEntry {
    id: usize,
    label: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
    is_tool: bool,
}

impl ActivityEntry {
    fn from_thread_entry(entry_index: usize, entry: &AgentThreadEntry, cx: &App) -> Option<Self> {
        match entry {
            AgentThreadEntry::UserMessage(_) => Some(Self {
                id: entry_index,
                label: "Request received".to_string(),
                status: "Queued",
                icon: IconName::UserCheck,
                tone: ActivityTone::Idle,
                is_tool: false,
            }),
            AgentThreadEntry::AssistantMessage(_) => Some(Self {
                id: entry_index,
                label: "Agent response updated".to_string(),
                status: "Updated",
                icon: IconName::ZedAgent,
                tone: ActivityTone::Idle,
                is_tool: false,
            }),
            AgentThreadEntry::ToolCall(tool_call) => {
                let (status, tone) = tool_status_label(&tool_call.status);
                Some(Self {
                    id: entry_index,
                    label: tool_call_label(tool_call, cx),
                    status,
                    icon: tool_kind_icon(tool_call.kind),
                    tone,
                    is_tool: true,
                })
            }
            AgentThreadEntry::CompletedPlan(_) => Some(Self {
                id: entry_index,
                label: "Plan completed".to_string(),
                status: "Done",
                icon: IconName::TodoComplete,
                tone: ActivityTone::Done,
                is_tool: false,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThreadSummary {
    status: &'static str,
    detail: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

fn summarize_thread_state(
    has_entries: bool,
    is_waiting_for_confirmation: bool,
    is_generating: bool,
    has_in_progress_tool_calls: bool,
    had_error: bool,
    last_tool_tone: Option<ActivityTone>,
) -> ThreadSummary {
    if !has_entries {
        return ThreadSummary {
            status: "Ready",
            detail: "No workspace activity yet",
            icon: IconName::Circle,
            tone: ActivityTone::Idle,
        };
    }

    if is_waiting_for_confirmation {
        return ThreadSummary {
            status: "Needs approval",
            detail: "A workspace tool is waiting",
            icon: IconName::Warning,
            tone: ActivityTone::Waiting,
        };
    }

    if is_generating {
        return ThreadSummary {
            status: "Working",
            detail: if has_in_progress_tool_calls {
                "Running workspace tools"
            } else {
                "Planning the next step"
            },
            icon: IconName::LoadCircle,
            tone: ActivityTone::Running,
        };
    }

    if had_error || last_tool_tone.is_some_and(ActivityTone::needs_attention) {
        return ThreadSummary {
            status: "Needs attention",
            detail: "The latest work did not complete",
            icon: IconName::XCircle,
            tone: ActivityTone::Failed,
        };
    }

    ThreadSummary {
        status: "Ready",
        detail: "Latest work is complete",
        icon: IconName::Check,
        tone: ActivityTone::Done,
    }
}

fn tool_status_label(status: &ToolCallStatus) -> (&'static str, ActivityTone) {
    match status {
        ToolCallStatus::Pending => ("Queued", ActivityTone::Running),
        ToolCallStatus::WaitingForConfirmation { .. } => ("Needs approval", ActivityTone::Waiting),
        ToolCallStatus::InProgress => ("Running", ActivityTone::Running),
        ToolCallStatus::Completed => ("Done", ActivityTone::Done),
        ToolCallStatus::Failed => ("Failed", ActivityTone::Failed),
        ToolCallStatus::Rejected => ("Rejected", ActivityTone::Failed),
        ToolCallStatus::Canceled => ("Canceled", ActivityTone::Failed),
    }
}

fn tool_call_label(tool_call: &ToolCall, cx: &App) -> String {
    let label = tool_call.label.read(cx).source();
    let label = label.lines().next().unwrap_or(label).trim();
    if label.is_empty() {
        tool_kind_label(tool_call.kind).to_string()
    } else {
        truncate_and_trailoff(label, MAX_ENTRY_LABEL_CHARS)
    }
}

fn tool_kind_label(kind: acp::ToolKind) -> &'static str {
    match kind {
        acp::ToolKind::Read => "Read workspace context",
        acp::ToolKind::Edit => "Edit files",
        acp::ToolKind::Delete => "Delete files",
        acp::ToolKind::Move => "Move files",
        acp::ToolKind::Search => "Search workspace",
        acp::ToolKind::Execute => "Run command",
        acp::ToolKind::Think => "Reason about task",
        acp::ToolKind::Fetch => "Fetch web context",
        acp::ToolKind::SwitchMode => "Switch mode",
        acp::ToolKind::Other | _ => "Use workspace tool",
    }
}

fn tool_kind_icon(kind: acp::ToolKind) -> IconName {
    match kind {
        acp::ToolKind::Read => IconName::File,
        acp::ToolKind::Edit => IconName::ToolPencil,
        acp::ToolKind::Delete => IconName::ToolDeleteFile,
        acp::ToolKind::Move => IconName::ArrowRightLeft,
        acp::ToolKind::Search => IconName::ToolSearch,
        acp::ToolKind::Execute => IconName::ToolTerminal,
        acp::ToolKind::Think => IconName::ToolThink,
        acp::ToolKind::Fetch => IconName::ToolWeb,
        acp::ToolKind::SwitchMode => IconName::ArrowRightLeft,
        acp::ToolKind::Other | _ => IconName::ToolHammer,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityTone {
    Idle,
    Running,
    Waiting,
    Done,
    Failed,
}

impl ActivityTone {
    fn color(self) -> Color {
        match self {
            ActivityTone::Idle => Color::Muted,
            ActivityTone::Running => Color::Accent,
            ActivityTone::Waiting => Color::Warning,
            ActivityTone::Done => Color::Success,
            ActivityTone::Failed => Color::Error,
        }
    }

    fn needs_attention(self) -> bool {
        matches!(self, ActivityTone::Waiting | ActivityTone::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_thread_state_prioritizes_approval() {
        let summary =
            summarize_thread_state(true, true, true, true, true, Some(ActivityTone::Failed));

        assert_eq!(summary.status, "Needs approval");
        assert_eq!(summary.tone, ActivityTone::Waiting);
    }

    #[test]
    fn summarize_thread_state_distinguishes_planning_from_tool_work() {
        let planning = summarize_thread_state(true, false, true, false, false, None);
        let running = summarize_thread_state(true, false, true, true, false, None);

        assert_eq!(planning.detail, "Planning the next step");
        assert_eq!(running.detail, "Running workspace tools");
    }

    #[test]
    fn summarize_thread_state_surfaces_failed_latest_tool() {
        let summary =
            summarize_thread_state(true, false, false, false, false, Some(ActivityTone::Failed));

        assert_eq!(summary.status, "Needs attention");
        assert_eq!(summary.tone, ActivityTone::Failed);
    }

    #[test]
    fn tool_status_labels_are_user_facing() {
        assert_eq!(
            tool_status_label(&ToolCallStatus::Pending),
            ("Queued", ActivityTone::Running)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::InProgress),
            ("Running", ActivityTone::Running)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::Completed),
            ("Done", ActivityTone::Done)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::Failed),
            ("Failed", ActivityTone::Failed)
        );
    }
}
