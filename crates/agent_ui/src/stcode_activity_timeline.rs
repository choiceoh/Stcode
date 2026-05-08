use acp_thread::{AcpThread, AgentThreadEntry, ThreadStatus, ToolCall, ToolCallStatus};
use agent_client_protocol as acp;
use anyhow::Result;
use git::{
    repository::{DiffType, UpstreamTrackingStatus},
    status::FileStatus,
};
use gpui::{
    Action, Context, Entity, EntityId, IntoElement, Render, RenderOnce, Subscription, WeakEntity,
};
use project::{
    Project,
    git_store::{Repository, StatusEntry},
};
use ui::{Button, ButtonStyle, Icon, IconName, Label, LabelSize, prelude::*};
use util::{paths::PathStyle, truncate_and_trailoff};
use workspace::{ItemHandle, OpenLog, StatusItemView, Workspace};
use zed_actions::{
    CreateWorktree, NewWorktreeBranchTarget, agent::ReviewBranchDiff, git as zed_git,
};

const MAX_TIMELINE_ENTRIES: usize = 2;
const MAX_ENTRY_LABEL_CHARS: usize = 48;
const MAX_SMART_PANEL_GOAL_CHARS: usize = 64;
const MAX_SMART_PANEL_FILES: usize = 2;
const MAX_SMART_TODO_ENTRIES: usize = 2;
const MAX_SMART_PARALLEL_LANES: usize = 4;
const MAX_TODO_LABEL_CHARS: usize = 56;
const MAX_WORKLINE_DETAIL_CHARS: usize = 72;
const MAX_STATUS_WORKLINE_DETAIL_CHARS: usize = 64;
const STCODE_ACTIVITY_SIDE_PANEL_WIDTH: Pixels = px(420.);
const STCODE_ACTIVITY_SIDE_PANEL_MIN_WIDTH: Pixels = px(360.);

#[derive(IntoElement)]
pub(crate) struct StcodeActivityTimeline {
    thread: Option<Entity<AcpThread>>,
    project: Entity<Project>,
    smart_run: Option<StcodeSmartRunSnapshot>,
    layout: StcodeActivityLayout,
}

impl StcodeActivityTimeline {
    pub(crate) fn summary(
        thread: Option<Entity<AcpThread>>,
        project: Entity<Project>,
        smart_run: Option<StcodeSmartRunSnapshot>,
    ) -> Self {
        Self {
            thread,
            project,
            smart_run,
            layout: StcodeActivityLayout::Summary,
        }
    }

    pub(crate) fn side_panel(
        thread: Option<Entity<AcpThread>>,
        project: Entity<Project>,
        smart_run: Option<StcodeSmartRunSnapshot>,
    ) -> Self {
        Self {
            thread,
            project,
            smart_run,
            layout: StcodeActivityLayout::SidePanel,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StcodeActivityLayout {
    Summary,
    SidePanel,
}

#[derive(Clone)]
struct ActivityTimelineSnapshots {
    has_thread_entries: bool,
    activity: ActivitySnapshot,
    smart_run: Option<StcodeSmartRunSnapshot>,
    smart_start: Option<SmartStartSnapshot>,
    smart_panel: Option<SmartPanelSnapshot>,
    smart_todo: Option<SmartTodoSnapshot>,
    smart_parallel: Option<SmartParallelSnapshot>,
    smart_merge: Option<SmartMergeSnapshot>,
}

impl ActivityTimelineSnapshots {
    fn from_parts(
        thread: Option<&AcpThread>,
        project: &Entity<Project>,
        smart_run: Option<StcodeSmartRunSnapshot>,
        cx: &App,
    ) -> Self {
        let has_thread_entries = thread.is_some_and(|thread| !thread.entries().is_empty());
        let activity = thread
            .map(|thread| ActivitySnapshot::from_thread(thread, cx))
            .unwrap_or_else(ActivitySnapshot::empty);
        let smart_todo = thread.and_then(|thread| SmartTodoSnapshot::from_thread(thread, cx));

        Self {
            has_thread_entries,
            activity,
            smart_run,
            smart_start: (!has_thread_entries)
                .then(|| SmartStartSnapshot::from_project(project, cx))
                .flatten(),
            smart_panel: SmartPanelSnapshot::from_project(project, thread, cx),
            smart_todo,
            smart_parallel: SmartParallelSnapshot::from_project(project, cx),
            smart_merge: SmartMergeSnapshot::from_project(project, cx),
        }
    }

    fn workline(&self) -> SmartWorklineSnapshot {
        SmartWorklineSnapshot::from_snapshots(
            self.has_thread_entries,
            &self.activity,
            self.smart_run.as_ref(),
            self.smart_start.as_ref(),
            self.smart_panel.as_ref(),
            self.smart_todo.as_ref(),
            self.smart_parallel.as_ref(),
            self.smart_merge.as_ref(),
        )
    }
}

pub struct StcodeWorklineStatusItem {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    observed_agent_panel_id: Option<EntityId>,
    observed_thread_id: Option<EntityId>,
    _workspace_subscription: Subscription,
    _project_subscription: Subscription,
    _agent_panel_subscription: Option<Subscription>,
    _thread_subscription: Option<Subscription>,
}

impl StcodeWorklineStatusItem {
    pub fn new(
        workspace: &Entity<Workspace>,
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _workspace_subscription = cx.observe(workspace, |_this, _workspace, cx| cx.notify());
        let _project_subscription = cx.observe(&project, |_this, _project, cx| cx.notify());

        Self {
            workspace: workspace.downgrade(),
            project,
            observed_agent_panel_id: None,
            observed_thread_id: None,
            _workspace_subscription,
            _project_subscription,
            _agent_panel_subscription: None,
            _thread_subscription: None,
        }
    }

    fn observe_agent_panel(
        &mut self,
        agent_panel: Option<&Entity<crate::AgentPanel>>,
        cx: &mut Context<Self>,
    ) {
        let agent_panel_id = agent_panel.map(|agent_panel| agent_panel.entity_id());
        if self.observed_agent_panel_id == agent_panel_id {
            return;
        }

        self.observed_agent_panel_id = agent_panel_id;
        self._agent_panel_subscription = agent_panel
            .map(|agent_panel| cx.observe(agent_panel, |_this, _agent_panel, cx| cx.notify()));
    }

    fn observe_thread(&mut self, thread: Option<&Entity<AcpThread>>, cx: &mut Context<Self>) {
        let thread_id = thread.map(|thread| thread.entity_id());
        if self.observed_thread_id == thread_id {
            return;
        }

        self.observed_thread_id = thread_id;
        self._thread_subscription =
            thread.map(|thread| cx.observe(thread, |_this, _thread, cx| cx.notify()));
    }
}

impl Render for StcodeWorklineStatusItem {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_panel = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).panel::<crate::AgentPanel>(cx));
        self.observe_agent_panel(agent_panel.as_ref(), cx);

        let (thread, smart_run) = agent_panel
            .as_ref()
            .map(|agent_panel| {
                let agent_panel = agent_panel.read(cx);
                (
                    agent_panel.active_agent_thread(cx),
                    agent_panel.stcode_smart_run_snapshot(cx),
                )
            })
            .unwrap_or((None, None));
        self.observe_thread(thread.as_ref(), cx);

        let thread = thread.as_ref().map(|thread| thread.read(cx));
        let snapshots = ActivityTimelineSnapshots::from_parts(thread, &self.project, smart_run, cx);
        render_smart_workline_status_bar(snapshots.workline(), cx)
    }
}

