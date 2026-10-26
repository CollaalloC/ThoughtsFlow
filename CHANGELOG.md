# Changelog

Notable user-facing changes are recorded here. Pre-release status and available installers are listed on [GitHub Releases](https://github.com/CollaalloC/ThoughtsFlow/releases).

## Unreleased

### Agent workbench

- Optionally request an existing OMP model selector for one task while inheriting OMP configuration by default. Freeze the selection in the handoff and validate Orca's requested/effective launch parameters; mismatches remain unknown and are never automatically redispatched. Model discovery and effort overrides are not implemented.
- Page Agent history in groups of 50 summaries and fetch immutable bodies only when expanded, with explicit navigation back to the newest records and selection retained across pages.
- Preserve unsent tasks and selected Missions while navigating. Confirmed dispatches clear only their unchanged draft, including after a view is reopened; newer edits remain intact.
- Prevent delayed initialization responses from overwriting newly created workspaces, Missions or newer environment checks.
- Improve source-graph direction arrows and zoom controls, and keep unused context trees collapsed.

- Make the Agent workbench the default entry point, with workspace creation and context selection independent of Provider setup. Keep direct-model conversations and Provider settings as compatibility features.
- Preview and freeze explicit context handoffs before dispatch, preserving exact source identities, context revisions and the full submitted task text. Existing model receipts and their hashes remain unchanged.
- Save immutable, bounded Agent output observations with Dispatch provenance, reuse selected evidence in later tasks, and inspect handoff/evidence relationships offline.
- Reuse the existing OMP harness through Orca; model, authentication, tools, Skills and approvals remain owned by the Agent. Native general-purpose Agent chat and automatic DAG scheduling remain future work.

### Compatibility

- Isolate Orca's `ORCA_AGENT_SESSION_ID` and child-only `ORCA_AGENT_SESSION_SPAWN_TOKEN` from child CLI calls so inherited session identity and ownership tokens cannot leak into the workbench coordinator.
- Respect the selected output source's `limited` flag and require an advancing cursor before offering another output page.
- Track OMP 18.12.6 and Orca 1.4.241 with fixed-source contract checks; installed versions observed were 18.12.5 and 1.4.237. Do not automatically upgrade installed runtimes or run OMP model discovery.

## 0.1.0-beta.2 — 2026-09-27

First public beta of ThoughtsFlow, previously developed under the spelling ThoughsFlow.

The earlier `v0.1.0-beta.1` source tag failed clean dependency installation and has no published installer release. Its tag remains unchanged.

### Added

- Local workspaces with precise response-level conversation branches, persistent context cursors, multiple immutable model runs and interruption recovery.
- Context preview, pin/exclude controls, immutable request receipts, and user-confirmed summaries and compaction checkpoints.
- Route comparison, context/response differences, decision markers and Markdown decision packet export.
- Streaming support for OpenAI-compatible Chat Completions, Anthropic Messages, Google Gemini and Ollama.
- Eighteen provider templates, bounded model discovery and session-only named credentials. Azure OpenAI remains a non-runnable template; Qwen and Z.AI currently require manual model IDs.
- Optional OMP Gateway connection through a separately configured Auth Broker.
- Optional local Orca integration for OMP task dispatch, parallel Agent work, collaboration replies, output inspection and durable operation records.
- Cross-platform runtime discovery, portable Node launchers, three-platform build/test CI and a separate native WebView fixture workflow.

### Performance and reliability

- Transactionally migrate the mutable default-model configuration marker to its corrected name, preserving values and immutable historical receipts; roll back conflicting or failed upgrades.
- Repair the WebView test dependency lock so clean `npm ci` agrees with the declared override.

- Overlap independent Agent snapshot reads while retaining Mission serialization and complete-result validation.
- Coalesce redundant refreshes, pause hidden-page polling and require fresh state before resumed Agent operations.
- Read context content through workspace-owned references, reducing dependence on unrelated workspace history without changing immutable receipts.
- Keep narrow-window context panels from covering primary controls.

### Release scope

- Early test release; platform packages are listed individually on the release page. Build success does not establish every interactive workflow on that platform.
- Orca, OMP and models are not bundled. Model accounts, gateways and external runtime permissions are configured separately.
- Existing application identifier, database filename and `THOUGHSFLOW_*` environment variables retain their original spelling for data and launch compatibility.
- No app-level database encryption, cloud sync, complete workspace import/restore, full-text search or automatic Agent result adoption.

Architecture, benchmark conditions and historical validation evidence are available in [docs](docs). Upstream attribution and license information are maintained in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
