use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;

use crate::domain::{
    BranchPointer, ContentBlock, ContextSnapshot, ConversationGraph, DecisionMark, ModelRun,
    ProviderProfile, ViewState, Workspace,
};

pub mod provider;

pub use provider::*;

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

    fn get_run(&self, id: &str) -> RepositoryFuture<'_, ModelRun>;

    fn persist_run_start(&self, start: PersistRunStart) -> RepositoryFuture<'_, ()>;

    /// Persists full accumulated buffers, not a token delta.
    fn checkpoint_run(&self, run: &ModelRun) -> RepositoryFuture<'_, ()>;

    fn finish_run(&self, run: &ModelRun) -> RepositoryFuture<'_, ()>;

    fn recover_interrupted_runs(&self, recovered_at: i64) -> RepositoryFuture<'_, u64>;

    fn get_run_snapshot(&self, run_id: &str) -> RepositoryFuture<'_, ContextSnapshot>;

    fn list_provider_profiles(&self) -> RepositoryFuture<'_, Vec<ProviderProfile>>;

    fn save_provider_profile(
        &self,
        profile: ProviderProfile,
    ) -> RepositoryFuture<'_, ProviderProfile>;

    fn save_branch_pointer(&self, pointer: BranchPointer) -> RepositoryFuture<'_, BranchPointer>;

    fn save_decision_mark(&self, mark: DecisionMark) -> RepositoryFuture<'_, DecisionMark>;

    fn save_view_state(&self, state: ViewState) -> RepositoryFuture<'_, ViewState>;
}
