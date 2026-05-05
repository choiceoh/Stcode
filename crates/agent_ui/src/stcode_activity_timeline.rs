use acp_thread::{AcpThread, AgentThreadEntry, ThreadStatus, ToolCall, ToolCallStatus};
use agent_client_protocol as acp;
use anyhow::Result;
use git::{repository::DiffType, status::FileStatus};
use gpui::{Action, Entity, IntoElement, RenderOnce};
use project::{
    Project,
    git_store::{Repository, StatusEntry},
};
use ui::{Button, ButtonStyle, Icon, IconName, Label, LabelSize, prelude::*};
use util::{paths::PathStyle, truncate_and_trailoff};
use zed_actions::{
    CreateWorktree, NewWorktreeBranchTarget, agent::ReviewBranchDiff, git as zed_git,
};

const MAX_TIMELINE_ENTRIES: usize = 4;
const MAX_ENTRY_LABEL_CHARS: usize = 72;
const MAX_SMART_PANEL_GOAL_CHARS: usize = 96;
const MAX_SMART_PANEL_FILES: usize = 3;
const MAX_SMART_TODO_ENTRIES: usize = 4;
const MAX_TODO_LABEL_CHARS: usize = 84;

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
        let thread = self.thread.as_ref().map(|thread| thread.read(cx));
        let (snapshot, smart_todo) = match thread {
            Some(thread) => (
                ActivitySnapshot::from_thread(thread, cx),
                SmartTodoSnapshot::from_thread(thread, cx),
            ),
            None => (ActivitySnapshot::empty(), None),
        };
        let smart_start = SmartStartSnapshot::from_project(&self.project, cx);
        let smart_panel = SmartPanelSnapshot::from_project(&self.project, thread, cx);
        let smart_parallel = SmartParallelSnapshot::from_project(&self.project, cx);
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
            .children(smart_panel.map(|snapshot| render_smart_panel_card(snapshot, cx)))
            .children(smart_todo.map(|snapshot| render_smart_todo_card(snapshot, cx)))
            .children(smart_start.map(|snapshot| render_smart_start_guard(snapshot, cx)))
            .children(smart_parallel.map(|snapshot| render_smart_parallel_card(snapshot, cx)))
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

fn render_smart_panel_card(snapshot: SmartPanelSnapshot, cx: &mut App) -> impl IntoElement {
    let review_repository = snapshot.repository.clone();
    let commit_action = git::ExpandCommitEditor.boxed_clone();
    let has_files = !snapshot.files.is_empty();
    let counts = snapshot.counts;
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
        .id("stcode-smart-panel-card")
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
                                    Label::new("AI Smart Panel")
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
                .child(render_smart_panel_metric(
                    "changed",
                    counts.changed_count,
                    "files",
                    snapshot.tone,
                    cx,
                ))
                .child(render_smart_panel_metric(
                    "staged",
                    counts.staged_count,
                    "staged",
                    ActivityTone::Done,
                    cx,
                ))
                .child(render_smart_panel_metric(
                    "unstaged",
                    counts.unstaged_count,
                    "unstaged",
                    ActivityTone::Waiting,
                    cx,
                ))
                .when(counts.conflicted_count > 0, |this| {
                    this.child(render_smart_panel_metric(
                        "conflicts",
                        counts.conflicted_count,
                        "conflicts",
                        ActivityTone::Failed,
                        cx,
                    ))
                })
                .when(counts.added_lines + counts.removed_lines > 0, |this| {
                    this.child(
                        ui::DiffStat::new(
                            "stcode-smart-panel-diff-stat",
                            counts.added_lines,
                            counts.removed_lines,
                        )
                        .label_size(LabelSize::XSmall),
                    )
                }),
        )
        .child(
            v_flex().w_full().gap_1().children(
                snapshot
                    .work_items
                    .into_iter()
                    .map(render_smart_panel_work_item),
            ),
        )
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(!has_files, |this| {
                    this.child(
                        Label::new("No local file changes")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                })
                .children(snapshot.files.into_iter().map(render_smart_panel_file)),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .child(
                    Button::new("stcode-smart-panel-review", "Review Files")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .disabled(!snapshot.can_review)
                        .on_click(move |_, window, cx| {
                            review_leftover_changes(review_repository.clone(), window, cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-panel-commit", "Commit")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .disabled(!snapshot.can_commit)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(commit_action.boxed_clone(), cx);
                        }),
                ),
        )
}

fn render_smart_panel_work_item(item: SmartPanelWorkItem) -> impl IntoElement {
    h_flex()
        .id(format!("stcode-smart-panel-work-item-{}", item.id))
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            Icon::new(item.icon)
                .size(IconSize::XSmall)
                .color(item.tone.color()),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .gap_0p5()
                .child(
                    Label::new(item.label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    Label::new(item.detail)
                        .size(LabelSize::XSmall)
                        .color(Color::Default)
                        .truncate(),
                ),
        )
        .child(
            Label::new(item.status)
                .size(LabelSize::XSmall)
                .color(item.tone.color()),
        )
}

fn render_smart_panel_metric(
    id: &'static str,
    value: usize,
    label: &'static str,
    tone: ActivityTone,
    cx: &mut App,
) -> impl IntoElement {
    h_flex()
        .id(format!("stcode-smart-panel-metric-{id}"))
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().colors().border)
        .child(
            Label::new(value.to_string())
                .size(LabelSize::XSmall)
                .color(tone.color()),
        )
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
}

