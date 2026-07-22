use std::sync::Arc;

use serde::Serialize;
use tauri::{State, ipc::Channel};
use url::Url;

use crate::application::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T> {
    pub api_version: u16,
    pub data: T,
}

impl<T> ApiResponse<T> {
    fn new(data: T) -> Self {
        Self {
            api_version: CONTRACT_VERSION,
            data,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAcknowledgement {
    pub accepted: bool,
}

struct TauriRunEventSink {
    channel: Channel<RunEventView>,
}

impl RunEventSink for TauriRunEventSink {
    fn send(&self, event: RunEventView) -> AppResult<()> {
        self.channel.send(event).map_err(|error| {
            AppError::internal(
                "run_event_delivery_failed",
                "The run event channel closed before delivery",
            )
            .with_details(serde_json::json!({ "cause": error.to_string() }))
        })
    }
}

#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, AppState>,
    include_archived: Option<bool>,
) -> AppResult<ApiResponse<Vec<WorkspaceSummary>>> {
    state
        .backend()
        .list_workspaces(include_archived.unwrap_or(false))
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn create_workspace(
    state: State<'_, AppState>,
    input: CreateWorkspaceInput,
) -> AppResult<ApiResponse<WorkspaceSummary>> {
    require_non_empty("name", &input.name)?;
    require_non_empty("goal", &input.goal)?;
    state
        .backend()
        .create_workspace(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn open_workspace(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ApiResponse<WorkspaceDetail>> {
    require_non_empty("id", &id)?;
    state
        .backend()
        .open_workspace(id)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn update_workspace(
    state: State<'_, AppState>,
    input: UpdateWorkspaceInput,
) -> AppResult<ApiResponse<WorkspaceSummary>> {
    require_non_empty("id", &input.id)?;
    if let Some(name) = &input.name {
        require_non_empty("name", name)?;
    }
    if let Some(goal) = &input.goal {
        require_non_empty("goal", goal)?;
    }
    state
        .backend()
        .update_workspace(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn inspect_context(
    state: State<'_, AppState>,
    input: InspectContextInput,
) -> AppResult<ApiResponse<ContextPreview>> {
    validate_inspect_context(&input)?;
    state
        .backend()
        .inspect_context(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn create_turn_and_start_run(
    state: State<'_, AppState>,
    input: CreateTurnAndStartRunInput,
    on_event: Channel<RunEventView>,
) -> AppResult<ApiResponse<RunHandle>> {
    validate_start_run(&input)?;
    let credentials: Arc<dyn SessionCredentialLookup> = state.credentials().clone();
    let events: Arc<dyn RunEventSink> = Arc::new(TauriRunEventSink { channel: on_event });
    state
        .backend()
        .create_turn_and_start_run(input, credentials, events)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn retry_run(
    state: State<'_, AppState>,
    input: RetryRunInput,
    on_event: Channel<RunEventView>,
) -> AppResult<ApiResponse<RunHandle>> {
    require_non_empty("runId", &input.run_id)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    require_non_empty("previewHash", &input.preview_hash)?;
    let credentials: Arc<dyn SessionCredentialLookup> = state.credentials().clone();
    let events: Arc<dyn RunEventSink> = Arc::new(TauriRunEventSink { channel: on_event });
    state
        .backend()
        .retry_run(input, credentials, events)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn cancel_run(
    state: State<'_, AppState>,
    run_id: String,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    require_non_empty("runId", &run_id)?;
    state.backend().cancel_run(run_id).await?;
    Ok(ApiResponse::new(CommandAcknowledgement { accepted: true }))
}

#[tauri::command]
pub async fn get_run_snapshot(
    state: State<'_, AppState>,
    run_id: String,
) -> AppResult<ApiResponse<RunSnapshotView>> {
    require_non_empty("runId", &run_id)?;
    state
        .backend()
        .get_run_snapshot(run_id)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn update_context_overrides(
    state: State<'_, AppState>,
    input: UpdateContextOverridesInput,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("itemId", &input.item_id)?;
    if input.pinned && !input.included {
        return Err(AppError::validation(
            "invalid_context_override",
            "A pinned Context item must also be included",
        ));
    }
    state.backend().update_context_overrides(input).await?;
    Ok(ApiResponse::new(CommandAcknowledgement { accepted: true }))
}

#[tauri::command]
pub async fn get_route_projection(
    state: State<'_, AppState>,
    input: GetRouteProjectionInput,
) -> AppResult<ApiResponse<RouteProjection>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    state
        .backend()
        .get_route_projection(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn update_view_state(
    state: State<'_, AppState>,
    input: UpdateViewStateInput,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("turnId", &input.turn_id)?;
    state.backend().update_view_state(input).await?;
    Ok(ApiResponse::new(CommandAcknowledgement { accepted: true }))
}

#[tauri::command]
pub async fn compare_runs(
    state: State<'_, AppState>,
    input: CompareRunsInput,
) -> AppResult<ApiResponse<CompareRunsResult>> {
    require_non_empty("leftRunId", &input.left_run_id)?;
    require_non_empty("rightRunId", &input.right_run_id)?;
    if input.left_run_id == input.right_run_id {
        return Err(AppError::validation(
            "identical_comparison_runs",
            "Choose two different model runs to compare",
        ));
    }
    state
        .backend()
        .compare_runs(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn mark_decision(
    state: State<'_, AppState>,
    input: MarkDecisionInput,
) -> AppResult<ApiResponse<DecisionMarkView>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("runId", &input.run_id)?;
    require_non_empty("reason", &input.reason)?;
    state
        .backend()
        .mark_decision(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn export_decision_packet(
    state: State<'_, AppState>,
    input: ExportDecisionPacketInput,
) -> AppResult<ApiResponse<ExportResult>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    state
        .backend()
        .export_decision_packet(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn list_provider_profiles(
    state: State<'_, AppState>,
) -> AppResult<ApiResponse<Vec<ProviderProfileView>>> {
    state
        .backend()
        .list_provider_profiles()
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn save_provider_profile(
    state: State<'_, AppState>,
    input: SaveProviderProfileInput,
) -> AppResult<ApiResponse<ProviderProfileView>> {
    validate_provider_profile(&input)?;
    state
        .backend()
        .save_provider_profile(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn set_session_credential(
    state: State<'_, AppState>,
    input: SetSessionCredentialInput,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let profile_id = input.provider_profile_id;
    state
        .credentials()
        .set(profile_id.clone(), input.credential)?;
    let _present = state.credentials().contains(&profile_id)?;
    Ok(ApiResponse::new(CommandAcknowledgement { accepted: true }))
}

#[tauri::command]
pub async fn test_provider_connection(
    state: State<'_, AppState>,
    input: TestProviderConnectionInput,
) -> AppResult<ApiResponse<ProviderConnectionResult>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let credential = state.credentials().get(&input.provider_profile_id)?;
    state
        .backend()
        .test_provider_connection(input, credential)
        .await
        .map(ApiResponse::new)
}

fn validate_inspect_context(input: &InspectContextInput) -> AppResult<()> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    Ok(())
}

fn validate_start_run(input: &CreateTurnAndStartRunInput) -> AppResult<()> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("prompt", &input.prompt)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    require_non_empty("previewHash", &input.preview_hash)?;
    Ok(())
}

fn validate_provider_profile(input: &SaveProviderProfileInput) -> AppResult<()> {
    require_non_empty("name", &input.name)?;
    require_non_empty("model", &input.model)?;
    validate_provider_url(&input.base_url)
}

fn validate_provider_url(value: &str) -> AppResult<()> {
    let url = Url::parse(value).map_err(|_| {
        AppError::validation(
            "invalid_provider_url",
            "Provider base URL must be an absolute HTTP or HTTPS URL",
        )
    })?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::validation(
            "embedded_provider_credential",
            "Provider credentials must not be embedded in the base URL",
        ));
    }

    let host = url.host_str().unwrap_or_default();
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]");
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        "http" => Err(AppError::validation(
            "insecure_remote_provider",
            "Remote model providers require HTTPS; HTTP is limited to loopback",
        )),
        _ => Err(AppError::validation(
            "unsupported_provider_scheme",
            "Provider base URL must use HTTPS or loopback HTTP",
        )),
    }
}

fn require_non_empty(field: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::validation(
            "missing_required_field",
            format!("{field} must not be empty"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base_url: &str) -> SaveProviderProfileInput {
        SaveProviderProfileInput {
            id: None,
            name: "Local model".into(),
            dialect: ProviderDialectView::Ollama,
            base_url: base_url.into(),
            model: "qwen3".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        }
    }

    #[test]
    fn allows_loopback_http_and_remote_https() {
        assert!(validate_provider_profile(&provider("http://127.0.0.1:11434")).is_ok());
        assert!(validate_provider_profile(&provider("https://models.example.com/v1")).is_ok());
    }

    #[test]
    fn rejects_remote_http_and_embedded_credentials() {
        assert_eq!(
            validate_provider_profile(&provider("http://models.example.com/v1"))
                .expect_err("remote HTTP must be rejected")
                .code,
            "insecure_remote_provider"
        );
        assert_eq!(
            validate_provider_profile(&provider("https://secret@models.example.com/v1"))
                .expect_err("embedded credentials must be rejected")
                .code,
            "embedded_provider_credential"
        );
    }

    #[test]
    fn inspect_allows_an_empty_draft_but_start_does_not() {
        let inspect = InspectContextInput {
            workspace_id: "workspace-1".into(),
            parent_run_id: None,
            prompt: String::new(),
            provider_profile_id: "provider-1".into(),
        };
        assert!(validate_inspect_context(&inspect).is_ok());

        let start = CreateTurnAndStartRunInput {
            workspace_id: inspect.workspace_id,
            parent_run_id: None,
            prompt: String::new(),
            provider_profile_id: inspect.provider_profile_id,
            preview_hash: "hash".into(),
        };
        assert_eq!(
            validate_start_run(&start)
                .expect_err("start requires a prompt")
                .code,
            "missing_required_field"
        );
    }
}
