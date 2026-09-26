# ThoughtsFlow release procedure

The first public release is `v0.1.0-beta.2`. A beta provides installable builds for testing; successful packaging does not establish that every operating-system version, model vendor, or Orca/OMP combination has been tested.

## Artifacts

The [installer workflow](../.github/workflows/release.yml) is manually dispatched **on a version tag**. It uses hosted native runners and produces:

| Target | Runner | Installable files |
| --- | --- | --- |
| macOS Apple Silicon | `macos-15` | `.dmg`, `.app.tar.gz` |
| macOS Intel | `macos-15-intel` | `.dmg`, `.app.tar.gz` |
| Windows x64 | `windows-2022` | NSIS `.exe` installer |
| Linux x64 | `ubuntu-22.04` | Debian `.deb` package |

GitHub documents the [native runner architectures](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job). Ubuntu 22.04 is the Linux build baseline to avoid requiring the newer glibc from a newer build host; see [Tauri's Linux compatibility guidance](https://v2.tauri.app/distribute/appimage/#limitations). The `.deb` uses distribution-provided GTK/WebKit libraries. AppImage is deferred until its additional bundled system-library licenses and corresponding-source obligations have been audited.

Installer names contain `ThoughtsFlow`, the package version, and the Rust target triple. A tar archive preserves macOS app executable modes and symlinks during artifact transport. These app archives are manual-install alternatives, not updater packages.

The workflow pins Rust 1.97.1 (matching the bundled standard-library notices) and uses `npm ci` and Cargo `--locked`, without `webview-e2e`, fixture configuration, credentials, or a test driver in production builds. It checks the tag against package, lockfile, Cargo, and Tauri versions. All four targets must complete before `release-assets` is created. The aggregate job checks each platform's source commit and asset hashes, then includes:

- Six installation files across four architectures/operating-system targets.
- Four `build-<target>.json` records containing the exact source commit and workflow run URL. These are build metadata, not signed attestations.
- `LICENSE`, `THIRD_PARTY_NOTICES.md`, and `THIRD_PARTY_LICENSES.tar.gz`.
- `SHA256SUMS.txt` covering every other release asset.

Each installer also includes the license resources configured in `src-tauri/tauri.conf.json`. OMP and Orca are separately installed runtimes; these installers do not redistribute them.

## Prepare the version

1. Finish the intended changes, review the diff, and confirm a clean working tree.
2. Update `package.json`, `package-lock.json` (including `packages[""].version`), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json` to the same version.
3. Refresh dependency notices after dependency changes. Preserve upstream attribution and license texts; review `THIRD_PARTY_NOTICES.md` and the contents of `licenses/` before publishing.
4. Update README and release notes with actual validation and known limitations. Keep the distinction between build success, fixture tests, and real model tests.
5. Run the documented checks and merge the reviewed source into `main`. Push `main` and wait for **Desktop CI** for that source commit to pass on all three platforms.

The repository's first publication and merge are performed explicitly with `gh` and Git. This workflow never creates a repository, changes visibility, pushes source, or deletes branches.

## Build from the tag

Run from the repository after the source is ready:

```sh
git tag -a v0.1.0-beta.2 -m 'ThoughtsFlow v0.1.0-beta.2'
git push origin v0.1.0-beta.2
gh workflow run release.yml --ref v0.1.0-beta.2
gh run list --workflow release.yml --limit 5
```

Select the run for the intended tag and commit. Use its actual numeric run ID below:

```sh
gh run view RUN_ID --json headSha,status,conclusion,jobs,url
gh run download RUN_ID --name release-assets --dir release-assets
```

Do not publish partial platform artifacts if any build or the aggregation job fails. Diagnose the run, commit any fix, and use a new beta version/tag if the release has already been published. Published tags and existing installation assets are not silently replaced.

## Verify and publish with gh

Confirm that the chosen installer run and Desktop CI both succeeded for the exact release source commit. Check `build-*.json` against `git rev-list -n 1 v0.1.0-beta.2`, inspect the intended release files, and verify the checksums. On Linux use `sha256sum -c SHA256SUMS.txt`; on macOS use `shasum -a 256 -c SHA256SUMS.txt`, from inside `release-assets`.

Install and launch each target available for testing. Record untested operating systems as untested; do not convert a packaging result into a claim of desktop acceptance. Existing local data should be backed up before beta testing.

Create `release-notes.md` with features, installation prerequisites, upstream acknowledgements, exact validation, and signing limitations. Then use the GitHub CLI to create a draft, upload the verified assets, inspect the result, and publish it as a prerelease:

```sh
gh release create v0.1.0-beta.2 --verify-tag --draft --prerelease \
  --title 'ThoughtsFlow v0.1.0-beta.2' --notes-file release-notes.md
gh release upload v0.1.0-beta.2 release-assets/*
gh release view v0.1.0-beta.2 --json tagName,isDraft,isPrerelease,assets,url
gh release edit v0.1.0-beta.2 --draft=false --prerelease --latest=false
```

The workflow has `contents: read` only. Publication uses the maintainer's authenticated `gh` session after verification, so an installer job cannot independently publish a release. No model credentials are required for packaging.

## Signing and installation limits

The initial beta has no trusted publisher certificate and is not notarized by Apple. A macOS binary may have an ad-hoc signature, which does not establish a publisher identity. Gatekeeper and Windows SmartScreen can warn about the download. State this on the release page; do not recommend disabling system-wide security controls. Future signed releases require maintainer-owned credentials and the platform-specific [Tauri macOS](https://v2.tauri.app/distribute/sign/macos/) and [Windows signing](https://v2.tauri.app/distribute/sign/windows/) setup.

The Windows installer uses the standard Tauri WebView2 bootstrapper policy, so a machine without WebView2 may require network access during installation. See [Tauri's Windows installer documentation](https://v2.tauri.app/distribute/windows-installer/). The Linux package requires a compatible distribution package manager and its GTK/WebKit runtime dependencies; it is not a universal Linux binary.

No automatic in-app updater is enabled. Download a newer beta from GitHub Releases when testing a later version.