impl StatusItemView for StcodeWorklineStatusItem {
    fn set_active_pane_item(
        &mut self,
        _active_pane_item: Option<&dyn ItemHandle>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StcodeSmartRunPhase {
    Pending,
    Active,
    Complete,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StcodeSmartRunSnapshot {
    pub(crate) title: String,
    pub(crate) status: &'static str,
    pub(crate) detail: String,
    pub(crate) phase: StcodeSmartRunPhase,
    pub(crate) steps: Vec<StcodeSmartRunStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StcodeSmartRunStep {
    pub(crate) label: &'static str,
    pub(crate) status: &'static str,
    pub(crate) phase: StcodeSmartRunPhase,
}

impl StcodeSmartRunSnapshot {
    fn should_render_card(&self) -> bool {
        self.phase != StcodeSmartRunPhase::Complete
    }
}

impl RenderOnce for StcodeActivityTimeline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let thread = self.thread.as_ref().map(|thread| thread.read(cx));
        let snapshots =
            ActivityTimelineSnapshots::from_parts(thread, &self.project, self.smart_run, cx);

        match self.layout {
            StcodeActivityLayout::Summary => render_activity_summary(snapshots, cx),
            StcodeActivityLayout::SidePanel => render_activity_side_panel(snapshots, cx),
        }
    }
}

fn render_activity_summary(snapshots: ActivityTimelineSnapshots, cx: &mut App) -> gpui::AnyElement {
    let activity = snapshots.activity;
    let live_run = snapshots
        .smart_run
        .as_ref()
        .filter(|snapshot| snapshot.should_render_card());
    let (status, detail, icon, tone) = live_run
        .map(|snapshot| {
            (
                snapshot.status,
                snapshot.detail.clone(),
                stcode_smart_run_phase_icon(snapshot.phase),
                stcode_smart_run_phase_tone(snapshot.phase),
            )
        })
        .unwrap_or((
            activity.status,
            activity.detail.to_string(),
            activity.icon,
            activity.tone,
        ));

    h_flex()
        .id("stcode-activity-summary")
        .flex_none()
        .w_full()
        .justify_between()
        .gap_3()
        .px_3()
        .py_2()
        .bg(cx.theme().colors().panel_background)
        .border_b_1()
        .border_color(cx.theme().colors().border)
        .child(
            h_flex()
                .min_w_0()
                .gap_2()
                .child(Icon::new(icon).size(IconSize::Small).color(tone.color()))
                .child(
                    Label::new("Workspace Activity")
                        .size(LabelSize::Default)
                        .color(Color::Muted),
                ),
        )
        .child(
            h_flex()
                .min_w_0()
                .gap_1()
                .child(
                    Label::new(status)
                        .size(LabelSize::Small)
                        .color(tone.color()),
                )
                .child(
                    Label::new(detail)
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                ),
        )
        .into_any_element()
}

fn render_activity_side_panel(
    snapshots: ActivityTimelineSnapshots,
    cx: &mut App,
) -> gpui::AnyElement {
    let workline = snapshots.workline();
    let ActivityTimelineSnapshots {
        activity,
        smart_run: _,
        smart_start,
        smart_panel,
        smart_todo,
        smart_parallel,
        smart_merge,
        ..
    } = snapshots;

    v_flex()
        .id("stcode-activity-side-panel")
        .flex_none()
        .h_full()
        .w(STCODE_ACTIVITY_SIDE_PANEL_WIDTH)
        .min_w(STCODE_ACTIVITY_SIDE_PANEL_MIN_WIDTH)
        .gap_1()
        .px_3()
        .py_2()
        .bg(cx.theme().colors().panel_background)
        .border_l_1()
        .border_color(cx.theme().colors().border)
        .overflow_y_scroll()
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
                            Icon::new(activity.icon)
                                .size(IconSize::Small)
                                .color(activity.tone.color()),
                        )
                        .child(
                            Label::new("AI Smart Panel")
                                .size(LabelSize::Default)
                                .color(Color::Muted),
                        ),
                )
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            Label::new(activity.status)
                                .size(LabelSize::Small)
                                .color(activity.tone.color()),
                        )
                        .child(
                            Label::new(activity.detail)
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        )
        .child(render_smart_workline_card(
            workline,
            smart_start,
            smart_panel,
            smart_todo,
            smart_parallel,
            smart_merge,
            cx,
        ))
        .children(activity.entries.into_iter().map(render_activity_entry))
        .into_any_element()
}

#[derive(Clone)]
struct SmartWorklineSnapshot {
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    primary_action: Option<SmartWorklineAction>,
    stages: Vec<SmartWorklineStage>,
}

