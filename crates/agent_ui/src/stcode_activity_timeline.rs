use acp_thread::{AcpThread, AgentThreadEntry, ThreadStatus, ToolCall, ToolCallStatus};
use agent_client_protocol as acp;
use anyhow::Result;
use git::{
    repository::{DiffType, UpstreamTrackingStatus},
    status::FileStatus,
};
use gpui::{
    Context, Entity, EntityId, IntoElement, Render, RenderOnce, Subscription, WeakEntity,
};
use project::{
    Project,
    git_store::{Repository, StatusEntry},
};
use ui::{Icon, IconName, Label, LabelSize, prelude::*};
use util::{paths::PathStyle, truncate_and_trailoff};
use workspace::{ItemHandle, StatusItemView, Workspace};
use zed_actions::{
    agent::ReviewBranchDiff,
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
        let snapshots = ActivityTimelineSnapshots::from_parts(
            thread,
            &self.project,
            smart_run,
            cx,
        );
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
        let snapshots = ActivityTimelineSnapshots::from_parts(
            thread,
            &self.project,
            self.smart_run,
            cx,
        );

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
                    Label::new("작업 공간")
                        .size(LabelSize::Large)
                        .color(Color::Muted),
                ),
        )
        .child(
            h_flex()
                .min_w_0()
                .gap_1()
                .child(
                    Label::new(status)
                        .size(LabelSize::Default)
                        .color(tone.color()),
                )
                .child(
                    Label::new(detail)
                        .size(LabelSize::Default)
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
    let activity = snapshots.activity;

    let active_stage = workline.stages.iter().find(|stage| stage.active);

    v_flex()
        .id("stcode-activity-side-panel")
        .flex_none()
        .h_full()
        .w(STCODE_ACTIVITY_SIDE_PANEL_WIDTH)
        .min_w(STCODE_ACTIVITY_SIDE_PANEL_MIN_WIDTH)
        .gap_4()
        .px_4()
        .py_3()
        .bg(cx.theme().colors().panel_background)
        .border_l_1()
        .border_color(cx.theme().colors().border)
        .overflow_y_scroll()
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Icon::new(workline.icon)
                                .size(IconSize::Medium)
                                .color(workline.tone.color()),
                        )
                        .child(
                            Label::new(workline.status)
                                .size(LabelSize::Large)
                                .color(workline.tone.color()),
                        ),
                )
                .child(
                    Label::new(workline.detail.clone())
                        .size(LabelSize::Default)
                        .color(Color::Muted)
                        .truncate(),
                ),
        )
        .when_some(active_stage, |this, stage| {
            this.child(
                h_flex()
                    .id(format!("stcode-active-stage-{}", stage.kind.label()))
                    .w_full()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().editor_background)
                    .child(
                        Icon::new(stage.icon)
                            .size(IconSize::Medium)
                            .color(stage.tone.color()),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        Label::new(stage.label)
                                            .size(LabelSize::Default)
                                            .color(Color::Default),
                                    )
                                    .child(
                                        Label::new(stage.status)
                                            .size(LabelSize::Small)
                                            .color(stage.tone.color()),
                                    ),
                            )
                            .child(
                                Label::new(stage.detail.clone())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    ),
            )
        })
        .children(activity.entries.into_iter().map(render_activity_entry))
        .into_any_element()
}

#[derive(Clone)]
struct SmartWorklineSnapshot {
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
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
            Self::Start => "시작",
            Self::Plan => "계획",
            Self::Parallel => "병렬",
            Self::Execute => "실행",
            Self::Review => "검토",
            Self::Merge => "병합",
        }
    }
}