fn render_smart_panel_file(file: SmartPanelFile) -> impl IntoElement {
    h_flex()
        .id(("stcode-smart-panel-file", file.id))
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            Icon::new(file.icon)
                .size(IconSize::XSmall)
                .color(file.tone.color()),
        )
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .gap_2()
                .child(
                    Label::new(file.path)
                        .size(LabelSize::XSmall)
                        .color(Color::Default)
                        .truncate(),
                )
                .child(
                    Label::new(file.status)
                        .size(LabelSize::XSmall)
                        .color(file.tone.color()),
                ),
        )
}

fn render_smart_todo_card(snapshot: SmartTodoSnapshot, cx: &mut App) -> impl IntoElement {
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
        .id("stcode-smart-todo-card")
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
                                    Label::new("AI Smart Todo")
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
                .child(render_smart_todo_metric(
                    "progress",
                    snapshot.progress_label,
                    ActivityTone::Done,
                    cx,
                ))
                .child(render_smart_todo_metric(
                    "left",
                    snapshot.left_label,
                    snapshot.tone,
                    cx,
                )),
        )
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .when(snapshot.items.is_empty(), |this| {
                    this.child(
                        Label::new("No active todo items")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                })
                .children(snapshot.items.into_iter().map(render_smart_todo_item)),
        )
}

fn render_smart_todo_metric(
    id: &'static str,
    label: String,
    tone: ActivityTone,
    cx: &mut App,
) -> impl IntoElement {
    h_flex()
        .id(format!("stcode-smart-todo-metric-{id}"))
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().colors().border)
        .child(
            Label::new(label)
                .size(LabelSize::XSmall)
                .color(tone.color()),
        )
}

fn render_smart_todo_item(item: SmartTodoItem) -> impl IntoElement {
    h_flex()
        .id(("stcode-smart-todo-item", item.id))
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            Icon::new(item.icon)
                .size(IconSize::XSmall)
                .color(item.tone.color()),
        )
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .gap_2()
                .child(
                    Label::new(item.label)
                        .size(LabelSize::XSmall)
                        .color(Color::Default)
                        .truncate(),
                )
                .child(
                    Label::new(item.status)
                        .size(LabelSize::XSmall)
                        .color(item.tone.color()),
                ),
        )
}

