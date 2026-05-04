use acp_thread::{AcpThread, AgentThreadEntry, ThreadStatus, ToolCall, ToolCallStatus};
use agent_client_protocol as acp;
use anyhow::Result;
use git::repository::DiffType;
use gpui::{Action, Entity, IntoElement, RenderOnce};
use project::{
    Project,
    git_store::{Repository, StatusEntry},
};
use ui::{Button, ButtonStyle, Icon, IconName, Label, LabelSize, prelude::*};
use util::truncate_and_trailoff;
use zed_actions::{agent::ReviewBranchDiff, git as zed_git};

const MAX_TIMELINE_ENTRIES: usize = 4;
const MAX_ENTRY_LABEL_CHARS: usize = 72;

#[derive(IntoElement)]
pub(crate) struct StcodeActivityTimeline {
    thread: Option<Entity<AcpThread>>,
    project: Entity<Project>,
}

impl StcodeActivityTimeline {
    pub(crate) fn new(thread: Option<Entity<AcpThread>>, project: Entity<Project>) -> Self {
        Self { thread, project }
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
        let smart_start = SmartStartSnapshot::from_project(&self.project, cx);
        let smart_merge = SmartMergeSnapshot::from_project(&self.project, cx);

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
            .children(smart_start.map(|snapshot| render_smart_start_guard(snapshot, cx)))
            .children(smart_merge.map(|snapshot| render_smart_merge_card(snapshot, cx)))
            .children(snapshot.entries.into_iter().map(render_activity_entry))
    }
}

fn render_smart_start_guard(snapshot: SmartStartSnapshot, cx: &mut App) -> impl IntoElement {
    let review_repository = snapshot.repository.clone();
    let worktree_action = zed_git::Worktree.boxed_clone();
    let stash_repository = snapshot.repository.clone();
    let commit_action = git::ExpandCommitEditor.boxed_clone();

    v_flex()
        .id("stcode-smart-start-guard")
        .mt_1()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().status().warning_border)
        .bg(cx.theme().status().warning_background)
        .p_2()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(
                    Icon::new(IconName::Warning)
                        .size(IconSize::Small)
                        .color(Color::Warning),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            Label::new("AI Smart Start")
                                .size(LabelSize::Small)
                                .color(Color::Default),
                        )
                        .child(
                            Label::new(snapshot.detail)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .child(
                    Button::new("stcode-smart-start-review", "Review")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, window, cx| {
                            review_leftover_changes(review_repository.clone(), window, cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-start-worktree", "Split Worktree")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(worktree_action.boxed_clone(), cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-start-stash", "Stash")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, _window, cx| {
                            stash_leftover_changes(stash_repository.clone(), cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-start-commit", "Commit")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(commit_action.boxed_clone(), cx);
                        }),
                ),
        )
}

