# Third-party notices

ThoughtsFlow's original source is distributed under the [MIT License](LICENSE).
Third-party components retain their own licenses and copyright notices.

## OMP and Orca

ThoughtsFlow learns from and interoperates with these independent projects:

- **[oh-my-pi / OMP](https://github.com/can1357/oh-my-pi)**: model-provider architecture, context-tree design, and the separately installed agent execution runtime. The verified v18.3.2 license is MIT; copyright Mario Zechner (2025), Can Bölük (2025–2026), and Stencil Labs, Inc. (2026).
- **[Orca](https://github.com/stablyai/orca)**: supervised multi-agent orchestration and separately installed runtime. The verified v1.4.212 license is MIT; copyright Lovecast Inc. (2026).

The application invokes a user's separately installed Orca CLI and optionally
connects to an OMP Gateway. It does not include their executables, SDKs, or source
trees. Source review found independently implemented Rust/React adapters, not a
vendored fork. The Context Tree design reference is OMP commit
`d16c6168c86f40fc44f25118c2fd06fe160fcb93`; its earlier MIT notice is retained too.
These acknowledgements do not imply affiliation or endorsement.

Exact upstream license copies are retained under `licenses/upstream/`.

## Bundled dependencies

The [dependency inventory](licenses/DEPENDENCIES.md) records 129 npm production
dependency installations and 517 Rust normal/build dependencies across targets.
It deliberately includes packages that a particular binary may not contain.
`licenses/dependencies.json` records versions, archive integrity, license-text
hashes and provenance. Full original license texts are under `licenses/texts/`;
shared or missing crate notices are documented in `licenses/overrides.json`.

MPL-2.0 covered sources are unmodified. Exact, checksum-verified source archives
for the five MPL components are included in `licenses/mpl-source/`; see
[source availability](licenses/MPL_SOURCE.md). These sources remain under MPL-2.0,
and ThoughtsFlow imposes no restriction on the source rights that license grants.

The Rust standard library's notices are under `licenses/toolchain/`.
SQLite's core is [public domain](https://www.sqlite.org/copyright.html);
the Rust wrapper's own license is retained in the dependency inventory.
Lucide's ISC license and the MIT notice for Feather-derived icons are both
retained in the original Lucide license text.

In an installed bundle, this file is `licenses/THIRD_PARTY_NOTICES.md` and
the source-tree `licenses/` directory is installed as `licenses/third-party/`.
Thus the inventory is `licenses/third-party/DEPENDENCIES.md` and MPL sources
are `licenses/third-party/mpl-source/`. On macOS these are under the app's
`Contents/Resources/` directory; other targets use Tauri's platform resource
directory. The same files are available in the public source repository.

See [the compliance record](docs/OPEN_SOURCE_COMPLIANCE.md) for audit scope and
regeneration instructions. Preserve this notice and the entire licenses
resource directory when redistributing an installer or application bundle.
