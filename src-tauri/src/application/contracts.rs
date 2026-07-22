use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONTRACT_VERSION: u16 = 1;
pub type EntityId = String;
pub type Timestamp = String;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: EntityId,
    pub name: String,
    pub goal: String,
    pub system_prompt: String,
    pub archived: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceInput {
    pub id: EntityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDetail {
    pub workspace: WorkspaceSummary,
    pub turns: Vec<TurnView>,
    pub selected_run_ids: BTreeMap<EntityId, EntityId>,
    pub adjacent_branches: Vec<AdjacentBranchView>,
    pub decision_marks: Vec<DecisionMarkView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdjacentBranchView {
    pub run_id: EntityId,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnView {
    pub id: EntityId,
    pub workspace_id: EntityId,
    pub parent_run_id: Option<EntityId>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at: Timestamp,
    pub runs: Vec<RunView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: EntityId,
    pub turn_id: EntityId,
    pub status: RunStatusView,
    pub output: String,
    pub reasoning: Option<String>,
    pub provider_profile_id: EntityId,
    pub provider_name: String,
    pub model: String,
    pub base_url: String,
    pub created_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub usage: Option<BTreeMap<String, u64>>,
    pub error: Option<RunErrorView>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatusView {
    /// Wire-language alias for the domain's `queued` state.
    Pending,
    Connecting,
    Streaming,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorView {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextItemView {
    pub id: EntityId,
    pub ordinal: u32,
    pub role: MessageRoleView,
    pub label: String,
    pub source: String,
    pub content: String,
    pub reason: String,
    pub estimated_tokens: u64,
    pub included: bool,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRoleView {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPreview {
    pub hash: String,
    pub estimated_tokens: u64,
    pub limit_tokens: u64,
    pub blocked: bool,
    pub warnings: Vec<String>,
    pub provider_profile_id: EntityId,
    pub provider_name: String,
    pub model: String,
    pub base_url: String,
    pub items: Vec<ContextItemView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InspectContextInput {
    pub workspace_id: EntityId,
    pub parent_run_id: Option<EntityId>,
    pub prompt: String,
    pub provider_profile_id: EntityId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTurnAndStartRunInput {
    pub workspace_id: EntityId,
    pub parent_run_id: Option<EntityId>,
    pub prompt: String,
    pub provider_profile_id: EntityId,
    pub preview_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetryRunInput {
    pub run_id: EntityId,
    pub provider_profile_id: EntityId,
    pub preview_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunHandle {
    pub turn_id: EntityId,
    pub run_id: EntityId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshotView {
    pub id: EntityId,
    pub run_id: EntityId,
    pub canonical_hash: String,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_revision: Option<u16>,
    pub provider_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_protocol: Option<ProviderStreamProtocolView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_placement: Option<ProviderAuthPlacementView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_name: Option<String>,
    pub additional_headers: BTreeMap<String, String>,
    pub model: String,
    pub base_url: String,
    pub parameters: BTreeMap<String, Value>,
    pub items: Vec<ContextItemView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateContextOverridesInput {
    pub workspace_id: EntityId,
    pub parent_run_id: Option<EntityId>,
    pub item_id: EntityId,
    pub included: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetRouteProjectionInput {
    pub workspace_id: EntityId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_run_id: Option<EntityId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteProjection {
    pub workspace_id: EntityId,
    pub nodes: Vec<RouteNodeView>,
    pub edges: Vec<RouteEdgeView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteNodeView {
    pub id: EntityId,
    pub turn_id: EntityId,
    pub title: String,
    pub summary: String,
    pub status: RunStatusView,
    pub x: f64,
    pub y: f64,
    pub is_current: bool,
    pub is_on_current_lineage: bool,
    pub runs: Vec<RouteRunPortView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteRunPortView {
    pub run_id: EntityId,
    pub label: String,
    pub model: String,
    pub status: RunStatusView,
    pub can_branch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteEdgeView {
    pub id: EntityId,
    pub source_run_id: EntityId,
    pub target_turn_id: EntityId,
    pub is_on_current_lineage: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateViewStateInput {
    pub workspace_id: EntityId,
    pub turn_id: EntityId,
    pub x: f64,
    pub y: f64,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompareRunsInput {
    pub left_run_id: EntityId,
    pub right_run_id: EntityId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompareRunsResult {
    pub left: ComparableRunView,
    pub right: ComparableRunView,
    pub answer: AnswerComparison,
    pub context_diff: ContextDiffView,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComparableRunView {
    pub run_id: EntityId,
    pub model: String,
    pub status: RunStatusView,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnswerComparison {
    pub left_markdown: String,
    pub right_markdown: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextDiffView {
    pub only_left: Vec<ContextDiffItemView>,
    pub only_right: Vec<ContextDiffItemView>,
    pub shared: Vec<ContextDiffItemView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextDiffItemView {
    pub id: EntityId,
    pub ordinal: u32,
    pub role: MessageRoleView,
    pub source: String,
    pub preview: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionStatusView {
    /// Wire-language alias for the domain's `adopted` state.
    Accepted,
    Rejected,
    /// Wire-language alias for the domain's `needs_validation` state.
    ToVerify,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarkDecisionInput {
    pub workspace_id: EntityId,
    pub run_id: EntityId,
    pub status: DecisionStatusView,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DecisionMarkView {
    pub id: EntityId,
    pub workspace_id: EntityId,
    pub run_id: EntityId,
    pub status: DecisionStatusView,
    pub reason: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportDecisionPacketInput {
    pub workspace_id: EntityId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub bytes_written: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderDialectView {
    OpenaiCompatible,
    Ollama,
    Anthropic,
    GoogleGenerativeAi,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStreamProtocolView {
    #[serde(rename = "openai_sse")]
    OpenAiSse,
    OllamaNdjson,
    AnthropicSse,
    GoogleSse,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthPlacementView {
    None,
    BearerHeader,
    ApiKeyHeader,
    QueryParam,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolProfileView {
    pub stream_protocol: ProviderStreamProtocolView,
    pub auth_placement: ProviderAuthPlacementView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_endpoint: Option<String>,
    pub requires_additional_headers: bool,
    pub additional_headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTemplateView {
    pub provider_id: EntityId,
    pub revision: u16,
    pub display_name: String,
    pub default_base_url: String,
    pub protocol: ProtocolProfileView,
    pub runtime_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfileView {
    pub id: EntityId,
    pub provider_id: EntityId,
    pub name: String,
    pub dialect: ProviderDialectView,
    pub base_url: String,
    pub model: String,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderProfileInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntityId>,
    pub provider_id: EntityId,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestProviderConnectionInput {
    pub provider_profile_id: EntityId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionResult {
    pub ok: bool,
    pub message: String,
}

/// Credential-free application request for Provider model discovery.
///
/// The Tauri command owns the stricter public union and passes any draft
/// credential separately so secrets cannot enter this serializable DTO.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListProviderModelsInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_profile_id: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<ProviderModelDraftInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModelDraftInput {
    pub provider_id: EntityId,
    pub base_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfoView {
    pub id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub supports_tools: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum RunEventView {
    #[serde(rename = "run-started")]
    RunStarted {
        api_version: u16,
        run_id: EntityId,
        at: Timestamp,
    },
    #[serde(rename = "text-delta")]
    TextDelta {
        api_version: u16,
        run_id: EntityId,
        text: String,
        at: Timestamp,
    },
    #[serde(rename = "reasoning-delta")]
    ReasoningDelta {
        api_version: u16,
        run_id: EntityId,
        text: String,
        at: Timestamp,
    },
    #[serde(rename = "usage-updated")]
    UsageUpdated {
        api_version: u16,
        run_id: EntityId,
        usage: BTreeMap<String, u64>,
        at: Timestamp,
    },
    #[serde(rename = "checkpoint-saved")]
    CheckpointSaved {
        api_version: u16,
        run_id: EntityId,
        metadata: BTreeMap<String, Value>,
        at: Timestamp,
    },
    #[serde(rename = "provider-metadata")]
    ProviderMetadata {
        api_version: u16,
        run_id: EntityId,
        metadata: BTreeMap<String, Value>,
        at: Timestamp,
    },
    #[serde(rename = "run-completed")]
    RunCompleted {
        api_version: u16,
        run_id: EntityId,
        at: Timestamp,
    },
    #[serde(rename = "run-failed")]
    RunFailed {
        api_version: u16,
        run_id: EntityId,
        error: RunErrorView,
        at: Timestamp,
    },
    #[serde(rename = "run-cancelled")]
    RunCancelled {
        api_version: u16,
        run_id: EntityId,
        at: Timestamp,
    },
    /// Fatal local durability error for which no terminal Run status could be
    /// committed. This deliberately is not a normal RunFailed terminal event.
    #[serde(rename = "persistence-failed")]
    PersistenceFailed {
        api_version: u16,
        run_id: EntityId,
        error: RunErrorView,
        at: Timestamp,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        ExportDecisionPacketInput, ModelInfoView, ProtocolProfileView, ProviderAuthPlacementView,
        ProviderStreamProtocolView, ProviderTemplateView, RunSnapshotView, WorkspaceSummary,
    };

    #[test]
    fn decision_packet_contract_rejects_webview_supplied_paths() {
        let result = serde_json::from_value::<ExportDecisionPacketInput>(serde_json::json!({
            "workspaceId": "workspace-1",
            "destination": "/tmp/overwrite-me"
        }));

        assert!(result.is_err());
    }

    #[test]
    fn workspace_contract_keeps_goal_and_system_prompt_independent() {
        let value = serde_json::to_value(WorkspaceSummary {
            id: "workspace-1".into(),
            name: "Architecture review".into(),
            goal: "Choose a migration path".into(),
            system_prompt: "Challenge unsupported assumptions".into(),
            archived: false,
            created_at: "2026-07-22T00:00:00Z".into(),
            updated_at: "2026-07-22T00:00:00Z".into(),
        })
        .unwrap();

        assert_eq!(value["goal"], "Choose a migration path");
        assert_eq!(value["systemPrompt"], "Challenge unsupported assumptions");
    }

    #[test]
    fn provider_template_contract_uses_stable_protocol_names() {
        let value = serde_json::to_value(ProviderTemplateView {
            provider_id: "anthropic".into(),
            revision: 2,
            display_name: "Anthropic".into(),
            default_base_url: "https://api.anthropic.com".into(),
            protocol: ProtocolProfileView {
                stream_protocol: ProviderStreamProtocolView::AnthropicSse,
                auth_placement: ProviderAuthPlacementView::ApiKeyHeader,
                auth_header_name: Some("x-api-key".into()),
                models_endpoint: None,
                requires_additional_headers: true,
                additional_headers: BTreeMap::from([(
                    "anthropic-version".into(),
                    "2023-06-01".into(),
                )]),
            },
            runtime_available: true,
        })
        .unwrap();

        assert_eq!(value["providerId"], "anthropic");
        assert_eq!(value["protocol"]["streamProtocol"], "anthropic_sse");
        assert_eq!(value["protocol"]["authPlacement"], "api_key_header");
        assert_eq!(value["protocol"]["authHeaderName"], "x-api-key");
        assert_eq!(value["runtimeAvailable"], true);
    }

    #[test]
    fn legacy_run_snapshot_omits_only_unresolved_provider_metadata() {
        let value = serde_json::to_value(RunSnapshotView {
            id: "snapshot-legacy".into(),
            run_id: "run-legacy".into(),
            canonical_hash: "hash-legacy".into(),
            created_at: "2026-07-22T00:00:00Z".into(),
            provider_id: None,
            template_revision: None,
            provider_name: "Legacy Ollama".into(),
            stream_protocol: None,
            auth_placement: None,
            auth_header_name: None,
            additional_headers: BTreeMap::new(),
            model: "qwen3".into(),
            base_url: "http://127.0.0.1:11434".into(),
            parameters: BTreeMap::from([("temperature".into(), serde_json::json!(0.7))]),
            items: Vec::new(),
        })
        .unwrap();

        assert!(value.get("providerId").is_none());
        assert!(value.get("templateRevision").is_none());
        assert!(value.get("streamProtocol").is_none());
        assert!(value.get("authPlacement").is_none());
        assert!(value.get("authHeaderName").is_none());
        assert_eq!(value["parameters"]["temperature"], 0.7);
    }

    #[test]
    fn model_info_contract_uses_camel_case_and_explicit_unknown_capabilities() {
        let value = serde_json::to_value(ModelInfoView {
            id: "fixture-model".into(),
            display_name: "Fixture Model".into(),
            context_window: None,
            supports_tools: None,
        })
        .expect("model metadata serializes");

        assert_eq!(value["id"], "fixture-model");
        assert_eq!(value["displayName"], "Fixture Model");
        assert!(value["contextWindow"].is_null());
        assert!(value["supportsTools"].is_null());
    }
}
