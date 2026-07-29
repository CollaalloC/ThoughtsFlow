use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{
    AuthPlacement, ContentBlock, DomainError, MessageRole, ModelRun, ProviderDialect, RunStatus,
    StreamProtocol, Turn,
};

pub const CONTEXT_COMPILER_VERSION: &str = "4";

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
            if block.content_hash != sha256_hex(block.content.as_bytes()) {
                return Err(DomainError::InvalidContentBlockHash { id });
            }
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

    pub fn turns(&self) -> impl ExactSizeIterator<Item = &Turn> {
        self.turns.values()
    }

    pub fn runs(&self) -> impl ExactSizeIterator<Item = &ModelRun> {
        self.runs.values()
    }

    pub fn content_blocks(&self) -> impl ExactSizeIterator<Item = &ContentBlock> {
        self.content_blocks.values()
    }

    fn validate_acyclic(&self) -> Result<(), DomainError> {
        let mut resolved = BTreeSet::new();
        for turn in self.turns.values() {
            if resolved.contains(turn.id.as_str()) {
                continue;
            }
            let mut path = Vec::new();
            let mut visiting = BTreeSet::new();
            let mut cursor = turn;
            loop {
                if resolved.contains(cursor.id.as_str()) {
                    break;
                }
                if !visiting.insert(cursor.id.as_str()) {
                    return Err(DomainError::CyclicAncestry {
                        turn_id: cursor.id.clone(),
                    });
                }
                path.push(cursor.id.as_str());
                let Some(parent_run_id) = cursor.parent_run_id.as_deref() else {
                    break;
                };
                let parent_run = &self.runs[parent_run_id];
                cursor = &self.turns[&parent_run.turn_id];
            }
            resolved.extend(path);
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextPin {
    pub source_ref: ContextSourceRef,
    pub content_block_id: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextOverrides {
    pub pinned_sources: Vec<ContextPin>,
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
    CompactionSummary,
    BranchSummary,
    CurrentPrompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextSourceRefKind {
    WorkspaceSystem,
    TurnPrompt,
    ModelRun,
    ContentBlock,
    CurrentPrompt,
    CheckpointSummary,
    BranchSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContextSourceRef {
    pub kind: ContextSourceRefKind,
    pub id: Option<String>,
}

impl ContextSourceRef {
    pub fn new(kind: ContextSourceRefKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: Some(id.into()),
        }
    }

    pub fn stable_id(&self) -> String {
        let prefix = match self.kind {
            ContextSourceRefKind::WorkspaceSystem => "workspace-system",
            ContextSourceRefKind::TurnPrompt => "turn-prompt",
            ContextSourceRefKind::ModelRun => "model-run",
            ContextSourceRefKind::ContentBlock => "content-block",
            ContextSourceRefKind::CurrentPrompt => "current-prompt",
            ContextSourceRefKind::CheckpointSummary => "checkpoint-summary",
            ContextSourceRefKind::BranchSummary => "branch-summary",
        };
        self.id
            .as_deref()
            .map_or_else(|| prefix.to_owned(), |id| format!("{prefix}:{id}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InclusionReason {
    SystemPolicy,
    ExactAncestorPath,
    ExplicitPin,
    LatestCompaction,
    BranchSummary,
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
    /// Legacy untyped source identifier retained for old receipts. New callers
    /// should use `source_ref`, which cannot be confused with content identity.
    pub source_id: Option<String>,
    pub source_ref: ContextSourceRef,
    pub source_kind: ContextSourceKind,
    pub role: MessageRole,
    pub content: String,
    pub content_block_id: String,
    pub content_hash: String,
    pub inclusion_reason: InclusionReason,
    pub mandatory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextManifest {
    pub compiler_version: String,
    pub items: Vec<RunContextItem>,
    pub estimated_chars: usize,
    pub canonical_hash: String,
    pub warnings: Vec<ContextWarning>,
    pub checkpoint_provenance: Option<ContextCheckpointProvenance>,
    pub branch_summary_provenance: Vec<ContextCheckpointProvenance>,
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
    /// The uncompressed exact root-to-run route. Pins and checkpoint summaries
    /// are projection inputs and therefore do not appear in this raw history.
    pub raw_items: Vec<RunContextItem>,
    pub applied_checkpoint: Option<ContextCheckpointProvenance>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextCheckpointKind {
    Compaction,
    BranchSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMaintenanceStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    Conflicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMaintenanceRun {
    pub id: String,
    pub workspace_id: String,
    pub kind: ContextCheckpointKind,
    pub status: ContextMaintenanceStatus,
    pub branch_pointer_id: Option<String>,
    pub branch_revision: Option<u64>,
    pub anchor_run_id: String,
    pub first_kept_run_id: Option<String>,
    pub source_run_ids: Vec<String>,
    pub source_hash: String,
    pub provider: Option<ProviderSnapshot>,
    pub request_json: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCheckpoint {
    pub id: String,
    pub workspace_id: String,
    pub maintenance_run_id: String,
    pub kind: ContextCheckpointKind,
    pub branch_pointer_id: Option<String>,
    pub branch_revision: Option<u64>,
    /// Candidate location on the exact Model Run path. Applying it additionally
    /// requires explicit branch visibility evidence in `ContextCompileInput`.
    pub anchor_run_id: String,
    /// First exact Model Run retained after a compaction summary.
    pub first_kept_run_id: Option<String>,
    pub summary: String,
    pub summary_content_block_id: String,
    pub source_run_ids: Vec<String>,
    pub source_hash: String,
    pub provider: Option<ProviderSnapshot>,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCheckpointProvenance {
    pub checkpoint_id: String,
    pub maintenance_run_id: String,
    pub kind: ContextCheckpointKind,
    pub branch_pointer_id: Option<String>,
    pub branch_revision: Option<u64>,
    pub anchor_run_id: String,
    pub first_kept_run_id: Option<String>,
    pub summary_content_block_id: String,
    pub source_run_ids: Vec<String>,
    pub source_hash: String,
}

impl ContextCheckpoint {
    pub fn provenance(&self) -> ContextCheckpointProvenance {
        ContextCheckpointProvenance {
            checkpoint_id: self.id.clone(),
            maintenance_run_id: self.maintenance_run_id.clone(),
            kind: self.kind,
            branch_pointer_id: self.branch_pointer_id.clone(),
            branch_revision: self.branch_revision,
            anchor_run_id: self.anchor_run_id.clone(),
            first_kept_run_id: self.first_kept_run_id.clone(),
            summary_content_block_id: self.summary_content_block_id.clone(),
            source_run_ids: self.source_run_ids.clone(),
            source_hash: self.source_hash.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCompileInput {
    pub request: ContextCompileRequest,
    pub checkpoints: Vec<ContextCheckpoint>,
    /// Checkpoints proven visible from the selected branch revision.
    ///
    /// `None` is valid for legacy, branch-neutral checkpoints, but branch-scoped
    /// checkpoints fail closed until the caller supplies visibility evidence.
    /// `Some(Vec::new())` explicitly means that the selected branch inherits no
    /// branch-scoped checkpoints.
    pub eligible_checkpoint_ids: Option<Vec<String>>,
}

impl ContextCompileInput {
    pub fn new(request: ContextCompileRequest) -> Self {
        Self {
            request,
            checkpoints: Vec::new(),
            eligible_checkpoint_ids: None,
        }
    }

    pub fn with_checkpoints(mut self, checkpoints: Vec<ContextCheckpoint>) -> Self {
        self.checkpoints = checkpoints;
        self
    }

    pub fn with_eligible_checkpoint_ids(mut self, eligible_checkpoint_ids: Vec<String>) -> Self {
        self.eligible_checkpoint_ids = Some(eligible_checkpoint_ids);
        self
    }
}

impl From<ContextCompileRequest> for ContextCompileInput {
    fn from(request: ContextCompileRequest) -> Self {
        Self::new(request)
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

    pub fn inspect<I>(
        &self,
        graph: &ConversationGraph,
        input: I,
    ) -> Result<ContextPreview, DomainError>
    where
        I: Into<ContextCompileInput>,
    {
        let ContextCompileInput {
            request,
            checkpoints,
            eligible_checkpoint_ids,
        } = input.into();
        if let Some(provider) = request.provider.as_ref() {
            provider.require_resolved_metadata()?;
        }
        let lineage = self.exact_lineage(
            graph,
            &request.workspace_id,
            request.parent_run_id.as_deref(),
        )?;
        let excluded: BTreeSet<_> = request
            .overrides
            .excluded_source_ids
            .iter()
            .map(String::as_str)
            .collect();

        let mut raw_items = Vec::with_capacity(lineage.len().saturating_mul(2).saturating_add(2));
        let system_source_id = format!("workspace:{}:system", request.workspace_id);
        push_item(
            &mut raw_items,
            (
                Some(system_source_id.clone()),
                ContextSourceRef::new(
                    ContextSourceRefKind::WorkspaceSystem,
                    request.workspace_id.clone(),
                ),
            ),
            ContextSourceKind::System,
            MessageRole::System,
            request.system_prompt.clone(),
            InclusionReason::SystemPolicy,
            true,
        );
        for (turn, run) in &lineage {
            push_lineage_pair(&mut raw_items, turn, run);
        }
        push_item(
            &mut raw_items,
            (
                None,
                ContextSourceRef::new(
                    ContextSourceRefKind::CurrentPrompt,
                    request.workspace_id.clone(),
                ),
            ),
            ContextSourceKind::CurrentPrompt,
            MessageRole::User,
            request.current_prompt.clone(),
            InclusionReason::CurrentPrompt,
            true,
        );

        let lineage_positions = lineage
            .iter()
            .enumerate()
            .map(|(position, (_, run))| (run.id.as_str(), position))
            .collect::<BTreeMap<_, _>>();
        let eligible_checkpoint_ids = eligible_checkpoint_ids
            .as_ref()
            .map(|ids| ids.iter().map(String::as_str).collect::<BTreeSet<_>>());
        for checkpoint in &checkpoints {
            if checkpoint.branch_pointer_id.is_some() != checkpoint.branch_revision.is_some() {
                return Err(DomainError::InvalidCheckpointBranchEvidence {
                    checkpoint_id: checkpoint.id.clone(),
                });
            }
            if checkpoint.kind == ContextCheckpointKind::BranchSummary
                && let Some(first_kept_run_id) = checkpoint.first_kept_run_id.as_ref()
            {
                return Err(DomainError::InvalidCheckpointBoundary {
                    checkpoint_id: checkpoint.id.clone(),
                    first_kept_run_id: first_kept_run_id.clone(),
                });
            }
            if checkpoint.workspace_id != request.workspace_id {
                return Err(DomainError::CheckpointOutsideWorkspace {
                    checkpoint_id: checkpoint.id.clone(),
                    workspace_id: request.workspace_id.clone(),
                });
            }
            if checkpoint.branch_pointer_id.is_some() && eligible_checkpoint_ids.is_none() {
                return Err(DomainError::MissingCheckpointVisibilityEvidence {
                    checkpoint_id: checkpoint.id.clone(),
                });
            }
        }
        let latest_compaction = checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint_is_visible(checkpoint, eligible_checkpoint_ids.as_ref())
            })
            .filter(|checkpoint| !checkpoint_is_excluded(&excluded, checkpoint))
            .filter(|checkpoint| checkpoint.kind == ContextCheckpointKind::Compaction)
            .filter_map(|checkpoint| {
                lineage_positions
                    .get(checkpoint.anchor_run_id.as_str())
                    .copied()
                    .map(|position| (position, checkpoint))
            })
            .max_by(|(left_position, left), (right_position, right)| {
                (*left_position, left.created_at, left.id.as_str()).cmp(&(
                    *right_position,
                    right.created_at,
                    right.id.as_str(),
                ))
            });
        let mut effective_start = 0;
        let mut compaction_order = None;
        let mut applied_checkpoint = None;
        let mut items = Vec::with_capacity(raw_items.len().saturating_add(checkpoints.len()));
        push_existing_item(&mut items, &raw_items[0]);
        if let Some((anchor_position, checkpoint)) = latest_compaction {
            effective_start = match checkpoint.first_kept_run_id.as_deref() {
                Some(run_id) => {
                    let kept_position =
                        lineage_positions.get(run_id).copied().ok_or_else(|| {
                            DomainError::InvalidCheckpointBoundary {
                                checkpoint_id: checkpoint.id.clone(),
                                first_kept_run_id: run_id.into(),
                            }
                        })?;
                    if kept_position > anchor_position {
                        return Err(DomainError::InvalidCheckpointBoundary {
                            checkpoint_id: checkpoint.id.clone(),
                            first_kept_run_id: run_id.into(),
                        });
                    }
                    kept_position
                }
                None => anchor_position.saturating_add(1),
            };
            push_checkpoint_item(
                &mut items,
                checkpoint,
                ContextSourceRefKind::CheckpointSummary,
                ContextSourceKind::CompactionSummary,
                InclusionReason::LatestCompaction,
            )?;
            compaction_order = Some((
                anchor_position,
                checkpoint.created_at,
                checkpoint.id.clone(),
            ));
            applied_checkpoint = Some(checkpoint.provenance());
        }

        let mut branch_summaries = checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint_is_visible(checkpoint, eligible_checkpoint_ids.as_ref())
            })
            .filter(|checkpoint| !checkpoint_is_excluded(&excluded, checkpoint))
            .filter(|checkpoint| checkpoint.kind == ContextCheckpointKind::BranchSummary)
            .filter_map(|checkpoint| {
                lineage_positions
                    .get(checkpoint.anchor_run_id.as_str())
                    .copied()
                    .map(|position| (position, checkpoint))
            })
            .filter(|(position, checkpoint)| {
                compaction_order.as_ref().is_none_or(|order| {
                    (*position, checkpoint.created_at, checkpoint.id.as_str())
                        > (order.0, order.1, order.2.as_str())
                })
            })
            .collect::<Vec<_>>();
        branch_summaries.sort_by(|(left_position, left), (right_position, right)| {
            (*left_position, left.created_at, left.id.as_str()).cmp(&(
                *right_position,
                right.created_at,
                right.id.as_str(),
            ))
        });
        let branch_summary_provenance = branch_summaries
            .iter()
            .map(|(_, checkpoint)| checkpoint.provenance())
            .collect::<Vec<_>>();

        for (lineage_position, _) in lineage.iter().enumerate().skip(effective_start) {
            let raw_position = 1 + lineage_position * 2;
            let turn_item = &raw_items[raw_position];
            if !source_is_excluded(
                &excluded,
                turn_item.source_id.as_deref(),
                &turn_item.source_ref,
            ) {
                push_existing_item(&mut items, turn_item);
            }
            let run_item = &raw_items[raw_position + 1];
            if !source_is_excluded(
                &excluded,
                run_item.source_id.as_deref(),
                &run_item.source_ref,
            ) {
                push_existing_item(&mut items, run_item);
            }
            for (_, checkpoint) in branch_summaries
                .iter()
                .filter(|(position, _)| *position == lineage_position)
            {
                push_checkpoint_item(
                    &mut items,
                    checkpoint,
                    ContextSourceRefKind::BranchSummary,
                    ContextSourceKind::BranchSummary,
                    InclusionReason::BranchSummary,
                )?;
            }
        }

        let mut warnings = Vec::new();
        let mut seen_pins = BTreeSet::new();
        for pin in &request.overrides.pinned_sources {
            let source_id = pin.source_ref.stable_id();
            if !seen_pins.insert(pin.clone()) {
                warnings.push(ContextWarning::DuplicatePinnedSource(source_id));
                continue;
            }
            if source_is_excluded(&excluded, pin.source_ref.id.as_deref(), &pin.source_ref) {
                warnings.push(ContextWarning::ExcludedPinnedSource(source_id));
                continue;
            }
            if let Some(existing) = items.iter_mut().find(|item| pin_matches_item(pin, item)) {
                existing.inclusion_reason = InclusionReason::ExplicitPin;
                continue;
            }
            if let Some(raw_item) = raw_items.iter().find(|item| pin_matches_item(pin, item)) {
                let mut raw_item = raw_item.clone();
                raw_item.inclusion_reason = InclusionReason::ExplicitPin;
                push_existing_item(&mut items, &raw_item);
                continue;
            }
            let pinned_item = resolve_pinned_item(graph, &checkpoints, &request.workspace_id, pin)?;
            push_existing_item(&mut items, &pinned_item);
        }

        push_existing_item(
            &mut items,
            raw_items
                .last()
                .expect("raw context always includes the current prompt"),
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
            applied_checkpoint.as_ref(),
            &branch_summary_provenance,
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
            warnings: warnings.clone(),
            checkpoint_provenance: applied_checkpoint.clone(),
            branch_summary_provenance,
        };
        Ok(ContextPreview {
            messages,
            manifest,
            raw_items,
            applied_checkpoint,
            estimated_chars,
            warnings,
            preview_hash: hash,
        })
    }

    pub fn compile<I>(
        &self,
        graph: &ConversationGraph,
        input: I,
        expected_preview_hash: &str,
    ) -> Result<CompiledContext, DomainError>
    where
        I: Into<ContextCompileInput>,
    {
        let preview = self.inspect(graph, input)?;
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

fn checkpoint_is_visible(
    checkpoint: &ContextCheckpoint,
    eligible_checkpoint_ids: Option<&BTreeSet<&str>>,
) -> bool {
    checkpoint.branch_pointer_id.is_none()
        || eligible_checkpoint_ids.is_some_and(|ids| ids.contains(checkpoint.id.as_str()))
}

fn checkpoint_is_excluded(
    excluded_source_ids: &BTreeSet<&str>,
    checkpoint: &ContextCheckpoint,
) -> bool {
    let source_ref_kind = match checkpoint.kind {
        ContextCheckpointKind::Compaction => ContextSourceRefKind::CheckpointSummary,
        ContextCheckpointKind::BranchSummary => ContextSourceRefKind::BranchSummary,
    };
    let source_ref = ContextSourceRef::new(source_ref_kind, checkpoint.id.clone());
    source_is_excluded(
        excluded_source_ids,
        Some(checkpoint.id.as_str()),
        &source_ref,
    )
}

fn push_lineage_pair(items: &mut Vec<RunContextItem>, turn: &Turn, run: &ModelRun) {
    push_item(
        items,
        (
            Some(turn.id.clone()),
            ContextSourceRef::new(ContextSourceRefKind::TurnPrompt, turn.id.clone()),
        ),
        ContextSourceKind::TurnPrompt,
        MessageRole::User,
        turn.prompt_markdown.clone(),
        InclusionReason::ExactAncestorPath,
        false,
    );
    push_item(
        items,
        (
            Some(run.id.clone()),
            ContextSourceRef::new(ContextSourceRefKind::ModelRun, run.id.clone()),
        ),
        ContextSourceKind::ModelRun,
        MessageRole::Assistant,
        run.output_markdown().to_owned(),
        InclusionReason::ExactAncestorPath,
        false,
    );
}

fn push_content_block_item(items: &mut Vec<RunContextItem>, block: &ContentBlock) {
    items.push(RunContextItem {
        position: items.len(),
        source_id: Some(block.id.clone()),
        source_ref: ContextSourceRef::new(ContextSourceRefKind::ContentBlock, block.id.clone()),
        source_kind: ContextSourceKind::Pinned,
        role: block.role,
        content: block.content.clone(),
        content_block_id: block.id.clone(),
        content_hash: block.content_hash.clone(),
        inclusion_reason: InclusionReason::ExplicitPin,
        mandatory: false,
    });
}

fn push_checkpoint_item(
    items: &mut Vec<RunContextItem>,
    checkpoint: &ContextCheckpoint,
    source_ref_kind: ContextSourceRefKind,
    source_kind: ContextSourceKind,
    inclusion_reason: InclusionReason,
) -> Result<(), DomainError> {
    checkpoint_summary_content_hash(checkpoint)?;
    push_item(
        items,
        (
            Some(checkpoint.id.clone()),
            ContextSourceRef::new(source_ref_kind, checkpoint.id.clone()),
        ),
        source_kind,
        MessageRole::System,
        checkpoint.summary.clone(),
        inclusion_reason,
        false,
    );
    Ok(())
}

fn push_checkpoint_content_block_item(
    items: &mut Vec<RunContextItem>,
    checkpoint: &ContextCheckpoint,
) -> Result<(), DomainError> {
    let content_hash = checkpoint_summary_content_hash(checkpoint)?;
    items.push(RunContextItem {
        position: items.len(),
        source_id: Some(checkpoint.summary_content_block_id.clone()),
        source_ref: ContextSourceRef::new(
            ContextSourceRefKind::ContentBlock,
            checkpoint.summary_content_block_id.clone(),
        ),
        source_kind: ContextSourceKind::Pinned,
        role: MessageRole::System,
        content: checkpoint.summary.clone(),
        content_block_id: checkpoint.summary_content_block_id.clone(),
        content_hash,
        inclusion_reason: InclusionReason::ExplicitPin,
        mandatory: false,
    });
    Ok(())
}

fn checkpoint_summary_content_hash(checkpoint: &ContextCheckpoint) -> Result<String, DomainError> {
    let content_hash = sha256_hex(checkpoint.summary.as_bytes());
    let expected_content_block_id =
        format!("block-{}-{content_hash}", role_name(MessageRole::System));
    if checkpoint.summary_content_block_id != expected_content_block_id {
        return Err(DomainError::InvalidCheckpointContentIdentity {
            checkpoint_id: checkpoint.id.clone(),
        });
    }
    Ok(content_hash)
}

fn push_item(
    items: &mut Vec<RunContextItem>,
    source: (Option<String>, ContextSourceRef),
    source_kind: ContextSourceKind,
    role: MessageRole,
    content: String,
    inclusion_reason: InclusionReason,
    mandatory: bool,
) {
    let (source_id, source_ref) = source;
    let content_hash = sha256_hex(content.as_bytes());
    items.push(RunContextItem {
        position: items.len(),
        source_id,
        source_ref,
        source_kind,
        role,
        content_block_id: format!("block-{}-{content_hash}", role_name(role)),
        content_hash,
        content,
        inclusion_reason,
        mandatory,
    });
}

fn push_existing_item(items: &mut Vec<RunContextItem>, item: &RunContextItem) {
    let mut item = item.clone();
    item.position = items.len();
    items.push(item);
}

fn pin_matches_item(pin: &ContextPin, item: &RunContextItem) -> bool {
    item.source_ref == pin.source_ref
        && item.content_block_id == pin.content_block_id
        && item.content_hash == pin.content_hash
}

fn resolve_pinned_item(
    graph: &ConversationGraph,
    checkpoints: &[ContextCheckpoint],
    workspace_id: &str,
    pin: &ContextPin,
) -> Result<RunContextItem, DomainError> {
    let source_id = pin.source_ref.stable_id();
    let id = pin
        .source_ref
        .id
        .as_deref()
        .ok_or_else(|| DomainError::MissingPinnedSource {
            source_id: source_id.clone(),
        })?;
    let mut candidates = Vec::with_capacity(1);
    match pin.source_ref.kind {
        ContextSourceRefKind::TurnPrompt => {
            let turn = graph
                .turn(id)
                .ok_or_else(|| DomainError::MissingPinnedSource {
                    source_id: source_id.clone(),
                })?;
            if turn.workspace_id != workspace_id {
                return Err(DomainError::CrossWorkspacePinnedSource {
                    source_id,
                    workspace_id: workspace_id.into(),
                });
            }
            push_item(
                &mut candidates,
                (
                    Some(turn.id.clone()),
                    ContextSourceRef::new(ContextSourceRefKind::TurnPrompt, turn.id.clone()),
                ),
                ContextSourceKind::TurnPrompt,
                MessageRole::User,
                turn.prompt_markdown.clone(),
                InclusionReason::ExplicitPin,
                false,
            );
        }
        ContextSourceRefKind::ModelRun => {
            let run = graph
                .run(id)
                .ok_or_else(|| DomainError::MissingPinnedSource {
                    source_id: source_id.clone(),
                })?;
            let turn =
                graph
                    .turn(&run.turn_id)
                    .ok_or_else(|| DomainError::MissingPinnedSource {
                        source_id: source_id.clone(),
                    })?;
            if turn.workspace_id != workspace_id {
                return Err(DomainError::CrossWorkspacePinnedSource {
                    source_id,
                    workspace_id: workspace_id.into(),
                });
            }
            push_item(
                &mut candidates,
                (
                    Some(run.id.clone()),
                    ContextSourceRef::new(ContextSourceRefKind::ModelRun, run.id.clone()),
                ),
                ContextSourceKind::ModelRun,
                MessageRole::Assistant,
                run.output_markdown().to_owned(),
                InclusionReason::ExplicitPin,
                false,
            );
        }
        ContextSourceRefKind::ContentBlock => {
            if let Some(block) = graph.content_block(id) {
                if block.workspace_id != workspace_id {
                    return Err(DomainError::CrossWorkspacePinnedSource {
                        source_id,
                        workspace_id: workspace_id.into(),
                    });
                }
                push_content_block_item(&mut candidates, block);
            } else {
                let checkpoint = checkpoints
                    .iter()
                    .find(|checkpoint| checkpoint.summary_content_block_id == id)
                    .ok_or_else(|| DomainError::MissingPinnedSource {
                        source_id: source_id.clone(),
                    })?;
                if checkpoint.workspace_id != workspace_id {
                    return Err(DomainError::CrossWorkspacePinnedSource {
                        source_id,
                        workspace_id: workspace_id.into(),
                    });
                }
                push_checkpoint_content_block_item(&mut candidates, checkpoint)?;
            }
        }
        ContextSourceRefKind::CheckpointSummary | ContextSourceRefKind::BranchSummary => {
            let expected_kind = match pin.source_ref.kind {
                ContextSourceRefKind::CheckpointSummary => ContextCheckpointKind::Compaction,
                ContextSourceRefKind::BranchSummary => ContextCheckpointKind::BranchSummary,
                _ => unreachable!("matched checkpoint source kinds"),
            };
            let checkpoint = checkpoints
                .iter()
                .find(|checkpoint| checkpoint.id == id && checkpoint.kind == expected_kind)
                .ok_or_else(|| DomainError::MissingPinnedSource {
                    source_id: source_id.clone(),
                })?;
            if checkpoint.workspace_id != workspace_id {
                return Err(DomainError::CrossWorkspacePinnedSource {
                    source_id,
                    workspace_id: workspace_id.into(),
                });
            }
            let (source_kind, reason) = match expected_kind {
                ContextCheckpointKind::Compaction => (
                    ContextSourceKind::CompactionSummary,
                    InclusionReason::LatestCompaction,
                ),
                ContextCheckpointKind::BranchSummary => (
                    ContextSourceKind::BranchSummary,
                    InclusionReason::BranchSummary,
                ),
            };
            push_checkpoint_item(
                &mut candidates,
                checkpoint,
                pin.source_ref.kind,
                source_kind,
                reason,
            )?;
        }
        ContextSourceRefKind::WorkspaceSystem | ContextSourceRefKind::CurrentPrompt => {
            return Err(DomainError::MissingPinnedSource { source_id });
        }
    }
    let mut candidate = candidates
        .pop()
        .expect("a resolved pin always materializes exactly one candidate");
    if !pin_matches_item(pin, &candidate) {
        return Err(DomainError::MissingPinnedSource { source_id });
    }
    candidate.inclusion_reason = InclusionReason::ExplicitPin;
    Ok(candidate)
}

fn source_is_excluded(
    excluded: &BTreeSet<&str>,
    legacy_source_id: Option<&str>,
    source_ref: &ContextSourceRef,
) -> bool {
    if excluded.is_empty() {
        return false;
    }
    legacy_source_id.is_some_and(|id| excluded.contains(id))
        || excluded.contains(source_ref.stable_id().as_str())
}

fn hash_manifest(
    compiler_version: &str,
    items: &[RunContextItem],
    provider: Option<&ProviderSnapshot>,
    checkpoint_provenance: Option<&ContextCheckpointProvenance>,
    branch_summary_provenance: &[ContextCheckpointProvenance],
) -> Result<String, DomainError> {
    let content_bytes = items.iter().map(|item| item.content.len()).sum::<usize>();
    let mut canonical =
        Vec::with_capacity(content_bytes.saturating_add(items.len().saturating_mul(320)));
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
        append_named_usize_field(
            &mut canonical,
            "provider.additional_headers.count",
            provider.additional_headers.len(),
        );
        for (name, value) in &provider.additional_headers {
            append_named_field(&mut canonical, "provider.additional_header.name", name);
            append_named_field(&mut canonical, "provider.additional_header.value", value);
        }
        append_named_field(&mut canonical, "provider.base_url", &provider.base_url);
        append_named_field(&mut canonical, "provider.model", &provider.model);
        append_named_usize_field(
            &mut canonical,
            "provider.parameters.count",
            provider.parameters.len(),
        );
        for (key, value) in &provider.parameters {
            append_named_field(&mut canonical, "provider.parameter.name", key);
            append_named_field(&mut canonical, "provider.parameter.value", value);
        }
    }
    append_checkpoint_provenance(&mut canonical, "manifest.compaction", checkpoint_provenance);
    append_named_usize_field(
        &mut canonical,
        "manifest.branch_summaries.count",
        branch_summary_provenance.len(),
    );
    for provenance in branch_summary_provenance {
        append_checkpoint_provenance(&mut canonical, "manifest.branch_summary", Some(provenance));
    }
    append_named_usize_field(&mut canonical, "manifest.items.count", items.len());
    append_named_field(
        &mut canonical,
        "manifest.items.schema",
        "position,source_kind,source_id,source_ref_kind,source_ref_id,role,content_block_id,content_hash,content,inclusion_reason,mandatory",
    );
    for item in items {
        // The schema fixes field order, while every value remains length-framed.
        // Repeating the same long field names for each item adds no identity
        // information and dominates deep context hashing in debug builds.
        append_usize_field(&mut canonical, item.position);
        append_field(&mut canonical, source_kind_name(item.source_kind));
        append_optional_field(&mut canonical, item.source_id.as_deref());
        append_field(&mut canonical, source_ref_kind_name(item.source_ref.kind));
        append_optional_field(&mut canonical, item.source_ref.id.as_deref());
        append_field(&mut canonical, role_name(item.role));
        append_field(&mut canonical, &item.content_block_id);
        append_field(&mut canonical, &item.content_hash);
        append_field(&mut canonical, &item.content);
        append_field(&mut canonical, reason_name(item.inclusion_reason));
        append_field(&mut canonical, if item.mandatory { "1" } else { "0" });
    }
    Ok(sha256_hex(&canonical))
}

fn dialect_name(dialect: ProviderDialect) -> &'static str {
    match dialect {
        ProviderDialect::OpenAiCompatible => "openai_compatible",
        ProviderDialect::Ollama => "ollama",
        ProviderDialect::Anthropic => "anthropic",
        ProviderDialect::GoogleGenerativeAi => "google_generative_ai",
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
    append_usize_digits(target, value.len());
    target.push(b':');
    target.extend_from_slice(value.as_bytes());
    target.push(b';');
}

fn append_usize_field(target: &mut Vec<u8>, value: usize) {
    append_usize_digits(target, decimal_len(value));
    target.push(b':');
    append_usize_digits(target, value);
    target.push(b';');
}

fn append_usize_digits(target: &mut Vec<u8>, mut value: usize) {
    let mut digits = [0_u8; 20];
    let mut cursor = digits.len();
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    target.extend_from_slice(&digits[cursor..]);
}

fn decimal_len(mut value: usize) -> usize {
    let mut len = 1;
    while value >= 10 {
        value /= 10;
        len += 1;
    }
    len
}

fn append_named_field(target: &mut Vec<u8>, name: &str, value: &str) {
    append_field(target, name);
    append_field(target, value);
}

fn append_named_usize_field(target: &mut Vec<u8>, name: &str, value: usize) {
    append_field(target, name);
    append_usize_field(target, value);
}

fn append_optional_field(target: &mut Vec<u8>, value: Option<&str>) {
    append_field(target, if value.is_some() { "1" } else { "0" });
    if let Some(value) = value {
        append_field(target, value);
    }
}

fn append_optional_named_field(target: &mut Vec<u8>, name: &str, value: Option<&str>) {
    append_usize_digits(target, name.len() + ".present".len());
    target.push(b':');
    target.extend_from_slice(name.as_bytes());
    target.extend_from_slice(b".present;");
    append_field(target, if value.is_some() { "1" } else { "0" });
    if let Some(value) = value {
        append_named_field(target, name, value);
    }
}

fn append_checkpoint_provenance(
    target: &mut Vec<u8>,
    prefix: &str,
    provenance: Option<&ContextCheckpointProvenance>,
) {
    append_named_field(
        target,
        &format!("{prefix}.present"),
        if provenance.is_some() { "1" } else { "0" },
    );
    let Some(provenance) = provenance else {
        return;
    };
    append_named_field(
        target,
        &format!("{prefix}.checkpoint_id"),
        &provenance.checkpoint_id,
    );
    append_named_field(
        target,
        &format!("{prefix}.maintenance_run_id"),
        &provenance.maintenance_run_id,
    );
    append_named_field(
        target,
        &format!("{prefix}.kind"),
        checkpoint_kind_name(provenance.kind),
    );
    append_optional_named_field(
        target,
        &format!("{prefix}.branch_pointer_id"),
        provenance.branch_pointer_id.as_deref(),
    );
    append_optional_named_field(
        target,
        &format!("{prefix}.branch_revision"),
        provenance
            .branch_revision
            .map(|revision| revision.to_string())
            .as_deref(),
    );
    append_named_field(
        target,
        &format!("{prefix}.anchor_run_id"),
        &provenance.anchor_run_id,
    );
    append_optional_named_field(
        target,
        &format!("{prefix}.first_kept_run_id"),
        provenance.first_kept_run_id.as_deref(),
    );
    append_named_field(
        target,
        &format!("{prefix}.summary_content_block_id"),
        &provenance.summary_content_block_id,
    );
    append_named_usize_field(
        target,
        &format!("{prefix}.source_run_ids.count"),
        provenance.source_run_ids.len(),
    );
    for run_id in &provenance.source_run_ids {
        append_named_field(target, &format!("{prefix}.source_run_id"), run_id);
    }
    append_named_field(
        target,
        &format!("{prefix}.source_hash"),
        &provenance.source_hash,
    );
}

fn checkpoint_kind_name(kind: ContextCheckpointKind) -> &'static str {
    match kind {
        ContextCheckpointKind::Compaction => "compaction",
        ContextCheckpointKind::BranchSummary => "branch_summary",
    }
}

fn source_kind_name(kind: ContextSourceKind) -> &'static str {
    match kind {
        ContextSourceKind::System => "system",
        ContextSourceKind::TurnPrompt => "turn_prompt",
        ContextSourceKind::ModelRun => "model_run",
        ContextSourceKind::Pinned => "pinned",
        ContextSourceKind::CompactionSummary => "compaction_summary",
        ContextSourceKind::BranchSummary => "branch_summary",
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn source_ref_kind_name(kind: ContextSourceRefKind) -> &'static str {
    match kind {
        ContextSourceRefKind::WorkspaceSystem => "workspace_system",
        ContextSourceRefKind::TurnPrompt => "turn_prompt",
        ContextSourceRefKind::ModelRun => "model_run",
        ContextSourceRefKind::ContentBlock => "content_block",
        ContextSourceRefKind::CurrentPrompt => "current_prompt",
        ContextSourceRefKind::CheckpointSummary => "checkpoint_summary",
        ContextSourceRefKind::BranchSummary => "branch_summary",
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
        InclusionReason::LatestCompaction => "latest_compaction",
        InclusionReason::BranchSummary => "branch_summary",
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

pub fn sha256_hex(input: &[u8]) -> String {
    format!("{:x}", Sha256::digest(input))
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
