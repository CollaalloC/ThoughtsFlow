use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use crate::domain::{
    BranchPointer, ContentBlock, ContextCheckpoint, ContextMaintenanceRun, ContextSnapshot,
    ContextSourceRef, ConversationGraph, DecisionMark, ModelRun, ProviderProfile, RunFailure,
    RunStatus, RunUsage, Turn, ViewState, Workspace,
};

pub mod provider;

pub use provider::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPacketExport {
    pub path: String,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPacketWriteError(pub String);

impl Display for DecisionPacketWriteError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DecisionPacketWriteError {}

/// Server-owned export boundary. Callers provide content, never a filesystem path.
pub trait DecisionPacketWriter: Send + Sync {
    fn write(
        &self,
        workspace_id: &str,
        markdown: &str,
    ) -> Result<DecisionPacketExport, DecisionPacketWriteError>;
}

pub type RepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RepositoryPortError>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoryPortError {
    NotFound {
        entity: &'static str,
        id: String,
    },
    Conflict(String),
    VersionConflict {
        resource: &'static str,
        id: String,
        expected: u64,
        actual: u64,
    },
    InvalidData(String),
    Unavailable(String),
}

impl Display for RepositoryPortError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(formatter, "{entity} not found: {id}"),
            Self::Conflict(message) => write!(formatter, "repository conflict: {message}"),
            Self::VersionConflict {
                resource,
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "{resource} version conflict for `{id}`: expected {expected}, actual {actual}"
            ),
            Self::InvalidData(message) => write!(formatter, "invalid repository data: {message}"),
            Self::Unavailable(message) => write!(formatter, "repository unavailable: {message}"),
        }
    }
}

impl Error for RepositoryPortError {}

