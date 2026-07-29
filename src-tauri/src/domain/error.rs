use std::error::Error;
use std::fmt::{Display, Formatter};

use super::RunStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainError {
    InvalidRunTransition {
        from: RunStatus,
        to: RunStatus,
    },
    TerminalRunMutation(RunStatus),
    DuplicateEntity {
        kind: &'static str,
        id: String,
    },
    InvalidContentBlockHash {
        id: String,
    },
    MissingTurnForRun {
        run_id: String,
        turn_id: String,
    },
    MissingParentRun {
        turn_id: String,
        parent_run_id: String,
    },
    CrossWorkspaceParent {
        turn_id: String,
        parent_run_id: String,
    },
    CyclicAncestry {
        turn_id: String,
    },
    MissingRequestedParentRun(String),
    RequestedParentOutsideWorkspace {
        run_id: String,
        workspace_id: String,
    },
    MissingPinnedSource {
        source_id: String,
    },
    CrossWorkspacePinnedSource {
        source_id: String,
        workspace_id: String,
    },
    CheckpointOutsideWorkspace {
        checkpoint_id: String,
        workspace_id: String,
    },
    InvalidCheckpointBoundary {
        checkpoint_id: String,
        first_kept_run_id: String,
    },
    InvalidCheckpointContentIdentity {
        checkpoint_id: String,
    },
    InvalidCheckpointBranchEvidence {
        checkpoint_id: String,
    },
    MissingCheckpointVisibilityEvidence {
        checkpoint_id: String,
    },
    PreviewHashMismatch {
        expected: String,
        actual: String,
    },
    ContextTooLarge {
        estimated_chars: usize,
        max_chars: usize,
    },
    UnresolvedProviderMetadata {
        field: &'static str,
    },
    InvalidPersistedRun {
        run_id: String,
        reason: &'static str,
    },
    ParentRunNotBranchable {
        turn_id: String,
        parent_run_id: String,
        status: RunStatus,
    },
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRunTransition { from, to } => {
                write!(formatter, "run cannot transition from {from:?} to {to:?}")
            }
            Self::TerminalRunMutation(status) => {
                write!(formatter, "{status:?} run is immutable")
            }
            Self::DuplicateEntity { kind, id } => write!(formatter, "duplicate {kind} id: {id}"),
            Self::InvalidContentBlockHash { id } => {
                write!(formatter, "content block {id} has an invalid content hash")
            }
            Self::MissingTurnForRun { run_id, turn_id } => {
                write!(formatter, "run {run_id} references missing turn {turn_id}")
            }
            Self::MissingParentRun {
                turn_id,
                parent_run_id,
            } => write!(
                formatter,
                "turn {turn_id} references missing parent run {parent_run_id}"
            ),
            Self::CrossWorkspaceParent {
                turn_id,
                parent_run_id,
            } => write!(
                formatter,
                "turn {turn_id} cannot inherit run {parent_run_id} from another workspace"
            ),
            Self::CyclicAncestry { turn_id } => {
                write!(formatter, "turn ancestry contains a cycle at {turn_id}")
            }
            Self::MissingRequestedParentRun(run_id) => {
                write!(formatter, "requested parent run does not exist: {run_id}")
            }
            Self::RequestedParentOutsideWorkspace {
                run_id,
                workspace_id,
            } => write!(
                formatter,
                "run {run_id} does not belong to workspace {workspace_id}"
            ),
            Self::MissingPinnedSource { source_id } => {
                write!(formatter, "pinned source does not exist: {source_id}")
            }
            Self::CrossWorkspacePinnedSource {
                source_id,
                workspace_id,
            } => write!(
                formatter,
                "pinned source {source_id} does not belong to workspace {workspace_id}"
            ),
            Self::CheckpointOutsideWorkspace {
                checkpoint_id,
                workspace_id,
            } => write!(
                formatter,
                "checkpoint {checkpoint_id} does not belong to workspace {workspace_id}"
            ),
            Self::InvalidCheckpointBoundary {
                checkpoint_id,
                first_kept_run_id,
            } => write!(
                formatter,
                "checkpoint {checkpoint_id} cannot keep run {first_kept_run_id} on this route"
            ),
            Self::InvalidCheckpointContentIdentity { checkpoint_id } => write!(
                formatter,
                "checkpoint {checkpoint_id} summary content identity is invalid"
            ),
            Self::InvalidCheckpointBranchEvidence { checkpoint_id } => write!(
                formatter,
                "checkpoint {checkpoint_id} has incomplete branch revision evidence"
            ),
            Self::MissingCheckpointVisibilityEvidence { checkpoint_id } => write!(
                formatter,
                "checkpoint {checkpoint_id} requires explicit branch visibility evidence"
            ),
            Self::PreviewHashMismatch { expected, actual } => write!(
                formatter,
                "context changed after inspection (expected {expected}, actual {actual})"
            ),
            Self::ContextTooLarge {
                estimated_chars,
                max_chars,
            } => write!(
                formatter,
                "context size {estimated_chars} exceeds limit {max_chars}"
            ),
            Self::UnresolvedProviderMetadata { field } => {
                write!(formatter, "Provider snapshot is missing resolved {field}")
            }
            Self::InvalidPersistedRun { run_id, reason } => {
                write!(formatter, "persisted run {run_id} is invalid: {reason}")
            }
            Self::ParentRunNotBranchable {
                turn_id,
                parent_run_id,
                status,
            } => write!(
                formatter,
                "turn {turn_id} cannot branch from {status:?} run {parent_run_id}"
            ),
        }
    }
}

impl Error for DomainError {}
