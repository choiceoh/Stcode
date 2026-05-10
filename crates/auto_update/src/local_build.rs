use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use db::kvp::KeyValueStore;
use futures_lite::{AsyncBufReadExt as _, StreamExt as _, io::BufReader};
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, WeakEntity, Window, actions, div,
    rems,
};
use ui::{
    ActiveTheme as _, Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color,
    Disableable as _, FluentBuilder as _, InteractiveElement as _, Label, LabelCommon as _,
    LabelSize, StyledExt as _, h_flex, v_flex,
};
use util::ResultExt as _;
use util::command::{Stdio, new_command};
use workspace::{DismissDecision, ModalView, Workspace};

use crate::copy_macos_app_bundle;

actions!(
    stcode_local_build,
    [
        /// Rebuild Stcode from a local source repository and replace the running app.
        RebuildAndRestart
    ]
);

const SOURCE_REPO_KVP_KEY: &str = "stcode-local-build-source-repo";
const LOG_TAIL_CAP: usize = 200;

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &RebuildAndRestart, window, cx| {
            workspace.toggle_modal(window, cx, |window, cx| LocalBuildModal::new(window, cx));
        });
    })
    .detach();
}

pub struct LocalBuildModal {
    focus_handle: FocusHandle,
    state: BuildState,
    source_repo: Option<PathBuf>,
    log_tail: VecDeque<SharedString>,
    show_log: bool,
    pending_build: Option<Task<()>>,
}

enum BuildState {
    Idle,
    Building { last_line: SharedString },
    Installing,
    Errored { message: SharedString },
}

impl LocalBuildModal {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let kvp = KeyValueStore::global(cx);
        cx.spawn(async move |this, cx| {
            let path = kvp
                .read_kvp(SOURCE_REPO_KVP_KEY)
                .log_err()
                .flatten()
                .map(PathBuf::from);
            this.update(cx, |this, cx| {
                this.source_repo = path;
                cx.notify();
            })
            .ok();
        })
        .detach();

        Self {
            focus_handle,
            state: BuildState::Idle,
            source_repo: None,
            log_tail: VecDeque::with_capacity(LOG_TAIL_CAP),
            show_log: false,
            pending_build: None,
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        cx.emit(DismissEvent);
    }

    fn is_busy(&self) -> bool {
        matches!(
            self.state,
            BuildState::Building { .. } | BuildState::Installing
        )
    }

