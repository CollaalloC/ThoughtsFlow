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
    pub provider_name: String,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfileView {
    pub id: EntityId,
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
    pub name: String,
    pub dialect: ProviderDialectView,
    pub base_url: String,
    pub model: String,
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<BTreeMap<String, Value>>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionCredentialInput {
    pub provider_profile_id: EntityId,
    pub credential: String,
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
    use super::{ExportDecisionPacketInput, WorkspaceSummary};

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
}
