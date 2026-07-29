use super::{RepositoryError, RepositoryResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInfo {
    pub version: i64,
    pub strict_tables: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub system_prompt: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub id: String,
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub prompt_block_id: String,
    pub prompt_markdown: String,
    pub title: String,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatusRecord {
    Queued,
    Connecting,
    Streaming,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl RunStatusRecord {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Streaming => "streaming",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }

    pub fn from_stored(value: &str) -> RepositoryResult<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "connecting" => Ok(Self::Connecting),
            "streaming" => Ok(Self::Streaming),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(RepositoryError::InvalidStoredValue(format!(
                "unknown model run status `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRunRecord {
    pub id: String,
    pub turn_id: String,
    pub workspace_id: String,
    pub provider_profile_id: Option<String>,
    pub model: String,
    pub status: RunStatusRecord,
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub provider_snapshot_json: String,
    pub usage_json: Option<String>,
    pub error_json: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub checkpointed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProviderProvenanceRecord {
    pub run_id: String,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlockRecord {
    pub id: String,
    pub role: String,
    pub content: String,
    pub content_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextManifestRecord {
    pub id: String,
    pub workspace_id: String,
    pub compiler_version: String,
    pub strategy: String,
    pub estimated_chars: i64,
    pub canonical_hash: String,
    pub warnings_json: String,
    pub checkpoint_provenance_json: Option<String>,
    pub branch_summary_provenance_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContextItemRecord {
    pub manifest_id: String,
    pub workspace_id: String,
    pub position: i64,
    pub source_id: Option<String>,
    pub source_ref_kind: String,
    pub source_ref_id: Option<String>,
    pub source_kind: String,
    pub role: String,
    pub content_block_id: String,
    pub inclusion_reason: String,
    pub mandatory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredContextItem {
    pub position: i64,
    pub source_id: Option<String>,
    pub source_ref_kind: Option<String>,
    pub source_ref_id: Option<String>,
    pub source_kind: String,
    pub role: String,
    pub content_block_id: String,
    pub content: String,
    pub content_hash: String,
    pub inclusion_reason: String,
    pub mandatory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnapshotRecord {
    pub id: String,
    pub run_id: String,
    pub manifest_id: String,
    pub workspace_id: String,
    pub provider_profile_id: Option<String>,
    pub provider_id: Option<String>,
    pub template_revision: Option<i64>,
    pub stream_protocol: Option<String>,
    pub auth_placement: Option<String>,
    pub auth_header_name: Option<String>,
    pub additional_headers_json: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub parameters_json: String,
    pub request_json: String,
    pub canonical_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRunReceipt {
    pub snapshot: ContextSnapshotRecord,
    pub manifest: ContextManifestRecord,
    pub items: Vec<StoredContextItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfileRecord {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub dialect: String,
    pub base_url: String,
    pub default_model: String,
    pub parameters_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPointerRecord {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub head_run_id: String,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCursorRecord {
    pub workspace_id: String,
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    pub version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCursorUpdateRecord {
    pub workspace_id: String,
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    pub expected_version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOverrideItemRecord {
    pub workspace_id: String,
    pub position: i64,
    pub operation: String,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub content_block_id: Option<String>,
    pub content_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDraftRecord {
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub version: i64,
    pub items: Vec<ContextOverrideItemRecord>,
    pub consumed_by_run_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDraftUpdateRecord {
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub expected_version: i64,
    pub content_blocks: Vec<ContentBlockRecord>,
    pub items: Vec<ContextOverrideItemRecord>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRevisionRecord {
    pub branch_pointer_id: String,
    pub workspace_id: String,
    pub revision: i64,
    pub name: String,
    pub head_run_id: String,
    pub change_kind: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCheckpointInheritanceRecord {
    pub workspace_id: String,
    pub branch_pointer_id: String,
    pub checkpoint_id: String,
    pub inherited_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartContextUpdateRecord {
    pub expected_cursor_version: i64,
    pub expected_draft_version: i64,
    pub expected_branch_pointer_id: Option<String>,
    pub expected_branch_version: Option<i64>,
    pub result_branch_pointer_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartOutcomeRecord {
    pub cursor: ContextCursorRecord,
    pub draft_version: i64,
    pub branch_pointer: Option<BranchPointerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMaintenanceRunRecord {
    pub id: String,
    pub workspace_id: String,
    pub kind: String,
    pub status: String,
    pub branch_pointer_id: Option<String>,
    pub branch_revision: Option<i64>,
    pub anchor_run_id: String,
    pub first_kept_run_id: Option<String>,
    pub source_run_ids_json: String,
    pub source_hash: String,
    pub provider_snapshot_json: Option<String>,
    pub request_json: String,
    pub summary_block_id: Option<String>,
    pub summary: Option<String>,
    pub error_json: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCheckpointRecord {
    pub id: String,
    pub workspace_id: String,
    pub maintenance_run_id: String,
    pub kind: String,
    pub branch_pointer_id: Option<String>,
    pub branch_revision: Option<i64>,
    pub anchor_run_id: String,
    pub first_kept_run_id: Option<String>,
    pub summary_block_id: String,
    pub summary: String,
    pub summary_content_hash: String,
    pub source_run_ids_json: String,
    pub source_hash: String,
    pub provider_snapshot_json: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceContextGuardRecord {
    pub workspace_id: String,
    pub expected_cursor_version: i64,
    pub expected_draft_version: Option<i64>,
    pub branch_pointer_id: Option<String>,
    pub expected_branch_version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishContextMaintenanceUpdateRecord {
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceContextRecords {
    pub turns: Vec<TurnRecord>,
    pub runs: Vec<ModelRunRecord>,
    pub content_blocks: Vec<ContentBlockRecord>,
    pub run_provider_provenance: Vec<RunProviderProvenanceRecord>,
    pub cursor: ContextCursorRecord,
    pub branch_pointers: Vec<BranchPointerRecord>,
    pub branch_revisions: Vec<BranchRevisionRecord>,
    pub branch_checkpoint_inheritance: Vec<BranchCheckpointInheritanceRecord>,
    pub draft: ContextDraftRecord,
    pub checkpoints: Vec<ContextCheckpointRecord>,
    pub view_states: Vec<ViewStateRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionMarkRecord {
    pub id: String,
    pub workspace_id: String,
    pub run_id: String,
    pub status: String,
    pub reason: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewStateRecord {
    pub workspace_id: String,
    pub view_key: String,
    pub state_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartBundle {
    /// `Some` for a newly-created Turn/branch and `None` for a retry on an
    /// existing Turn. A retry still inserts a new Run, manifest and snapshot.
    pub turn: Option<TurnRecord>,
    pub run: ModelRunRecord,
    pub content_blocks: Vec<ContentBlockRecord>,
    pub manifest: ContextManifestRecord,
    pub context_items: Vec<RunContextItemRecord>,
    pub snapshot: ContextSnapshotRecord,
    pub branch_pointer: Option<BranchPointerRecord>,
    pub context_update: Option<RunStartContextUpdateRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCheckpoint {
    /// Full accumulated buffers, not deltas. Replaying a checkpoint is safe.
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub usage_json: Option<String>,
    pub checkpointed_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointWriteOutcome {
    Saved,
    SkippedTerminal(RunStatusRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFinish {
    pub status: RunStatusRecord,
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub usage_json: Option<String>,
    pub error_json: Option<String>,
    pub finished_at: i64,
}