    fn pick_source_repo(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let directories = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose Stcode Source Folder".into()),
        });
        let kvp = KeyValueStore::global(cx);

        cx.spawn(async move |this, cx| {
            let chosen = match directories.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    log::warn!("local_build: prompt_for_paths returned error: {error:#}");
                    None
                }
                Err(error) => {
                    log::warn!("local_build: prompt_for_paths channel cancelled: {error:#}");
                    None
                }
            };

            let Some(path) = chosen else {
                return;
            };

            if let Err(error) = kvp
                .write_kvp(
                    SOURCE_REPO_KVP_KEY.to_string(),
                    path.to_string_lossy().into_owned(),
                )
                .await
            {
                log::warn!("local_build: failed to persist source repo path: {error:#}");
            }

            this.update(cx, |this, cx| {
                this.source_repo = Some(path);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_build(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(source_repo) = self.source_repo.clone() else {
            return;
        };
        if matches!(
            self.state,
            BuildState::Building { .. } | BuildState::Installing
        ) {
            return;
        }

        self.state = BuildState::Building {
            last_line: "Starting build…".into(),
        };
        self.log_tail.clear();
        cx.notify();

        let weak = cx.entity().downgrade();
        let task = cx.spawn(async move |this, cx| {
            let result = run_build_and_install(source_repo, weak, cx.clone()).await;
            let succeeded = result.is_ok();
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.state = BuildState::Installing;
                        cx.notify();
                    }
                    Err(error) => {
                        this.append_log(format!("error: {error:#}").into());
                        this.state = BuildState::Errored {
                            message: format!("{error}").into(),
                        };
                        this.show_log = true;
                        cx.notify();
                    }
                }
                this.pending_build = None;
            })
            .ok();
            if succeeded {
                cx.update(|cx| cx.restart());
            }
        });
        self.pending_build = Some(task);
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state = BuildState::Idle;
        cx.notify();
        self.start_build(window, cx);
    }

    fn copy_log(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let combined = self
            .log_tail
            .iter()
            .map(|line| line.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(combined));
    }

    fn toggle_log(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_log = !self.show_log;
        cx.notify();
    }

    fn append_log(&mut self, line: SharedString) {
        if self.log_tail.len() == LOG_TAIL_CAP {
            self.log_tail.pop_front();
        }
        if let BuildState::Building { last_line } = &mut self.state {
            *last_line = line.clone();
        }
        self.log_tail.push_back(line);
    }

    fn render_status(&self) -> impl IntoElement {
        let line: SharedString = match (&self.state, &self.source_repo) {
            (BuildState::Building { last_line }, _) => format!("Building… {}", last_line).into(),
            (BuildState::Installing, _) => "Installing & restarting…".into(),
            (BuildState::Errored { message }, _) => format!("Build failed: {}", message).into(),
            (_, None) => "Choose your local Stcode source folder to begin.".into(),
            (_, Some(path)) => path.display().to_string().into(),
        };
        let color = match self.state {
            BuildState::Errored { .. } => Color::Error,
            BuildState::Building { .. } | BuildState::Installing => Color::Default,
            _ => Color::Muted,
        };
        Label::new(line).size(LabelSize::Small).color(color)
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_repo = self.source_repo.is_some();
        let busy = self.is_busy();
        let errored = matches!(self.state, BuildState::Errored { .. });

        let pick_button = Button::new("pick-source-repo", "Choose Source Folder…")
            .style(ButtonStyle::Outlined)
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.pick_source_repo(window, cx);
            }));

        let primary_button = if errored {
            Some(
                Button::new("retry", "Retry")
                    .style(ButtonStyle::Filled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.retry(window, cx);
                    })),
            )
        } else if has_repo {
            Some(
                Button::new("start-build", "Build & Install")
                    .style(ButtonStyle::Filled)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_build(window, cx);
                    })),
            )
        } else {
            None
        };

        let copy_button = (errored || self.show_log).then(|| {
            Button::new("copy-log", "Copy Log")
                .style(ButtonStyle::Outlined)
                .disabled(self.log_tail.is_empty())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.copy_log(window, cx);
                }))
        });

        let toggle_label = if self.show_log {
            "Hide details"
        } else {
            "Show details"
        };
        let toggle_button = (!self.log_tail.is_empty()).then(|| {
            Button::new("toggle-log", toggle_label)
                .style(ButtonStyle::Subtle)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_log(window, cx);
                }))
        });

        h_flex()
            .gap_2()
            .child(pick_button)
            .when_some(primary_button, |this, button| this.child(button))
            .when_some(copy_button, |this, button| this.child(button))
            .when_some(toggle_button, |this, button| this.child(button))
    }

    fn render_log(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = self
            .log_tail
            .iter()
            .map(|line| line.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        div()
            .max_h(rems(20.))
            .p_2()
            .bg(theme.colors().editor_background)
            .border_t_1()
            .border_color(theme.colors().border_variant)
            .child(
                Label::new(SharedString::from(body))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

impl ModalView for LocalBuildModal {
    fn on_before_dismiss(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> DismissDecision {
        DismissDecision::Dismiss(!self.is_busy())
    }
}
impl EventEmitter<DismissEvent> for LocalBuildModal {}
impl Focusable for LocalBuildModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LocalBuildModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let show_log = self.show_log && !self.log_tail.is_empty();
        v_flex()
            .key_context("LocalBuildModal")
            .on_action(cx.listener(Self::cancel))
            .elevation_3(cx)
            .w(rems(32.))
            .overflow_hidden()
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.colors().border_variant)
                    .child(Label::new("Build & Install Local Stcode").size(LabelSize::Default))
                    .child(self.render_status()),
            )
            .child(
                div()
                    .p_3()
                    .bg(theme.colors().editor_background)
                    .child(self.render_actions(cx)),
            )
            .when(show_log, |this| this.child(self.render_log(cx)))
    }
}

