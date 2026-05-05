# Stcode Notes

These notes replace the imported Zed documentation set. Stcode documentation should explain the product in the user's language, not preserve upstream IDE docs by default.

## North Star

Stcode is a vibe-coding console for a non-coder:

- the user describes intent
- several AI agents split the work
- the app shows progress in human terms
- code, terminal output, Git, tests, PRs, and merges are handled by agents

The interface should feel closer to a focused operations room for AI work than a traditional editor manual.

## What To Keep From Zed

Keep the pieces that make the experience high quality:

- `ui_input` and `editor` for normal text-editing expectations
- `agent_ui`, `agent`, and model-provider crates for AI work
- `workspace`, `project`, `worktree`, `git`, and `terminal` for actual implementation capability
- GPUI, theme, component, assets, and platform crates needed to render the app well

The editor is kept because AI agents need it even when the user does not. Stcode should hide unnecessary editor complexity from the user, but retain real editing machinery for agent work, review, diagnostics, search, and diff context.

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

## What To Remove First

Remove upstream material that does not help Stcode's user:

- Zed website documentation
- release and distribution automation
- Cloudflare, Nix, Docker, sponsorship, and public-community operations
- hosted Zed collaboration server and deployment tooling
- sample extensions
- broad IDE panels that are not part of the non-coder multi-agent workflow

## Current Pruning Rule

Do not trim deep runtime crates just because they look large. First identify which user-facing Stcode workflow they support, then remove only the branches that are not needed by that workflow.
