# ThoughtsFlow Domain Language

This glossary names the concepts that define ThoughtsFlow's local AI reasoning and decision workspace.

## Provider Configuration

**Provider Template**:
An immutable vendor preset that identifies a model provider and describes its default endpoint and protocol capabilities without containing credentials.
_Avoid_: Provider config, vendor file, preset profile

**Provider Profile**:
A user-owned local configuration that selects a Provider Template and records endpoint, model, and parameter choices for runs.
_Avoid_: Provider Template, account, credential

**Protocol Profile**:
The wire-protocol and authentication-placement description attached to a Provider Template.
_Avoid_: Provider Profile, dialect config

**Stream Protocol**:
The provider-specific request and streaming-response family used to exchange model messages.
_Avoid_: Vendor, transport

**Session Credential**:
A secret held only for the lifetime of the Rust process and associated with one Provider Profile.
_Avoid_: API key record, saved credential

**Model Catalog**:
A bounded, normalized list of model metadata obtained through the Rust-owned strategy of a Provider Template; it is not persisted as authoritative application data.
_Avoid_: Model database, cached models, Provider response

**Model Discovery**:
A metadata-only lookup against a Model Catalog that never includes workspace Context and persists only a model explicitly selected when saving a Provider Profile.
_Avoid_: Model run, connection test, context request

## Reasoning Lineage

**Turn**:
A user prompt bound to one exact parent Model Run, or to no parent when it begins a workspace.
_Avoid_: Message, chat node

**Model Run**:
One immutable attempt by a configured model to answer a Turn.
_Avoid_: Answer, retry result

**Context Receipt**:
The immutable ordered record of content and provider metadata actually used for one Model Run.
_Avoid_: Preview, context settings

**Context Cursor**:
The persisted, versioned selection of one active Model Run and an optional exact Branch Pointer; it projects a route but never rewrites topology.
_Avoid_: Current message, selected Turn, route state

**Context Draft**:
The persisted, versioned pin/exclude overrides for the next send on one exact parent Run. A successful send consumes it; a path switch atomically rebases it.
_Avoid_: Receipt, composer text, global context settings

**Context Checkpoint**:
Immutable evidence for a user-confirmed compaction or branch summary, including exact source Runs, source hash, boundary, result, and Provider snapshot when a Provider generated it.
_Avoid_: Run stream checkpoint, hidden truncation, mutable summary

**Branch Pointer**:
A named, versioned reference to one branch head. Continuing its current head advances it; continuing or retrying history creates a new pointer.
_Avoid_: Route, parent link, UI tab

**Route**:
A lineage of Turns connected through exact Model Runs.
_Avoid_: Thread, canvas path

## Agent Collaboration

**Mission**:
A user-owned collaboration objective attached to one ThoughtsFlow workspace and one code project, containing work delegated to agents.
_Avoid_: Model Run, conversation branch, provider request

**Orchestration Run**:
The external coordination scope that groups Agent Tasks and their Dispatches for one Mission.
_Avoid_: Model Run, model answer, Provider request

**Agent Task**:
An independently reviewable piece of work with an explicit scope, constraints, and acceptance criteria.
_Avoid_: Prompt, terminal, agent process

**Dispatch**:
One authoritative attempt by an agent to execute an Agent Task; a later attempt does not overwrite its result.
_Avoid_: Model Run, Task, retry in place

**Agent Operation**:
One user request to control collaboration, with a durable identity and evidence of its outcome; command acceptance is separate from task completion.
_Avoid_: Agent result, inferred success
