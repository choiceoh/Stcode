# Installing Stcode On macOS

Stcode should be installable, runnable, and updateable without understanding GitHub Releases.

## Install

Run the installer script on macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/choiceoh/Stcode/codex/stcode-trim-test-fixtures/script/stcode-install-macos | bash
```

The script downloads the latest Stcode DMG for the current Mac architecture, verifies the `.sha256` checksum, installs `Stcode.app` into `/Applications`, and opens it.

## Run

After installation:

```sh
open -a Stcode
```

The install script opens Stcode automatically unless `--no-open` is passed.

## Update

Run the same command again:

```sh
curl -fsSL https://raw.githubusercontent.com/choiceoh/Stcode/codex/stcode-trim-test-fixtures/script/stcode-install-macos | bash
```

Re-running the installer replaces `/Applications/Stcode.app` with the latest published release. The app also checks Stcode GitHub Releases through the in-app updater, so a user who installed once can receive later releases without manually downloading another DMG.

Inside Stcode, use the bottom AI Workline `Update` control to check for a new release without opening a traditional editor menu.

## Options

Install somewhere else:

```sh
script/stcode-install-macos --install-dir "$HOME/Applications"
```

Install a specific version:

```sh
script/stcode-install-macos --version 1.2.3
```

Install without opening the app:

```sh
script/stcode-install-macos --no-open
```

Print the resolved download URLs:

```sh
script/stcode-install-macos --print-download-url
```
