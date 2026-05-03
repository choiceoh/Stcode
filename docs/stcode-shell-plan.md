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

The terminal panel is hidden by default in Stcode mode because users should not need to operate a terminal directly. This does not remove terminal or execution support. Agent tools and workspace execution surfaces remain available; Stcode should surface progress through the agent panel, tool cards, and later a dedicated activity timeline.

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
