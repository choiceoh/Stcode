# Pruning Plan

This repository intentionally imported Zed 1.0 broadly, then started removing what Stcode does not need.

## Completed In This Slice

- Rewrote the top-level README around Stcode's product direction.
- Replaced upstream Zed contributor guidance with Stcode-specific contribution notes.
- Removed imported Zed documentation, website, cloud, release, extension sample, and public infrastructure files.
- Removed sample extension crates and upstream `xtask`/compliance tooling from the Cargo workspace.
- Documented that the editor runtime stays as the AI-agent workbench.
- Removed the hosted Zed collaboration server crate, standalone eval/import/benchmark CLIs, and upstream operational scripts that do not support Stcode.

## Keep For Now

- `agent_ui`, `agent`, `agent_settings`, `agent_servers`
- `ui_input`, `editor`, `multi_buffer`, `text`, `rope`
- `workspace`, `project`, `worktree`, `git`, `git_ui`
- `terminal`, `terminal_view`, `task`
- `language_model` and provider crates
- GPUI, UI, component, theme, asset, and platform crates

## Editor Decision

Keep the editor runtime. The user should not have to operate a traditional code editor, but AI agents still need one. The editor gives agents real buffers, cursor and selection behavior, diagnostics, search, diffs, terminal-adjacent context, and review surfaces. Stcode should remove the broad Zed IDE shell around that engine, not the editing engine itself.

That means `editor`, `ui_input`, `multi_buffer`, `text`, `rope`, project/worktree state, Git, terminal, task execution, and agent UI stay in the core pruning set. Panels and flows that only serve a human power-user IDE can move out after a Stcode app shell exists.

## Next Removal Passes

1. Replace the Zed app entrypoint with a Stcode shell that opens directly into the agent workspace.
2. Hide or remove editor panels that are not part of vibe-coding, multi-agent work, or review.
3. Collapse provider setup into a small non-coder model configuration flow.
4. Reduce collaboration/call/channel surfaces unless they directly support local multi-agent coordination.
5. Rename visible Zed branding after the runtime shell is narrowed.

## Validation Anchor

Until the app entrypoint is split, use this as the main smoke check:

```sh
cargo check -p agent_ui
```

## Measured Code Shape

After the second repository cleanup:

- workspace packages: 223
- workspace packages in the `agent_ui` normal dependency closure: 141
- workspace packages outside that closure: 82

The outside-closure group includes obvious future removal candidates, but also the current Zed binary entrypoint and platform/app-shell crates. Do not delete that group blindly. The safer order is:

1. create a Stcode app entrypoint
2. point default local checks at the Stcode entrypoint
3. remove Zed app-shell panels and platform surfaces that are no longer referenced
4. keep `agent_ui` green after each removal pass

High-signal outside-closure candidates to inspect next:

- broad IDE panels: `project_panel`, `outline_panel`, `markdown_preview`, `settings_ui`, `theme_selector`
- upstream operational tools: `auto_update_helper`, `auto_update_ui`, `crashes`, `install_cli`
- language/extension breadth: `languages`, `language_tools`, `language_selector`, `zed_extension_api`
- app-shell surfaces: `zed`, `activity_indicator`, `command_palette`, `title_bar`, `sidebar`
- diagnostics and instrumentation: `input_latency_ui`, `miniprofiler_ui`