fn render_smart_workline_status_bar(
    snapshot: SmartWorklineSnapshot,
    _cx: &mut App,
) -> gpui::AnyElement {
    let detail = truncate_and_trailoff(&snapshot.detail, MAX_STATUS_WORKLINE_DETAIL_CHARS);

    h_flex()
        .id("stcode-ai-workline-control-bar")
        .min_w_0()
        .gap_2()
        .overflow_hidden()
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
                            Label::new("AI 워크라인")
                                .size(LabelSize::Default)
                                .color(Color::Default),
                        )
                        .child(
                            Label::new(snapshot.status)
                                .size(LabelSize::Small)
                                .color(snapshot.tone.color()),
                        ),
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
            snapshot.status,
            &snapshot.detail,
            snapshot.icon,
            snapshot.tone,
            active_stage,
        );
    }

    smart_workline_stage(
        SmartWorklineStageKind::Start,
        "준비",
        "작업 공간이 깨끗합니다.",
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
            "준비",
            "워크라인을 막는 할 일이 없습니다.",
            ActivityTone::Done,
        )
    } else {
        (
            "대기",
            "작업을 시작하면 계획이 생성됩니다.",
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
            "대기",
            "아직 레인 정보가 없습니다.",
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
            "대기",
            "아직 에이전트가 시작되지 않았습니다.",
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
            "대기",
            "아직 검토 정보가 없습니다.",
            IconName::ListTodo,
            ActivityTone::Idle,
            active_stage,
        );
    };

    let (status, tone) = if snapshot.counts.conflicted_count > 0 {
        ("차단됨", ActivityTone::Failed)
    } else if snapshot.counts.changed_count > 0 {
        ("검토", ActivityTone::Waiting)
    } else if has_thread_entries {
        ("깔끔함", ActivityTone::Done)
    } else {
        ("대기", ActivityTone::Idle)
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
            "대기",
            "아직 병합 정보가 없습니다.",
            IconName::PullRequest,
            ActivityTone::Idle,
            active_stage,
        );
    };

    let (status, tone) = if snapshot.can_create_pull_request {
        ("준비", ActivityTone::Waiting)
    } else if has_thread_entries && snapshot.tone.needs_attention() {
        (snapshot.status, snapshot.tone)
    } else {
        ("대기", ActivityTone::Idle)
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
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    can_review_changes: bool,
    can_stash_changes: bool,
    can_commit_changes: bool,
    can_split_lane: bool,
}

impl SmartStartSnapshot {
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
        let entries = repository_ref
            .cached_status()
            .filter(|entry| entry.status.has_changes())
            .collect::<Vec<_>>();
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
        let linked_worktree_count = repository_ref.linked_worktrees().len();
        let is_linked_worktree = repository_ref.is_linked_worktree();
        let shared_branch_lane_count =
            smart_shared_branch_lane_count(&repository_ref, branch_ref.as_deref());
        let state = smart_start_state(
            changed_count,
            conflicted_count,
            is_linked_worktree,
            linked_worktree_count,
            shared_branch_lane_count,
        )?;

