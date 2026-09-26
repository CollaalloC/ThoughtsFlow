# Work tracking

The public repository is `CollaalloC/ThoughtsFlow`. Use GitHub Issues for public bug reports and feature requests; keep detailed bounded implementation plans in local Markdown. Publishing source and the first prerelease was explicitly authorized by the user on 2026-09-26.

Existing bounded plans live under `docs/`, including `REFRESH_STORAGE_PLAN.md`. Each plan records its starting commit, behavior, validation and deferred scope. For work that actually spans multiple sessions, use `.scratch/<feature>/issues/` with explicit blocking dependencies. Do not manufacture tickets for a single bounded change.

Implementation follows the user's continuing authorization: validate the affected public interfaces, review against the recorded starting commit and plan, and make functional commits. Future pushes or releases follow the request in that session. User decisions in the conversation override workflow defaults.
