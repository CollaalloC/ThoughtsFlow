# Frozen legacy SQLite fixtures

`legacy-v1.sqlite` through `legacy-v4.sqlite` are checked-in databases created
from the released migrations at those exact schema versions. Each file includes
representative workspace, Run, immutable Context Receipt, Provider, and branch
rows plus SQLx's original SHA-384 migration checksums.

Repository migration tests copy these bytes to a temporary path and open that
copy through `SqliteRepository::connect`, so the test cannot accidentally build
its "legacy" input with the current production migrator.

The generator refuses to overwrite existing fixtures. Regeneration is an
explicit review event: delete the exact fixture files, verify that migrations
`0001` through `0004` are unchanged, then run:

```sh
bash src-tauri/tests/fixtures/sqlite/generate-legacy-fixtures.sh
```

## Frozen v7 product-rename baseline

`legacy-v7.sqlite` is a synthetic pre-rename database frozen from the publicly
tagged `v0.1.0-beta.1` source (`35cfb0ca00eac0b8bd7f11eef42a9ac17b032b47`).
SHA-256: `aa27cfd708aaa95b987628d347cf350045f039653e956fdfbb4b61b6a64fbdf4`.

It was built by copying `legacy-v4.sqlite`, checking its first four migration
checksums against that tag, and applying the tag's `0005_context_tree.sql`,
`0006_agents.sql`, and `0007_workspace_content_lookup.sql`. Its SQLx migration
rows contain the SHA-384 of those exact tagged SQL files. The v1-v4 fixture
files and all historical migration SQL remain unchanged.

The added test data consists of the mutable `provider-upgrade` profile with
`{"_thoughsflowIsDefault":"true","temperature":"0.25"}`, one extra completed
Run/Context Receipt (`run-marker` / `snapshot-marker`), and synthetic Agent
mission/operation evidence. Their JSON deliberately includes `ThoughsFlow`
and `_thoughsflowIsDefault`; the snapshot and manifest hashes equal the
SHA-256 of that fixture's original raw request bytes. These strings are
historical evidence to preserve, not migration targets. There are no user
credentials or live Agent identifiers.

As with v1-v4, older placeholder Receipt/content hashes make this a migration
fixture, not a complete interactive workspace export. The native IPC test
inserts a valid empty workspace into a temporary copy before upgrade and
checks that the old Provider's selected model/settings still drive an actual
request to a local HTTP fixture afterward. Repository tests compare original
evidence columns byte for byte, exercise JSON boolean/string values, reject
conflicting/duplicate markers, and inject a failure after v8's profile writes
to prove the production SQLx transaction rolls everything back.

Do not regenerate this file from the current production migrator: that would
erase its independence from the migration being tested.

## Frozen pre-commit development v5

`development-v5.sqlite` is synthetic. It copies `legacy-v4.sqlite` and applies
the independently reconstructed pre-commit v5 SQL frozen in
`../../../migration-compat/0005_context_tree_development.sql`. Its migration
history retains that script's original SHA-384; the public canonical v5 SQL is
not substituted. Fixture SHA-256:
`4fbce9855a8adf0b4840d17f22e88285e35d0fc971b9074ae77f6c03f2a4296e`.

The fixture sets `provider-upgrade` to the old string default marker and
temperature `0.25`, keeping `legacy-model`. Added `run-marker` and
`snapshot-marker` rows contain deliberately old product/marker names. Their
manifest and snapshot hashes match the original synthetic request bytes.
No actual application database, user prompt, credential, or Agent identifier
was copied into this file.

Tests exercise the production startup path, preserve every original migration
metadata field and historical receipt value, verify the new constraints and
restart, and reject unknown checksums, incomplete histories, and schema drift.
A pair of synthetic conflicting checkpoint timestamps proves that the whole
development upgrade rolls back rather than rewriting historical timestamps or
leaving migration 8's mutable marker change partly committed.