fn render_smart_merge_card(snapshot: SmartMergeSnapshot, cx: &mut App) -> impl IntoElement {
    let review_repository = snapshot.repository.clone();
    let create_pull_request_action = zed_git::CreatePullRequest.boxed_clone();
    let commit_action = git::ExpandCommitEditor.boxed_clone();
    let (border_color, background_color) = match snapshot.tone {
        ActivityTone::Done => (
            cx.theme().status().success_border,
            cx.theme().status().success_background,
        ),
        ActivityTone::Failed => (
            cx.theme().status().error_border,
            cx.theme().status().error_background,
        ),
        _ => (
            cx.theme().status().warning_border,
            cx.theme().status().warning_background,
        ),
    };

    v_flex()
        .id("stcode-smart-merge-card")
        .mt_1()
        .gap_2()
        .rounded_sm()
        .border_1()
        .border_color(border_color)
        .bg(background_color)
        .p_2()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(
                    Icon::new(snapshot.icon)
                        .size(IconSize::Small)
                        .color(snapshot.tone.color()),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_2()
                                .child(
                                    Label::new("AI Smart Merge")
                                        .size(LabelSize::Small)
                                        .color(Color::Default),
                                )
                                .child(
                                    Label::new(snapshot.status)
                                        .size(LabelSize::XSmall)
                                        .color(snapshot.tone.color()),
                                ),
                        )
                        .child(
                            Label::new(snapshot.detail)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .child(
                    Button::new("stcode-smart-merge-review", "Review Merge")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .disabled(!snapshot.can_review)
                        .on_click(move |_, window, cx| {
                            review_merge_readiness(review_repository.clone(), window, cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-merge-pr", "Open PR")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .disabled(!snapshot.can_create_pull_request)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(create_pull_request_action.boxed_clone(), cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-merge-commit", "Commit")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .disabled(!snapshot.can_commit)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(commit_action.boxed_clone(), cx);
                        }),
                ),
        )
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

#[derive(Clone)]
struct SmartStartSnapshot {
    repository: Entity<Repository>,
    detail: String,
}

impl SmartStartSnapshot {
    fn from_project(project: &Entity<Project>, cx: &App) -> Option<Self> {
        let repository = project.read(cx).active_repository(cx)?;
        let repository_ref = repository.read(cx);
        let entries = repository_ref
            .cached_status()
            .filter(|entry| entry.status.has_changes())
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return None;
        }

        let branch_name = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string());
        let detail = smart_start_detail(branch_name.as_deref(), &entries);

        Some(Self { repository, detail })
    }
}

fn smart_start_detail(branch_name: Option<&str>, entries: &[StatusEntry]) -> String {
    let changed_count = entries.len();
    let conflicted_count = entries
        .iter()
        .filter(|entry| entry.status.is_conflicted())
        .count();
    let staged_count = entries
        .iter()
        .filter(|entry| entry.status.staging().has_staged())
        .count();
    let unstaged_count = entries
        .iter()
        .filter(|entry| entry.status.staging().has_unstaged())
        .count();

    let branch = branch_name.unwrap_or("detached HEAD");
    let file_label = if changed_count == 1 { "file" } else { "files" };

    if conflicted_count > 0 {
        return format!(
            "{changed_count} changed {file_label} remain on {branch}, including {conflicted_count} conflict(s). Resolve or isolate them before starting the next session."
        );
    }

    format!(
        "{changed_count} changed {file_label} remain on {branch}: {staged_count} staged, {unstaged_count} unstaged. Choose how to hand them off before starting clean."
    )
}

#[derive(Clone)]
struct SmartMergeSnapshot {
    repository: Entity<Repository>,
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    can_review: bool,
    can_create_pull_request: bool,
    can_commit: bool,
}

impl SmartMergeSnapshot {
    fn from_project(project: &Entity<Project>, cx: &App) -> Option<Self> {
        let repository = project.read(cx).active_repository(cx)?;
        let repository_ref = repository.read(cx);
        let branch_name = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string());
        let entries = repository_ref
            .cached_status()
            .filter(|entry| entry.status.has_changes())
            .collect::<Vec<_>>();
        let changed_count = entries.len();
        let conflicted_count = entries
            .iter()
            .filter(|entry| entry.status.is_conflicted())
            .count();
        let state = smart_merge_state(branch_name.as_deref(), changed_count, conflicted_count);

