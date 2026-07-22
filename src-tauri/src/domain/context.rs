use std::collections::{BTreeMap, BTreeSet};

use super::{
    AuthPlacement, ContentBlock, DomainError, MessageRole, ModelRun, ProviderDialect, RunStatus,
    StreamProtocol, Turn,
};

pub const CONTEXT_COMPILER_VERSION: &str = "3";

#[derive(Clone, Debug)]
pub struct ConversationGraph {
    turns: BTreeMap<String, Turn>,
    runs: BTreeMap<String, ModelRun>,
    content_blocks: BTreeMap<String, ContentBlock>,
}

impl ConversationGraph {
    pub fn try_new(
        turns: Vec<Turn>,
        runs: Vec<ModelRun>,
        content_blocks: Vec<ContentBlock>,
    ) -> Result<Self, DomainError> {
        let mut turn_index = BTreeMap::new();
        for turn in turns {
            let id = turn.id.clone();
            if turn_index.insert(id.clone(), turn).is_some() {
                return Err(DomainError::DuplicateEntity { kind: "turn", id });
            }
        }

        let mut run_index = BTreeMap::new();
        for run in runs {
            let id = run.id.clone();
            if run_index.insert(id.clone(), run).is_some() {
                return Err(DomainError::DuplicateEntity { kind: "run", id });
            }
        }

        let mut block_index = BTreeMap::new();
        for block in content_blocks {
            let id = block.id.clone();
            if block_index.insert(id.clone(), block).is_some() {
                return Err(DomainError::DuplicateEntity {
                    kind: "content block",
                    id,
                });
            }
        }

        for run in run_index.values() {
            if !turn_index.contains_key(&run.turn_id) {
                return Err(DomainError::MissingTurnForRun {
                    run_id: run.id.clone(),
                    turn_id: run.turn_id.clone(),
                });
            }
        }

        for turn in turn_index.values() {
            let Some(parent_run_id) = turn.parent_run_id.as_ref() else {
                continue;
            };
            let parent_run =
                run_index
                    .get(parent_run_id)
                    .ok_or_else(|| DomainError::MissingParentRun {
                        turn_id: turn.id.clone(),
                        parent_run_id: parent_run_id.clone(),
                    })?;
            let parent_turn = &turn_index[&parent_run.turn_id];
            if parent_turn.workspace_id != turn.workspace_id {
                return Err(DomainError::CrossWorkspaceParent {
                    turn_id: turn.id.clone(),
                    parent_run_id: parent_run_id.clone(),
                });
            }
            if !run_is_usable_as_parent(parent_run) {
                return Err(DomainError::ParentRunNotBranchable {
                    turn_id: turn.id.clone(),
                    parent_run_id: parent_run_id.clone(),
                    status: parent_run.status(),
                });
            }
        }

        let graph = Self {
            turns: turn_index,
            runs: run_index,
            content_blocks: block_index,
        };
        graph.validate_acyclic()?;
        Ok(graph)
    }

    pub fn turn(&self, id: &str) -> Option<&Turn> {
        self.turns.get(id)
    }

    pub fn run(&self, id: &str) -> Option<&ModelRun> {
        self.runs.get(id)
    }

    pub fn content_block(&self, id: &str) -> Option<&ContentBlock> {
        self.content_blocks.get(id)
    }

