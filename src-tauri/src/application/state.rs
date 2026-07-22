use std::{future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

pub struct AppState {
    backend: Arc<dyn ApplicationBackend>,
    credentials: Arc<SessionCredentialStore>,
}

impl AppState {
    pub fn new(backend: Arc<dyn ApplicationBackend>) -> Self {
        Self {
            backend,
            credentials: Arc::new(SessionCredentialStore::default()),
        }
    }

    pub fn backend(&self) -> &Arc<dyn ApplicationBackend> {
        &self.backend
    }

    pub fn credentials(&self) -> &Arc<SessionCredentialStore> {
        &self.credentials
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        let _ = self.credentials.clear();
    }
}