async fn run_build_and_install(
    source_repo: PathBuf,
    modal: WeakEntity<LocalBuildModal>,
    cx: gpui::AsyncApp,
) -> Result<()> {
    let script_path = source_repo.join("script/stcode-release-macos");
    if !smol::fs::metadata(&script_path).await.map(|m| m.is_file()).unwrap_or(false) {
        anyhow::bail!(
            "release script not found at {} — is this the correct source folder?",
            script_path.display()
        );
    }

    let version = read_dev_version(&source_repo).await?;
    let target = host_target()?;

    append(&modal, &cx, format!("→ {} {}", script_path.display(), version).into());

    let mut cmd = new_command("bash");
    cmd.arg(&script_path)
        .arg(&version)
        .arg(&target)
        .current_dir(&source_repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", script_path.display()))?;
    let stdout = child.stdout.take().context("missing stdout pipe")?;
    let stderr = child.stderr.take().context("missing stderr pipe")?;

    let pump_stdout_task = cx.spawn({
        let modal = modal.clone();
        let cx = cx.clone();
        async move |_| pump(stdout, modal, cx).await
    });
    let pump_stderr_task = cx.spawn({
        let modal = modal.clone();
        let cx = cx.clone();
        async move |_| pump(stderr, modal, cx).await
    });

    let status = child.status().await.context("failed to wait for build")?;
    pump_stdout_task.await;
    pump_stderr_task.await;

    if !status.success() {
        anyhow::bail!("build script exited with {}", status);
    }

    let stage_app = source_repo
        .join("target")
        .join("stcode-release")
        .join(format!("stage-{target}"))
        .join("Stcode.app");
    if !smol::fs::metadata(&stage_app).await.map(|m| m.is_dir()).unwrap_or(false) {
        anyhow::bail!("expected staged app at {}", stage_app.display());
    }

    let running_app = cx.update(|cx| cx.app_path())?;

    append(
        &modal,
        &cx,
        format!("→ install to {}", running_app.display()).into(),
    );

    let mut source_app: OsString = stage_app.into_os_string();
    source_app.push("/");
    copy_macos_app_bundle(source_app, &running_app).await?;

    Ok(())
}

async fn pump(
    reader: impl futures_lite::AsyncRead + Unpin,
    modal: WeakEntity<LocalBuildModal>,
    cx: gpui::AsyncApp,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next().await {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        append(&modal, &cx, line.into());
    }
}

fn append(modal: &WeakEntity<LocalBuildModal>, cx: &gpui::AsyncApp, line: SharedString) {
    let _ = modal.update(&mut cx.clone(), |this, cx| {
        this.append_log(line);
        cx.notify();
    });
}

async fn read_dev_version(source_repo: &Path) -> Result<String> {
    let cargo_toml = source_repo.join("crates/stcode/Cargo.toml");
    let contents = smol::fs::read_to_string(&cargo_toml)
        .await
        .with_context(|| format!("failed to read {}", cargo_toml.display()))?;

    let base_version = contents
        .lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("version")?.trim_start();
            let rest = rest.strip_prefix('=')?.trim();
            let value = rest
                .split_once('#')
                .map(|(value, _)| value)
                .unwrap_or(rest)
                .trim();
            let unquoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'))?;
            Some(unquoted.to_string())
        })
        .ok_or_else(|| anyhow!("no version field in {}", cargo_toml.display()))?;

    let sha_output = new_command("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(source_repo)
        .output()
        .await
        .context("failed to run git rev-parse")?;
    let sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    if sha.is_empty() {
        Ok(base_version)
    } else {
        Ok(format!("{base_version}-dev{sha}"))
    }
}

fn host_target() -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => anyhow::bail!("unsupported architecture for local build: {other}"),
    };
    if std::env::consts::OS != "macos" {
        anyhow::bail!("local build is only supported on macOS");
    }
    Ok(format!("{arch}-apple-darwin"))
}
