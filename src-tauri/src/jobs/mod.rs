//! Small persistent lifecycle jobs. Provider streaming itself is coordinated by
//! the application service; this module contains restart-safe maintenance only.

use crate::infrastructure::sqlite::{RepositoryResult, SqliteRepository};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupRecoveryReport {
    pub interrupted_runs: u64,
    pub recovered_at: i64,
}

pub struct StartupRecovery;

impl StartupRecovery {
    /// Run once after migrations and before accepting model commands.
    pub async fn run(
        repository: &SqliteRepository,
        recovered_at: i64,
    ) -> RepositoryResult<StartupRecoveryReport> {
        let interrupted_runs = repository.recover_interrupted_runs(recovered_at).await?;
        Ok(StartupRecoveryReport {
            interrupted_runs,
            recovered_at,
        })
    }
}