impl SmartWorklineSnapshot {
    fn from_snapshots(
        has_thread_entries: bool,
        activity: &ActivitySnapshot,
        smart_run: Option<&StcodeSmartRunSnapshot>,
        smart_start: Option<&SmartStartSnapshot>,
        smart_panel: Option<&SmartPanelSnapshot>,
        smart_todo: Option<&SmartTodoSnapshot>,
        smart_parallel: Option<&SmartParallelSnapshot>,
        smart_merge: Option<&SmartMergeSnapshot>,
    ) -> Self {
        let active_stage = smart_workline_active_stage(
            has_thread_entries,
            activity,
            smart_run,
            smart_start,
            smart_panel,
            smart_todo,
            smart_parallel,
            smart_merge,
        );
        let stages = vec![
            smart_workline_start_stage(smart_start, active_stage),
            smart_workline_plan_stage(smart_todo, has_thread_entries, active_stage),
            smart_workline_parallel_stage(smart_parallel, active_stage),
            smart_workline_execute_stage(activity, has_thread_entries, active_stage),
            smart_workline_review_stage(smart_panel, has_thread_entries, active_stage),
            smart_workline_merge_stage(smart_merge, has_thread_entries, active_stage),
        ];
        let display_stage = stages
            .iter()
            .find(|stage| stage.active)
            .or_else(|| stages.last())
            .expect("workline always has stages");

        Self {
            status: display_stage.status,
            detail: display_stage.detail.clone(),
            icon: display_stage.icon,
            tone: display_stage.tone,
            primary_action: active_stage.map(SmartWorklineAction::from_stage),
            stages,
        }
    }
}

#[derive(Clone)]
struct SmartWorklineStage {
    kind: SmartWorklineStageKind,
    label: &'static str,
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmartWorklineStageKind {
    Start,
    Plan,
    Parallel,
    Execute,
    Review,
    Merge,
}

impl SmartWorklineStageKind {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Plan => "Plan",
            Self::Parallel => "Parallel",
            Self::Execute => "Execute",
            Self::Review => "Review",
            Self::Merge => "Merge",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SmartWorklineAction {
    Start,
    Panel,
    Parallel,
    Merge,
}

impl SmartWorklineAction {
    fn from_stage(stage: SmartWorklineStageKind) -> Self {
        match stage {
            SmartWorklineStageKind::Start => Self::Start,
            SmartWorklineStageKind::Plan
            | SmartWorklineStageKind::Execute
            | SmartWorklineStageKind::Review => Self::Panel,
            SmartWorklineStageKind::Parallel => Self::Parallel,
            SmartWorklineStageKind::Merge => Self::Merge,
        }
    }

