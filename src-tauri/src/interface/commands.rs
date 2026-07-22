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
    input: RetryRunInput,
    on_event: Channel<RunEventView>,
) -> AppResult<ApiResponse<RunHandle>> {
    require_non_empty("runId", &input.run_id)?;
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    require_non_empty("previewHash", &input.preview_hash)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
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
    } = input;
    validate_provider_profile(&profile)?;
    let profile_id = profile
        .id
        .get_or_insert_with(|| Uuid::new_v4().to_string())
        .clone();
    let provider_profile_lock = state.provider_profile_lock(&profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
    let backend = state.backend().clone();
    save_provider_profile_securely(
        state.credentials(),
        profile,
        session_credential,
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
}

async fn save_provider_profile_securely<Save, SaveFuture>(
    credentials: &SessionCredentialStore,
    input: SaveProviderProfileInput,
    session_credential: Option<String>,
    save: Save,
) -> AppResult<ProviderProfileView>
where
    Save: FnOnce(SaveProviderProfileInput) -> SaveFuture,
    SaveFuture: std::future::Future<Output = AppResult<ProviderProfileView>>,
{
    if let Some(profile_id) = input.id.as_deref() {
        credentials.remove(profile_id)?;
    }

    let profile = save(input).await?;
    credentials.remove(&profile.id)?;
    if let Some(credential) = session_credential {
        credentials.set(profile.id.clone(), credential)?;
    }
    Ok(profile)
}

#[tauri::command]
pub async fn set_session_credential(
    state: State<'_, AppState>,
    input: SetSessionCredentialInput,
) -> AppResult<ApiResponse<CommandAcknowledgement>> {
    require_non_empty("providerProfileId", &input.provider_profile_id)?;
    let provider_profile_lock = state.provider_profile_lock(&input.provider_profile_id)?;
    let _provider_profile_operation = provider_profile_lock.lock().await;
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
            "sessionCredential": "session-secret"
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

    #[tokio::test]
    async fn updating_a_provider_profile_clears_its_old_credential_before_a_failed_save() {
        let credentials = SessionCredentialStore::default();
        credentials
            .set("profile-1".into(), "old-secret".into())
            .expect("credential is stored");
        let mut input = provider("https://models.example.com/v1");
        input.id = Some("profile-1".into());

        let result = save_provider_profile_securely(
            &credentials,
            input,
            Some("replacement-secret".into()),
            |_| async {
                assert!(
                    !credentials
                        .contains("profile-1")
                        .expect("credential store is readable"),
                    "the old credential must be gone before the backend can publish a new endpoint"
                );
                Err(AppError::internal("save_failed", "profile was not saved"))
            },
        )
        .await;

        assert_eq!(result.expect_err("save must fail").code, "save_failed");
        assert!(
            !credentials
                .contains("profile-1")
                .expect("credential store is readable")
        );
    }

    #[tokio::test]
    async fn saving_a_provider_profile_without_a_new_key_leaves_no_session_credential() {
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

        let result = save_provider_profile_securely(&credentials, input, None, |_| async {
            Ok(saved.clone())
        })
        .await
        .expect("profile is saved");

        assert_eq!(result, saved);
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
            Some("replacement-secret".into()),
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
    }
}
