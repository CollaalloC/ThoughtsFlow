# Open-source release provenance and notices

Reviewed for the first ThoughtsFlow public beta on 2026-09-26. This records
the inspected source and distribution materials; it is not a claim that an
automated scan can prove ownership or every possible legal obligation.

## Project and upstream boundaries

ThoughtsFlow's original source is MIT licensed. The user authorized public
open-source publication. Review of the source tree, package manifests, lockfiles,
and the project's design references found independent Rust/React implementations
and CLI/HTTP integration, with no copied OMP/Orca source tree, SDK dependency, or
bundled executable. Names used for compatibility do not imply endorsement.

| Project | Inspected source | License / preserved notice |
| --- | --- | --- |
| OMP | [v18.3.2](https://github.com/can1357/oh-my-pi/blob/v18.3.2/LICENSE) | MIT; Mario Zechner, Can Bölük, Stencil Labs, Inc.; `licenses/upstream/OMP-v18.3.2-LICENSE.txt` |
| OMP Context Tree design reference | [d16c6168c86f40fc44f25118c2fd06fe160fcb93](https://github.com/can1357/oh-my-pi/blob/d16c6168c86f40fc44f25118c2fd06fe160fcb93/LICENSE) | MIT; Mario Zechner and Can Bölük; `licenses/upstream/OMP-d16c6168-LICENSE.txt` |
| Orca | [v1.4.212](https://github.com/stablyai/orca/blob/v1.4.212/LICENSE) | MIT; Lovecast Inc.; `licenses/upstream/Orca-v1.4.212-LICENSE.txt` |
| Locally referenced Orca build | [a932147308d1dab3a2a0bc2c688a5cd978b65ffb](https://github.com/stablyai/orca/blob/a932147308d1dab3a2a0bc2c688a5cd978b65ffb/LICENSE) | Exact LICENSE blob matches v1.4.212 (`fbf46fc17dacb1f65e03e2b6290eb53ba94b149d`) |

Orca owns the supervised OMP sessions. Both runtimes must be installed separately.
OMP Gateway is also an optional external service. If a future release vendors
code, SDKs, executables, assets, or modified upstream files, review their actual
version and subcomponent licenses and retain their notices before packaging.
The current MIT project license does not replace licenses on those components.

## Dependency distribution

`scripts/generate-license-notices.py` reads `package-lock.json`, installed npm
production dependency licenses, and locked Cargo metadata. Rust traversal keeps
normal/build edges and excludes development-only edges. It includes all target
platforms, so the result is a conservative attribution inventory, not an exact
binary SBOM. Build-time packages and unused platform packages can appear.

The initial inventory contains **129 npm package installations** and **517 Rust
packages**. npm source checksums come from the lockfile; Rust archive checksums
come from Cargo.lock. Original license texts and notices are deduplicated by
SHA-256 while each package retains its own links and copyright attribution.
No application credentials, private registry URLs, or local home paths belong in
the generated artifacts.

Some published Rust crates omit license files. `licenses/overrides.json` records
42 reviewed package cases: exact repository revisions from `.cargo_vcs_info.json`,
shared repository notices from the same release, or explicitly identified
canonical license text. The `objc2` repository's Apple SDK notice is retained;
its linked MIT terms are included in full, and package authors remain in the
inventory. For `r-efi`, the archive's AUTHORS contains its permission and copyright
notice; MIT is selected from the offered alternatives. Alternatives in SPDX
expressions remain alternatives; their inclusion does not combine obligations.

The five MPL-2.0 packages are `cssparser`, `cssparser-macros`, `dtoa-short`,
`option-ext`, and `selectors`. Their **exact unmodified registry source archives**
are included in the source repository and application resources, not merely
linked to an upstream website. Their hashes match Cargo.lock. The source remains
MPL-2.0 licensed and recipients can obtain it directly from their installation;
see `licenses/MPL_SOURCE.md`. This implements the source-availability and notice
steps in [MPL 2.0 sections 3.1–3.4](https://www.mozilla.org/en-US/MPL/2.0/).

The standard library is supplied by Rust rather than Cargo.lock. Its dedicated
`COPYRIGHT-library.html` and linked license files are preserved under
`licenses/toolchain/`; release builders must refresh these from their actual
compiler if the version differs from the initial local Rust 1.97.1 build.
The full text retains license exceptions and copyright holders that the
application's MIT license alone would not cover.

Platform frameworks such as macOS WebKit and Windows WebView2 remain external
runtimes. The first Linux packaging uses `.deb` and depends on system GTK/WebKit.
AppImage is deferred: bundling system libraries requires a separate inventory,
license/notices review, and any corresponding-source/relinking obligations.
Provider services, model weights, OAuth accounts, and Apple/Microsoft SDKs retain
their own terms; no rights to them are granted by this repository's MIT license.

## Regenerate and verify

Use Python 3.9+, Node 24, and the declared Rust toolchain:

```sh
npm ci
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
python3 scripts/generate-license-notices.py
python3 scripts/generate-license-notices.py --check
```

Generation is offline after the locked packages have been fetched. A missing
license, changed installed npm version, or absent covered source stops generation
instead of guessing a license. If a new archive omits its notice, inspect its
exact source revision and add a reviewed override with provenance.

`--check` works without Cargo or network access: it checks both lockfile hashes,
all referenced license text hashes, and all included MPL source archive hashes.
Run it before release packaging. Lockfile or dependency changes require a new
inventory and a review of any newly introduced license expression.

The Tauri resource mapping installs ThoughtsFlow's LICENSE, the top-level
THIRD_PARTY_NOTICES.md, and this entire `licenses/` tree. Verify the actual
installer's resources before uploading it. Keep MPL source archives and notices
available with each released version, including when replacing a download.
