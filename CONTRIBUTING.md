# Contributing To Stcode

Stcode is being built as a personal, local-first AI coding workspace. The user is not expected to know code, Git, test names, or project layout.

When changing this repository, prefer work that makes that experience more real:

- reduce surfaces that are only useful for maintaining upstream Zed
- keep production-quality editor input and agent UI behavior
- make multi-agent work visible, resumable, and easy to understand
- automate Git and verification instead of asking the user to operate them manually
- document decisions in plain language before adding new machinery

## Before Removing Code

Treat removal as product design, not cleanup for its own sake.

Keep code if it supports one of these current pillars:

- text input, selection, IME, cursor, editor buffers, or paste behavior
- agent UI, chat threads, agent orchestration, tool calls, or model providers
- workspace, project, file tree, terminal, task execution, or Git safety
- GPUI rendering, themes, components, assets, or platform integration needed by the app

Remove or isolate code when it is only for upstream Zed operations such as public docs, release automation, hosted services, extension samples, sponsorship, cloud deployment, or broad IDE surfaces that Stcode does not expose.

Do not remove the editor runtime simply because the user is not expected to edit code manually. In Stcode, the editor is the working surface for AI agents: it gives them real buffers, selections, diagnostics, diffs, and review context. The pruning target is the general-purpose IDE shell around that engine, not the engine itself.

## Checks

Run focused checks for the area touched. For the current baseline, the most useful smoke check is:

```sh
cargo check -p agent_ui
```

For repository-shape changes, also run:

```sh
cargo metadata --no-deps --format-version 1
```