    fn validate_acyclic(&self) -> Result<(), DomainError> {
        for turn in self.turns.values() {
            let mut seen = BTreeSet::new();
            let mut cursor = turn;
            while let Some(parent_run_id) = cursor.parent_run_id.as_deref() {
                if !seen.insert(cursor.id.as_str()) {
                    return Err(DomainError::CyclicAncestry {
                        turn_id: cursor.id.clone(),
                    });
                }
                let parent_run = &self.runs[parent_run_id];
                cursor = &self.turns[&parent_run.turn_id];
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextPolicy {
    pub compiler_version: String,
    pub max_chars: usize,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            compiler_version: CONTEXT_COMPILER_VERSION.into(),
            max_chars: 100_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextOverrides {
    pub pinned_source_ids: Vec<String>,
    pub excluded_source_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCompileRequest {
    pub workspace_id: String,
    pub system_prompt: String,
    pub parent_run_id: Option<String>,
    pub current_prompt: String,
    pub overrides: ContextOverrides,
    /// Provider routing/configuration is optional for pure context previews,
    /// but must be present in the send path so a changed endpoint or model
    /// invalidates the preview hash.
    pub provider: Option<ProviderSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextSourceKind {
    System,
    TurnPrompt,
    ModelRun,
    Pinned,
    CurrentPrompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InclusionReason {
    SystemPolicy,
    ExactAncestorPath,
    ExplicitPin,
    CurrentPrompt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunContextItem {
    pub position: usize,
    pub source_id: Option<String>,
    pub source_kind: ContextSourceKind,
    pub role: MessageRole,
    pub content: String,
    pub content_hash: String,
    pub inclusion_reason: InclusionReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextManifest {
    pub compiler_version: String,
    pub items: Vec<RunContextItem>,
    pub estimated_chars: usize,
    pub canonical_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextWarning {
    ExcludedPinnedSource(String),
    DuplicatePinnedSource(String),
    ExceedsLimit {
        estimated_chars: usize,
        max_chars: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextPreview {
    pub messages: Vec<CanonicalMessage>,
    pub manifest: ContextManifest,
    pub estimated_chars: usize,
    pub warnings: Vec<ContextWarning>,
    pub preview_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledContext {
    pub messages: Vec<CanonicalMessage>,
    pub manifest: ContextManifest,
    pub estimated_chars: usize,
    pub warnings: Vec<ContextWarning>,
    pub canonical_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSnapshot {
    pub profile_id: String,
    pub provider_id: Option<String>,
    pub template_revision: Option<u16>,
    pub provider_name: String,
    pub dialect: ProviderDialect,
    pub stream_protocol: Option<StreamProtocol>,
    pub auth_placement: Option<AuthPlacement>,
    pub auth_header_name: Option<String>,
    pub additional_headers: BTreeMap<String, String>,
    pub base_url: String,
    pub model: String,
    pub parameters: BTreeMap<String, String>,
}

impl ProviderSnapshot {
    pub fn require_resolved_metadata(&self) -> Result<(), DomainError> {
        if self
            .provider_id
            .as_deref()
            .is_none_or(|provider_id| provider_id.trim().is_empty())
        {
            return Err(DomainError::UnresolvedProviderMetadata {
                field: "Provider Template identity",
            });
        }
        if self.template_revision.is_none_or(|revision| revision == 0) {
            return Err(DomainError::UnresolvedProviderMetadata {
                field: "Provider Template revision",
            });
        }
        if self.stream_protocol.is_none() {
            return Err(DomainError::UnresolvedProviderMetadata {
                field: "stream protocol",
            });
        }
        let auth_placement =
            self.auth_placement
                .ok_or(DomainError::UnresolvedProviderMetadata {
                    field: "authentication placement",
                })?;
        if auth_placement != AuthPlacement::None
            && self
                .auth_header_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err(DomainError::UnresolvedProviderMetadata {
                field: "authentication field name",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSnapshot {
    pub id: String,
    pub run_id: String,
    pub manifest: ContextManifest,
    pub provider: ProviderSnapshot,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalRequest {
    pub messages: Vec<CanonicalMessage>,
    pub provider: ProviderSnapshot,
}

#[derive(Clone, Debug)]
pub struct ContextCompiler {
    policy: ContextPolicy,
}

impl ContextCompiler {
    pub fn new(policy: ContextPolicy) -> Self {
        Self { policy }
    }

    pub fn inspect(
        &self,
        graph: &ConversationGraph,
        request: ContextCompileRequest,
    ) -> Result<ContextPreview, DomainError> {
        if let Some(provider) = request.provider.as_ref() {
            provider.require_resolved_metadata()?;
        }
        let excluded: BTreeSet<_> = request
            .overrides
            .excluded_source_ids
            .iter()
            .map(String::as_str)
            .collect();
        let mut items = Vec::new();
        let system_source_id = format!("workspace:{}:system", request.workspace_id);
        if !request.system_prompt.is_empty() && !excluded.contains(system_source_id.as_str()) {
            push_item(
                &mut items,
                Some(system_source_id),
                ContextSourceKind::System,
                MessageRole::System,
                request.system_prompt,
                InclusionReason::SystemPolicy,
            );
        }

        for (turn, run) in self.exact_lineage(
            graph,
            &request.workspace_id,
            request.parent_run_id.as_deref(),
        )? {
            if !excluded.contains(turn.id.as_str()) {
                push_item(
                    &mut items,
                    Some(turn.id.clone()),
                    ContextSourceKind::TurnPrompt,
                    MessageRole::User,
                    turn.prompt_markdown.clone(),
                    InclusionReason::ExactAncestorPath,
                );
            }
            if !excluded.contains(run.id.as_str()) {
                push_item(
                    &mut items,
                    Some(run.id.clone()),
                    ContextSourceKind::ModelRun,
                    MessageRole::Assistant,
                    run.output_markdown().to_owned(),
                    InclusionReason::ExactAncestorPath,
                );
            }
        }

        let mut warnings = Vec::new();
        let mut seen_pins = BTreeSet::new();
        for source_id in &request.overrides.pinned_source_ids {
            if !seen_pins.insert(source_id.as_str()) {
                warnings.push(ContextWarning::DuplicatePinnedSource(source_id.clone()));
                continue;
            }
            if excluded.contains(source_id.as_str()) {
                warnings.push(ContextWarning::ExcludedPinnedSource(source_id.clone()));
                continue;
            }
            let block =
                graph
                    .content_block(source_id)
                    .ok_or_else(|| DomainError::MissingPinnedSource {
                        source_id: source_id.clone(),
                    })?;
            if block.workspace_id != request.workspace_id {
                return Err(DomainError::CrossWorkspacePinnedSource {
                    source_id: source_id.clone(),
                    workspace_id: request.workspace_id.clone(),
                });
            }
            push_item(
                &mut items,
                Some(block.id.clone()),
                ContextSourceKind::Pinned,
                block.role,
                block.content.clone(),
                InclusionReason::ExplicitPin,
            );
        }

        push_item(
            &mut items,
            None,
            ContextSourceKind::CurrentPrompt,
            MessageRole::User,
            request.current_prompt,
            InclusionReason::CurrentPrompt,
        );

        let estimated_chars = items.iter().map(|item| item.content.chars().count()).sum();
        if estimated_chars > self.policy.max_chars {
            warnings.push(ContextWarning::ExceedsLimit {
                estimated_chars,
                max_chars: self.policy.max_chars,
            });
        }
        let hash = hash_manifest(
            &self.policy.compiler_version,
            &items,
            request.provider.as_ref(),
        )?;
        let messages = items
            .iter()
            .map(|item| CanonicalMessage {
                role: item.role,
                content: item.content.clone(),
            })
            .collect();
        let manifest = ContextManifest {
            compiler_version: self.policy.compiler_version.clone(),
            items,
            estimated_chars,
            canonical_hash: hash.clone(),
        };
        Ok(ContextPreview {
            messages,
            manifest,
            estimated_chars,
            warnings,
            preview_hash: hash,
        })
    }

    pub fn compile(
        &self,
        graph: &ConversationGraph,
        request: ContextCompileRequest,
        expected_preview_hash: &str,
    ) -> Result<CompiledContext, DomainError> {
        let preview = self.inspect(graph, request)?;
        if preview.preview_hash != expected_preview_hash {
            return Err(DomainError::PreviewHashMismatch {
                expected: expected_preview_hash.into(),
                actual: preview.preview_hash,
            });
        }
        if preview.estimated_chars > self.policy.max_chars {
            return Err(DomainError::ContextTooLarge {
                estimated_chars: preview.estimated_chars,
                max_chars: self.policy.max_chars,
            });
        }
        Ok(CompiledContext {
            messages: preview.messages,
            manifest: preview.manifest,
            estimated_chars: preview.estimated_chars,
            warnings: preview.warnings,
            canonical_hash: expected_preview_hash.into(),
        })
    }

    fn exact_lineage<'a>(
        &self,
        graph: &'a ConversationGraph,
        workspace_id: &str,
        parent_run_id: Option<&str>,
    ) -> Result<Vec<(&'a Turn, &'a ModelRun)>, DomainError> {
        let mut reverse = Vec::new();
        let mut cursor = parent_run_id;
        while let Some(run_id) = cursor {
            let run = graph
                .run(run_id)
                .ok_or_else(|| DomainError::MissingRequestedParentRun(run_id.into()))?;
            let turn = graph
                .turn(&run.turn_id)
                .expect("validated graph always contains a run's turn");
            if turn.workspace_id != workspace_id {
                return Err(DomainError::RequestedParentOutsideWorkspace {
                    run_id: run_id.into(),
                    workspace_id: workspace_id.into(),
                });
            }
            reverse.push((turn, run));
            cursor = turn.parent_run_id.as_deref();
        }
        reverse.reverse();
        Ok(reverse)
    }
}

fn push_item(
    items: &mut Vec<RunContextItem>,
    source_id: Option<String>,
    source_kind: ContextSourceKind,
    role: MessageRole,
    content: String,
    inclusion_reason: InclusionReason,
) {
    items.push(RunContextItem {
        position: items.len(),
        source_id,
        source_kind,
        role,
        content_hash: sha256_hex(content.as_bytes()),
        content,
        inclusion_reason,
    });
}

fn hash_manifest(
    compiler_version: &str,
    items: &[RunContextItem],
    provider: Option<&ProviderSnapshot>,
) -> Result<String, DomainError> {
    let mut canonical = Vec::new();
    append_named_field(
        &mut canonical,
        "manifest.compiler_version",
        compiler_version,
    );
    append_named_field(
        &mut canonical,
        "provider.present",
        if provider.is_some() { "1" } else { "0" },
    );
    if let Some(provider) = provider {
        provider.require_resolved_metadata()?;
        append_named_field(&mut canonical, "provider.profile_id", &provider.profile_id);
        append_optional_named_field(
            &mut canonical,
            "provider.provider_id",
            provider.provider_id.as_deref(),
        );
        append_optional_named_field(
            &mut canonical,
            "provider.template_revision",
            provider
                .template_revision
                .map(|revision| revision.to_string())
                .as_deref(),
        );
        append_named_field(&mut canonical, "provider.name", &provider.provider_name);
        append_named_field(
            &mut canonical,
            "provider.dialect",
            dialect_name(provider.dialect),
        );
        append_optional_named_field(
            &mut canonical,
            "provider.stream_protocol",
            provider.stream_protocol.map(stream_protocol_name),
        );
        append_optional_named_field(
            &mut canonical,
            "provider.auth_placement",
            provider.auth_placement.map(auth_placement_name),
        );
        append_optional_named_field(
            &mut canonical,
            "provider.auth_header_name",
            provider.auth_header_name.as_deref(),
        );
        append_named_field(
            &mut canonical,
            "provider.additional_headers.count",
            &provider.additional_headers.len().to_string(),
        );
        for (name, value) in &provider.additional_headers {
            append_named_field(&mut canonical, "provider.additional_header.name", name);
            append_named_field(&mut canonical, "provider.additional_header.value", value);
        }
        append_named_field(&mut canonical, "provider.base_url", &provider.base_url);
        append_named_field(&mut canonical, "provider.model", &provider.model);
        append_named_field(
            &mut canonical,
            "provider.parameters.count",
            &provider.parameters.len().to_string(),
        );
        for (key, value) in &provider.parameters {
            append_named_field(&mut canonical, "provider.parameter.name", key);
            append_named_field(&mut canonical, "provider.parameter.value", value);
        }
    }
    append_named_field(
        &mut canonical,
        "manifest.items.count",
        &items.len().to_string(),
    );
    for item in items {
        append_named_field(
            &mut canonical,
            "manifest.item.position",
            &item.position.to_string(),
        );
        append_named_field(
            &mut canonical,
            "manifest.item.source_kind",
            source_kind_name(item.source_kind),
        );
        append_optional_named_field(
            &mut canonical,
            "manifest.item.source_id",
            item.source_id.as_deref(),
        );
        append_named_field(&mut canonical, "manifest.item.role", role_name(item.role));
        append_named_field(&mut canonical, "manifest.item.content", &item.content);
        append_named_field(
            &mut canonical,
            "manifest.item.inclusion_reason",
            reason_name(item.inclusion_reason),
        );
    }
    Ok(sha256_hex(&canonical))
}

fn dialect_name(dialect: ProviderDialect) -> &'static str {
    match dialect {
        ProviderDialect::OpenAiCompatible => "openai_compatible",
        ProviderDialect::Ollama => "ollama",
    }
}

fn stream_protocol_name(protocol: StreamProtocol) -> &'static str {
    match protocol {
        StreamProtocol::OpenAiSse => "openai_sse",
        StreamProtocol::OllamaNdjson => "ollama_ndjson",
        StreamProtocol::AnthropicSse => "anthropic_sse",
        StreamProtocol::GoogleSse => "google_sse",
    }
}

fn auth_placement_name(placement: AuthPlacement) -> &'static str {
    match placement {
        AuthPlacement::None => "none",
        AuthPlacement::BearerHeader => "bearer_header",
        AuthPlacement::ApiKeyHeader => "api_key_header",
        AuthPlacement::QueryParam => "query_param",
    }
}

fn append_field(target: &mut Vec<u8>, value: &str) {
    target.extend_from_slice(value.len().to_string().as_bytes());
    target.push(b':');
    target.extend_from_slice(value.as_bytes());
    target.push(b';');
}

fn append_named_field(target: &mut Vec<u8>, name: &str, value: &str) {
    append_field(target, name);
    append_field(target, value);
}

fn append_optional_named_field(target: &mut Vec<u8>, name: &str, value: Option<&str>) {
    append_named_field(
        target,
        &format!("{name}.present"),
        if value.is_some() { "1" } else { "0" },
    );
    if let Some(value) = value {
        append_named_field(target, name, value);
    }
}

fn source_kind_name(kind: ContextSourceKind) -> &'static str {
    match kind {
        ContextSourceKind::System => "system",
        ContextSourceKind::TurnPrompt => "turn_prompt",
        ContextSourceKind::ModelRun => "model_run",
        ContextSourceKind::Pinned => "pinned",
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn role_name(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn reason_name(reason: InclusionReason) -> &'static str {
    match reason {
        InclusionReason::SystemPolicy => "system_policy",
        InclusionReason::ExactAncestorPath => "exact_ancestor_path",
        InclusionReason::ExplicitPin => "explicit_pin",
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

pub fn sha256_hex(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    state.iter().map(|word| format!("{word:08x}")).collect()
}

impl ContextSnapshot {
    pub fn into_request(self) -> CanonicalRequest {
        CanonicalRequest {
            messages: self
                .manifest
                .items
                .iter()
                .map(|item| CanonicalMessage {
                    role: item.role,
                    content: item.content.clone(),
                })
                .collect(),
            provider: self.provider,
        }
    }
}

pub fn run_is_usable_as_parent(run: &ModelRun) -> bool {
    match run.status() {
        RunStatus::Completed => true,
        RunStatus::Interrupted | RunStatus::Failed | RunStatus::Cancelled => {
            !run.output_markdown().is_empty()
        }
        RunStatus::Queued | RunStatus::Connecting | RunStatus::Streaming => false,
    }
}
