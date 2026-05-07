# Stcode Release Automation

Stcode releases are published from GitHub Actions and attach macOS DMGs to a GitHub Release. The in-app updater reads these release assets through GitHub Releases, so the asset names must stay predictable.

The normal path is automatic: when a pull request is merged into `main` or `codex/stcode-trim-test-fixtures`, the release workflow reads the latest `vX.Y.Z` tag, increments the patch version, creates the next tag, builds DMGs, and publishes the GitHub Release. No manual tag creation is required.

## Release Assets

The workflow builds two macOS DMGs:

- `Stcode-<version>-aarch64.dmg`
- `Stcode-<version>-x86_64.dmg`

Each DMG also gets a `.sha256` file. The updater ignores checksum files and selects the DMG for the user's architecture.

## Running A Release

The default release path is PR merge:

1. Merge a pull request into `main` or `codex/stcode-trim-test-fixtures`.
2. The workflow resolves the next patch version from existing `vX.Y.Z` tags.
3. The workflow builds macOS DMGs.
4. The workflow creates the next GitHub Release and tag automatically.

The first automatic release uses `v1.0.0` if no existing `vX.Y.Z` tag is present.

Release workflow runs are serialized so simultaneous PR merges do not resolve the same next patch version.

Manual releases are still available through the `Release Stcode` workflow in GitHub Actions.

For a normal manual release:

1. Open `Actions -> Release Stcode`.
2. Run the workflow with `version` set to a semver value such as `1.2.3`.
3. Leave `draft` enabled for the first run.
4. Inspect the generated GitHub Release assets.
5. Publish the draft release when the DMGs are good.

Manual releases create or update the matching `v<version>` release. Tag pushes do not trigger this workflow, because automatic releases create tags themselves and a tag trigger would rebuild the same release twice.

## Local Packaging

On macOS, the same packaging step can be run locally:

```sh
script/stcode-release-macos 1.2.3 aarch64-apple-darwin
```

The script expects `cargo-bundle`:

```sh
cargo install cargo-bundle --locked
```

The output is written to `target/stcode-release/`.

## Signing And Notarization

Without signing secrets, the workflow uses ad-hoc signing. That is useful for internal test builds, but public macOS releases should use Developer ID signing and notarization.

Optional GitHub secrets:

- `MACOS_CERTIFICATE_P12`: base64-encoded Developer ID Application `.p12`
- `MACOS_CERTIFICATE_PASSWORD`: password for the `.p12`
- `MACOS_CODESIGN_IDENTITY`: signing identity override, if the default `Developer ID Application` is not specific enough
- `APPLE_ID`: Apple ID used for notarization
- `APPLE_TEAM_ID`: Apple Developer Team ID
- `APPLE_APP_SPECIFIC_PASSWORD`: app-specific password for `notarytool`

When all Apple notarization secrets are present, the workflow submits and staples each DMG after packaging.
