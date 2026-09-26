# ThoughtsFlow naming and existing local data

The project, public repository, application name, npm package and Rust executable are now **ThoughtsFlow** / `thoughtsflow`. The spelling `ThoughsFlow` in earlier commits and captured test evidence is historical.

To preserve existing developer installations, the internal application identifier remains `io.thoughsflow.desktop`, the database filename remains `thoughsflow.sqlite3`, and existing `THOUGHSFLOW_*` environment variables continue to work. These stable identifiers are not the public project name. The database file stays in its existing location. Historical database migrations and frozen upgrade fixtures are unchanged.

Schema 8 migrates the mutable Provider Profile default marker from `_thoughsflowIsDefault` to `_thoughtsflowIsDefault` at startup. It preserves the existing value and JSON type, including legacy string values. The migration runs in one transaction; a conflicting pair of markers or another failure rolls back the entire upgrade. Equal markers are consolidated under the new name. No historical Context Receipt, Agent Operation receipt, request payload or corresponding hash is rewritten.

The test build continues to use `io.thoughsflow.desktop.webview-e2e` and its separate temporary data directory. Renaming the executable does not enable the test driver in production installers.

One precisely identified, pre-commit development variant of schema 5 lacked two constraints. Its frozen script and complete SHA384 are retained in `src-tauri/migration-compat/`. Only that known checksum, a contiguous validated migration history and an exact schema match activate the compatibility path. The original checksum remains unchanged; an outer transaction upgrades through schema 9 and adds the missing constraints. A constraint conflict rolls back the entire compatibility upgrade, including the default marker change. Unknown checksums still fail validation. Canonical migration files are never rewritten. Back up existing development data before upgrading.