        Some(Self {
            repository,
            status: state.status(),
            detail: smart_start_detail(
                state,
                branch_name.as_deref(),
                changed_count,
                conflicted_count,
                staged_count,
                unstaged_count,
                linked_worktree_count,
                shared_branch_lane_count,
            ),
            icon: state.icon(),
            tone: state.tone(),
            can_review_changes: changed_count > 0,
            can_stash_changes: changed_count > 0,
            can_commit_changes: changed_count > 0,
            can_split_lane: state.can_split_lane(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartStartState {
    Conflicts,
    LeftoverChanges,
    BranchShared,
    NeedsLane,
}

impl SmartStartState {
    fn status(self) -> &'static str {
        match self {
            Self::Conflicts => "차단됨",
            Self::LeftoverChanges => "인계 필요",
            Self::BranchShared => "브랜치 겹침",
            Self::NeedsLane => "분할 권장",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Conflicts | Self::BranchShared => IconName::GitMergeConflict,
            Self::LeftoverChanges => IconName::GitCommit,
            Self::NeedsLane => IconName::GitBranchPlus,
        }
    }

    fn tone(self) -> ActivityTone {
        match self {
            Self::Conflicts | Self::BranchShared => ActivityTone::Failed,
            Self::LeftoverChanges | Self::NeedsLane => ActivityTone::Waiting,
        }
    }

    fn can_split_lane(self) -> bool {
        matches!(self, Self::BranchShared | Self::NeedsLane)
    }
}

fn smart_start_state(
    changed_count: usize,
    conflicted_count: usize,
    is_linked_worktree: bool,
    _linked_worktree_count: usize,
    shared_branch_lane_count: usize,
) -> Option<SmartStartState> {
    if conflicted_count > 0 {
        Some(SmartStartState::Conflicts)
    } else if changed_count > 0 {
        Some(SmartStartState::LeftoverChanges)
    } else if shared_branch_lane_count > 0 {
        Some(SmartStartState::BranchShared)
    } else if !is_linked_worktree {
        Some(SmartStartState::NeedsLane)
    } else {
        None
    }
}

fn smart_start_detail(
    state: SmartStartState,
    branch_name: Option<&str>,
    changed_count: usize,
    conflicted_count: usize,
    staged_count: usize,
    unstaged_count: usize,
    linked_worktree_count: usize,
    shared_branch_lane_count: usize,
) -> String {
    let branch = branch_name.unwrap_or("detached HEAD");

    match state {
        SmartStartState::Conflicts => {
            let file_label = if changed_count == 1 { "file" } else { "files" };
            format!(
                "{changed_count} changed {file_label} remain on {branch}, {conflicted_count}개 충돌 포함. 다음 세션 전에 해결하거나 격리하세요."
            )
        }
        SmartStartState::LeftoverChanges => {
            let file_label = if changed_count == 1 { "file" } else { "files" };
            format!(
                "{changed_count}개 변경 파일이 있습니다: {branch}: {staged_count}개 스테이징, {unstaged_count}개 미스테이징. Preserve or stash them before starting clean."
            )
        }
        SmartStartState::BranchShared => {
            let lane_label = if shared_branch_lane_count == 1 {
                "lane"
            } else {
                "lanes"
            };
            format!(
                "{branch}은(는) 이미 {shared_branch_lane_count}개 연결된 레인에서 사용 중입니다. 다른 에이전트가 편집하기 전에 새 레인에서 다음 세션을 시작하세요."
            )
        }
        SmartStartState::NeedsLane => {
            if linked_worktree_count == 0 {
                format!(
                    "{branch}은(는) 아직 메인 체크아웃에 있음. Create an isolated lane before autonomous work starts."
                )
            } else {
                let lane_label = if linked_worktree_count == 1 {
                    "lane exists"
                } else {
                    "lanes exist"
                };
                format!(
                    "{branch}은(는) 아직 메인 체크아웃에 있음 while {linked_worktree_count} linked {lane_label}. Move this session into its own lane before starting."
                )
            }
        }
    }
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
            SmartPanelState::Clean => "깨끗함",
            SmartPanelState::InProgress => "진행 중",
            SmartPanelState::Blocked => "차단됨",
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
        return ("충돌", IconName::GitMergeConflict, ActivityTone::Failed);
    }

    if status.is_untracked() {
        return ("새 파일", IconName::File, ActivityTone::Waiting);
    }

    let staging = status.staging();
    if staging.has_staged() && staging.has_unstaged() {
        ("일부", IconName::Diff, ActivityTone::Waiting)
    } else if staging.has_staged() {
        ("스테이징", IconName::Check, ActivityTone::Done)
    } else {
        ("변경됨", IconName::Diff, ActivityTone::Waiting)
    }
}

fn smart_panel_thread_summary(thread: Option<&AcpThread>, cx: &App) -> ThreadSummary {
    let Some(thread) = thread else {
        return ThreadSummary {
            status: "준비",
            detail: "아직 활동 없음",
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
        .unwrap_or_else(|| "아직 목표가 없습니다".to_string());

    SmartPanelWorkItem {
        id: "goal",
        label: "현재 목표",
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
            label: "레인",
            detail: format!(
                "{branch}과(와) {} linked {lane_label}",
                counts.shared_branch_lane_count
            ),
            status: "격침",
            icon: IconName::GitMergeConflict,
            tone: ActivityTone::Failed,
        };
    }

    if counts.is_linked_worktree {
        SmartPanelWorkItem {
            id: "lane",
            label: "레인",
            detail: format!("{branch}은(는) 이 세션에서 격리됨"),
            status: "격리됨",
            icon: IconName::GitWorktree,
            tone: ActivityTone::Done,
        }
    } else {
        SmartPanelWorkItem {
            id: "lane",
            label: "레인",
            detail: format!("{branch}은(는) 아직 메인 체크아웃에 있음"),
            status: "분할",
            icon: IconName::GitBranchPlus,
            tone: ActivityTone::Waiting,
        }
    }
}

fn smart_panel_check_item(thread: Option<&AcpThread>, cx: &App) -> SmartPanelWorkItem {
    let Some(check) = latest_execute_tool_snapshot(thread, cx) else {
        return SmartPanelWorkItem {
            id: "check",
            label: "최근 확인",
            detail: "아직 실행된 명령이 없습니다".to_string(),
            status: "대기",
            icon: IconName::ToolTerminal,
            tone: ActivityTone::Idle,
        };
    };

    SmartPanelWorkItem {
        id: "check",
        label: "최근 확인",
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
        label: "병합 준비",
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
            SmartTodoState::Empty => "할 일 없음",
            SmartTodoState::Planned => "준비",
            SmartTodoState::Working => "작업 중",
            SmartTodoState::AutonomyBlocked => "자율 차단",
            SmartTodoState::Blocked => "차단됨",
            SmartTodoState::Complete => "완료",
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
            "아직 계획이 없습니다. 에이전트에게 작업을 나누달라고 요청하세요.".to_string()
        }
        SmartTodoState::Planned => {
            let current = current_label.unwrap_or("다음 계획 단계");
            format!("{pending}개 할 일이 남았습니다. 다음: {current}.")
        }
        SmartTodoState::Working => {
            let current = current_label
                .or(latest_tool_label)
                .unwrap_or("작업 실행 중");
            format!("에이전트 작업 중: {current}.")
        }
        SmartTodoState::AutonomyBlocked => {
            let tool = latest_tool_label.unwrap_or("작업 도구");
            format!("자율 차단: {tool}가(이) 도구 권한을 기다리고 있습니다.")
        }
        SmartTodoState::Blocked => {
            let tool = latest_tool_label.unwrap_or("the latest workspace step");
            format!("최근 작업 단계에서 차단됨. 추가 작업 전에 실패를 확인하세요.")
        }
        SmartTodoState::Complete => format!("{completed}/{total}개 할 일 완료."),
    }
}

fn smart_todo_progress_label(completed: u32, total: u32) -> String {
    if total == 0 {
        "계획 없음".to_string()
    } else {
        format!("{completed}/{total} 완료")
    }
}

fn smart_todo_left_label(pending: u32) -> String {
    if pending == 1 {
        "1 남음".to_string()
    } else {
        format!("{pending} 남음")
    }
}

fn plan_entry_label(entry: &acp_thread::PlanEntry, cx: &App) -> String {
    truncate_and_trailoff(entry.content.read(cx).source().trim(), MAX_TODO_LABEL_CHARS)
}

fn plan_entry_status(status: acp::PlanEntryStatus) -> (&'static str, IconName, ActivityTone) {
    match status {
        acp::PlanEntryStatus::InProgress => {
            ("진행 중", IconName::TodoProgress, ActivityTone::Running)
        }
        acp::PlanEntryStatus::Completed => ("완료", IconName::TodoComplete, ActivityTone::Done),
        acp::PlanEntryStatus::Pending | _ => {
            ("대기 중", IconName::TodoPending, ActivityTone::Waiting)
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
pub(crate) struct SmartParallelSnapshot {
    status: &'static str,
    detail: String,
    icon: IconName,
    tone: ActivityTone,
    lanes: Vec<SmartParallelLane>,
    pub(crate) state: SmartParallelState,
}

#[derive(Clone)]
struct SmartParallelLane {
    label: String,
    branch_ref: Option<String>,
    path_label: String,
    overlaps_active_branch: bool,
}

impl SmartParallelLane {
    fn path_label_str(&self) -> &str {
        if self.path_label.is_empty() {
            "?"
        } else {
            &self.path_label
        }
    }
}

impl SmartParallelSnapshot {
    pub(crate) fn from_project(project: &Entity<Project>, cx: &App) -> Option<Self> {
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
            state,
        })
    }

    fn should_render_card(&self) -> bool {
        self.tone.needs_attention()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmartParallelState {
    Isolated,
    NeedsLane,
    BranchShared,
}

impl SmartParallelState {
    fn status(self) -> &'static str {
        match self {
            SmartParallelState::Isolated => "격리됨",
            SmartParallelState::NeedsLane => "분할 권장",
            SmartParallelState::BranchShared => "브랜치 겹침",
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
                format!("{branch}은(는) 이 세션에서 격리된 레인에서 실행 중입니다.")
            } else {
                let lane_label = if linked_worktree_count == 1 {
                    "other lane"
                } else {
                    "other lanes"
                };
                format!(
                    "{branch}은(는) {linked_worktree_count} {lane_label}; parallel agents can work without sharing this checkout."
                )
            }
        }
        SmartParallelState::NeedsLane => {
            if linked_worktree_count == 0 {
                format!(
                    "{branch}은(는) 아직 메인 체크아웃에 있음. Create a lane before starting parallel agent work."
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
                "{branch}도 여기에 있습니다: {duplicate_branch_count} {lane_label}{overlap_detail}. 레인을 전환하거나 새 레인을 만드세요."
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
            SmartMergeState::Ready => "준비",
            SmartMergeState::NeedsPublish => "게시 필요",
            SmartMergeState::NeedsSync => "동기화 필요",
            SmartMergeState::NeedsCheckpoint => "체크포인트 필요",
            SmartMergeState::HasConflicts => "차단됨",
            SmartMergeState::ProtectedBranch => "베이스 브랜치",
            SmartMergeState::Detached => "브랜치 없음",
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
            "{branch}은(는) 깔끔하고 게시되어 AI 스마트 병합이 가능합니다."
        ),
        SmartMergeState::NeedsPublish => {
            let ahead = ahead_count
                .map(|count| format!(" {count}개 커밋이 업스트림보다 앞서 있습니다."))
                .unwrap_or_default();
            format!(
                "{branch}은(는) 로컬은 깔끔하지만 게시가 필요합니다.{ahead} AI 스마트 병합이 프리시, PR 생성 후 병합합니다."
            )
        }
        SmartMergeState::NeedsSync => {
            let behind = behind_count
                .map(|count| format!(" {count}개 업스트림 커밋이 앞서 있습니다."))
                .unwrap_or_default();
            format!(
                "{branch}은(는) 병합 전에 베이스 동기화가 필요합니다.{behind} AI 스마트 병합이 리베이스 또는 풀 후 계속합니다."
            )
        }
        SmartMergeState::NeedsCheckpoint => {
            let file_label = if changed_count == 1 { "file" } else { "files" };
            format!(
                "{changed_count}개 변경 파일이 있습니다: {branch}. AI 스마트 병합이 검토, 커밋, 테스트, 푸시, PR 생성 후 병합합니다."
            )
        }
        SmartMergeState::HasConflicts => format!(
            "{conflicted_count}개 충돌이 있습니다: {branch}. AI 스마트 병합이 계속하기 전에 해결합니다."
        ),
        SmartMergeState::ProtectedBranch => format!(
            "{branch}은(는) 베이스 브랜치입니다. 작업 브랜치로 분할해야 합니다."
        ),
        SmartMergeState::Detached => {
            "detached HEAD 상태입니다. PR 준비 전에 브랜치를 만드세요."
                .to_string()
        }
    }
}

fn is_merge_base_branch(branch_name: &str) -> bool {
    let branch_name = branch_name.rsplit('/').next().unwrap_or(branch_name);
    matches!(branch_name, "main" | "master" | "trunk")
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
            status: "준비",
            detail: "아직 활동 없음",
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
                label: "요청 수신".to_string(),
                status: "대기",
                icon: IconName::UserCheck,
                tone: ActivityTone::Idle,
                is_tool: false,
            }),
            AgentThreadEntry::AssistantMessage(_) => Some(Self {
                id: entry_index,
                label: "에이전트 응답 갱신".to_string(),
                status: "갱신",
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
                label: "계획 완료".to_string(),
                status: "완료",
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
            status: "준비",
            detail: "아직 활동 없음",
            icon: IconName::Circle,
            tone: ActivityTone::Idle,
        };
    }

    if is_waiting_for_confirmation {
        return ThreadSummary {
            status: "자율 차단",
            detail: "도구 권한 대기 중",
            icon: IconName::Warning,
            tone: ActivityTone::Waiting,
        };
    }

    if is_generating {
        return ThreadSummary {
            status: "작업 중",
            detail: if has_in_progress_tool_calls {
                "도구 실행 중"
            } else {
                "계획 중"
            },
            icon: IconName::LoadCircle,
            tone: ActivityTone::Running,
        };
    }

    if had_error || last_tool_tone.is_some_and(ActivityTone::needs_attention) {
        return ThreadSummary {
            status: "주의 필요",
            detail: "최근 작업이 완료되지 않음",
            icon: IconName::XCircle,
            tone: ActivityTone::Failed,
        };
    }

    ThreadSummary {
        status: "준비",
        detail: "최근 작업 완료",
        icon: IconName::Check,
        tone: ActivityTone::Done,
    }
}

fn tool_status_label(status: &ToolCallStatus) -> (&'static str, ActivityTone) {
    match status {
        ToolCallStatus::Pending => ("대기", ActivityTone::Running),
        ToolCallStatus::WaitingForConfirmation { .. } => {
            ("권한 차단", ActivityTone::Waiting)
        }
        ToolCallStatus::InProgress => ("실행 중", ActivityTone::Running),
        ToolCallStatus::Completed => ("완료", ActivityTone::Done),
        ToolCallStatus::Failed => ("실패", ActivityTone::Failed),
        ToolCallStatus::Rejected => ("거부", ActivityTone::Failed),
        ToolCallStatus::Canceled => ("취소", ActivityTone::Failed),
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

        assert_eq!(summary.status, "자율 차단");
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

        assert_eq!(summary.status, "주의 필요");
        assert_eq!(summary.tone, ActivityTone::Failed);
    }

    #[test]
    fn tool_status_labels_are_user_facing() {
        assert_eq!(
            tool_status_label(&ToolCallStatus::Pending),
            ("대기 중", ActivityTone::Running)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::InProgress),
            ("실행 중", ActivityTone::Running)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::Completed),
            ("완료", ActivityTone::Done)
        );
        assert_eq!(
            tool_status_label(&ToolCallStatus::Failed),
            ("실패", ActivityTone::Failed)
        );
    }

    #[test]
    fn smart_start_detail_summarizes_clean_handoff_counts() {
        let detail = smart_start_detail(
            SmartStartState::LeftoverChanges,
            Some("feature"),
            3,
            0,
            1,
            2,
            0,
            0,
        );

        assert!(detail.contains("3개 변경"));
        assert!(detail.contains("1개 스테이징"));
        assert!(detail.contains("2개 미스테이징"));
    }

    #[test]
    fn smart_start_detail_prioritizes_conflicts() {
        let detail = smart_start_detail(
            SmartStartState::Conflicts,
            Some("feature"),
            2,
            1,
            0,
            1,
            0,
            0,
        );

        assert!(detail.contains("1개 충돌"));
        assert!(detail.contains("해결하거나 격리"));
    }

    #[test]
    fn smart_start_state_surfaces_lane_risk_before_work_starts() {
        assert_eq!(
            smart_start_state(0, 0, false, 2, 0),
            Some(SmartStartState::NeedsLane)
        );
        assert_eq!(
            smart_start_state(0, 0, true, 2, 1),
            Some(SmartStartState::BranchShared)
        );
        assert_eq!(smart_start_state(0, 0, true, 0, 0), None);
    }

    #[test]
    fn smart_start_detail_explains_lane_start_gate() {
        let split = smart_start_detail(
            SmartStartState::NeedsLane,
            Some("feature"),
            0,
            0,
            0,
            0,
            2,
            0,
        );
        let overlap = smart_start_detail(
            SmartStartState::BranchShared,
            Some("feature"),
            0,
            0,
            0,
            0,
            2,
            1,
        );

        assert!(split.contains("메인 체크아웃"));
        assert!(overlap.contains("이미"));
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
        assert!(detail.contains("1개 스테이징"));
        assert!(detail.contains("2개 미스테이징"));
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

        assert_eq!(item.status, "격침");
        assert!(item.detail.contains("격칩니다"));
        assert_eq!(item.tone, ActivityTone::Failed);
    }

    #[test]
    fn smart_panel_merge_item_reuses_merge_readiness() {
        let item = smart_panel_merge_item(Some("feature"), 0, 0, SmartMergeState::Ready);

        assert_eq!(item.status, "준비");
        assert!(item.detail.contains("깔끔하고 게시되어"));
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
            ("새 파일", IconName::File, ActivityTone::Waiting)
        );
        assert_eq!(
            smart_panel_file_status(git::status::FileStatus::index(
                git::status::StatusCode::Modified,
            )),
            ("스테이징", IconName::Check, ActivityTone::Done)
        );
        assert_eq!(
            smart_panel_file_status(git::status::FileStatus::Unmerged(
                git::status::UnmergedStatus {
                    first_head: git::status::UnmergedStatusCode::Updated,
                    second_head: git::status::UnmergedStatusCode::Updated,
                },
            )),
            ("충돌", IconName::GitMergeConflict, ActivityTone::Failed)
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

        assert!(detail.contains("3 할 일"));
        assert!(detail.contains("다음: Run validation"));
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

        assert!(autonomy_blocker.contains("자율 차단"));
        assert!(autonomy_blocker.contains("도구 권한"));
        assert!(blocked.contains("차단됨"));
    }

    #[test]
    fn smart_todo_labels_are_compact() {
        assert_eq!(smart_todo_progress_label(2, 5), "2/5 완료");
        assert_eq!(smart_todo_progress_label(0, 0), "계획 없음");
        assert_eq!(smart_todo_left_label(1), "1 남음");
        assert_eq!(smart_todo_left_label(3), "3 남음");
    }

    #[test]
    fn plan_entry_status_is_review_facing() {
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::InProgress),
            ("진행 중", IconName::TodoProgress, ActivityTone::Running)
        );
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::Completed),
            ("완료", IconName::TodoComplete, ActivityTone::Done)
        );
        assert_eq!(
            plan_entry_status(acp::PlanEntryStatus::Pending),
            ("대기 중", IconName::TodoPending, ActivityTone::Waiting)
        );
    }

    #[test]
    fn smart_workline_controls_keep_idle_bar_focused() {
        let controls = smart_workline_controls(SmartWorklineControlState {
            active_stage: None,
            has_thread_entries: false,
            has_start_gate: false,
            activity_live: false,
            todo_live: false,
            panel_needs_review: false,
            parallel_needs_attention: false,
            merge_available: false,
            update_state: SmartWorklineUpdateState::Idle,
        });

        assert_eq!(
            workline_control_actions(&controls),
            vec![
                (SmartWorklineAction::Start, false),
                (SmartWorklineAction::Update, false),
                (SmartWorklineAction::Logs, false),
            ]
        );
    }

    #[test]
    fn smart_workline_controls_promote_active_start_gate() {
        let controls = smart_workline_controls(SmartWorklineControlState {
            active_stage: Some(SmartWorklineStageKind::Start),
            has_thread_entries: false,
            has_start_gate: true,
            activity_live: false,
            todo_live: false,
            panel_needs_review: false,
            parallel_needs_attention: false,
            merge_available: false,
            update_state: SmartWorklineUpdateState::Idle,
        });

        assert_eq!(
            workline_control_actions(&controls),
            vec![
                (SmartWorklineAction::Start, true),
                (SmartWorklineAction::Update, false),
                (SmartWorklineAction::Logs, false),
            ]
        );
    }

    #[test]
    fn smart_workline_controls_share_review_merge_parallel_state() {
        let controls = smart_workline_controls(SmartWorklineControlState {
            active_stage: Some(SmartWorklineStageKind::Review),
            has_thread_entries: true,
            has_start_gate: false,
            activity_live: false,
            todo_live: false,
            panel_needs_review: true,
            parallel_needs_attention: true,
            merge_available: true,
            update_state: SmartWorklineUpdateState::Idle,
        });

        assert_eq!(
            workline_control_actions(&controls),
            vec![
                (SmartWorklineAction::Panel, true),
                (SmartWorklineAction::Merge, false),
                (SmartWorklineAction::Parallel, false),
                (SmartWorklineAction::Update, false),
                (SmartWorklineAction::Logs, false),
            ]
        );
    }

    #[test]
    fn smart_workline_update_control_reflects_update_status() {
        let controls = smart_workline_controls(SmartWorklineControlState {
            active_stage: None,
            has_thread_entries: false,
            has_start_gate: false,
            activity_live: false,
            todo_live: false,
            panel_needs_review: false,
            parallel_needs_attention: false,
            merge_available: false,
            update_state: SmartWorklineUpdateState::Updated,
        });

        let update = controls
            .iter()
            .find(|control| control.action == SmartWorklineAction::Update)
            .expect("update control should be present");
        assert_eq!(update.label(), "Restart");
        assert!(update.is_visually_active());
        assert!(!update.is_disabled());
        assert!(update.should_restart());
    }

    #[test]
    fn smart_workline_update_control_blocks_busy_clicks() {
        let controls = smart_workline_controls(SmartWorklineControlState {
            active_stage: None,
            has_thread_entries: false,
            has_start_gate: false,
            activity_live: false,
            todo_live: false,
            panel_needs_review: false,
            parallel_needs_attention: false,
            merge_available: false,
            update_state: SmartWorklineUpdateState::Downloading,
        });

        let update = controls
            .iter()
            .find(|control| control.action == SmartWorklineAction::Update)
            .expect("update control should be present");
        assert_eq!(update.label(), "Downloading");
        assert!(update.is_visually_active());
        assert!(update.is_disabled());
        assert!(update.is_loading());
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

        assert!(detail.contains("메인 체크아웃"));
        assert!(detail.contains("레인"));
        assert!(detail.contains("its own lane"));
    }

    #[test]
    fn smart_parallel_detail_explains_isolated_lane() {
        let detail =
            smart_parallel_detail(SmartParallelState::Isolated, Some("feature"), 1, 0, &[]);

        assert!(detail.contains("격리됨"));
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

        assert!(detail.contains("여기에 있습니다"));
        assert!(detail.contains("레인"));
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

        assert!(detail.contains("3개 변경"));
        assert!(detail.contains("검토, 커밋"));
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

    fn workline_control_actions(
        controls: &[SmartWorklineControl],
    ) -> Vec<(SmartWorklineAction, bool)> {
        controls
            .iter()
            .map(|control| (control.action, control.active))
            .collect()
    }
}
