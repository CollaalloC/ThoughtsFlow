# ThoughtsFlow naming and existing local data

The project, public repository, application name, npm package and Rust executable are now **ThoughtsFlow** / `thoughtsflow`. The spelling `ThoughsFlow` in earlier commits and captured test evidence is historical.

To preserve existing developer installations, the internal application identifier remains `io.thoughsflow.desktop`, the database filename remains `thoughsflow.sqlite3`, the stored default-profile key remains `_thoughsflowIsDefault`, and existing `THOUGHSFLOW_*` environment variables continue to work. These stable identifiers are not the public project name. No workspace data is renamed, moved or deleted by this change. Historical database migrations and frozen upgrade fixtures are unchanged.

The test build continues to use `io.thoughsflow.desktop.webview-e2e` and its separate temporary data directory. Renaming the executable does not enable the test driver in production installers.
