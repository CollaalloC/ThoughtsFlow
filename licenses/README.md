# License materials

This directory is included as a resource in ThoughtsFlow installers. Third-party
licenses apply to their respective components, not to the project as a whole.

- `DEPENDENCIES.md` and `dependencies.json`: generated package inventory and integrity.
- `texts/`: deduplicated original third-party license/notice text, named by SHA-256.
- `overrides.json` and `overrides/`: reviewed provenance for crate archives without a usable license file.
- `MPL_SOURCE.md` and `mpl-source/`: exact unmodified covered-source archives and their availability notice.
- `upstream/`: exact OMP/Orca MIT license copies at the reviewed versions.
- `toolchain/`: Rust standard library copyright and license texts.

In installers this whole directory is under `licenses/third-party/` in the
application resources. See `docs/OPEN_SOURCE_COMPLIANCE.md` in the source
repository for audit scope and regeneration instructions.
