# ThoughsFlow Domain Language

This glossary names the concepts that define ThoughsFlow's local AI reasoning and decision workspace.

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

**Route**:
A lineage of Turns connected through exact Model Runs.
_Avoid_: Thread, canvas path