fn render_smart_parallel_card(snapshot: SmartParallelSnapshot, cx: &mut App) -> impl IntoElement {
    let create_lane_action = CreateWorktree {
        worktree_name: None,
        branch_target: NewWorktreeBranchTarget::CurrentBranch,
    }
    .boxed_clone();
    let manage_lanes_action = zed_git::Worktree.boxed_clone();
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
        .id("stcode-smart-parallel-card")
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
                                    Label::new("AI Smart Parallel")
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
                    Button::new("stcode-smart-parallel-create-lane", "Create Lane")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(create_lane_action.boxed_clone(), cx);
                        }),
                )
                .child(
                    Button::new("stcode-smart-parallel-manage-lanes", "Manage Lanes")
                        .label_size(LabelSize::XSmall)
                        .style(ButtonStyle::Outlined)
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(manage_lanes_action.boxed_clone(), cx);
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
struct SmartPanelSnapshot {
    repository: Entity<Repository>,
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    counts: SmartPanelCounts,
    work_items: Vec<SmartPanelWorkItem>,
    files: Vec<SmartPanelFile>,
    can_review: bool,
    can_commit: bool,
}

impl SmartPanelSnapshot {
    fn from_project(
        project: &Entity<Project>,
        thread: Option<&AcpThread>,
        cx: &App,
    ) -> Option<Self> {
        let repository = project.read(cx).active_repository(cx)?;
        let repository_ref = repository.read(cx);
        let branch_name = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string());
        let branch_ref = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.ref_name.to_string());
        let entries = repository_ref
            .cached_status()
            .filter(|entry| entry.status.has_changes())
            .collect::<Vec<_>>();
        let shared_branch_lane_count =
            smart_shared_branch_lane_count(&repository_ref, branch_ref.as_deref());
        let counts = smart_panel_counts(
            &entries,
            repository_ref.linked_worktrees().len(),
            repository_ref.is_linked_worktree(),
            shared_branch_lane_count,
        );
        let state = smart_panel_state(counts.changed_count, counts.conflicted_count);
        let merge_state = smart_merge_state(
            branch_name.as_deref(),
            counts.changed_count,
            counts.conflicted_count,
        );
        let thread_summary = smart_panel_thread_summary(thread, cx);
        let work_items = smart_panel_work_items(
            thread,
            thread_summary,
            branch_name.as_deref(),
            counts,
            merge_state,
            cx,
        );
        let files = entries
            .iter()
            .take(MAX_SMART_PANEL_FILES)
            .enumerate()
            .map(|(id, entry)| {
                SmartPanelFile::from_status_entry(id, entry, repository_ref.path_style)
            })
            .collect();

        Some(Self {
            repository,
            status: state.status(),
            detail: smart_panel_detail(state, branch_name.as_deref(), counts),
            icon: state.icon(),
            tone: state.tone(),
            counts,
            work_items,
            files,
            can_review: counts.changed_count > 0,
            can_commit: counts.changed_count > 0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SmartPanelCounts {
    changed_count: usize,
    staged_count: usize,
    unstaged_count: usize,
    conflicted_count: usize,
    untracked_count: usize,
    added_lines: usize,
    removed_lines: usize,
    linked_worktree_count: usize,
    is_linked_worktree: bool,
    shared_branch_lane_count: usize,
}

#[derive(Clone)]
struct SmartPanelWorkItem {
    id: &'static str,
    label: &'static str,
    detail: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

#[derive(Clone)]
struct SmartPanelFile {
    id: usize,
    path: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

impl SmartPanelFile {
    fn from_status_entry(id: usize, entry: &StatusEntry, path_style: PathStyle) -> Self {
        let (status, icon, tone) = smart_panel_file_status(entry.status);

        Self {
            id,
            path: entry.repo_path.display(path_style).to_string(),
            status,
            icon,
            tone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartPanelState {
    Clean,
    InProgress,
    Blocked,
}

impl SmartPanelState {
    fn status(self) -> &'static str {
        match self {
            SmartPanelState::Clean => "Clean",
            SmartPanelState::InProgress => "In progress",
            SmartPanelState::Blocked => "Blocked",
        }
    }

    fn icon(self) -> IconName {
        match self {
            SmartPanelState::Clean => IconName::ListTodo,
            SmartPanelState::InProgress => IconName::Diff,
            SmartPanelState::Blocked => IconName::GitMergeConflict,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            SmartPanelState::Clean => ActivityTone::Done,
            SmartPanelState::InProgress => ActivityTone::Waiting,
            SmartPanelState::Blocked => ActivityTone::Failed,
        }
    }
}

fn smart_panel_counts(
    entries: &[StatusEntry],
    linked_worktree_count: usize,
    is_linked_worktree: bool,
    shared_branch_lane_count: usize,
) -> SmartPanelCounts {
    SmartPanelCounts {
        changed_count: entries.len(),
        staged_count: entries
            .iter()
            .filter(|entry| entry.status.staging().has_staged())
            .count(),
        unstaged_count: entries
            .iter()
            .filter(|entry| entry.status.staging().has_unstaged())
            .count(),
        conflicted_count: entries
            .iter()
            .filter(|entry| entry.status.is_conflicted())
            .count(),
        untracked_count: entries
            .iter()
            .filter(|entry| entry.status.is_untracked())
            .count(),
        added_lines: entries
            .iter()
            .filter_map(|entry| entry.diff_stat)
            .map(|stat| stat.added as usize)
            .sum(),
        removed_lines: entries
            .iter()
            .filter_map(|entry| entry.diff_stat)
            .map(|stat| stat.deleted as usize)
            .sum(),
        linked_worktree_count,
        is_linked_worktree,
        shared_branch_lane_count,
    }
}

fn smart_shared_branch_lane_count(repository: &Repository, branch_ref: Option<&str>) -> usize {
    branch_ref
        .map(|branch_ref| {
            repository
                .linked_worktrees()
                .iter()
                .filter(|worktree| {
                    worktree
                        .ref_name
                        .as_ref()
                        .is_some_and(|ref_name| ref_name.as_ref() == branch_ref)
                })
                .count()
        })
        .unwrap_or(0)
}

fn smart_panel_state(changed_count: usize, conflicted_count: usize) -> SmartPanelState {
    if conflicted_count > 0 {
        SmartPanelState::Blocked
    } else if changed_count > 0 {
        SmartPanelState::InProgress
    } else {
        SmartPanelState::Clean
    }
}

fn smart_panel_detail(
    state: SmartPanelState,
    branch_name: Option<&str>,
    counts: SmartPanelCounts,
) -> String {
    let branch = branch_name.unwrap_or("detached HEAD");

    match state {
        SmartPanelState::Clean => {
            let lane_detail = if counts.is_linked_worktree {
                "this session is isolated"
            } else if counts.linked_worktree_count > 0 {
                "linked lanes are available"
            } else {
                "create a lane before parallel work"
            };
            format!("{branch}: no local file changes; {lane_detail}.")
        }
        SmartPanelState::InProgress => {
            let file_label = if counts.changed_count == 1 {
                "file"
            } else {
                "files"
            };
            let line_detail = if counts.added_lines + counts.removed_lines > 0 {
                format!(", +{} -{}", counts.added_lines, counts.removed_lines)
            } else {
                String::new()
            };
            let untracked_detail = if counts.untracked_count > 0 {
                format!(", {} new", counts.untracked_count)
            } else {
                String::new()
            };
            format!(
                "{branch}: {} changed {file_label}, {} staged, {} unstaged{untracked_detail}{line_detail}.",
                counts.changed_count, counts.staged_count, counts.unstaged_count
            )
        }
        SmartPanelState::Blocked => {
            let file_label = if counts.changed_count == 1 {
                "file"
            } else {
                "files"
            };
            format!(
                "{branch}: {} conflict(s) across {} changed {file_label}. Resolve blockers before continuing.",
                counts.conflicted_count, counts.changed_count
            )
        }
    }
}

fn smart_panel_file_status(status: FileStatus) -> (&'static str, IconName, ActivityTone) {
    if status.is_conflicted() {
        return ("Conflict", IconName::GitMergeConflict, ActivityTone::Failed);
    }

    if status.is_untracked() {
        return ("New", IconName::File, ActivityTone::Waiting);
    }

    let staging = status.staging();
    if staging.has_staged() && staging.has_unstaged() {
        ("Partial", IconName::Diff, ActivityTone::Waiting)
    } else if staging.has_staged() {
        ("Staged", IconName::Check, ActivityTone::Done)
    } else {
        ("Changed", IconName::Diff, ActivityTone::Waiting)
    }
}

fn smart_panel_thread_summary(thread: Option<&AcpThread>, cx: &App) -> ThreadSummary {
    let Some(thread) = thread else {
        return ThreadSummary {
            status: "Ready",
            detail: "No workspace activity yet",
            icon: IconName::Circle,
            tone: ActivityTone::Idle,
        };
    };

    let latest_tool_tone = latest_tool_snapshot(thread, cx).map(|tool| tool.tone);
    summarize_thread_state(
        !thread.entries().is_empty(),
        thread.is_waiting_for_confirmation(),
        thread.status() == ThreadStatus::Generating,
        thread.has_in_progress_tool_calls(),
        thread.had_error(),
        latest_tool_tone,
    )
}

fn smart_panel_work_items(
    thread: Option<&AcpThread>,
    thread_summary: ThreadSummary,
    branch_name: Option<&str>,
    counts: SmartPanelCounts,
    merge_state: SmartMergeState,
    cx: &App,
) -> Vec<SmartPanelWorkItem> {
    vec![
        smart_panel_goal_item(thread, thread_summary, cx),
        smart_panel_lane_item(branch_name, counts),
        smart_panel_check_item(thread, cx),
        smart_panel_merge_item(
            branch_name,
            counts.changed_count,
            counts.conflicted_count,
            merge_state,
        ),
    ]
}

fn smart_panel_goal_item(
    thread: Option<&AcpThread>,
    thread_summary: ThreadSummary,
    cx: &App,
) -> SmartPanelWorkItem {
    let detail = thread
        .and_then(|thread| smart_panel_goal_from_thread(thread, cx))
        .unwrap_or_else(|| "No active goal yet".to_string());

    SmartPanelWorkItem {
        id: "goal",
        label: "Current goal",
        detail,
        status: thread_summary.status,
        icon: IconName::UserCheck,
        tone: thread_summary.tone,
    }
}

fn smart_panel_goal_from_thread(thread: &AcpThread, cx: &App) -> Option<String> {
    thread.entries().iter().rev().find_map(|entry| {
        let AgentThreadEntry::UserMessage(message) = entry else {
            return None;
        };

        let label =
            smart_panel_compact_label(message.content.to_markdown(cx), MAX_SMART_PANEL_GOAL_CHARS);
        if label.is_empty() { None } else { Some(label) }
    })
}

fn smart_panel_lane_item(
    branch_name: Option<&str>,
    counts: SmartPanelCounts,
) -> SmartPanelWorkItem {
    let branch = branch_name.unwrap_or("detached HEAD");

    if counts.shared_branch_lane_count > 0 {
        let lane_label = if counts.shared_branch_lane_count == 1 {
            "lane"
        } else {
            "lanes"
        };
        return SmartPanelWorkItem {
            id: "lane",
            label: "Lane",
            detail: format!(
                "{branch} overlaps {} linked {lane_label}",
                counts.shared_branch_lane_count
            ),
            status: "Overlap",
            icon: IconName::GitMergeConflict,
            tone: ActivityTone::Failed,
        };
    }

    if counts.is_linked_worktree {
        SmartPanelWorkItem {
            id: "lane",
            label: "Lane",
            detail: format!("{branch} is isolated for this session"),
            status: "Isolated",
            icon: IconName::GitWorktree,
            tone: ActivityTone::Done,
        }
    } else {
        SmartPanelWorkItem {
            id: "lane",
            label: "Lane",
            detail: format!("{branch} is still on the main checkout"),
            status: "Split",
            icon: IconName::GitBranchPlus,
            tone: ActivityTone::Waiting,
        }
    }
}

fn smart_panel_check_item(thread: Option<&AcpThread>, cx: &App) -> SmartPanelWorkItem {
    let Some(check) = latest_execute_tool_snapshot(thread, cx) else {
        return SmartPanelWorkItem {
            id: "check",
            label: "Last check",
            detail: "No command check has run in this thread yet".to_string(),
            status: "Waiting",
            icon: IconName::ToolTerminal,
            tone: ActivityTone::Idle,
        };
    };

    SmartPanelWorkItem {
        id: "check",
        label: "Last check",
        detail: check.label,
        status: check.status,
        icon: check.icon,
        tone: check.tone,
    }
}

fn smart_panel_merge_item(
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
    merge_state: SmartMergeState,
) -> SmartPanelWorkItem {
    SmartPanelWorkItem {
        id: "merge",
        label: "Merge readiness",
        detail: smart_merge_detail(merge_state, branch_name, changed_count, conflicted_count),
        status: merge_state.status(),
        icon: merge_state.icon(),
        tone: merge_state.tone(),
    }
}

fn latest_execute_tool_snapshot(
    thread: Option<&AcpThread>,
    cx: &App,
) -> Option<LatestToolSnapshot> {
    thread?.entries().iter().rev().find_map(|entry| {
        let AgentThreadEntry::ToolCall(tool_call) = entry else {
            return None;
        };
        if !matches!(tool_call.kind, acp::ToolKind::Execute) {
            return None;
        }

        let (status, tone) = tool_status_label(&tool_call.status);
        Some(LatestToolSnapshot {
            label: tool_call_label(tool_call, cx),
            status,
            icon: IconName::ToolTerminal,
            tone,
        })
    })
}

fn smart_panel_compact_label(text: impl AsRef<str>, max_chars: usize) -> String {
    let compact = text
        .as_ref()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_and_trailoff(compact.trim(), max_chars)
}

#[derive(Clone)]
struct SmartTodoSnapshot {
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    progress_label: String,
    left_label: String,
    items: Vec<SmartTodoItem>,
}

impl SmartTodoSnapshot {
    fn from_thread(thread: &AcpThread, cx: &App) -> Option<Self> {
        let has_entries = !thread.entries().is_empty();
        let plan = thread.plan();
        let has_plan = !plan.is_empty();

        if !has_entries && !has_plan {
            return None;
        }

        let stats = plan.stats();
        let total = plan.entries.len() as u32;
        let completed = stats.completed;
        let pending = stats.pending;
        let current_label = stats
            .in_progress_entry
            .map(|entry| plan_entry_label(entry, cx));
        let latest_tool = latest_tool_snapshot(thread, cx);
        let latest_tool_label = latest_tool.as_ref().map(|tool| tool.label.clone());
        let latest_tool_tone = latest_tool.as_ref().map(|tool| tool.tone);
        let state = smart_todo_state(
            has_plan,
            pending,
            thread.is_waiting_for_confirmation(),
            thread.status() == ThreadStatus::Generating,
            thread.has_in_progress_tool_calls(),
            thread.had_error(),
            latest_tool_tone,
        );
        let mut items = plan
            .entries
            .iter()
            .enumerate()
            .take(MAX_SMART_TODO_ENTRIES)
            .map(|(id, entry)| SmartTodoItem::from_plan_entry(id, entry, cx))
            .collect::<Vec<_>>();

        if items.is_empty()
            && let Some(tool) = latest_tool
        {
            items.push(SmartTodoItem {
                id: 0,
                label: tool.label,
                status: tool.status,
                icon: tool.icon,
                tone: tool.tone,
            });
        }

        Some(Self {
            status: state.status(),
            detail: smart_todo_detail(
                state,
                current_label.as_deref(),
                latest_tool_label.as_deref(),
                pending,
                completed,
                total,
            ),
            icon: state.icon(),
            tone: state.tone(),
            progress_label: smart_todo_progress_label(completed, total),
            left_label: smart_todo_left_label(pending),
            items,
        })
    }
}

#[derive(Clone)]
struct SmartTodoItem {
    id: usize,
    label: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

impl SmartTodoItem {
    fn from_plan_entry(id: usize, entry: &acp_thread::PlanEntry, cx: &App) -> Self {
        let (status, icon, tone) = plan_entry_status(entry.status.clone());

        Self {
            id,
            label: plan_entry_label(entry, cx),
            status,
            icon,
            tone,
        }
    }
}

struct LatestToolSnapshot {
    label: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartTodoState {
    Empty,
    Planned,
    Working,
    AutonomyBlocked,
    Blocked,
    Complete,
}

impl SmartTodoState {
    fn status(self) -> &'static str {
        match self {
            SmartTodoState::Empty => "No todo",
            SmartTodoState::Planned => "Ready",
            SmartTodoState::Working => "Working",
            SmartTodoState::AutonomyBlocked => "Autonomy blocked",
            SmartTodoState::Blocked => "Blocked",
            SmartTodoState::Complete => "Complete",
        }
    }

    fn icon(self) -> IconName {
        match self {
            SmartTodoState::Empty => IconName::ListTodo,
            SmartTodoState::Planned => IconName::TodoPending,
            SmartTodoState::Working => IconName::TodoProgress,
            SmartTodoState::AutonomyBlocked => IconName::Warning,
            SmartTodoState::Blocked => IconName::XCircle,
            SmartTodoState::Complete => IconName::TodoComplete,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            SmartTodoState::Empty => ActivityTone::Idle,
            SmartTodoState::Planned => ActivityTone::Waiting,
            SmartTodoState::Working => ActivityTone::Running,
            SmartTodoState::AutonomyBlocked => ActivityTone::Waiting,
            SmartTodoState::Blocked => ActivityTone::Failed,
            SmartTodoState::Complete => ActivityTone::Done,
        }
    }
}

fn smart_todo_state(
    has_plan: bool,
    pending: u32,
    is_waiting_for_confirmation: bool,
    is_generating: bool,
    has_in_progress_tool_calls: bool,
    had_error: bool,
    latest_tool_tone: Option<ActivityTone>,
) -> SmartTodoState {
    if is_waiting_for_confirmation {
        return SmartTodoState::AutonomyBlocked;
    }

    if had_error || latest_tool_tone.is_some_and(ActivityTone::needs_attention) {
        return SmartTodoState::Blocked;
    }

    if is_generating || has_in_progress_tool_calls {
        return SmartTodoState::Working;
    }

    if has_plan && pending > 0 {
        return SmartTodoState::Planned;
    }

    if has_plan {
        SmartTodoState::Complete
    } else {
        SmartTodoState::Empty
    }
}

fn smart_todo_detail(
    state: SmartTodoState,
    current_label: Option<&str>,
    latest_tool_label: Option<&str>,
    pending: u32,
    completed: u32,
    total: u32,
) -> String {
    match state {
        SmartTodoState::Empty => {
            "No live todo plan yet. Ask the agent to break the task into tracked steps.".to_string()
        }
        SmartTodoState::Planned => {
            let current = current_label.unwrap_or("next planned step");
            format!("{pending} todo step(s) remain. Next: {current}.")
        }
        SmartTodoState::Working => {
            let current = current_label
                .or(latest_tool_label)
                .unwrap_or("workspace work is running");
            format!("Agent is working now: {current}.")
        }
        SmartTodoState::AutonomyBlocked => {
            let tool = latest_tool_label.unwrap_or("a workspace tool");
            format!("Autonomy blocker: {tool} is waiting on tool permission.")
        }
        SmartTodoState::Blocked => {
            let tool = latest_tool_label.unwrap_or("the latest workspace step");
            format!("Blocked by {tool}. Review the failure before starting more work.")
        }
        SmartTodoState::Complete => format!("{completed}/{total} todo step(s) complete."),
    }
}

fn smart_todo_progress_label(completed: u32, total: u32) -> String {
    if total == 0 {
        "no plan".to_string()
    } else {
        format!("{completed}/{total} done")
    }
}

fn smart_todo_left_label(pending: u32) -> String {
    if pending == 1 {
        "1 left".to_string()
    } else {
        format!("{pending} left")
    }
}

fn plan_entry_label(entry: &acp_thread::PlanEntry, cx: &App) -> String {
    truncate_and_trailoff(entry.content.read(cx).source().trim(), MAX_TODO_LABEL_CHARS)
}

fn plan_entry_status(status: acp::PlanEntryStatus) -> (&'static str, IconName, ActivityTone) {
    match status {
        acp::PlanEntryStatus::InProgress => {
            ("Doing", IconName::TodoProgress, ActivityTone::Running)
        }
        acp::PlanEntryStatus::Completed => ("Done", IconName::TodoComplete, ActivityTone::Done),
        acp::PlanEntryStatus::Pending | _ => {
            ("Queued", IconName::TodoPending, ActivityTone::Waiting)
        }
    }
}

fn latest_tool_snapshot(thread: &AcpThread, cx: &App) -> Option<LatestToolSnapshot> {
    thread.entries().iter().rev().find_map(|entry| {
        let AgentThreadEntry::ToolCall(tool_call) = entry else {
            return None;
        };

        let (status, tone) = tool_status_label(&tool_call.status);
        Some(LatestToolSnapshot {
            label: tool_call_label(tool_call, cx),
            status,
            icon: tool_kind_icon(tool_call.kind),
            tone,
        })
    })
}

#[derive(Clone)]
struct SmartParallelSnapshot {
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
}

impl SmartParallelSnapshot {
    fn from_project(project: &Entity<Project>, cx: &App) -> Option<Self> {
        let repository = project.read(cx).active_repository(cx)?;
        let repository_ref = repository.read(cx);
        let branch_name = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string());
        let branch_ref = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.ref_name.to_string());
        let duplicate_branch_count = branch_ref
            .as_deref()
            .map(|branch_ref| {
                repository_ref
                    .linked_worktrees()
                    .iter()
                    .filter(|worktree| {
                        worktree
                            .ref_name
                            .as_ref()
                            .is_some_and(|ref_name| ref_name.as_ref() == branch_ref)
                    })
                    .count()
            })
            .unwrap_or(0);
        let linked_worktree_count = repository_ref.linked_worktrees().len();
        let state =
            smart_parallel_state(repository_ref.is_linked_worktree(), duplicate_branch_count);

        Some(Self {
            status: state.status(),
            detail: smart_parallel_detail(
                state,
                branch_name.as_deref(),
                linked_worktree_count,
                duplicate_branch_count,
            ),
            icon: state.icon(),
            tone: state.tone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartParallelState {
    Isolated,
    NeedsLane,
    BranchShared,
}

impl SmartParallelState {
    fn status(self) -> &'static str {
        match self {
            SmartParallelState::Isolated => "Isolated",
            SmartParallelState::NeedsLane => "Split recommended",
            SmartParallelState::BranchShared => "Branch overlap",
        }
    }

    fn icon(self) -> IconName {
        match self {
            SmartParallelState::Isolated => IconName::GitWorktree,
            SmartParallelState::NeedsLane => IconName::GitBranchPlus,
            SmartParallelState::BranchShared => IconName::GitMergeConflict,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            SmartParallelState::Isolated => ActivityTone::Done,
            SmartParallelState::NeedsLane => ActivityTone::Waiting,
            SmartParallelState::BranchShared => ActivityTone::Failed,
        }
    }
}

fn smart_parallel_state(
    is_linked_worktree: bool,
    duplicate_branch_count: usize,
) -> SmartParallelState {
    if duplicate_branch_count > 0 {
        return SmartParallelState::BranchShared;
    }

    if is_linked_worktree {
        SmartParallelState::Isolated
    } else {
        SmartParallelState::NeedsLane
    }
}

fn smart_parallel_detail(
    state: SmartParallelState,
    branch_name: Option<&str>,
    linked_worktree_count: usize,
    duplicate_branch_count: usize,
) -> String {
    let branch = branch_name.unwrap_or("detached HEAD");

    match state {
        SmartParallelState::Isolated => {
            if linked_worktree_count == 0 {
                format!("{branch} is already running in an isolated lane for this session.")
            } else {
                let lane_label = if linked_worktree_count == 1 {
                    "other lane"
                } else {
                    "other lanes"
                };
                format!(
                    "{branch} is isolated from {linked_worktree_count} {lane_label}; parallel agents can work without sharing this checkout."
                )
            }
        }
        SmartParallelState::NeedsLane => {
            if linked_worktree_count == 0 {
                format!(
                    "{branch} is still on the main checkout. Create a lane before starting parallel agent work."
                )
            } else {
                let lane_label = if linked_worktree_count == 1 {
                    "lane exists"
                } else {
                    "lanes exist"
                };
                format!(
                    "{branch} is the main checkout while {linked_worktree_count} linked {lane_label}. Move this session into its own lane before parallel work."
                )
            }
        }
        SmartParallelState::BranchShared => {
            let lane_label = if duplicate_branch_count == 1 {
                "linked lane"
            } else {
                "linked lanes"
            };
            format!(
                "{branch} also appears in {duplicate_branch_count} {lane_label}. Switch lanes or create a fresh lane before more agents edit it."
            )
        }
    }
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
            status: "Autonomy blocked",
            detail: "Tool permission is blocking",
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
        ToolCallStatus::WaitingForConfirmation { .. } => {
            ("Permission blocked", ActivityTone::Waiting)
        }
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
    fn summarize_thread_state_prioritizes_autonomy_blockers() {
        let summary =
            summarize_thread_state(true, true, true, true, true, Some(ActivityTone::Failed));

        assert_eq!(summary.status, "Autonomy blocked");
        assert_eq!(summary.detail, "Tool permission is blocking");
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
    fn smart_panel_counts_summarize_work_scope() {
        let counts = smart_panel_counts(
            &[
                status_entry_with_diff(
                    git::status::FileStatus::index(git::status::StatusCode::Modified),
                    10,
                    2,
                ),
                status_entry_with_diff(git::status::FileStatus::Untracked, 0, 0),
                status_entry(git::status::FileStatus::Unmerged(
                    git::status::UnmergedStatus {
                        first_head: git::status::UnmergedStatusCode::Updated,
                        second_head: git::status::UnmergedStatusCode::Updated,
                    },
                )),
            ],
            2,
            true,
            1,
        );

        assert_eq!(counts.changed_count, 3);
        assert_eq!(counts.staged_count, 1);
        assert_eq!(counts.unstaged_count, 2);
        assert_eq!(counts.conflicted_count, 1);
        assert_eq!(counts.untracked_count, 1);
        assert_eq!(counts.added_lines, 10);
        assert_eq!(counts.removed_lines, 2);
        assert_eq!(counts.linked_worktree_count, 2);
        assert!(counts.is_linked_worktree);
        assert_eq!(counts.shared_branch_lane_count, 1);
    }

    #[test]
    fn smart_panel_state_prioritizes_blockers() {
        assert_eq!(smart_panel_state(0, 0), SmartPanelState::Clean);
        assert_eq!(smart_panel_state(2, 0), SmartPanelState::InProgress);
        assert_eq!(smart_panel_state(2, 1), SmartPanelState::Blocked);
    }

    #[test]
    fn smart_panel_detail_summarizes_dirty_scope() {
        let detail = smart_panel_detail(
            SmartPanelState::InProgress,
            Some("feature"),
            SmartPanelCounts {
                changed_count: 3,
                staged_count: 1,
                unstaged_count: 2,
                conflicted_count: 0,
                untracked_count: 1,
                added_lines: 12,
                removed_lines: 4,
                linked_worktree_count: 0,
                is_linked_worktree: false,
                shared_branch_lane_count: 0,
            },
        );

        assert!(detail.contains("feature: 3 changed files"));
        assert!(detail.contains("1 staged"));
        assert!(detail.contains("2 unstaged"));
        assert!(detail.contains("1 new"));
        assert!(detail.contains("+12 -4"));
    }

    #[test]
    fn smart_panel_detail_summarizes_clean_lane() {
        let detail = smart_panel_detail(
            SmartPanelState::Clean,
            Some("feature"),
            SmartPanelCounts {
                changed_count: 0,
                staged_count: 0,
                unstaged_count: 0,
                conflicted_count: 0,
                untracked_count: 0,
                added_lines: 0,
                removed_lines: 0,
                linked_worktree_count: 1,
                is_linked_worktree: true,
                shared_branch_lane_count: 0,
            },
        );

        assert!(detail.contains("feature: no local file changes"));
        assert!(detail.contains("this session is isolated"));
    }

    #[test]
    fn smart_panel_lane_item_surfaces_branch_overlap() {
        let item = smart_panel_lane_item(
            Some("feature"),
            SmartPanelCounts {
                changed_count: 0,
                staged_count: 0,
                unstaged_count: 0,
                conflicted_count: 0,
                untracked_count: 0,
                added_lines: 0,
                removed_lines: 0,
                linked_worktree_count: 2,
                is_linked_worktree: true,
                shared_branch_lane_count: 1,
            },
        );

        assert_eq!(item.status, "Overlap");
        assert!(item.detail.contains("feature overlaps 1 linked lane"));
        assert_eq!(item.tone, ActivityTone::Failed);
    }

    #[test]
    fn smart_panel_merge_item_reuses_merge_readiness() {
        let item = smart_panel_merge_item(Some("feature"), 0, 0, SmartMergeState::Ready);

        assert_eq!(item.status, "Ready");
        assert!(item.detail.contains("feature is clean locally"));
        assert_eq!(item.tone, ActivityTone::Done);
    }

    #[test]
    fn smart_panel_compact_label_flattens_multiline_goals() {
        assert_eq!(
            smart_panel_compact_label("Build smart panel\n\nthen run checks", 80),
            "Build smart panel then run checks"
        );
    }

    #[test]
    fn smart_panel_file_status_is_review_facing() {
        assert_eq!(
            smart_panel_file_status(git::status::FileStatus::Untracked),
            ("New", IconName::File, ActivityTone::Waiting)
        );
        assert_eq!(
            smart_panel_file_status(git::status::FileStatus::index(
                git::status::StatusCode::Modified,
            )),
            ("Staged", IconName::Check, ActivityTone::Done)
        );
        assert_eq!(
            smart_panel_file_status(git::status::FileStatus::Unmerged(
                git::status::UnmergedStatus {
                    first_head: git::status::UnmergedStatusCode::Updated,
                    second_head: git::status::UnmergedStatusCode::Updated,
                },
            )),
            ("Conflict", IconName::GitMergeConflict, ActivityTone::Failed)
        );
    }

    #[test]
    fn smart_todo_state_prioritizes_autonomy_blockers() {
        assert_eq!(
            smart_todo_state(true, 2, true, true, true, true, Some(ActivityTone::Failed)),
            SmartTodoState::AutonomyBlocked
        );
        assert_eq!(
            smart_todo_state(true, 2, false, false, false, true, None),
            SmartTodoState::Blocked
        );
        assert_eq!(
            smart_todo_state(true, 2, false, true, false, false, None),
            SmartTodoState::Working
        );
    }

    #[test]
    fn smart_todo_state_distinguishes_plan_lifecycle() {
        assert_eq!(
            smart_todo_state(true, 2, false, false, false, false, None),
            SmartTodoState::Planned
        );
        assert_eq!(
            smart_todo_state(true, 0, false, false, false, false, None),
            SmartTodoState::Complete
        );
        assert_eq!(
            smart_todo_state(false, 0, false, false, false, false, None),
            SmartTodoState::Empty
        );
    }

    #[test]
    fn smart_todo_detail_points_to_next_action() {
        let detail = smart_todo_detail(
            SmartTodoState::Planned,
            Some("Run validation"),
            None,
            3,
            1,
            4,
        );

        assert!(detail.contains("3 todo step(s) remain"));
        assert!(detail.contains("Next: Run validation"));
    }

    #[test]
    fn smart_todo_detail_surfaces_autonomy_and_runtime_blockers() {
        let autonomy_blocker = smart_todo_detail(
            SmartTodoState::AutonomyBlocked,
            None,
            Some("Run command"),
            1,
            0,
            1,
        );
        let blocked = smart_todo_detail(SmartTodoState::Blocked, None, Some("Run tests"), 1, 0, 1);

        assert!(autonomy_blocker.contains("Autonomy blocker: Run command"));
        assert!(autonomy_blocker.contains("tool permission"));
        assert!(blocked.contains("Blocked by Run tests"));
    }

    #[test]
    fn smart_todo_labels_are_compact() {
        assert_eq!(smart_todo_progress_label(2, 5), "2/5 done");
        assert_eq!(smart_todo_progress_label(0, 0), "no plan");
        assert_eq!(smart_todo_left_label(1), "1 left");
        assert_eq!(smart_todo_left_label(3), "3 left");
    }

    #[test]
    fn plan_entry_status_is_review_facing() {
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::InProgress),
            ("Doing", IconName::TodoProgress, ActivityTone::Running)
        );
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::Completed),
            ("Done", IconName::TodoComplete, ActivityTone::Done)
        );
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::Pending),
            ("Queued", IconName::TodoPending, ActivityTone::Waiting)
        );
    }

    #[test]
    fn smart_parallel_state_recommends_lane_for_main_checkout() {
        assert_eq!(
            smart_parallel_state(false, 0),
            SmartParallelState::NeedsLane
        );
    }

    #[test]
    fn smart_parallel_state_accepts_linked_worktree() {
        assert_eq!(smart_parallel_state(true, 0), SmartParallelState::Isolated);
    }

    #[test]
    fn smart_parallel_state_prioritizes_branch_overlap() {
        assert_eq!(
            smart_parallel_state(true, 1),
            SmartParallelState::BranchShared
        );
        assert_eq!(
            smart_parallel_state(false, 1),
            SmartParallelState::BranchShared
        );
    }

    #[test]
    fn smart_parallel_detail_explains_main_checkout_risk() {
        let detail = smart_parallel_detail(SmartParallelState::NeedsLane, Some("main"), 2, 0);

        assert!(detail.contains("main is the main checkout"));
        assert!(detail.contains("2 linked lanes exist"));
        assert!(detail.contains("its own lane"));
    }

    #[test]
    fn smart_parallel_detail_explains_isolated_lane() {
        let detail = smart_parallel_detail(SmartParallelState::Isolated, Some("feature"), 1, 0);

        assert!(detail.contains("feature is isolated"));
        assert!(detail.contains("1 other lane"));
    }

    #[test]
    fn smart_parallel_detail_explains_branch_overlap() {
        let detail = smart_parallel_detail(SmartParallelState::BranchShared, Some("feature"), 2, 2);

        assert!(detail.contains("feature also appears"));
        assert!(detail.contains("2 linked lanes"));
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

    fn status_entry_with_diff(
        status: git::status::FileStatus,
        added: u32,
        deleted: u32,
    ) -> StatusEntry {
        StatusEntry {
            repo_path: git::repository::RepoPath::new("src/main.rs")
                .expect("test path should be a valid repo path"),
            status,
            diff_stat: Some(git::status::DiffStat { added, deleted }),
        }
    }
}
