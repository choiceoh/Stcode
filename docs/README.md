# Stcode Notes

These notes replace the imported Zed documentation set. Stcode documentation should explain the product in the user's language, not preserve upstream IDE docs by default.

## North Star

Stcode is a vibe-coding console for a non-coder:

- the user describes intent
- several AI agents split the work
- the app shows progress in human terms
- local models can be added from common local runtimes
- code, terminal output, Git, tests, PRs, and merges are handled by agents

The interface should feel closer to a focused operations room for AI work than a traditional editor manual.

## What To Keep From Zed

Keep the pieces that make the experience high quality:

- `ui_input` and `editor` for normal text-editing expectations
- `agent_ui`, `agent`, and model-provider crates for AI work
- Ollama, LM Studio, vLLM, SGLang, and OpenAI-compatible provider paths for local models
- `workspace`, `project`, `worktree`, `git`, and `terminal` for actual implementation capability
- GPUI, theme, component, assets, and platform crates needed to render the app well

The editor is kept because AI agents need it even when the user does not. Stcode should hide unnecessary editor complexity from the user, but retain real editing machinery for agent work, review, diagnostics, search, and diff context.

The visible shell should treat AI work as the primary workflow. In Stcode mode, the bottom bar is an AI Workline control surface, not a manual editor status bar. Human-editor signals such as Plain Text, language server status, encoding, and line endings should stay hidden unless they are part of an agent review or debugging surface. The bottom Workline summary and the right AI Smart Panel must read the same state model so action readiness and blocker state stay consistent. App upkeep belongs there too: update checks should be reachable from the Workline instead of only from editor-style menus.

AI Smart Merge is a one-click autonomous merge run, not a shortcut to a manual PR page. It should checkpoint local work, run focused checks, push, create or update a PR, watch CI, fix failures, merge when clean, delete the merged remote branch when safe, and sync the local base branch. The user should see the runbook state in the workline instead of babysitting Git, PR, and CI tools.

The center editor pane is secondary. When it has no files, diffs, or review buffers, Stcode should collapse it so the agent workspace and AI Smart Panel own the first screen. When agent work opens a real buffer, the editor area should reappear as an inspection surface instead of a permanent blank IDE canvas.

Stcode should not interrupt autonomous work with account, trial, upgrade, or reauthentication upsells. If credentials are missing, the user should be sent to model/provider configuration. If a cloud model is out of quota, the user should be sent to model switching or local-provider setup, not an account sales flow.

## Editor Boundary

Preserve the editor as an agent workbench, not as a full manual IDE surface.

Keep:

- buffer, selection, and file-save machinery
- patch, diff, diagnostic, search, and review context
- enough editor rendering for agents and users to inspect changes
- project and terminal integration needed for real implementation work

Remove or avoid reintroducing:

- keymap editors and shortcut-discovery UI
- go-to-line and tab-switcher modals
- snippet-management UI intended for manual typing
- onboarding or settings pages for human editor customization
- modal editing and other hand-driven editing modes
- large test fixtures, golden data, evaluation runners, and visual-test harnesses

## What To Remove First

Remove upstream material that does not help Stcode's user:

- Zed website documentation
- upstream release and distribution automation that targets Zed infrastructure instead of Stcode GitHub Releases
- Cloudflare, Nix, Docker, sponsorship, and public-community operations
- hosted Zed collaboration server and deployment tooling
- sample extensions
- broad IDE panels that are not part of the non-coder multi-agent workflow
- large upstream test/eval corpora that do not ship with or directly power the app

## Current Pruning Rule

Do not trim deep runtime crates just because they look large. First identify which user-facing Stcode workflow they support, then remove only the branches that are not needed by that workflow.
