use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{State, ipc::Channel};
use url::Url;
use uuid::Uuid;

use crate::application::*;
use crate::ports::provider::SessionCredential;

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
pub async fn preview_context_transition(
    state: State<'_, AppState>,
    input: PreviewContextTransitionInput,
) -> AppResult<ApiResponse<ContextPreview>> {
    validate_preview_context_transition(&input)?;
    state
        .backend()
        .preview_context_transition(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn get_context_tree(
    state: State<'_, AppState>,
    input: GetContextTreeInput,
) -> AppResult<ApiResponse<ContextTreeProjection>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    state
        .backend()
        .get_context_tree(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn set_active_context(
    state: State<'_, AppState>,
    input: SetActiveContextInput,
) -> AppResult<ApiResponse<ContextCursorView>> {
    validate_set_active_context(&input)?;
    state
        .backend()
        .set_active_context(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn rename_branch(
    state: State<'_, AppState>,
    input: RenameBranchInput,
) -> AppResult<ApiResponse<ContextBranchView>> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("branchId", &input.branch_id)?;
    require_non_empty("name", &input.name)?;
    state
        .backend()
        .rename_branch(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn update_context_draft(
    state: State<'_, AppState>,
    input: UpdateContextDraftInput,
) -> AppResult<ApiResponse<UpdateContextDraftResult>> {
    validate_context_draft(&input)?;
    state
        .backend()
        .update_context_draft(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn create_context_checkpoint(
    state: State<'_, AppState>,
    input: CreateContextCheckpointInput,
) -> AppResult<ApiResponse<ContextCheckpointView>> {
    validate_context_checkpoint(&input)?;
    state
        .backend()
        .create_context_checkpoint(input)
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn summarize_and_set_active_context(
    state: State<'_, AppState>,
    input: SummarizeAndSetActiveContextInput,
) -> AppResult<ApiResponse<SummarizeAndSetActiveContextResult>> {
    validate_summarize_context(&input)?;
    #[cfg(feature = "webview-e2e")]
    let client_operation_id = input.client_operation_id.clone();
    #[cfg(feature = "webview-e2e")]
    capture_context_maintenance_input_if_requested(&input)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    let credentials: Arc<dyn SessionCredentialLookup> = state.credentials().clone();
    let result = state
        .backend()
        .summarize_and_set_active_context(input, credentials)
        .await?;
    #[cfg(feature = "webview-e2e")]
    abort_after_context_maintenance_commit_if_requested(&client_operation_id);
    Ok(ApiResponse::new(result))
}

#[cfg(feature = "webview-e2e")]
const CRASH_AFTER_MAINTENANCE_COMMIT_ENV: &str =
    "THOUGHSFLOW_WEBVIEW_E2E_CRASH_AFTER_MAINTENANCE_COMMIT";

#[cfg(feature = "webview-e2e")]
fn should_abort_after_context_maintenance_commit(
    requested_client_operation_id: Option<&str>,
    completed_client_operation_id: &str,
) -> bool {
    requested_client_operation_id == Some(completed_client_operation_id)
}

#[cfg(feature = "webview-e2e")]
fn abort_after_context_maintenance_commit_if_requested(client_operation_id: &str) {
    let requested_client_operation_id = std::env::var(CRASH_AFTER_MAINTENANCE_COMMIT_ENV).ok();
    if should_abort_after_context_maintenance_commit(
        requested_client_operation_id.as_deref(),
        client_operation_id,
    ) {
        std::process::abort();
    }
}

#[cfg(feature = "webview-e2e")]
fn capture_context_maintenance_input_if_requested(
    input: &SummarizeAndSetActiveContextInput,
) -> AppResult<()> {
    let requested_client_operation_id = std::env::var(CRASH_AFTER_MAINTENANCE_COMMIT_ENV).ok();
    if !should_abort_after_context_maintenance_commit(
        requested_client_operation_id.as_deref(),
        &input.client_operation_id,
    ) {
        return Ok(());
    }
    let Some(data_dir) = std::env::var_os("THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR") else {
        return Err(AppError::internal(
            "webview_e2e_data_dir",
            "The WebView E2E data directory is required for maintenance capture",
        ));
    };
    let data_dir = std::path::PathBuf::from(data_dir);
    if !data_dir.is_absolute() {
        return Err(AppError::internal(
            "webview_e2e_data_dir",
            "The WebView E2E data directory must be absolute",
        ));
    }
    let capture_path = data_dir.join("maintenance-replay-input.json");
    let serialized = serde_json::to_vec(input).map_err(|error| {
        AppError::internal(
            "webview_e2e_capture_serialize",
            "The WebView E2E maintenance input could not be serialized",
        )
        .with_details(serde_json::json!({ "cause": error.to_string() }))
    })?;
    std::fs::write(&capture_path, serialized).map_err(|error| {
        AppError::internal(
            "webview_e2e_capture_write",
            "The WebView E2E maintenance input could not be captured",
        )
        .with_details(serde_json::json!({
            "path": capture_path,
            "cause": error.to_string(),
        }))
    })
}

#[tauri::command]
pub async fn cancel_context_maintenance(
    state: State<'_, AppState>,
    client_operation_id: String,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    validate_client_operation_id(&client_operation_id)?;
    state
        .backend()
        .cancel_context_maintenance(client_operation_id)
        .await?;
    Ok(ApiResponse::new(CommandAcknowledgement { accepted: true }))
}

#[tauri::command]
pub async fn create_turn_and_start_run(
    state: State<'_, AppState>,
    input: CreateTurnAndStartRunInput,
    on_event: Channel<RunEventView>,
) -> AppResult<ApiResponse<RunHandle>> {
    validate_start_run(&input)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
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
    input: RetryRunCommandInput,
    on_event: Channel<RunEventView>,
) -> AppResult<ApiResponse<RunHandle>> {
    let RetryRunCommandInput {
        run_id,
        provider_profile_id,
        preview_hash,
        credential_id,
        branch_id,
        expected_cursor_version,
        expected_branch_version,
        expected_draft_version,
    } = input;
    let input = RetryRunInput {
        run_id,
        provider_profile_id,
        preview_hash,
        branch_id,
        expected_cursor_version,
        expected_branch_version,
        expected_draft_version,
    };
    require_non_empty("runId", &input.run_id)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    require_non_empty("previewHash", &input.preview_hash)?;
    if let Some(credential_id) = credential_id.as_deref() {
        require_non_empty("credentialId", credential_id)?;
    }
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    let provider_profile_id = input.provider_profile_id.clone();
    let backend = state.backend().clone();
    let events: Arc<dyn RunEventSink> = Arc::new(TauriRunEventSink { channel: on_event });
    retry_run_with_selected_credential(
        state.credentials().clone(),
        &provider_profile_id,
        credential_id.as_deref(),
        move |credentials| async move { backend.retry_run(input, credentials, events).await },
    )
    .await
    .map(ApiResponse::new)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryRunCommandInput {
    run_id: EntityId,
    provider_profile_id: EntityId,
    preview_hash: String,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    branch_id: Option<EntityId>,
    expected_cursor_version: u64,
    #[serde(default)]
    expected_branch_version: Option<u64>,
    expected_draft_version: u64,
}

async fn retry_run_with_selected_credential<Start, StartFuture>(
    credentials: Arc<SessionCredentialStore>,
    provider_profile_id: &str,
    credential_id: Option<&str>,
    start: Start,
) -> AppResult<RunHandle>
where
    Start: FnOnce(Arc<dyn SessionCredentialLookup>) -> StartFuture,
    StartFuture: std::future::Future<Output = AppResult<RunHandle>>,
{
    let lookup: Arc<dyn SessionCredentialLookup> = match credential_id {
        Some(credential_id) => credentials.selected_lookup(provider_profile_id, credential_id)?,
        None => credentials.clone(),
    };
    let handle = start(lookup).await?;
    if let Some(credential_id) = credential_id {
        credentials.activate(provider_profile_id, credential_id)?;
    }
    Ok(handle)
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
pub async fn list_provider_templates(
    state: State<'_, AppState>,
) -> AppResult<ApiResponse<Vec<ProviderTemplateView>>> {
    state
        .backend()
        .list_provider_templates()
        .await
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn list_provider_models(
    state: State<'_, AppState>,
    input: ListProviderModelsCommandInput,
) -> AppResult<ApiResponse<Vec<ModelInfoView>>> {
    match validate_model_discovery_command(input)? {
        ProviderModelCommandSource::SavedProfile(provider_profile_id) => {
            let provider_profile_lock = state.provider_profile_lock(&provider_profile_id)?;
            let _provider_profile_operation = provider_profile_lock.lock().await;
            let credential = state
                .credentials()
                .get(&provider_profile_id)?
                .map(|value| {
                    value
                        .as_str()
                        .map(|secret| SessionCredential::new(secret.to_owned()))
                })
                .transpose()?;
            state
                .backend()
                .list_provider_models(
                    ListProviderModelsInput {
                        provider_profile_id: Some(provider_profile_id),
                        draft: None,
                    },
                    credential,
                )
                .await
                .map(ApiResponse::new)
        }
        ProviderModelCommandSource::Draft { input, credential } => state
            .backend()
            .list_provider_models(input, credential.map(SessionCredential::new))
            .await
            .map(ApiResponse::new),
    }
}

/// Command-only model discovery envelope. It deliberately does not implement
/// `Debug`: a draft credential is process-memory-only and must not be emitted
/// by generic command diagnostics.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListProviderModelsCommandInput {
    #[serde(default)]
    provider_profile_id: Option<String>,
    #[serde(default)]
    draft: Option<ProviderModelDraftCommandInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderModelDraftCommandInput {
    provider_id: String,
    base_url: String,
    #[serde(default)]
    session_credential: Option<String>,
}

enum ProviderModelCommandSource {
    SavedProfile(String),
    Draft {
        input: ListProviderModelsInput,
        credential: Option<String>,
    },
}

fn validate_model_discovery_command(
    input: ListProviderModelsCommandInput,
) -> AppResult<ProviderModelCommandSource> {
    match (input.provider_profile_id, input.draft) {
        (Some(provider_profile_id), None) => {
            require_non_empty("providerProfileId", &provider_profile_id)?;
            Ok(ProviderModelCommandSource::SavedProfile(
                provider_profile_id,
            ))
        }
        (None, Some(draft)) => {
            require_non_empty("draft.providerId", &draft.provider_id)?;
            require_non_empty("draft.baseUrl", &draft.base_url)?;
            validate_provider_url(&draft.base_url)?;
            Ok(ProviderModelCommandSource::Draft {
                input: ListProviderModelsInput {
                    provider_profile_id: None,
                    draft: Some(ProviderModelDraftInput {
                        provider_id: draft.provider_id,
                        base_url: draft.base_url,
                    }),
                },
                credential: draft.session_credential.filter(|value| !value.is_empty()),
            })
        }
        _ => Err(AppError::validation(
            "invalid_provider_model_source",
            "Choose exactly one Provider Profile or draft Provider target",
        )),
    }
}

#[tauri::command]
pub async fn save_provider_profile(
    state: State<'_, AppState>,
    input: SaveProviderProfileCommandInput,
) -> AppResult<ApiResponse<ProviderProfileView>> {
    let SaveProviderProfileCommandInput {
        mut profile,
        session_credential,
        session_credential_label,
    } = input;
    validate_provider_profile(&profile)?;
    let session_credential =
        validate_initial_session_credential(session_credential_label, session_credential)?;
    let profile_id = profile
        .id
        .get_or_insert_with(|| Uuid::new_v4().to_string())
        .clone();
    let provider_profile_lock = state.provider_profile_lock(&profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    let existing_profile = state
        .backend()
        .list_provider_profiles()
        .await?
        .into_iter()
        .find(|candidate| candidate.id == profile_id);
    let clear_existing_credentials = existing_profile
        .as_ref()
        .is_some_and(|existing| provider_credential_target_changed(existing, &profile));
    let backend = state.backend().clone();
    save_provider_profile_securely(
        state.credentials(),
        profile,
        session_credential,
        clear_existing_credentials,
        move |input| async move { backend.save_provider_profile(input).await },
    )
    .await
    .map(ApiResponse::new)
}

/// Command-only envelope. It deliberately does not implement `Debug` so a
/// session credential cannot be exposed by generic command diagnostics. The
/// flattened profile fields preserve the existing IPC contract while keeping
/// the credential out of the persistent profile DTO.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderProfileCommandInput {
    #[serde(flatten)]
    profile: SaveProviderProfileInput,
    #[serde(default)]
    session_credential: Option<String>,
    #[serde(default)]
    session_credential_label: Option<String>,
}

async fn save_provider_profile_securely<Save, SaveFuture>(
    credentials: &SessionCredentialStore,
    input: SaveProviderProfileInput,
    session_credential: Option<(String, String)>,
    clear_existing_credentials: bool,
    save: Save,
) -> AppResult<ProviderProfileView>
where
    Save: FnOnce(SaveProviderProfileInput) -> SaveFuture,
    SaveFuture: std::future::Future<Output = AppResult<ProviderProfileView>>,
{
    let profile = save(input).await?;
    if clear_existing_credentials || session_credential.is_some() {
        credentials.remove(&profile.id)?;
    }
    if let Some((label, credential)) = session_credential {
        credentials.upsert(profile.id.clone(), None, label, credential)?;
    }
    Ok(profile)
}

fn validate_initial_session_credential(
    label: Option<String>,
    credential: Option<String>,
) -> AppResult<Option<(String, String)>> {
    match (label, credential) {
        (None, None) => Ok(None),
        (label, Some(credential)) if !credential.is_empty() => Ok(Some((
            validate_session_credential_label(label.unwrap_or_else(|| "Default".into()))?,
            credential,
        ))),
        (Some(_), None) | (_, Some(_)) => Err(AppError::validation(
            "invalid_session_credential",
            "A non-empty credential and its label must be supplied together",
        )),
    }
}

fn provider_credential_target_changed(
    existing: &ProviderProfileView,
    update: &SaveProviderProfileInput,
) -> bool {
    existing.provider_id != update.provider_id
        || existing.base_url.trim_end_matches('/') != update.base_url.trim_end_matches('/')
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCredentialProfileInput {
    provider_profile_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSessionCredentialCommandInput {
    provider_profile_id: String,
    #[serde(default)]
    credential_id: Option<String>,
    credential_label: String,
    credential: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateSessionCredentialCommandInput {
    provider_profile_id: String,
    credential_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReorderSessionCredentialsCommandInput {
    provider_profile_id: String,
    ordered_credential_ids: Vec<String>,
}

#[tauri::command]
pub async fn list_session_credentials(
    state: State<'_, AppState>,
    input: ProviderCredentialProfileInput,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    require_provider_profile_exists(&state, &input.provider_profile_id).await?;
    state
        .credentials()
        .summaries(&input.provider_profile_id)
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn set_session_credential(
    state: State<'_, AppState>,
    input: SetSessionCredentialCommandInput,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    require_provider_profile_exists(&state, &input.provider_profile_id).await?;
    let profile_id = input.provider_profile_id;
    state.credentials().upsert(
        profile_id.clone(),
        input.credential_id,
        input.credential_label,
        input.credential,
    )?;
    state
        .credentials()
        .summaries(&profile_id)
        .map(ApiResponse::new)
}

#[tauri::command]
pub async fn activate_session_credential(
    state: State<'_, AppState>,
    input: ActivateSessionCredentialCommandInput,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>> {
    mutate_session_credentials(state, input.provider_profile_id, |store, profile_id| {
        store.activate(profile_id, &input.credential_id)
    })
    .await
}

#[tauri::command]
pub async fn reorder_session_credentials(
    state: State<'_, AppState>,
    input: ReorderSessionCredentialsCommandInput,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>> {
    mutate_session_credentials(state, input.provider_profile_id, |store, profile_id| {
        store.reorder(profile_id, &input.ordered_credential_ids)
    })
    .await
}

#[tauri::command]
pub async fn remove_session_credential(
    state: State<'_, AppState>,
    input: ActivateSessionCredentialCommandInput,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>> {
    mutate_session_credentials(state, input.provider_profile_id, |store, profile_id| {
        store.remove_one(profile_id, &input.credential_id)
    })
    .await
}

async fn mutate_session_credentials<Mutation>(
    state: State<'_, AppState>,
    provider_profile_id: String,
    mutation: Mutation,
) -> AppResult<ApiResponse<Vec<SessionCredentialSummary>>>
where
    Mutation: FnOnce(&SessionCredentialStore, &str) -> AppResult<()>,
{
    require_non_empty("providerProfileId", &provider_profile_id)?;
    let provider_profile_lock = state.provider_profile_lock(&provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    require_provider_profile_exists(&state, &provider_profile_id).await?;
    mutation(state.credentials(), &provider_profile_id)?;
    state
        .credentials()
        .summaries(&provider_profile_id)
        .map(ApiResponse::new)
}

async fn require_provider_profile_exists(
    state: &State<'_, AppState>,
    provider_profile_id: &str,
) -> AppResult<()> {
    if state
        .backend()
        .list_provider_profiles()
        .await?
        .iter()
        .any(|profile| profile.id == provider_profile_id)
    {
        Ok(())
    } else {
        Err(AppError::validation(
            "provider_profile_not_found",
            "Provider Profile was not found",
        ))
    }
}

#[tauri::command]
pub async fn test_provider_connection(
    state: State<'_, AppState>,
    input: TestProviderConnectionInput,
) -> AppResult<ApiResponse<ProviderConnectionResult>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
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
    if let Some(branch_id) = input.branch_id.as_deref() {
        require_non_empty("branchId", branch_id)?;
    }
    Ok(())
}

fn validate_preview_context_transition(input: &PreviewContextTransitionInput) -> AppResult<()> {
    validate_inspect_context(&InspectContextInput {
        workspace_id: input.workspace_id.clone(),
        parent_run_id: input.parent_run_id.clone(),
        prompt: input.prompt.clone(),
        provider_profile_id: input.provider_profile_id.clone(),
        branch_id: input.branch_id.clone(),
    })
}

fn validate_set_active_context(input: &SetActiveContextInput) -> AppResult<()> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    if let Some(run_id) = input.run_id.as_deref() {
        require_non_empty("runId", run_id)?;
    }
    if let Some(branch_id) = input.branch_id.as_deref() {
        require_non_empty("branchId", branch_id)?;
    }
    Ok(())
}

fn validate_context_draft(input: &UpdateContextDraftInput) -> AppResult<()> {
    require_non_empty("workspaceId", &input.workspace_id)?;
    let mut identities = std::collections::BTreeSet::new();
    for item in &input.items {
        if let Some(id) = item.source_ref.id.as_deref() {
            require_non_empty("sourceRef.id", id)?;
        }
        let identity = format!("{:?}:{:?}", item.source_ref.kind, item.source_ref.id);
        if !identities.insert(identity) {
            return Err(AppError::validation(
                "duplicate_context_draft_item",
                "A Context Draft can override each typed source only once",
            ));
        }
        if item.pinned && !item.included {
            return Err(AppError::validation(
                "invalid_context_draft_item",
                "A pinned Context item must also be included",
            ));
        }
        if item.pinned
            && item
                .content_block_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
        {
            return Err(AppError::validation(
                "missing_content_block_identity",
                "A pinned Context item must identify its exact Content Block",
            ));
        }
        if matches!(
            item.source_ref.kind,
            ContextSourceKindView::WorkspaceSystem | ContextSourceKindView::CurrentPrompt
        ) && (!item.included || item.pinned)
        {
            return Err(AppError::validation(
                "mandatory_context_item",
                "System and current-prompt Context items are mandatory and cannot be pinned",
            ));
        }
    }
    Ok(())
}

fn validate_context_checkpoint(input: &CreateContextCheckpointInput) -> AppResult<()> {
    validate_client_operation_id(&input.client_operation_id)?;
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("branchId", &input.branch_id)?;
    require_non_empty("summary", &input.summary)?;
    validate_checkpoint_range(
        input.kind,
        &input.source_run_ids,
        input.first_kept_run_id.as_deref(),
    )
}

fn validate_summarize_context(input: &SummarizeAndSetActiveContextInput) -> AppResult<()> {
    validate_client_operation_id(&input.client_operation_id)?;
    require_non_empty("workspaceId", &input.workspace_id)?;
    require_non_empty("targetRunId", &input.target_run_id)?;
    require_non_empty("branchId", &input.branch_id)?;
    require_non_empty("summaryPrompt", &input.summary_prompt)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    validate_checkpoint_range(
        ContextCheckpointKindView::Compaction,
        &input.source_run_ids,
        input.first_kept_run_id.as_deref(),
    )
}

fn validate_checkpoint_range(
    kind: ContextCheckpointKindView,
    source_run_ids: &[EntityId],
    first_kept_run_id: Option<&str>,
) -> AppResult<()> {
    if source_run_ids.is_empty() {
        return Err(AppError::validation(
            "empty_checkpoint_source",
            "A Context checkpoint must name at least one source Run",
        ));
    }
    let mut unique = std::collections::BTreeSet::new();
    for run_id in source_run_ids {
        require_non_empty("sourceRunIds", run_id)?;
        if !unique.insert(run_id) {
            return Err(AppError::validation(
                "duplicate_checkpoint_source",
                "A Context checkpoint source range cannot contain duplicate Runs",
            ));
        }
    }
    if kind == ContextCheckpointKindView::Compaction
        && first_kept_run_id.is_none_or(|id| id.trim().is_empty())
    {
        return Err(AppError::validation(
            "missing_checkpoint_boundary",
            "A compaction checkpoint must identify the first kept Run",
        ));
    }
    if kind == ContextCheckpointKindView::BranchSummary
        && let Some(first_kept_run_id) = first_kept_run_id
    {
        return Err(AppError::validation(
            "unexpected_checkpoint_boundary",
            "A branch-summary checkpoint cannot carry a compaction kept boundary",
        )
        .with_details(serde_json::json!({
            "kind": "branch-summary",
            "firstKeptRunId": first_kept_run_id,
        })));
    }
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
    if let Some(profile_id) = input.id.as_deref() {
        require_non_empty("id", profile_id)?;
    }
    require_non_empty("providerId", &input.provider_id)?;
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
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::validation(
            "invalid_provider_url",
            "Provider base URL must not contain a query string or fragment",
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

fn validate_client_operation_id(value: &str) -> AppResult<()> {
    require_non_empty("clientOperationId", value)?;
    Uuid::parse_str(value).map_err(|_| {
        AppError::validation(
            "invalid_client_operation_id",
            "clientOperationId must be a UUID generated once per maintenance operation",
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base_url: &str) -> SaveProviderProfileInput {
        SaveProviderProfileInput {
            id: None,
            provider_id: "ollama".into(),
            name: "Local model".into(),
            base_url: base_url.into(),
            model: "qwen3".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        }
    }

    #[test]
    fn save_provider_profile_command_deserializes_the_flattened_camel_case_contract() {
        let input: SaveProviderProfileCommandInput = serde_json::from_value(serde_json::json!({
            "id": "profile-1",
            "providerId": "openai-compatible",
            "name": "Team gateway",
            "baseUrl": "https://models.example.com/v1",
            "model": "gpt-4.1",
            "isDefault": true,
            "parameters": {
                "temperature": 0.25,
                "max_output_tokens": 2048,
                "stop": ["END"]
            },
            "sessionCredential": "session-secret",
            "sessionCredentialLabel": "Primary"
        }))
        .expect("the DesktopBridge payload must deserialize");

        assert_eq!(input.profile.id.as_deref(), Some("profile-1"));
        assert_eq!(input.profile.provider_id, "openai-compatible");
        assert_eq!(input.profile.name, "Team gateway");
        assert_eq!(input.profile.base_url, "https://models.example.com/v1");
        assert_eq!(input.profile.model, "gpt-4.1");
        assert!(input.profile.is_default);
        let parameters = input.profile.parameters.expect("parameters are preserved");
        assert_eq!(
            parameters.get("temperature"),
            Some(&serde_json::json!(0.25))
        );
        assert_eq!(
            parameters.get("max_output_tokens"),
            Some(&serde_json::json!(2048))
        );
        assert_eq!(parameters.get("stop"), Some(&serde_json::json!(["END"])));
        assert_eq!(input.session_credential.as_deref(), Some("session-secret"));
        assert_eq!(input.session_credential_label.as_deref(), Some("Primary"));
    }

    #[test]
    fn retry_command_accepts_one_exact_credential_id_and_rejects_unknown_fields() {
        let input: RetryRunCommandInput = serde_json::from_value(serde_json::json!({
            "runId": "run-failed",
            "providerProfileId": "profile-1",
            "previewHash": "sha256:preview",
            "credentialId": "credential-backup",
            "branchId": "branch-1",
            "expectedCursorVersion": 2,
            "expectedBranchVersion": 3,
            "expectedDraftVersion": 4
        }))
        .expect("exact credential retry command deserializes");
        assert_eq!(input.run_id, "run-failed");
        assert_eq!(input.provider_profile_id, "profile-1");
        assert_eq!(input.preview_hash, "sha256:preview");
        assert_eq!(input.credential_id.as_deref(), Some("credential-backup"));

        assert!(
            serde_json::from_value::<RetryRunCommandInput>(serde_json::json!({
                "runId": "run-failed",
                "providerProfileId": "profile-1",
                "previewHash": "sha256:preview",
                "credentialId": "credential-backup",
                "expectedCursorVersion": 2,
                "expectedDraftVersion": 4,
                "credential": "must-not-be-accepted"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn exact_credential_retry_uses_the_selected_secret_and_activates_only_after_success() {
        let credentials = Arc::new(SessionCredentialStore::default());
        let primary = credentials
            .upsert(
                "profile-1".into(),
                None,
                "Primary".into(),
                "secret-primary".into(),
            )
            .expect("primary credential is stored");
        let backup = credentials
            .upsert(
                "profile-1".into(),
                None,
                "Backup".into(),
                "secret-backup".into(),
            )
            .expect("backup credential is stored");

        let error = retry_run_with_selected_credential(
            credentials.clone(),
            "profile-1",
            Some(&backup.credential_id),
            |lookup| async move {
                assert_eq!(
                    lookup
                        .credential_for("profile-1")?
                        .expect("selected credential exists")
                        .as_str()?,
                    "secret-backup"
                );
                Err(AppError::validation(
                    "context_preview_stale",
                    "preview changed",
                ))
            },
        )
        .await
        .expect_err("a failed Run start is returned");
        assert_eq!(error.code, "context_preview_stale");
        assert_eq!(
            credentials
                .summaries("profile-1")
                .expect("summaries remain readable")[0]
                .credential_id,
            primary.credential_id,
            "failed starts must not change the active credential"
        );

        let handle = retry_run_with_selected_credential(
            credentials.clone(),
            "profile-1",
            Some(&backup.credential_id),
            |_| async {
                Ok(RunHandle {
                    turn_id: "turn-1".into(),
                    run_id: "run-recovered".into(),
                    cursor_version: 2,
                    draft_version: 3,
                    branch_id: "branch-1".into(),
                    branch_version: 4,
                })
            },
        )
        .await
        .expect("successful Run start activates the selected credential");
        assert_eq!(handle.run_id, "run-recovered");
        assert_eq!(
            credentials
                .summaries("profile-1")
                .expect("summaries remain readable")[0]
                .credential_id,
            backup.credential_id
        );
    }

    #[test]
    fn named_credential_command_envelopes_are_strict_and_return_safe_summaries() {
        let input: SetSessionCredentialCommandInput = serde_json::from_value(serde_json::json!({
            "providerProfileId": "profile-1",
            "credentialId": "credential-1",
            "credentialLabel": "Backup",
            "credential": "session-secret"
        }))
        .expect("named credential command deserializes");
        assert_eq!(input.provider_profile_id, "profile-1");
        assert_eq!(input.credential_id.as_deref(), Some("credential-1"));
        assert_eq!(input.credential_label, "Backup");
        assert_eq!(input.credential, "session-secret");

        assert!(
            serde_json::from_value::<SetSessionCredentialCommandInput>(serde_json::json!({
                "providerProfileId": "profile-1",
                "credentialLabel": "Backup",
                "credential": "session-secret",
                "unexpected": true
            }))
            .is_err(),
            "secret ingress rejects unknown fields"
        );

        let serialized = serde_json::to_string(&SessionCredentialSummary {
            credential_id: "credential-1".into(),
            label: "Backup".into(),
            order: 1,
            is_active: false,
        })
        .expect("safe summary serializes");
        assert_eq!(
            serialized,
            r#"{"credentialId":"credential-1","label":"Backup","order":1,"isActive":false}"#
        );
        assert!(!serialized.contains("session-secret"));
    }

    #[test]
    fn initial_credential_validation_is_explicit_and_legacy_label_is_safe() {
        assert_eq!(
            validate_initial_session_credential(None, Some("session-secret".into()))
                .expect("legacy credential gets a stable label")
                .expect("credential is present")
                .0,
            "Default"
        );
        for (label, credential) in [
            (Some("Primary".into()), None),
            (None, Some(String::new())),
            (Some("bad\nlabel".into()), Some("session-secret".into())),
        ] {
            assert!(
                validate_initial_session_credential(label, credential).is_err(),
                "invalid credential pairs fail before the Profile save"
            );
        }
    }

    #[test]
    fn only_provider_identity_or_endpoint_changes_invalidate_session_credentials() {
        let existing = ProviderProfileView {
            id: "profile-1".into(),
            provider_id: "openai".into(),
            name: "Existing".into(),
            dialect: ProviderDialectView::OpenaiCompatible,
            base_url: "https://api.example.com/v1/".into(),
            model: "model-a".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        };
        let mut update = SaveProviderProfileInput {
            id: Some(existing.id.clone()),
            provider_id: existing.provider_id.clone(),
            name: "Renamed".into(),
            base_url: "https://api.example.com/v1".into(),
            model: "model-b".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        };
        assert!(!provider_credential_target_changed(&existing, &update));
        update.base_url = "https://other.example.com/v1".into();
        assert!(provider_credential_target_changed(&existing, &update));
        update.base_url = existing.base_url.clone();
        update.provider_id = "anthropic".into();
        assert!(provider_credential_target_changed(&existing, &update));
    }

    #[test]
    fn model_discovery_command_accepts_the_strict_profile_or_draft_union() {
        let profile: ListProviderModelsCommandInput = serde_json::from_value(serde_json::json!({
            "providerProfileId": "profile-1"
        }))
        .expect("saved Profile discovery deserializes");
        assert!(matches!(
            validate_model_discovery_command(profile).expect("saved Profile is valid"),
            ProviderModelCommandSource::SavedProfile(id) if id == "profile-1"
        ));

        let draft: ListProviderModelsCommandInput = serde_json::from_value(serde_json::json!({
            "draft": {
                "providerId": "google",
                "baseUrl": "https://generativelanguage.googleapis.com/v1beta",
                "sessionCredential": "draft-secret"
            }
        }))
        .expect("draft discovery deserializes");
        let ProviderModelCommandSource::Draft { input, credential } =
            validate_model_discovery_command(draft).expect("draft is valid")
        else {
            panic!("the draft source must remain a draft");
        };
        assert_eq!(credential.as_deref(), Some("draft-secret"));
        assert_eq!(
            input.draft.expect("credential-free draft").provider_id,
            "google"
        );

        let secret = SessionCredential::new(credential.expect("draft credential is present"));
        assert_eq!(secret.expose_secret(), "draft-secret");
        assert_eq!(format!("{secret:?}"), "SessionCredential(<redacted>)");
    }

    #[test]
    fn model_discovery_command_rejects_ambiguous_or_unknown_shapes() {
        for payload in [
            serde_json::json!({}),
            serde_json::json!({
                "providerProfileId": "profile-1",
                "draft": {
                    "providerId": "openai",
                    "baseUrl": "https://api.openai.com/v1"
                }
            }),
        ] {
            let input: ListProviderModelsCommandInput =
                serde_json::from_value(payload).expect("the envelope shape deserializes");
            let error = match validate_model_discovery_command(input) {
                Ok(_) => panic!("ambiguous model discovery source must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.code, "invalid_provider_model_source");
        }

        assert!(
            serde_json::from_value::<ListProviderModelsCommandInput>(serde_json::json!({
                "providerProfileId": "profile-1",
                "sessionCredential": "must-not-be-accepted-here"
            }))
            .is_err(),
            "credentials are accepted only inside a draft target"
        );
    }

    #[test]
    fn static_model_drafts_still_enforce_the_provider_url_policy() {
        for (base_url, code) in [
            ("http://api.anthropic.com", "insecure_remote_provider"),
            (
                "https://secret@api.anthropic.com",
                "embedded_provider_credential",
            ),
            (
                "https://api.anthropic.com?key=secret",
                "invalid_provider_url",
            ),
            ("https://api.anthropic.com#models", "invalid_provider_url"),
        ] {
            let input: ListProviderModelsCommandInput = serde_json::from_value(serde_json::json!({
                "draft": {
                    "providerId": "anthropic",
                    "baseUrl": base_url
                }
            }))
            .expect("the draft contract deserializes");
            let error = match validate_model_discovery_command(input) {
                Ok(_) => panic!("unsafe static-catalog URL must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.code, code);
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
        assert_eq!(
            validate_provider_profile(&provider("https://models.example.com/v1?key=secret"))
                .expect_err("query parameters must not persist in a Provider Base URL")
                .code,
            "invalid_provider_url"
        );
    }

    #[test]
    fn updating_a_provider_profile_requires_a_non_empty_id() {
        let mut input = provider("https://models.example.com/v1");
        input.id = Some("  ".into());

        assert_eq!(
            validate_provider_profile(&input)
                .expect_err("an update needs a concrete Profile ID")
                .code,
            "missing_required_field"
        );
    }

    #[test]
    fn inspect_allows_an_empty_draft_but_start_does_not() {
        let inspect = InspectContextInput {
            workspace_id: "workspace-1".into(),
            parent_run_id: None,
            prompt: String::new(),
            provider_profile_id: "provider-1".into(),
            branch_id: None,
        };
        assert!(validate_inspect_context(&inspect).is_ok());

        let start = CreateTurnAndStartRunInput {
            workspace_id: inspect.workspace_id,
            parent_run_id: None,
            prompt: String::new(),
            provider_profile_id: inspect.provider_profile_id,
            preview_hash: "hash".into(),
            branch_id: None,
            expected_cursor_version: 0,
            expected_branch_version: None,
            expected_draft_version: 0,
        };
        assert_eq!(
            validate_start_run(&start)
                .expect_err("start requires a prompt")
                .code,
            "missing_required_field"
        );
    }

    #[test]
    fn context_draft_rejects_mandatory_exclusion_and_untyped_pins() {
        let mandatory = UpdateContextDraftInput {
            workspace_id: "workspace-1".into(),
            parent_run_id: None,
            expected_draft_version: 0,
            items: vec![ContextDraftItemInput {
                source_ref: ContextSourceRefView {
                    kind: ContextSourceKindView::WorkspaceSystem,
                    id: Some("workspace-1".into()),
                },
                content_block_id: Some("block-system".into()),
                included: false,
                pinned: false,
            }],
        };
        assert_eq!(
            validate_context_draft(&mandatory)
                .expect_err("the System Context cannot be removed")
                .code,
            "mandatory_context_item"
        );

        let pin_without_block = UpdateContextDraftInput {
            items: vec![ContextDraftItemInput {
                source_ref: ContextSourceRefView {
                    kind: ContextSourceKindView::ModelRun,
                    id: Some("run-1".into()),
                },
                content_block_id: None,
                included: true,
                pinned: true,
            }],
            ..mandatory
        };
        assert_eq!(
            validate_context_draft(&pin_without_block)
                .expect_err("a pin needs exact Content Block identity")
                .code,
            "missing_content_block_identity"
        );
    }

    #[cfg(feature = "webview-e2e")]
    #[test]
    fn maintenance_commit_crash_hook_requires_the_exact_operation_id() {
        let completed = "0f6f8d8b-9065-4bb6-91d3-9bfec751b2d4";

        assert!(!should_abort_after_context_maintenance_commit(
            None, completed
        ));
        assert!(!should_abort_after_context_maintenance_commit(
            Some("c17e5958-c8ef-4543-8915-4dd73c665280"),
            completed,
        ));
        assert!(should_abort_after_context_maintenance_commit(
            Some(completed),
            completed,
        ));
    }

    #[test]
    fn compaction_requires_an_explicit_nonempty_source_range_and_kept_boundary() {
        let input = CreateContextCheckpointInput {
            client_operation_id: Uuid::new_v4().to_string(),
            workspace_id: "workspace-1".into(),
            branch_id: "branch-1".into(),
            kind: ContextCheckpointKindView::Compaction,
            source_run_ids: vec!["run-1".into()],
            first_kept_run_id: None,
            summary: "Summary".into(),
            expected_cursor_version: 1,
            expected_branch_version: 2,
        };

        let mut invalid_operation = input.clone();
        invalid_operation.client_operation_id = "new-on-every-retry".into();
        assert_eq!(
            validate_context_checkpoint(&invalid_operation)
                .expect_err("maintenance idempotency keys must be UUIDs")
                .code,
            "invalid_client_operation_id"
        );
        assert_eq!(
            validate_context_checkpoint(&input)
                .expect_err("compaction must retain an explicit tail")
                .code,
            "missing_checkpoint_boundary"
        );

        let branch_summary_with_boundary = CreateContextCheckpointInput {
            kind: ContextCheckpointKindView::BranchSummary,
            first_kept_run_id: Some("run-1".into()),
            ..input
        };
        assert_eq!(
            validate_context_checkpoint(&branch_summary_with_boundary)
                .expect_err("branch summaries cannot smuggle in compaction semantics")
                .code,
            "unexpected_checkpoint_boundary"
        );
    }

    #[tokio::test]
    async fn a_failed_profile_save_preserves_the_previous_session_credential() {
        let credentials = SessionCredentialStore::default();
        credentials
            .set("profile-1".into(), "old-secret".into())
            .expect("credential is stored");
        let mut input = provider("https://models.example.com/v1");
        input.id = Some("profile-1".into());

        let result = save_provider_profile_securely(
            &credentials,
            input,
            Some(("Replacement".into(), "replacement-secret".into())),
            true,
            |_| async {
                assert!(
                    credentials
                        .contains("profile-1")
                        .expect("credential store is readable"),
                    "the old credential remains authoritative until the Profile save commits"
                );
                Err(AppError::internal("save_failed", "profile was not saved"))
            },
        )
        .await;

        assert_eq!(result.expect_err("save must fail").code, "save_failed");
        assert_eq!(
            credentials
                .get("profile-1")
                .expect("credential store is readable")
                .expect("old credential remains present")
                .as_str()
                .expect("credential is valid UTF-8"),
            "old-secret"
        );
    }

    #[tokio::test]
    async fn an_unchanged_profile_save_without_a_new_key_preserves_credentials() {
        let credentials = SessionCredentialStore::default();
        credentials
            .set("profile-1".into(), "old-secret".into())
            .expect("credential is stored");
        let mut input = provider("https://models.example.com/v1");
        input.id = Some("profile-1".into());
        let saved = ProviderProfileView {
            id: "profile-1".into(),
            provider_id: "ollama".into(),
            name: "Local model".into(),
            dialect: ProviderDialectView::Ollama,
            base_url: "https://models.example.com/v1".into(),
            model: "qwen3".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        };

        let result = save_provider_profile_securely(&credentials, input, None, false, |_| async {
            Ok(saved.clone())
        })
        .await
        .expect("profile is saved");

        assert_eq!(result, saved);
        assert_eq!(
            credentials
                .get("profile-1")
                .expect("credential store is readable")
                .expect("credential remains present")
                .as_str()
                .expect("credential is valid UTF-8"),
            "old-secret"
        );
    }

    #[tokio::test]
    async fn a_committed_provider_target_change_clears_old_credentials() {
        let credentials = SessionCredentialStore::default();
        credentials
            .set("profile-1".into(), "old-secret".into())
            .expect("credential is stored");
        let mut input = provider("https://new-endpoint.example.com/v1");
        input.id = Some("profile-1".into());
        let saved = ProviderProfileView {
            id: "profile-1".into(),
            provider_id: "ollama".into(),
            name: "Local model".into(),
            dialect: ProviderDialectView::Ollama,
            base_url: input.base_url.clone(),
            model: "qwen3".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        };

        save_provider_profile_securely(&credentials, input, None, true, |_| async { Ok(saved) })
            .await
            .expect("Profile change commits");

        assert!(
            !credentials
                .contains("profile-1")
                .expect("credential store is readable")
        );
    }

    #[tokio::test]
    async fn saving_a_provider_profile_atomically_installs_its_new_session_credential() {
        let credentials = SessionCredentialStore::default();
        credentials
            .set("profile-1".into(), "old-secret".into())
            .expect("credential is stored");
        let mut input = provider("https://models.example.com/v1");
        input.id = Some("profile-1".into());
        let saved = ProviderProfileView {
            id: "profile-1".into(),
            provider_id: "ollama".into(),
            name: "Local model".into(),
            dialect: ProviderDialectView::Ollama,
            base_url: "https://models.example.com/v1".into(),
            model: "qwen3".into(),
            is_default: true,
            parameters: Some(std::collections::BTreeMap::new()),
        };

        let result = save_provider_profile_securely(
            &credentials,
            input,
            Some(("Replacement".into(), "replacement-secret".into())),
            false,
            |_| async { Ok(saved.clone()) },
        )
        .await
        .expect("profile and credential are saved");

        assert_eq!(result, saved);
        let credential = credentials
            .get("profile-1")
            .expect("credential store is readable")
            .expect("replacement credential is present");
        assert_eq!(
            credential.as_str().expect("credential is valid UTF-8"),
            "replacement-secret"
        );
        assert_eq!(
            credentials
                .summaries("profile-1")
                .expect("summaries are readable")[0]
                .label,
            "Replacement"
        );
    }
}
