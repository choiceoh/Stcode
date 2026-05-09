# Syncing Stcode With Upstream Zed

Stcode is a fork that started from the Zed source tree. When you want to pull in a newer upstream drop, use the helper script instead of manually rebuilding the same fetch, branch, and merge flow every time.

The script does three things that matter for repeat syncs:

1. adds an `upstream` remote for `zed-industries/zed`
2. enables repo-local `git rerere` and `rerere.autoupdate`
3. prepares a dedicated `sync/upstream-*` branch before merging the chosen upstream ref

## One-time setup

```sh
script/stcode-sync-upstream setup
```

That keeps the configuration local to this repository. It does not touch your global Git settings.

## Normal sync flow

```sh
script/stcode-sync-upstream sync
```

By default this will:

- fetch `origin` and `upstream`
- use `origin/main` as the Stcode base branch
- create or reuse `sync/upstream-main`
- merge the latest `origin/main` into that sync branch first
- merge `upstream/main`
- run `cargo metadata --no-deps --format-version 1`
- run `cargo check -p agent_ui`

## Syncing a specific upstream tag or branch

```sh
script/stcode-sync-upstream sync --upstream-ref v1.0.0
```

You can also pass another upstream branch name:

```sh
script/stcode-sync-upstream sync --upstream-ref nightly
```

## Reusing a custom sync branch

```sh
script/stcode-sync-upstream sync --branch sync/upstream-v1-0-0 --upstream-ref v1.0.0
```

The default branch name is safe for repeated use. If you want a different long-lived integration branch, pass `--branch` explicitly.

## Running Stcode-specific follow-ups after the merge

```sh
script/stcode-sync-upstream sync \
  --post-sync 'patch -p0 < compact.patch' \
  --check './script/clippy -p agent_ui'
```

Use `--post-sync` for deterministic follow-up steps such as reapplying a maintained patch or regenerating derived files. Use `--check` to add extra verification after the default smoke checks.

If you want to replace the default checks entirely, combine `--no-check` with one or more `--check` commands.

## Conflict reuse with rerere

When an upstream merge conflicts, resolve it once and commit the merge as usual:

```sh
git status
git add <resolved files>
git commit
```

Because `git rerere` is enabled locally, Git records the conflict resolution. The next time the same conflict shape appears during an upstream sync, Git can replay that resolution automatically or stage the updated result for you.

## Dry runs and fetch-only runs

Preview the commands without making changes:

```sh
script/stcode-sync-upstream sync --dry-run
```

Only refresh `origin` and `upstream` without switching branches or merging:

```sh
script/stcode-sync-upstream sync --fetch-only
```
