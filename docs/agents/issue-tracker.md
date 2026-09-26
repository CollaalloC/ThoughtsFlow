# Local work tracking

This repository has no configured Git remote. Use local Markdown for planned work; do not create remote issues or publish source implicitly.

Existing bounded plans live under `docs/`, including `REFRESH_STORAGE_PLAN.md`. Each plan records its starting commit, behavior, validation and deferred scope. For work that actually spans multiple sessions, use `.scratch/<feature>/issues/` with explicit blocking dependencies. Do not manufacture tickets for a single bounded change.

Implementation follows the user's continuing authorization: validate the affected public interfaces, review against the recorded starting commit and plan, and make functional local commits. User decisions in the conversation override workflow defaults.