    fn boxed_action(self) -> Box<dyn Action> {
        match self {
            Self::Start => crate::StcodeSmartStart.boxed_clone(),
            Self::Panel => crate::StcodeSmartPanel.boxed_clone(),
            Self::Parallel => crate::StcodeSmartParallel.boxed_clone(),
            Self::Merge => crate::StcodeSmartMerge.boxed_clone(),
        }
    }
}

fn render_smart_workline_status_bar(
    snapshot: SmartWorklineSnapshot,
    _cx: &mut App,
) -> gpui::AnyElement {
    let active_action = snapshot
        .stages
        .iter()
        .find(|stage| stage.active)
        .map(|stage| SmartWorklineAction::from_stage(stage.kind));
    let detail = truncate_and_trailoff(&snapshot.detail, MAX_STATUS_WORKLINE_DETAIL_CHARS);

    h_flex()
        .id("stcode-ai-workline-control-bar")
        .min_w_0()
        .gap_2()
        .overflow_hidden()
        .child(
            h_flex()
                .min_w_0()
                .gap_2()
                .child(
                    Icon::new(snapshot.icon)
                        .size(IconSize::Medium)
                        .color(snapshot.tone.color()),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    Label::new("AI Workline")
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
                            Label::new(detail)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .child(render_smart_workline_status_action(
                    "stcode-status-start",
                    "Start",
                    SmartWorklineAction::Start.boxed_action(),
                    active_action == Some(SmartWorklineAction::Start),
                ))
                .child(render_smart_workline_status_action(
                    "stcode-status-review",
                    "Review",
                    SmartWorklineAction::Panel.boxed_action(),
                    active_action == Some(SmartWorklineAction::Panel),
                ))
                .child(render_smart_workline_status_action(
                    "stcode-status-merge",
                    "Merge",
                    SmartWorklineAction::Merge.boxed_action(),
                    active_action == Some(SmartWorklineAction::Merge),
                ))
                .child(render_smart_workline_status_action(
                    "stcode-status-parallel",
                    "Parallel",
                    SmartWorklineAction::Parallel.boxed_action(),
                    active_action == Some(SmartWorklineAction::Parallel),
                ))
                .child(render_smart_workline_status_action(
                    "stcode-status-logs",
                    "Logs",
                    OpenLog.boxed_clone(),
                    false,
                )),
        )
        .into_any_element()
}

fn render_smart_workline_status_action(
    id: &'static str,
    label: &'static str,
    action: Box<dyn Action>,
    active: bool,
) -> impl IntoElement {
    Button::new(id, label)
        .size(ButtonSize::Default)
        .label_size(LabelSize::Small)
        .style(if active {
            ButtonStyle::Filled
        } else {
            ButtonStyle::Subtle
        })
        .on_click(move |_, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        })
}

fn render_smart_workline_card(
    snapshot: SmartWorklineSnapshot,
    smart_start: Option<SmartStartSnapshot>,
    smart_panel: Option<SmartPanelSnapshot>,
    smart_todo: Option<SmartTodoSnapshot>,
    smart_parallel: Option<SmartParallelSnapshot>,
    smart_merge: Option<SmartMergeSnapshot>,
    cx: &mut App,
) -> impl IntoElement {
    let detail_rows = smart_workline_detail_rows(
        smart_panel.as_ref(),
        smart_todo.as_ref(),
        smart_parallel.as_ref(),
        smart_merge.as_ref(),
    );
    let has_detail_rows = !detail_rows.is_empty();
    let primary_action = snapshot
        .primary_action
        .map(SmartWorklineAction::boxed_action);
    let start_review_repository = smart_start
        .as_ref()
        .map(|snapshot| snapshot.repository.clone());
    let start_stash_repository = smart_start
        .as_ref()
        .map(|snapshot| snapshot.repository.clone());
    let panel_review_repository = smart_panel
        .as_ref()
        .map(|snapshot| snapshot.repository.clone());
    let merge_review_repository = smart_merge
        .as_ref()
        .map(|snapshot| snapshot.repository.clone());
    let can_start_handoff = smart_start.is_some();
    let can_review_files = smart_panel
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_review);
    let can_commit_panel = smart_panel
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_commit);
    let can_manage_parallel = smart_parallel
        .as_ref()
        .is_some_and(|snapshot| snapshot.should_render_card());
    let can_review_merge = smart_merge
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_review);
    let can_open_pull_request = smart_merge
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_create_pull_request);
    let can_run_smart_merge = smart_merge
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_run_smart_merge);
    let can_commit_merge = smart_merge
        .as_ref()
        .is_some_and(|snapshot| snapshot.can_commit);
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
        .id("stcode-smart-workline-card")
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
                                    Label::new("AI Smart Workline")
                                        .size(LabelSize::Default)
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
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                ),
        )
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .children(snapshot.stages.into_iter().map(render_smart_workline_stage)),
        )
        .when(has_detail_rows, |this| {
            this.child(
                v_flex()
                    .id("stcode-smart-workline-details")
                    .w_full()
                    .gap_0p5()
                    .children(
                        detail_rows
                            .into_iter()
                            .map(render_smart_workline_detail_row),
                    ),
            )
        })
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .when_some(primary_action, |this, action| {
                    this.child(
                        Button::new("stcode-smart-workline-continue", "Continue")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_start_handoff, |this| {
                    this.when_some(start_review_repository, |this, repository| {
                        this.child(
                            Button::new("stcode-smart-workline-start-review", "Review")
                                .label_size(LabelSize::XSmall)
                                .style(ButtonStyle::Outlined)
                                .on_click(move |_, window, cx| {
                                    review_leftover_changes(repository.clone(), window, cx);
                                }),
                        )
                    })
                    .child(
                        Button::new("stcode-smart-workline-split", "Split Worktree")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(zed_git::Worktree.boxed_clone(), cx);
                            }),
                    )
                    .when_some(start_stash_repository, |this, repository| {
                        this.child(
                            Button::new("stcode-smart-workline-stash", "Stash")
                                .label_size(LabelSize::XSmall)
                                .style(ButtonStyle::Outlined)
                                .on_click(move |_, _window, cx| {
                                    stash_leftover_changes(repository.clone(), cx);
                                }),
                        )
                    })
                    .child(
                        Button::new("stcode-smart-workline-start-commit", "Commit")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(git::ExpandCommitEditor.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_manage_parallel, |this| {
                    this.child(
                        Button::new("stcode-smart-workline-create-lane", "Create Lane")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    CreateWorktree {
                                        worktree_name: None,
                                        branch_target: NewWorktreeBranchTarget::CurrentBranch,
                                    }
                                    .boxed_clone(),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new("stcode-smart-workline-manage-lanes", "Manage Lanes")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(zed_git::Worktree.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_review_files, |this| {
                    this.when_some(panel_review_repository, |this, repository| {
                        this.child(
                            Button::new("stcode-smart-workline-review-files", "Review Files")
                                .label_size(LabelSize::XSmall)
                                .style(ButtonStyle::Outlined)
                                .on_click(move |_, window, cx| {
                                    review_leftover_changes(repository.clone(), window, cx);
                                }),
                        )
                    })
                })
                .when(can_commit_panel, |this| {
                    this.child(
                        Button::new("stcode-smart-workline-panel-commit", "Commit")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(git::ExpandCommitEditor.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_review_merge, |this| {
                    this.when_some(merge_review_repository, |this, repository| {
                        this.child(
                            Button::new("stcode-smart-workline-review-merge", "Review Merge")
                                .label_size(LabelSize::XSmall)
                                .style(ButtonStyle::Outlined)
                                .on_click(move |_, window, cx| {
                                    review_merge_readiness(repository.clone(), window, cx);
                                }),
                        )
                    })
                })
                .when(can_run_smart_merge, |this| {
                    this.child(
                        Button::new("stcode-smart-workline-ai-merge", "AI Merge")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(crate::StcodeSmartMerge.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_open_pull_request, |this| {
                    this.child(
                        Button::new("stcode-smart-workline-open-pr", "Create PR Link")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window
                                    .dispatch_action(zed_git::CreatePullRequest.boxed_clone(), cx);
                            }),
                    )
                })
                .when(can_commit_merge, |this| {
                    this.child(
                        Button::new("stcode-smart-workline-merge-commit", "Commit")
                            .label_size(LabelSize::XSmall)
                            .style(ButtonStyle::Outlined)
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(git::ExpandCommitEditor.boxed_clone(), cx);
                            }),
                    )
                }),
        )
}

#[derive(Clone)]
struct SmartWorklineDetailRow {
    id: String,
    label: String,
    detail: String,
    status: &'static str,
    icon: IconName,
    tone: ActivityTone,
}

fn smart_workline_detail_rows(
    smart_panel: Option<&SmartPanelSnapshot>,
    smart_todo: Option<&SmartTodoSnapshot>,
    smart_parallel: Option<&SmartParallelSnapshot>,
    smart_merge: Option<&SmartMergeSnapshot>,
) -> Vec<SmartWorklineDetailRow> {
    let mut rows = Vec::new();

    if let Some(todo) = smart_todo {
        rows.push(SmartWorklineDetailRow {
            id: "todo-progress".to_string(),
            label: "Plan".to_string(),
            detail: format!("{} · {}", todo.progress_label, todo.left_label),
            status: todo.status,
            icon: todo.icon,
            tone: todo.tone,
        });

        if let Some(item) = todo
            .items
            .iter()
            .find(|item| item.tone.is_live())
            .or_else(|| todo.items.first())
        {
            rows.push(SmartWorklineDetailRow {
                id: format!("todo-item-{}", item.id),
                label: "Todo".to_string(),
                detail: smart_panel_compact_label(&item.label, MAX_WORKLINE_DETAIL_CHARS),
                status: item.status,
                icon: item.icon,
                tone: item.tone,
            });
        }
    }

    if let Some(parallel) = smart_parallel.filter(|snapshot| snapshot.should_render_card()) {
        let detail = parallel
            .lanes
            .iter()
            .find(|lane| lane.overlaps_active_branch)
            .or_else(|| parallel.lanes.first())
            .map(|lane| {
                format!(
                    "{} · {} · {}",
                    lane.label,
                    lane.branch_label(),
                    lane.path_label
                )
            })
            .unwrap_or_else(|| parallel.detail.clone());

        rows.push(SmartWorklineDetailRow {
            id: "parallel-lane".to_string(),
            label: "Lane".to_string(),
            detail: smart_panel_compact_label(detail, MAX_WORKLINE_DETAIL_CHARS),
            status: parallel.status,
            icon: parallel.icon,
            tone: parallel.tone,
        });
    }

    if let Some(panel) = smart_panel {
        rows.push(SmartWorklineDetailRow {
            id: "workspace".to_string(),
            label: "Workspace".to_string(),
            detail: smart_panel_compact_label(&panel.detail, MAX_WORKLINE_DETAIL_CHARS),
            status: panel.status,
            icon: panel.icon,
            tone: panel.tone,
        });

        if let Some(merge) = smart_merge.filter(|snapshot| snapshot.should_render_card()) {
            rows.push(SmartWorklineDetailRow {
                id: "merge".to_string(),
                label: "Merge".to_string(),
                detail: smart_panel_compact_label(&merge.detail, MAX_WORKLINE_DETAIL_CHARS),
                status: merge.status,
                icon: merge.icon,
                tone: merge.tone,
            });
        }

        if let Some(item) = panel
            .work_items
            .iter()
            .find(|item| item.tone.needs_attention())
            .or_else(|| panel.work_items.iter().find(|item| item.tone.is_live()))
            .or_else(|| panel.work_items.iter().find(|item| item.id == "goal"))
        {
            rows.push(SmartWorklineDetailRow {
                id: format!("work-item-{}", item.id),
                label: item.label.to_string(),
                detail: smart_panel_compact_label(&item.detail, MAX_WORKLINE_DETAIL_CHARS),
                status: item.status,
                icon: item.icon,
                tone: item.tone,
            });
        }

        if let Some(file) = panel.files.first() {
            rows.push(SmartWorklineDetailRow {
                id: format!("file-{}", file.id),
                label: "File".to_string(),
                detail: smart_panel_compact_label(&file.path, MAX_WORKLINE_DETAIL_CHARS),
                status: file.status,
                icon: file.icon,
                tone: file.tone,
            });
        }
    }

    if rows.is_empty()
        && let Some(merge) = smart_merge
    {
        rows.push(SmartWorklineDetailRow {
            id: "merge".to_string(),
            label: "Merge".to_string(),
            detail: smart_panel_compact_label(&merge.detail, MAX_WORKLINE_DETAIL_CHARS),
            status: merge.status,
            icon: merge.icon,
            tone: merge.tone,
        });
    }

    rows.truncate(4);
    rows
}

fn render_smart_workline_detail_row(row: SmartWorklineDetailRow) -> impl IntoElement {
    h_flex()
        .id(format!("stcode-smart-workline-detail-{}", row.id))
        .w_full()
        .min_w_0()
        .gap_1p5()
        .px_1p5()
        .child(
            Icon::new(row.icon)
                .size(IconSize::XSmall)
                .color(row.tone.color()),
        )
        .child(
            Label::new(row.label)
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            Label::new(row.detail)
                .size(LabelSize::XSmall)
                .color(Color::Default)
                .truncate(),
        )
        .child(
            Label::new(row.status)
                .size(LabelSize::XSmall)
                .color(row.tone.color()),
        )
}

fn render_smart_workline_stage(stage: SmartWorklineStage) -> impl IntoElement {
    h_flex()
        .id(format!(
            "stcode-smart-workline-stage-{}",
            stage.kind.label()
        ))
        .w_full()
        .min_w_0()
        .gap_2()
        .px_1p5()
        .py_1()
        .rounded_sm()
        .when(stage.active, |this| this.border_1())
        .child(
            Icon::new(stage.icon)
                .size(IconSize::XSmall)
                .color(stage.tone.color()),
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
                            Label::new(stage.label)
                                .size(LabelSize::XSmall)
                                .color(Color::Default),
                        )
                        .child(
                            Label::new(stage.status)
                                .size(LabelSize::XSmall)
                                .color(stage.tone.color()),
                        ),
                )
                .child(
                    Label::new(stage.detail)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .truncate(),
                ),
        )
}

fn smart_workline_stage(
    kind: SmartWorklineStageKind,
    status: &'static str,
    detail: impl AsRef<str>,
    icon: IconName,
    tone: ActivityTone,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    SmartWorklineStage {
        kind,
        label: kind.label(),
        status,
        detail: smart_panel_compact_label(detail, MAX_WORKLINE_DETAIL_CHARS),
        icon,
        tone,
        active: active_stage == Some(kind),
    }
}

fn smart_workline_active_stage(
    has_thread_entries: bool,
    activity: &ActivitySnapshot,
    smart_run: Option<&StcodeSmartRunSnapshot>,
    smart_start: Option<&SmartStartSnapshot>,
    smart_panel: Option<&SmartPanelSnapshot>,
    smart_todo: Option<&SmartTodoSnapshot>,
    smart_parallel: Option<&SmartParallelSnapshot>,
    smart_merge: Option<&SmartMergeSnapshot>,
) -> Option<SmartWorklineStageKind> {
    if smart_start.is_some() {
        return Some(SmartWorklineStageKind::Start);
    }

    if let Some(run) = smart_run.filter(|snapshot| snapshot.should_render_card()) {
        return Some(smart_workline_stage_from_run(run));
    }

    if smart_parallel.is_some_and(|snapshot| snapshot.should_render_card()) {
        return Some(SmartWorklineStageKind::Parallel);
    }

    if let Some(todo) = smart_todo.filter(|snapshot| snapshot.should_render_card()) {
        return if todo.tone == ActivityTone::Running {
            Some(SmartWorklineStageKind::Execute)
        } else {
            Some(SmartWorklineStageKind::Plan)
        };
    }

    if activity.tone.is_live() {
        return Some(SmartWorklineStageKind::Execute);
    }

    if smart_panel.is_some_and(|snapshot| snapshot.should_render_card()) {
        return Some(SmartWorklineStageKind::Review);
    }

    if smart_merge.is_some_and(|snapshot| {
        snapshot.can_create_pull_request || (has_thread_entries && snapshot.should_render_card())
    }) {
        return Some(SmartWorklineStageKind::Merge);
    }

    None
}

fn smart_workline_stage_from_run(snapshot: &StcodeSmartRunSnapshot) -> SmartWorklineStageKind {
    if snapshot.title.contains("Start") {
        SmartWorklineStageKind::Start
    } else if snapshot.title.contains("Parallel") {
        SmartWorklineStageKind::Parallel
    } else if snapshot.title.contains("Merge") {
        SmartWorklineStageKind::Merge
    } else {
        SmartWorklineStageKind::Execute
    }
}

fn smart_workline_start_stage(
    smart_start: Option<&SmartStartSnapshot>,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    if let Some(snapshot) = smart_start {
        return smart_workline_stage(
            SmartWorklineStageKind::Start,
            "Handoff",
            &snapshot.detail,
            IconName::Warning,
            ActivityTone::Waiting,
            active_stage,
        );
    }

    smart_workline_stage(
        SmartWorklineStageKind::Start,
        "Ready",
        "Workspace handoff is clean.",
        IconName::Check,
        ActivityTone::Done,
        active_stage,
    )
}

fn smart_workline_plan_stage(
    smart_todo: Option<&SmartTodoSnapshot>,
    has_thread_entries: bool,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    if let Some(snapshot) = smart_todo {
        return smart_workline_stage(
            SmartWorklineStageKind::Plan,
            snapshot.status,
            &snapshot.detail,
            snapshot.icon,
            snapshot.tone,
            active_stage,
        );
    }

    let (status, detail, tone) = if has_thread_entries {
        (
            "Ready",
            "No live todo plan is blocking the workline.",
            ActivityTone::Done,
        )
    } else {
        (
            "Waiting",
            "Start a task to create the first plan.",
            ActivityTone::Idle,
        )
    };

    smart_workline_stage(
        SmartWorklineStageKind::Plan,
        status,
        detail,
        IconName::ListTodo,
        tone,
        active_stage,
    )
}

fn smart_workline_parallel_stage(
    smart_parallel: Option<&SmartParallelSnapshot>,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    let Some(snapshot) = smart_parallel else {
        return smart_workline_stage(
            SmartWorklineStageKind::Parallel,
            "Waiting",
            "No repository lane information is available yet.",
            IconName::GitWorktree,
            ActivityTone::Idle,
            active_stage,
        );
    };

    let tone = if snapshot.tone == ActivityTone::Done {
        ActivityTone::Done
    } else {
        snapshot.tone
    };
    smart_workline_stage(
        SmartWorklineStageKind::Parallel,
        snapshot.status,
        &snapshot.detail,
        snapshot.icon,
        tone,
        active_stage,
    )
}

fn smart_workline_execute_stage(
    activity: &ActivitySnapshot,
    has_thread_entries: bool,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    let (status, detail, icon, tone) = if has_thread_entries {
        (
            activity.status,
            activity.detail,
            activity.icon,
            activity.tone,
        )
    } else {
        (
            "Waiting",
            "No agent execution has started yet.",
            IconName::ZedAgent,
            ActivityTone::Idle,
        )
    };

    smart_workline_stage(
        SmartWorklineStageKind::Execute,
        status,
        detail,
        icon,
        tone,
        active_stage,
    )
}

fn smart_workline_review_stage(
    smart_panel: Option<&SmartPanelSnapshot>,
    has_thread_entries: bool,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    let Some(snapshot) = smart_panel else {
        return smart_workline_stage(
            SmartWorklineStageKind::Review,
            "Waiting",
            "No workspace review state is available yet.",
            IconName::ListTodo,
            ActivityTone::Idle,
            active_stage,
        );
    };

    let (status, tone) = if snapshot.counts.conflicted_count > 0 {
        ("Blocked", ActivityTone::Failed)
    } else if snapshot.counts.changed_count > 0 {
        ("Review", ActivityTone::Waiting)
    } else if has_thread_entries {
        ("Clean", ActivityTone::Done)
    } else {
        ("Waiting", ActivityTone::Idle)
    };

    smart_workline_stage(
        SmartWorklineStageKind::Review,
        status,
        &snapshot.detail,
        snapshot.icon,
        tone,
        active_stage,
    )
}

fn smart_workline_merge_stage(
    smart_merge: Option<&SmartMergeSnapshot>,
    has_thread_entries: bool,
    active_stage: Option<SmartWorklineStageKind>,
) -> SmartWorklineStage {
    let Some(snapshot) = smart_merge else {
        return smart_workline_stage(
            SmartWorklineStageKind::Merge,
            "Waiting",
            "No merge readiness is available yet.",
            IconName::PullRequest,
            ActivityTone::Idle,
            active_stage,
        );
    };

    let (status, tone) = if snapshot.can_create_pull_request {
        ("Ready", ActivityTone::Waiting)
    } else if has_thread_entries && snapshot.tone.needs_attention() {
        (snapshot.status, snapshot.tone)
    } else {
        ("Waiting", ActivityTone::Idle)
    };

    smart_workline_stage(
        SmartWorklineStageKind::Merge,
        status,
        &snapshot.detail,
        snapshot.icon,
        tone,
        active_stage,
    )
}

fn stcode_smart_run_phase_icon(phase: StcodeSmartRunPhase) -> IconName {
    match phase {
        StcodeSmartRunPhase::Pending => IconName::Circle,
        StcodeSmartRunPhase::Active => IconName::LoadCircle,
        StcodeSmartRunPhase::Complete => IconName::Check,
        StcodeSmartRunPhase::Blocked => IconName::Warning,
    }
}

fn stcode_smart_run_phase_tone(phase: StcodeSmartRunPhase) -> ActivityTone {
    match phase {
        StcodeSmartRunPhase::Pending => ActivityTone::Idle,
        StcodeSmartRunPhase::Active => ActivityTone::Running,
        StcodeSmartRunPhase::Complete => ActivityTone::Done,
        StcodeSmartRunPhase::Blocked => ActivityTone::Failed,
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
        let tracking_status = repository_ref
            .branch
            .as_ref()
            .and_then(|branch| branch.tracking_status());
        let has_upstream = repository_ref
            .branch
            .as_ref()
            .is_some_and(|branch| branch.upstream.is_some());
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
            tracking_status,
            has_upstream,
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

    fn should_render_card(&self) -> bool {
        self.tone.is_live()
            || self.counts.changed_count > 0
            || self.counts.conflicted_count > 0
            || self.counts.shared_branch_lane_count > 0
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
        detail: smart_merge_detail(
            merge_state,
            branch_name,
            changed_count,
            conflicted_count,
            None,
            None,
        ),
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
        if !has_plan && !latest_tool_tone.is_some_and(ActivityTone::is_live) {
            return None;
        }

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

    fn should_render_card(&self) -> bool {
        self.tone.is_live()
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
    lanes: Vec<SmartParallelLane>,
}

#[derive(Clone)]
struct SmartParallelLane {
    label: String,
    branch_ref: Option<String>,
    path_label: String,
    overlaps_active_branch: bool,
}

impl SmartParallelLane {
    fn branch_label(&self) -> &str {
        self.branch_ref
            .as_deref()
            .map(smart_branch_ref_label)
            .unwrap_or("detached HEAD")
    }
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
        let lanes = smart_parallel_lanes(repository_ref.linked_worktrees(), branch_ref.as_deref());
        let duplicate_branch_count = lanes
            .iter()
            .filter(|lane| lane.overlaps_active_branch)
            .count();
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
                &lanes,
            ),
            icon: state.icon(),
            tone: state.tone(),
            lanes,
        })
    }

    fn should_render_card(&self) -> bool {
        self.tone.needs_attention()
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
    lanes: &[SmartParallelLane],
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
            let overlap_labels = lanes
                .iter()
                .filter(|lane| lane.overlaps_active_branch)
                .map(|lane| lane.label.as_str())
                .take(3)
                .collect::<Vec<_>>()
                .join(", ");
            let overlap_detail = if overlap_labels.is_empty() {
                String::new()
            } else {
                format!(": {overlap_labels}")
            };
            format!(
                "{branch} also appears in {duplicate_branch_count} {lane_label}{overlap_detail}. Switch lanes or create a fresh lane before more agents edit it."
            )
        }
    }
}

fn smart_parallel_lanes(
    linked_worktrees: &[git::repository::Worktree],
    active_branch_ref: Option<&str>,
) -> Vec<SmartParallelLane> {
    linked_worktrees
        .iter()
        .take(MAX_SMART_PARALLEL_LANES)
        .map(|worktree| {
            let branch_ref = worktree.ref_name.as_ref().map(ToString::to_string);
            let overlaps_active_branch = active_branch_ref
                .zip(branch_ref.as_deref())
                .is_some_and(|(active, linked)| active == linked);

            SmartParallelLane {
                label: smart_lane_label(&worktree.path),
                branch_ref,
                path_label: worktree.path.display().to_string(),
                overlaps_active_branch,
            }
        })
        .collect()
}

fn smart_lane_label(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
}

fn smart_branch_ref_label(ref_name: &str) -> &str {
    ref_name
        .strip_prefix("refs/heads/")
        .or_else(|| ref_name.strip_prefix("refs/remotes/"))
        .unwrap_or(ref_name)
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
    can_run_smart_merge: bool,
}

impl SmartMergeSnapshot {
    fn from_project(project: &Entity<Project>, cx: &App) -> Option<Self> {
        let repository = project.read(cx).active_repository(cx)?;
        let repository_ref = repository.read(cx);
        let branch_name = repository_ref
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string());
        let tracking_status = repository_ref
            .branch
            .as_ref()
            .and_then(|branch| branch.tracking_status());
        let has_upstream = repository_ref
            .branch
            .as_ref()
            .is_some_and(|branch| branch.upstream.is_some());
        let entries = repository_ref
            .cached_status()
            .filter(|entry| entry.status.has_changes())
            .collect::<Vec<_>>();
        let changed_count = entries.len();
        let conflicted_count = entries
            .iter()
            .filter(|entry| entry.status.is_conflicted())
            .count();
        let state = smart_merge_state(
            branch_name.as_deref(),
            changed_count,
            conflicted_count,
            tracking_status,
            has_upstream,
        );

        Some(Self {
            repository,
            status: state.status(),
            detail: smart_merge_detail(
                state,
                branch_name.as_deref(),
                changed_count,
                conflicted_count,
                tracking_status.map(|status| status.ahead),
                tracking_status.map(|status| status.behind),
            ),
            icon: state.icon(),
            tone: state.tone(),
            can_review: branch_name
                .as_deref()
                .is_some_and(|branch_name| !is_merge_base_branch(branch_name)),
            can_create_pull_request: state == SmartMergeState::Ready,
            can_commit: changed_count > 0,
            can_run_smart_merge: state.can_automerge(),
        })
    }

    fn should_render_card(&self) -> bool {
        self.can_run_smart_merge || self.tone.needs_attention()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartMergeState {
    Ready,
    NeedsPublish,
    NeedsSync,
    NeedsCheckpoint,
    HasConflicts,
    ProtectedBranch,
    Detached,
}

impl SmartMergeState {
    fn status(self) -> &'static str {
        match self {
            SmartMergeState::Ready => "Ready",
            SmartMergeState::NeedsPublish => "Publish needed",
            SmartMergeState::NeedsSync => "Sync needed",
            SmartMergeState::NeedsCheckpoint => "Checkpoint needed",
            SmartMergeState::HasConflicts => "Blocked",
            SmartMergeState::ProtectedBranch => "Base branch",
            SmartMergeState::Detached => "No branch",
        }
    }

    fn icon(self) -> IconName {
        match self {
            SmartMergeState::Ready => IconName::PullRequest,
            SmartMergeState::NeedsPublish => IconName::GitBranchPlus,
            SmartMergeState::NeedsSync => IconName::PullRequest,
            SmartMergeState::NeedsCheckpoint => IconName::GitCommit,
            SmartMergeState::HasConflicts => IconName::GitMergeConflict,
            SmartMergeState::ProtectedBranch | SmartMergeState::Detached => IconName::GitBranch,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            SmartMergeState::Ready => ActivityTone::Done,
            SmartMergeState::HasConflicts => ActivityTone::Failed,
            SmartMergeState::NeedsPublish
            | SmartMergeState::NeedsSync
            | SmartMergeState::NeedsCheckpoint
            | SmartMergeState::ProtectedBranch
            | SmartMergeState::Detached => ActivityTone::Waiting,
        }
    }

    fn can_automerge(self) -> bool {
        !matches!(self, SmartMergeState::Detached)
    }
}

fn smart_merge_state(
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
    tracking_status: Option<UpstreamTrackingStatus>,
    has_upstream: bool,
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

    if !has_upstream {
        return SmartMergeState::NeedsPublish;
    }

    let Some(tracking_status) = tracking_status else {
        return SmartMergeState::NeedsPublish;
    };

    if tracking_status.behind > 0 {
        return SmartMergeState::NeedsSync;
    }

    if tracking_status.ahead > 0 {
        return SmartMergeState::NeedsPublish;
    }

    SmartMergeState::Ready
}

fn smart_merge_detail(
    state: SmartMergeState,
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
    ahead_count: Option<u32>,
    behind_count: Option<u32>,
) -> String {
    let branch = branch_name.unwrap_or("detached HEAD");

    match state {
        SmartMergeState::Ready => format!(
            "{branch} is clean, published, and ready for AI Smart Merge to watch checks and merge."
        ),
        SmartMergeState::NeedsPublish => {
            let ahead = ahead_count
                .map(|count| format!(" {count} local commit(s) are ahead of upstream."))
                .unwrap_or_default();
            format!(
                "{branch} is clean locally but still needs publishing.{ahead} AI Smart Merge can push, create the PR, watch checks, and merge."
            )
        }
        SmartMergeState::NeedsSync => {
            let behind = behind_count
                .map(|count| format!(" {count} upstream commit(s) are ahead."))
                .unwrap_or_default();
            format!(
                "{branch} needs a base sync before merge.{behind} AI Smart Merge should rebase or pull, then continue through PR and CI."
            )
        }
        SmartMergeState::NeedsCheckpoint => {
            let file_label = if changed_count == 1 { "file" } else { "files" };
            format!(
                "{changed_count} changed {file_label} remain on {branch}. AI Smart Merge should review, commit, test, push, open the PR, and continue to merge."
            )
        }
        SmartMergeState::HasConflicts => format!(
            "{conflicted_count} conflict(s) remain on {branch}. AI Smart Merge should resolve them before continuing to checks, PR, and merge."
        ),
        SmartMergeState::ProtectedBranch => format!(
            "{branch} is the base branch. AI Smart Merge should split work into a task branch before preparing the PR."
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
            .filter(|entry| entry.tone.is_live())
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

    fn is_live(self) -> bool {
        matches!(
            self,
            ActivityTone::Running | ActivityTone::Waiting | ActivityTone::Failed
        )
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
        assert!(item.detail.contains("feature is clean, published"));
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
        let detail = smart_parallel_detail(SmartParallelState::NeedsLane, Some("main"), 2, 0, &[]);

        assert!(detail.contains("main is the main checkout"));
        assert!(detail.contains("2 linked lanes exist"));
        assert!(detail.contains("its own lane"));
    }

    #[test]
    fn smart_parallel_detail_explains_isolated_lane() {
        let detail =
            smart_parallel_detail(SmartParallelState::Isolated, Some("feature"), 1, 0, &[]);

        assert!(detail.contains("feature is isolated"));
        assert!(detail.contains("1 other lane"));
    }

    #[test]
    fn smart_parallel_detail_explains_branch_overlap() {
        let detail = smart_parallel_detail(
            SmartParallelState::BranchShared,
            Some("feature"),
            2,
            2,
            &[
                SmartParallelLane {
                    label: "lane-a".to_string(),
                    branch_ref: Some("refs/heads/feature".to_string()),
                    path_label: "/worktrees/lane-a".to_string(),
                    overlaps_active_branch: true,
                },
                SmartParallelLane {
                    label: "lane-b".to_string(),
                    branch_ref: Some("refs/heads/feature".to_string()),
                    path_label: "/worktrees/lane-b".to_string(),
                    overlaps_active_branch: true,
                },
            ],
        );

        assert!(detail.contains("feature also appears"));
        assert!(detail.contains("2 linked lanes"));
        assert!(detail.contains("lane-a, lane-b"));
    }

    #[test]
    fn smart_merge_state_requires_a_branch() {
        assert_eq!(
            smart_merge_state(None, 0, 0, None, false),
            SmartMergeState::Detached
        );
    }

    #[test]
    fn smart_merge_state_blocks_base_branches() {
        assert_eq!(
            smart_merge_state(Some("main"), 0, 0, None, false),
            SmartMergeState::ProtectedBranch
        );
        assert_eq!(
            smart_merge_state(Some("origin/master"), 0, 0, None, false),
            SmartMergeState::ProtectedBranch
        );
    }

    #[test]
    fn smart_merge_state_blocks_dirty_work() {
        assert_eq!(
            smart_merge_state(Some("feature"), 2, 0, None, false),
            SmartMergeState::NeedsCheckpoint
        );
    }

    #[test]
    fn smart_merge_state_prioritizes_conflicts() {
        assert_eq!(
            smart_merge_state(Some("feature"), 2, 1, None, false),
            SmartMergeState::HasConflicts
        );
    }

    #[test]
    fn smart_merge_state_accepts_clean_feature_branch() {
        assert_eq!(
            smart_merge_state(
                Some("feature"),
                0,
                0,
                Some(UpstreamTrackingStatus {
                    ahead: 0,
                    behind: 0,
                }),
                true,
            ),
            SmartMergeState::Ready
        );
    }

    #[test]
    fn smart_merge_state_tracks_publish_and_sync_work() {
        assert_eq!(
            smart_merge_state(Some("feature"), 0, 0, None, false),
            SmartMergeState::NeedsPublish
        );
        assert_eq!(
            smart_merge_state(
                Some("feature"),
                0,
                0,
                Some(UpstreamTrackingStatus {
                    ahead: 2,
                    behind: 0,
                }),
                true,
            ),
            SmartMergeState::NeedsPublish
        );
        assert_eq!(
            smart_merge_state(
                Some("feature"),
                0,
                0,
                Some(UpstreamTrackingStatus {
                    ahead: 0,
                    behind: 1,
                }),
                true,
            ),
            SmartMergeState::NeedsSync
        );
    }

    #[test]
    fn smart_merge_detail_explains_checkpoint_requirement() {
        let detail = smart_merge_detail(
            SmartMergeState::NeedsCheckpoint,
            Some("feature"),
            3,
            0,
            None,
            None,
        );

        assert!(detail.contains("3 changed files remain on feature"));
        assert!(detail.contains("review, commit, test"));
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
