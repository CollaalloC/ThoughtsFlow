use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use super::*;

pub type AppResult<T> = Result<T, AppError>;
pub type AppFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl AppError {
    pub fn validation(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: Value::Null,
        }
    }

    pub fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
            details: Value::Null,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

pub trait RunEventSink: Send + Sync {
    fn send(&self, event: RunEventView) -> AppResult<()>;
}

/// Object-safe application boundary. Infrastructure composes its concrete
/// implementation; the Tauri interface never reaches into SQL or providers.
pub trait ApplicationBackend: Send + Sync + 'static {
    fn list_workspaces(&self, include_archived: bool) -> AppFuture<'_, Vec<WorkspaceSummary>>;
    fn create_workspace(&self, input: CreateWorkspaceInput) -> AppFuture<'_, WorkspaceSummary>;
    fn open_workspace(&self, workspace_id: EntityId) -> AppFuture<'_, WorkspaceDetail>;
    fn update_workspace(&self, input: UpdateWorkspaceInput) -> AppFuture<'_, WorkspaceSummary>;
    fn inspect_context(&self, input: InspectContextInput) -> AppFuture<'_, ContextPreview>;
    fn create_turn_and_start_run(
        &self,
        input: CreateTurnAndStartRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppFuture<'_, RunHandle>;
    fn retry_run(
        &self,
        input: RetryRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppFuture<'_, RunHandle>;
    fn cancel_run(&self, run_id: EntityId) -> AppFuture<'_, ()>;
    fn get_run_snapshot(&self, run_id: EntityId) -> AppFuture<'_, RunSnapshotView>;
    fn update_context_overrides(&self, input: UpdateContextOverridesInput) -> AppFuture<'_, ()>;
    fn get_route_projection(
        &self,
        input: GetRouteProjectionInput,
    ) -> AppFuture<'_, RouteProjection>;
    fn update_view_state(&self, input: UpdateViewStateInput) -> AppFuture<'_, ()>;
    fn compare_runs(&self, input: CompareRunsInput) -> AppFuture<'_, CompareRunsResult>;
    fn mark_decision(&self, input: MarkDecisionInput) -> AppFuture<'_, DecisionMarkView>;
    fn export_decision_packet(
        &self,
        input: ExportDecisionPacketInput,
    ) -> AppFuture<'_, ExportResult>;
    fn list_provider_profiles(&self) -> AppFuture<'_, Vec<ProviderProfileView>>;
    fn list_provider_templates(&self) -> AppFuture<'_, Vec<ProviderTemplateView>>;
    fn save_provider_profile(
        &self,
        input: SaveProviderProfileInput,
    ) -> AppFuture<'_, ProviderProfileView>;
    fn test_provider_connection(
        &self,
        input: TestProviderConnectionInput,
        credential: Option<SessionCredentialValue>,
    ) -> AppFuture<'_, ProviderConnectionResult>;
}

#[derive(Default)]
pub(crate) struct ProviderProfileOperationLocks {
    locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl ProviderProfileOperationLocks {
    pub(crate) fn for_profile(&self, profile_id: &str) -> AppResult<Arc<AsyncMutex<()>>> {
        let mut locks = self.locks.lock().map_err(|_| {
            AppError::internal(
                "provider_profile_lock_unavailable",
                "Provider Profile operation lock is unavailable",
            )
        })?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(profile_id).and_then(Weak::upgrade) {
            return Ok(lock);
        }

        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(profile_id.to_owned(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

pub struct AppState {
    backend: Arc<dyn ApplicationBackend>,
    credentials: Arc<SessionCredentialStore>,
    provider_profile_operations: ProviderProfileOperationLocks,
}

impl AppState {
    pub fn new(backend: Arc<dyn ApplicationBackend>) -> Self {
        Self {
            backend,
            credentials: Arc::new(SessionCredentialStore::default()),
            provider_profile_operations: ProviderProfileOperationLocks::default(),
        }
    }

    pub fn backend(&self) -> &Arc<dyn ApplicationBackend> {
        &self.backend
    }

    pub fn credentials(&self) -> &Arc<SessionCredentialStore> {
        &self.credentials
    }

    pub(crate) fn provider_profile_lock(&self, profile_id: &str) -> AppResult<Arc<AsyncMutex<()>>> {
        self.provider_profile_operations.for_profile(profile_id)
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        let _ = self.credentials.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ProviderProfileOperationLocks;

    #[test]
    fn same_provider_profile_operations_are_serialized() {
        let locks = ProviderProfileOperationLocks::default();
        let first = locks
            .for_profile("profile-1")
            .expect("operation lock is available");
        let second = locks
            .for_profile("profile-1")
            .expect("operation lock is available");
        assert!(Arc::ptr_eq(&first, &second));

        let held = first
            .try_lock()
            .expect("the first operation acquires the lock");
        assert!(
            second.try_lock().is_err(),
            "a second operation for the same Profile must wait"
        );
        drop(held);
        let _next = second
            .try_lock()
            .expect("the next same-Profile operation proceeds after release");
    }

    #[test]
    fn different_provider_profile_operations_can_run_in_parallel() {
        let locks = ProviderProfileOperationLocks::default();
        let first = locks
            .for_profile("profile-1")
            .expect("operation lock is available");
        let second = locks
            .for_profile("profile-2")
            .expect("operation lock is available");
        assert!(!Arc::ptr_eq(&first, &second));

        let _first_guard = first
            .try_lock()
            .expect("the first Profile lock is available");
        let _second_guard = second
            .try_lock()
            .expect("a different Profile lock remains available in parallel");
    }
}
