use super::DomainError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Queued,
    Connecting,
    Streaming,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Streaming => "streaming",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "connecting" => Some(Self::Connecting),
            "streaming" => Some(Self::Streaming),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDraft {
    pub id: String,
    pub turn_id: String,
    pub provider_profile_id: Option<String>,
    pub model: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub status: Option<u16>,
}

impl RunFailure {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            code: "run_failed".into(),
            message: message.into(),
            retryable: false,
            status: None,
        }
    }
}

impl RunUsage {
    pub const fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRun {
    pub id: String,
    pub turn_id: String,
    pub provider_profile_id: Option<String>,
    pub model: String,
    pub created_at: i64,
    status: RunStatus,
    output_markdown: String,
    reasoning_markdown: String,
    error: Option<RunFailure>,
    usage: Option<RunUsage>,
    started_at: Option<i64>,
    checkpointed_at: Option<i64>,
    finished_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStateSnapshot {
    pub status: RunStatus,
    pub output_markdown: String,
    pub reasoning_markdown: String,
    pub error: Option<RunFailure>,
    pub usage: Option<RunUsage>,
    pub started_at: Option<i64>,
    pub checkpointed_at: Option<i64>,
    pub finished_at: Option<i64>,
}

impl ModelRun {
    pub fn queued(draft: RunDraft) -> Self {
        Self {
            id: draft.id,
            turn_id: draft.turn_id,
            provider_profile_id: draft.provider_profile_id,
            model: draft.model,
            created_at: draft.created_at,
            status: RunStatus::Queued,
            output_markdown: String::new(),
            reasoning_markdown: String::new(),
            error: None,
            usage: None,
            started_at: None,
            checkpointed_at: None,
            finished_at: None,
        }
    }

    pub fn rehydrate(draft: RunDraft, state: RunStateSnapshot) -> Result<Self, DomainError> {
        if state.status.is_terminal() && state.finished_at.is_none() {
            return Err(DomainError::InvalidPersistedRun {
                run_id: draft.id,
                reason: "terminal run has no finished_at timestamp",
            });
        }
        if !state.status.is_terminal() && state.finished_at.is_some() {
            return Err(DomainError::InvalidPersistedRun {
                run_id: draft.id,
                reason: "non-terminal run has a finished_at timestamp",
            });
        }
        if matches!(state.status, RunStatus::Connecting | RunStatus::Streaming)
            && state.started_at.is_none()
        {
            return Err(DomainError::InvalidPersistedRun {
                run_id: draft.id,
                reason: "active run has no started_at timestamp",
            });
        }
        Ok(Self {
            id: draft.id,
            turn_id: draft.turn_id,
            provider_profile_id: draft.provider_profile_id,
            model: draft.model,
            created_at: draft.created_at,
            status: state.status,
            output_markdown: state.output_markdown,
            reasoning_markdown: state.reasoning_markdown,
            error: state.error,
            usage: state.usage,
            started_at: state.started_at,
            checkpointed_at: state.checkpointed_at,
            finished_at: state.finished_at,
        })
    }

    pub fn state_snapshot(&self) -> RunStateSnapshot {
        RunStateSnapshot {
            status: self.status,
            output_markdown: self.output_markdown.clone(),
            reasoning_markdown: self.reasoning_markdown.clone(),
            error: self.error.clone(),
            usage: self.usage,
            started_at: self.started_at,
            checkpointed_at: self.checkpointed_at,
            finished_at: self.finished_at,
        }
    }

    pub fn status(&self) -> RunStatus {
        self.status
    }

    pub fn output_markdown(&self) -> &str {
        &self.output_markdown
    }

    pub fn reasoning_markdown(&self) -> &str {
        &self.reasoning_markdown
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|failure| failure.message.as_str())
    }

    pub fn failure(&self) -> Option<&RunFailure> {
        self.error.as_ref()
    }

    pub fn usage(&self) -> Option<RunUsage> {
        self.usage
    }

    pub fn started_at(&self) -> Option<i64> {
        self.started_at
    }

    pub fn checkpointed_at(&self) -> Option<i64> {
        self.checkpointed_at
    }

    pub fn finished_at(&self) -> Option<i64> {
        self.finished_at
    }

    pub fn connect(&mut self, at: i64) -> Result<(), DomainError> {
        self.transition(RunStatus::Queued, RunStatus::Connecting)?;
        self.started_at = Some(at);
        Ok(())
    }

    pub fn begin_streaming(&mut self, _at: i64) -> Result<(), DomainError> {
        self.transition(RunStatus::Connecting, RunStatus::Streaming)?;
        Ok(())
    }

    pub fn checkpoint(
        &mut self,
        output_delta: &str,
        reasoning_delta: &str,
        at: i64,
    ) -> Result<(), DomainError> {
        self.ensure_streaming()?;
        self.output_markdown.push_str(output_delta);
        self.reasoning_markdown.push_str(reasoning_delta);
        self.checkpointed_at = Some(at);
        Ok(())
    }

    pub fn complete(&mut self, usage: Option<RunUsage>, at: i64) -> Result<(), DomainError> {
        self.finish(RunStatus::Completed, None, usage, at)
    }

    pub fn fail(&mut self, error: impl Into<String>, at: i64) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalRunMutation(self.status));
        }
        self.status = RunStatus::Failed;
        self.error = Some(RunFailure::message(error));
        self.finished_at = Some(at);
        Ok(())
    }

    pub fn cancel(&mut self, at: i64) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalRunMutation(self.status));
        }
        self.status = RunStatus::Cancelled;
        self.finished_at = Some(at);
        Ok(())
    }

    pub fn interrupt(&mut self, at: i64) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalRunMutation(self.status));
        }
        self.status = RunStatus::Interrupted;
        self.finished_at = Some(at);
        Ok(())
    }

    fn transition(&mut self, from: RunStatus, to: RunStatus) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalRunMutation(self.status));
        }
        if self.status != from {
            return Err(DomainError::InvalidRunTransition {
                from: self.status,
                to,
            });
        }
        self.status = to;
        Ok(())
    }

    fn ensure_streaming(&self) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalRunMutation(self.status));
        }
        if self.status != RunStatus::Streaming {
            return Err(DomainError::InvalidRunTransition {
                from: self.status,
                to: RunStatus::Streaming,
            });
        }
        Ok(())
    }

    fn finish(
        &mut self,
        to: RunStatus,
        error: Option<RunFailure>,
        usage: Option<RunUsage>,
        at: i64,
    ) -> Result<(), DomainError> {
        self.ensure_streaming()?;
        self.status = to;
        self.error = error;
        self.usage = usage;
        self.finished_at = Some(at);
        Ok(())
    }
}