        Some(Self {
            repository,
            status: state.status(),
            detail: smart_merge_detail(
                state,
                branch_name.as_deref(),
                changed_count,
                conflicted_count,
            ),
            icon: state.icon(),
            tone: state.tone(),
            can_review: branch_name
                .as_deref()
                .is_some_and(|branch_name| !is_merge_base_branch(branch_name)),
            can_create_pull_request: state == SmartMergeState::Ready,
            can_commit: changed_count > 0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartMergeState {
    Ready,
    NeedsCheckpoint,
    HasConflicts,
    ProtectedBranch,
    Detached,
}

impl SmartMergeState {
    fn status(self) -> &'static str {
        match self {
            SmartMergeState::Ready => "Ready",
            SmartMergeState::NeedsCheckpoint => "Checkpoint needed",
            SmartMergeState::HasConflicts => "Blocked",
            SmartMergeState::ProtectedBranch => "Base branch",
            SmartMergeState::Detached => "No branch",
        }
    }

    fn icon(self) -> IconName {
        match self {
            SmartMergeState::Ready => IconName::PullRequest,
            SmartMergeState::NeedsCheckpoint => IconName::GitCommit,
            SmartMergeState::HasConflicts => IconName::GitMergeConflict,
            SmartMergeState::ProtectedBranch | SmartMergeState::Detached => IconName::GitBranch,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            SmartMergeState::Ready => ActivityTone::Done,
            SmartMergeState::HasConflicts => ActivityTone::Failed,
            SmartMergeState::NeedsCheckpoint
            | SmartMergeState::ProtectedBranch
            | SmartMergeState::Detached => ActivityTone::Waiting,
        }
    }
}

fn smart_merge_state(
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
) -> SmartMergeState {
    let Some(branch_name) = branch_name else {
        return SmartMergeState::Detached;
    };

    if conflicted_count > 0 {
        return SmartMergeState::HasConflicts;
    }

    if changed_count > 0 {
        return SmartMergeState::NeedsCheckpoint;
    }

    if is_merge_base_branch(branch_name) {
        return SmartMergeState::ProtectedBranch;
    }

    SmartMergeState::Ready
}

fn smart_merge_detail(
    state: SmartMergeState,
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
) -> String {
    let branch = branch_name.unwrap_or("detached HEAD");

    match state {
        SmartMergeState::Ready => format!(
            "{branch} is clean locally. Review the merge diff or open a PR so checks can take over."
        ),
        SmartMergeState::NeedsCheckpoint => {
            let file_label = if changed_count == 1 { "file" } else { "files" };
            format!(
                "{changed_count} changed {file_label} remain on {branch}. Commit or stash them before Smart Merge can prepare a clean PR."
            )
        }
        SmartMergeState::HasConflicts => format!(
            "{conflicted_count} conflict(s) remain on {branch}. Resolve them before Smart Merge can make this merge-ready."
        ),
        SmartMergeState::ProtectedBranch => format!(
            "{branch} is the base branch. Split into a task branch before asking Smart Merge to prepare a PR."
        ),
        SmartMergeState::Detached => {
            "This workspace is detached. Create a task branch before Smart Merge can prepare a PR."
                .to_string()
        }
    }
}

fn is_merge_base_branch(branch_name: &str) -> bool {
    let branch_name = branch_name.rsplit('/').next().unwrap_or(branch_name);
    matches!(branch_name, "main" | "master" | "trunk")
}

fn review_leftover_changes(repository: Entity<Repository>, window: &mut Window, cx: &mut App) {
    let branch_name = repository
        .read(cx)
        .branch
        .as_ref()
        .map(|branch| branch.name().to_string())
        .unwrap_or_else(|| "working tree".to_string());
    let diff_receiver = repository.update(cx, |repository, cx| {
        repository.diff(DiffType::HeadToWorktree, cx)
    });

    window
        .spawn(cx, async move |cx| -> Result<()> {
            let diff_text = diff_receiver.await??;
            if diff_text.trim().is_empty() {
                return Ok(());
            }

            cx.update(|window, cx| {
                window.dispatch_action(
                    Box::new(ReviewBranchDiff {
                        diff_text: diff_text.into(),
                        base_ref: branch_name.into(),
                    }),
                    cx,
                );
            })?;

            Ok(())
        })
        .detach_and_log_err(cx);
}

fn review_merge_readiness(repository: Entity<Repository>, window: &mut Window, cx: &mut App) {
    let default_branch_receiver =
        repository.update(cx, |repository, _| repository.default_branch(true));

    window
        .spawn(cx, async move |cx| -> Result<()> {
            let base_ref = default_branch_receiver
                .await??
                .unwrap_or_else(|| "main".into());
            let diff_base_ref = base_ref.clone();
            let diff_receiver = cx.update(|_, cx| {
                repository.update(cx, |repository, cx| {
                    repository.diff(
                        DiffType::MergeBase {
                            base_ref: diff_base_ref,
                        },
                        cx,
                    )
                })
            })?;
            let diff_text = diff_receiver.await??;
            if diff_text.trim().is_empty() {
                return Ok(());
            }

            cx.update(|window, cx| {
                window.dispatch_action(
                    Box::new(ReviewBranchDiff {
                        diff_text: diff_text.into(),
                        base_ref,
                    }),
                    cx,
                );
            })?;

            Ok(())
        })
        .detach_and_log_err(cx);
}

fn stash_leftover_changes(repository: Entity<Repository>, cx: &mut App) {
    repository
        .update(cx, |repository, cx| repository.stash_all(cx))
        .detach_and_log_err(cx);
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

    #[test]
    fn smart_start_detail_summarizes_clean_handoff_counts() {
        let detail = smart_start_detail(
            Some("feature"),
            &[
                status_entry(git::status::FileStatus::index(
                    git::status::StatusCode::Modified,
                )),
                status_entry(git::status::FileStatus::worktree(
                    git::status::StatusCode::Modified,
                )),
                status_entry(git::status::FileStatus::Untracked),
            ],
        );

        assert!(detail.contains("3 changed files remain on feature"));
        assert!(detail.contains("1 staged"));
        assert!(detail.contains("2 unstaged"));
    }

    #[test]
    fn smart_start_detail_prioritizes_conflicts() {
        let detail = smart_start_detail(
            Some("feature"),
            &[
                status_entry(git::status::FileStatus::Unmerged(
                    git::status::UnmergedStatus {
                        first_head: git::status::UnmergedStatusCode::Updated,
                        second_head: git::status::UnmergedStatusCode::Updated,
                    },
                )),
                status_entry(git::status::FileStatus::worktree(
                    git::status::StatusCode::Modified,
                )),
            ],
        );

        assert!(detail.contains("including 1 conflict"));
        assert!(detail.contains("Resolve or isolate them"));
    }

    #[test]
    fn smart_merge_state_requires_a_branch() {
        assert_eq!(smart_merge_state(None, 0, 0), SmartMergeState::Detached);
    }

    #[test]
    fn smart_merge_state_blocks_base_branches() {
        assert_eq!(
            smart_merge_state(Some("main"), 0, 0),
            SmartMergeState::ProtectedBranch
        );
        assert_eq!(
            smart_merge_state(Some("origin/master"), 0, 0),
            SmartMergeState::ProtectedBranch
        );
    }

    #[test]
    fn smart_merge_state_blocks_dirty_work() {
        assert_eq!(
            smart_merge_state(Some("feature"), 2, 0),
            SmartMergeState::NeedsCheckpoint
        );
    }

    #[test]
    fn smart_merge_state_prioritizes_conflicts() {
        assert_eq!(
            smart_merge_state(Some("feature"), 2, 1),
            SmartMergeState::HasConflicts
        );
    }

    #[test]
    fn smart_merge_state_accepts_clean_feature_branch() {
        assert_eq!(
            smart_merge_state(Some("feature"), 0, 0),
            SmartMergeState::Ready
        );
    }

    #[test]
    fn smart_merge_detail_explains_checkpoint_requirement() {
        let detail = smart_merge_detail(SmartMergeState::NeedsCheckpoint, Some("feature"), 3, 0);

        assert!(detail.contains("3 changed files remain on feature"));
        assert!(detail.contains("Commit or stash"));
    }

    fn status_entry(status: git::status::FileStatus) -> StatusEntry {
        StatusEntry {
            repo_path: git::repository::RepoPath::new("src/main.rs")
                .expect("test path should be a valid repo path"),
            status,
            diff_stat: None,
        }
    }
}
