use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use crate::domain::{
    BranchPointer, ContentBlock, ContextSnapshot, ConversationGraph, DecisionMark, ModelRun,
    ProviderProfile, RunFailure, RunStatus, RunUsage, Turn, ViewState, Workspace,
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
    NotFound { entity: &'static str, id: String },
    Conflict(String),
    InvalidData(String),
    Unavailable(String),
}

impl Display for RepositoryPortError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { entity, id } => write!(formatter, "{entity} not found: {id}"),
            Self::Conflict(message) => write!(formatter, "repository conflict: {message}"),
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

    fn persist_run_start(&self, start: PersistRunStart) -> RepositoryFuture<'_, ()>;

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
