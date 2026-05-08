# Stcode Shell Plan

Stcode should start from Zed 1.0's production editor, workspace, terminal, Git, and agent foundations, but the default app shape should not feel like a general-purpose IDE.

The first shell milestone keeps the runtime shared with the current `zed` binary and adds a separate `stcode` binary inside `crates/zed`. The two binaries use the same bootstrap path so startup behavior does not fork. The difference is launch mode:

- `LaunchMode::Zed` preserves the imported Zed startup path.
- `LaunchMode::Stcode` applies a runtime-only Stcode layout pass after workspaces open.

## First Milestone

The `stcode` binary should:

- initialize through the shared bootstrap path
- publish `AppLaunchMode::Stcode` so shared UI code can branch without new bootstrap wiring
- restore or create workspaces normally
- leave user settings files unchanged
- avoid persisting its simplified shell layout into the restored Zed workspace state
- close broad IDE panels at runtime: project, outline, Git, collaboration, and terminal
- open and focus the agent panel as the main work surface
- show Stcode as the visible app name in app-level menu and about surfaces
- use Stcode names in first-run AI onboarding, native agent labels, and usage-limit prompts
- route visible Help-menu repository, bug-report, and feature-request paths to Stcode in Stcode mode
- use Stcode names in the general welcome/onboarding setup flow
- replace broad IDE menu groups with an agent-focused Stcode menu bar that keeps workspace basics while removing Selection, Go, Run, debugger, terminal, dock, and panel navigation menus from the default surface
- filter the command palette in Stcode mode so hidden IDE panels, debugger, terminal, task, pane-splitting, and project-structure actions do not reappear as primary commands
- render the workspace welcome screen as a Stcode start surface with agent-first actions, workspace language, and Stcode-specific recent workspace labels
- use Stcode workspace terminology in recent workspace pickers, sidebars, and remote workspace selection surfaces
- use Stcode workspace terminology in title bar workspace controls, remote workspace status, trust prompts, and shared-workspace hints
- use Stcode workspace terminology in the File and Settings menus, including workspace-specific settings and multi-root workspace actions
- apply a Stcode autonomy policy at launch so tool confirmations become auto-run defaults, wait notifications are suppressed, and new agent threads start in isolated worktrees by default
- route Stcode's New Thread action through linked worktree creation when the current workspace is backed by Git, so a new session starts in its own lane instead of reusing the active workspace
- name those new worktree lanes from the typed task prompt, with a short uniqueness suffix, so parallel sessions are recognizable in lane lists instead of appearing as random labels
- open a fresh agent draft automatically in the newly-created worktree lane, even when the source workspace did not already contain typed prompt text
- auto-submit prompt text that was already typed before Stcode created the new worktree lane, so the autonomous run starts after the lane handoff completes
- render a Stcode-only workspace activity timeline in the agent panel so users can see whether the agent is ready, working, blocked by tool permission, or blocked by a failed tool
- render an AI Smart Panel work board that summarizes the current goal, lane isolation, latest command check, merge readiness, active branch, local changed files, staged and unstaged counts, conflicts, line-level diff stats, and the first files that need review
- render an AI Smart Todo card that summarizes the live agent plan, next action, autonomy blockers, tool failures, and completed/pending progress
- render an AI Smart Start guard when uncommitted workspace changes remain, with direct paths to review them with the agent, split into an isolated worktree, stash them, or commit them
- render an AI Smart Parallel card that detects whether the current session is already in an isolated worktree lane, is still on the main checkout, or overlaps a branch used by another linked worktree, with direct paths to create or manage lanes
- render an AI Smart Merge readiness card that explains whether the current branch is clean enough for merge prep, blocked by local changes or conflicts, or still on a base branch, with direct paths to review the merge diff, commit, or open a PR
- replace the traditional editor status bar in Stcode mode with an AI Workline control bar that hides human editor state such as active language, encoding, line ending, and LSP status
- keep the bottom AI Workline control bar and the right AI Smart Panel on the same workline snapshot so their ready, waiting, blocked, and merge states cannot drift apart
- collapse the empty center editor pane in Stcode mode so the agent workspace and AI Smart Panel own the first screen, while reopening the editor area automatically when files, diffs, or review buffers are present
- suppress account, trial, upgrade, and reauthentication upsell surfaces in Stcode mode; missing model credentials should route to model/provider configuration, and quota blocks should route to model switching
- use a dedicated Stcode app icon across bundled app metadata, runtime About surfaces, Linux launcher resources, and Windows icon resources
- check Stcode GitHub Releases for bundled app updates so macOS users can install once and receive later Stcode releases through the in-app updater instead of manually downloading every new build
- wire AI Smart Start, Panel, Parallel, and Merge buttons to auto-submitted agent prompts so those cards can start autonomous handoff, status review, lane cleanup, and merge-prep runs directly
- include the live branch, lane isolation, branch overlap, change counts, conflict counts, diff stats, and changed-file links in AI Smart prompts so autonomous runs start with actionable workspace context
- persist and render an AI Smart Run card that tracks the active smart workflow through snapshot capture, prompt submission, agent execution, blocker state, and the final checkpoint

The terminal panel is hidden by default in Stcode mode because users should not need to operate a terminal directly. This does not remove terminal or execution support. Agent tools and workspace execution surfaces remain available; Stcode surfaces progress through the agent panel, tool cards, and the workspace activity timeline.

## AI Smart Layer

Stcode's product surface should make Git, CI, worktrees, and parallel agent coordination feel like one managed workflow instead of separate developer tools.

- AI Smart Start: start each session from an isolated worktree when possible, route the default New Thread action to lane creation, name the new lane from the task prompt, auto-open the destination agent draft, auto-submit transferred prompt text, and surface leftover changes, main-checkout starts, and branch-overlap risk as handoff tasks before the next session starts
- AI Smart Parallel: keep parallel agents isolated so they do not edit the same worktree or overwrite each other's work
- AI Smart Panel: show the current goal, todo state, lane isolation, changed files, checks, blockers, PR state, and merge readiness in a right-side work panel
- AI Smart Merge: take a task through the full merge runbook automatically: checkpoint local work, run focused checks, push, create or update the PR, watch CI, fix failures, merge when clean, delete the remote branch when safe, and sync the local base branch
- AI Smart Workline: provide one shared state model for the right-side detail board and the bottom summary actions, with Start, Review, Merge, Parallel, and Logs available as the main controls

## What Stays Shared

For now, Stcode still relies on Zed's app bootstrap, settings, themes, workspace restoration, editor buffers, project state, Git integration, terminal runtime, and agent UI initialization. This is intentional. The first goal is to make the Stcode entrypoint real without duplicating a large and fragile startup path.

Shared crates should read `workspace::AppLaunchMode` when they need to adjust visible copy, onboarding, panel policy, or other Stcode-specific shell behavior. That keeps the launch-mode split explicit without threading another boolean through every initializer.

## Later Extraction Point

Move from `crates/zed` to a dedicated `crates/stcode_app` only after the Stcode launch mode has made the retained surface obvious. A good extraction point is when the Stcode shell can name which initialized systems are required for agent work, which broad IDE surfaces are merely hidden, and which Zed-only surfaces can be removed instead of initialized.

## Validation

Use these checks for the first shell milestone:

```sh
cargo metadata --no-deps --format-version 1
cargo check -p zed --bin stcode
```
