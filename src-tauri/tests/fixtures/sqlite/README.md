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
