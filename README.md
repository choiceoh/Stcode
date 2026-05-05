# Stcode

Stcode is a local-first vibe-coding workspace for people who do not want to drive a code editor by hand.

The product direction is simple:

- describe what you want in normal language
- split work across multiple AI agents
- add local models from Ollama, LM Studio, or any OpenAI-compatible local server
- keep the code, Git state, and safety checks visible without making the user learn them
- use Zed 1.0's editor, input, agent UI, terminal, project, and GPUI foundations as the base

This repository currently starts from the Zed v1.0.0 source tree and is being carved down into a focused Stcode app. The baseline keeps the heavy pieces that make chat input, editor behavior, agent panels, workspace state, and terminal-backed execution feel real, while removing upstream Zed website, release, extension sample, hosted collaboration server, and infrastructure material that does not serve Stcode.

## Product Shape

Stcode is not trying to be another general-purpose IDE.

It is for a user who can say "make this app", "fix the weird input behavior", "split this among agents", or "open a PR and merge it" without knowing what files, commands, branches, or test targets should be touched.

The application should make these things first-class:

- a strong chat/message input with normal cursor, selection, IME, history, paste, and multiline behavior
- a multi-agent work board where several agents can investigate, implement, and verify in parallel
- an understandable activity timeline instead of raw terminal noise
- automatic Git/worktree handling for branches, commits, PRs, and merges
- local-model setup that does not require a cloud account when the user has a local runtime
- enough editor and terminal power for agents to work seriously, without exposing every IDE surface to the user

The editor remains part of Stcode, but its role changes. It is not the main surface for a non-coder to operate by hand; it is the agent workbench. Agents still need real buffers, cursor behavior, selections, diagnostics, search, diffs, terminals, and project context to make reliable changes.

## Editor Boundary

Stcode keeps the editor machinery that agents need to understand and change code: buffers, selections, diagnostics, search, diffs, file state, project context, and review surfaces.

Stcode does not keep editor surfaces whose main purpose is human hand-driving. Keymap editing, which-key discovery, go-to-line popups, tab-switcher modals, snippet management UI, modal editing, and similar shortcut-first workflows are outside the product boundary unless they directly support autonomous agent work.

Test fixtures, golden corpora, evaluation runners, and visual-test harnesses are also outside the product boundary. Keep small test-support utilities only when retained runtime crates still need them for local validation.

## Current Baseline

The current codebase is intentionally broad because it was imported from Zed 1.0 before pruning. The important retained areas are:

- `crates/agent_ui`: agent-facing UI surface
- `crates/agent`: agent orchestration and tool flow
- `crates/ui_input`: production text input behavior
- `crates/editor`: editor buffer and interaction behavior used by agents and review surfaces
- `crates/workspace`, `crates/project`, `crates/worktree`: workspace and project state
- `crates/git`, `crates/git_ui`: Git integration that can later be simplified for non-coders
- `crates/terminal`, `crates/terminal_view`: execution surface for agents
- `crates/gpui`, `crates/ui`, `crates/component`: UI foundation

The default binary is still Zed's app entrypoint while Stcode is being reshaped. Renaming, narrowing startup, and replacing Zed-branded surfaces are follow-up pruning tasks.

## Local Checks

Useful first checks:

```sh
cargo metadata --no-deps --format-version 1
cargo check -p agent_ui
```

`cargo check -p agent_ui` is the main smoke check for the current direction because it keeps the agent UI, input editor, editor, project, terminal, and model-provider stack connected.

## License

Stcode is based on Zed v1.0.0. The imported source includes GPL, AGPL, and Apache-licensed components. Keep the upstream license files and any required notices intact while pruning.