/// The facts that must commit together before any provider request is sent.
/// A retry sets `turn` to `None` and still inserts a new Run and Snapshot.
#[derive(Clone, Debug)]
pub struct PersistRunStart {
    pub turn: Option<crate::domain::Turn>,
    pub run: ModelRun,
    pub snapshot: ContextSnapshot,
    pub content_blocks: Vec<ContentBlock>,
    pub branch_pointer: Option<BranchPointer>,
    /// When present, cursor advancement, branch CAS, and one-shot draft
    /// consumption commit in the same transaction as the immutable Run receipt.
    pub context_update: Option<RunStartContextUpdate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStartContextUpdate {
    pub expected_cursor_version: u64,
    pub expected_draft_version: u64,
    /// Branch selected when the preview was compiled. For a historical fork
    /// this is the old branch; for a normal append it is also the branch being
    /// advanced.
    pub expected_branch_pointer_id: Option<String>,
    pub expected_branch_version: Option<u64>,
    /// Branch that owns the new Run after commit. A historical fork uses the
    /// newly-created pointer from `PersistRunStart::branch_pointer`.
    pub result_branch_pointer_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistRunStartOutcome {
    pub cursor: ContextCursor,
    pub draft_version: u64,
    pub branch_pointer: Option<BranchPointer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCursor {
    pub workspace_id: String,
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    /// `0` is a deterministic read-only fallback for a workspace that has
    /// never persisted a v5 cursor. The first successful CAS stores version 1.
    pub version: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextCursorUpdate {
    pub workspace_id: String,
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    pub expected_version: u64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextOverrideOperation {
    Pin,
    Exclude,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextOverrideItem {
    pub position: usize,
    pub operation: ContextOverrideOperation,
    pub source_ref: ContextSourceRef,
    pub content_block_id: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextDraft {
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub version: u64,
    pub items: Vec<ContextOverrideItem>,
    pub consumed_by_run_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextDraftUpdate {
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub expected_version: u64,
    /// Exact immutable blocks first materialized by this draft update, such
    /// as a fresh leaf answer that has not appeared in a later Receipt yet.
    pub content_blocks: Vec<ContentBlock>,
    pub items: Vec<ContextOverrideItem>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceContextGuard {
    pub workspace_id: String,
    pub expected_cursor_version: u64,
    pub expected_draft_version: Option<u64>,
    pub branch_pointer_id: Option<String>,
    pub expected_branch_version: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishContextMaintenanceContextUpdate {
    pub active_run_id: Option<String>,
    pub branch_pointer_id: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchRevisionChange {
    MigrationBaseline,
    Created,
    Advanced,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRevision {
    pub branch_pointer_id: String,
    pub workspace_id: String,
    pub revision: u64,
    pub name: String,
    pub head_run_id: String,
    pub change: BranchRevisionChange,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchCheckpointInheritance {
    pub workspace_id: String,
    pub branch_pointer_id: String,
    pub checkpoint_id: String,
    pub inherited_at: i64,
}

#[derive(Clone, Debug)]
pub struct WorkspaceContextData {
    pub workspace_id: String,
    pub graph: ConversationGraph,
    /// Kept alongside the validated graph so tree projection never performs
    /// per-node repository reads.
    pub turns: Vec<Turn>,
    pub runs: Vec<ModelRun>,
    pub run_provider_provenance: Vec<RunProviderProvenance>,
    pub cursor: ContextCursor,
    pub branch_pointers: Vec<BranchPointer>,
    pub branch_revisions: Vec<BranchRevision>,
    /// Explicit, immutable visibility evidence. Callers must not infer
    /// checkpoint inheritance from timestamps or mutable branch heads.
    pub branch_checkpoint_inheritance: Vec<BranchCheckpointInheritance>,
    pub draft: ContextDraft,
    pub checkpoints: Vec<ContextCheckpoint>,
    pub view_states: Vec<ViewState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCheckpoint {
    /// Full accumulated buffers, not deltas. Replaying a checkpoint is safe.
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub usage: Option<RunUsage>,
    pub checkpointed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunFinish {
    pub status: RunStatus,
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub usage: Option<RunUsage>,
    pub error: Option<RunFailure>,
    pub finished_at: i64,
}

/// Immutable provider identity projected from the Context Snapshot captured
/// for a Run. This read model keeps workspace hydration independent from the
/// concrete snapshot storage schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunProviderProvenance {
    pub run_id: String,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMaintenanceStart {
    pub run: ContextMaintenanceRun,
    /// True only for the caller that atomically inserted this operation.
    /// Replays must never repeat external Provider work.
    pub started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointOutcome {
    Saved,
    /// A late checkpoint lost a race with terminal persistence. The immutable
    /// terminal Run remains authoritative and must not become a storage error.
    SkippedTerminal(RunStatus),
}

/// Narrow hot-path persistence boundary used by the provider event loop.
///
/// Keeping this separate from `RepositoryPort` lets the runtime depend on only
/// the three operations it performs and makes storage-race tests inexpensive.
pub trait RunPersistencePort: Send + Sync {
    fn mark_run_streaming(&self, run_id: &str, at: i64) -> RepositoryFuture<'_, ()>;

    fn checkpoint_run(
        &self,
        run_id: &str,
        checkpoint: RunCheckpoint,
    ) -> RepositoryFuture<'_, CheckpointOutcome>;

    fn finish_run(&self, run_id: &str, finish: RunFinish) -> RepositoryFuture<'_, ()>;
}

/// Persistence boundary for the single controlled SQLite writer.
///
/// Implementations must make `persist_run_start` atomic. A successful return
/// is the application service's authorization to begin external I/O.
pub trait RepositoryPort: Send + Sync {
    fn list_workspaces(&self, include_archived: bool) -> RepositoryFuture<'_, Vec<Workspace>>;

    fn get_workspace(&self, id: &str) -> RepositoryFuture<'_, Workspace>;

    fn save_workspace(&self, workspace: Workspace) -> RepositoryFuture<'_, Workspace>;

    fn load_conversation_graph(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, ConversationGraph>;

    fn get_turn(&self, id: &str) -> RepositoryFuture<'_, Turn>;

    fn list_turns(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<Turn>>;

    fn get_run(&self, id: &str) -> RepositoryFuture<'_, ModelRun>;

    fn list_runs_for_turn(&self, turn_id: &str) -> RepositoryFuture<'_, Vec<ModelRun>>;

    fn list_run_provider_provenance(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<RunProviderProvenance>>;

    fn mark_run_connecting(&self, run_id: &str, at: i64) -> RepositoryFuture<'_, ()>;

    fn persist_run_start(
        &self,
        start: PersistRunStart,
    ) -> RepositoryFuture<'_, PersistRunStartOutcome>;

    fn recover_interrupted_runs(&self, recovered_at: i64) -> RepositoryFuture<'_, u64>;

    fn get_run_snapshot(&self, run_id: &str) -> RepositoryFuture<'_, ContextSnapshot>;

    fn list_provider_profiles(&self) -> RepositoryFuture<'_, Vec<ProviderProfile>>;

    fn get_provider_profile(&self, id: &str) -> RepositoryFuture<'_, ProviderProfile>;

    fn save_provider_profile(
        &self,
        profile: ProviderProfile,
    ) -> RepositoryFuture<'_, ProviderProfile>;

    fn save_branch_pointer(&self, pointer: BranchPointer) -> RepositoryFuture<'_, BranchPointer>;

    fn list_branch_pointers(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<BranchPointer>>;

    fn rename_branch(
        &self,
        branch_pointer_id: &str,
        name: &str,
        expected_version: u64,
        updated_at: i64,
    ) -> RepositoryFuture<'_, BranchPointer>;

    fn get_context_cursor(&self, workspace_id: &str) -> RepositoryFuture<'_, ContextCursor>;

    fn set_context_cursor(
        &self,
        update: ContextCursorUpdate,
    ) -> RepositoryFuture<'_, ContextCursor>;

    fn set_context_cursor_and_rebase_draft(
        &self,
        update: ContextCursorUpdate,
        expected_draft_version: u64,
    ) -> RepositoryFuture<'_, ContextCursor>;

    fn get_context_draft(&self, workspace_id: &str) -> RepositoryFuture<'_, ContextDraft>;

    fn update_context_draft(
        &self,
        update: ContextDraftUpdate,
    ) -> RepositoryFuture<'_, ContextDraft>;

    fn load_workspace_context_data(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, WorkspaceContextData>;

    fn start_context_maintenance(
        &self,
        run: ContextMaintenanceRun,
        guard: MaintenanceContextGuard,
    ) -> RepositoryFuture<'_, ContextMaintenanceStart>;

    fn get_context_maintenance_run(
        &self,
        maintenance_run_id: &str,
    ) -> RepositoryFuture<'_, ContextMaintenanceRun>;

    fn get_context_checkpoint_for_maintenance(
        &self,
        maintenance_run_id: &str,
    ) -> RepositoryFuture<'_, Option<ContextCheckpoint>>;

    fn recover_interrupted_context_maintenance(
        &self,
        recovered_at: i64,
    ) -> RepositoryFuture<'_, u64>;

    /// Terminal maintenance state and the optional immutable checkpoint commit
    /// atomically. Completed runs require both checkpoint and summary block;
    /// failed/cancelled/conflicted runs require neither.
    fn finish_context_maintenance(
        &self,
        run: ContextMaintenanceRun,
        checkpoint: Option<ContextCheckpoint>,
        summary_block: Option<ContentBlock>,
        guard: MaintenanceContextGuard,
        context_update: Option<FinishContextMaintenanceContextUpdate>,
    ) -> RepositoryFuture<'_, ContextMaintenanceRun>;

    fn list_context_maintenance_runs(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<ContextMaintenanceRun>>;

    fn list_context_checkpoints(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<ContextCheckpoint>>;

    fn save_decision_mark(&self, mark: DecisionMark) -> RepositoryFuture<'_, DecisionMark>;

    fn get_decision_mark(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> RepositoryFuture<'_, DecisionMark>;

    fn list_decision_marks(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<DecisionMark>>;

    fn save_view_state(&self, state: ViewState) -> RepositoryFuture<'_, ViewState>;

    fn get_view_state(&self, workspace_id: &str, view_key: &str)
    -> RepositoryFuture<'_, ViewState>;
}
