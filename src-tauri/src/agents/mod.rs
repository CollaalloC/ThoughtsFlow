//! Local execution control: Orca owns workers; ThoughtsFlow owns intent and receipts.
mod commands;
mod context;
mod executable;
mod orca;
mod receipt;
mod service;
pub(crate) mod store;

pub use commands::*;
pub use service::AgentService;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnvironment {
    pub available: bool,
    pub running: bool,
    pub orca_version: Option<String>,
    pub omp_version: Option<String>,
    pub runtime_id: Option<String>,
    pub projects: Vec<AgentProject>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentProject {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMission {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub repository_path: String,
    pub objective: String,
    pub run_id: Option<String>,
    pub coordinator_handle: Option<String>,
    pub runtime_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    pub id: String,
    pub title: String,
    pub spec: String,
    pub status: String,
    pub dispatch_id: Option<String>,
    pub terminal_state: Option<String>,
    pub liveness: Option<String>,
    pub attention: Option<String>,
    pub can_release: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub id: String,
    pub r#type: String,
    pub body: String,
    pub task_id: Option<String>,
    pub dispatch_id: Option<String>,
    pub requires_reply: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    pub mission: AgentMission,
    pub tasks: Vec<AgentTask>,
    pub messages: Vec<AgentMessage>,
    pub connected: bool,
    pub can_start_tasks: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOperation {
    pub id: String,
    pub mission_id: String,
    pub kind: String,
    pub status: String,
    pub receipt: Value,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutput {
    pub text: String,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub source: Option<String>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMissionInput {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub objective: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTaskInput {
    pub operation_id: String,
    pub mission_id: String,
    pub title: String,
    pub spec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<AgentHandoffConfirmation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextSelection {
    pub include_active_context: bool,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAgentHandoffInput {
    pub mission_id: String,
    pub title: String,
    pub spec: String,
    pub context: AgentContextSelection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffConfirmation {
    pub context: AgentContextSelection,
    pub preview_hash: String,
    pub expected_cursor_version: u64,
    pub expected_draft_version: u64,
    pub expected_branch_id: Option<String>,
    pub expected_branch_version: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffSource {
    pub kind: String,
    pub id: String,
    pub role: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoffPreview {
    pub hash: String,
    #[serde(default)]
    pub title: String,
    pub text: String,
    pub compiler_version: String,
    pub source_run_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub sources: Vec<AgentHandoffSource>,
    pub parent_run_id: Option<String>,
    pub branch_id: Option<String>,
    pub cursor_version: u64,
    pub draft_version: u64,
    pub branch_version: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub operation_id: String,
    pub workspace_id: String,
    pub mission_id: String,
    pub created_at: String,
    pub preview: AgentHandoffPreview,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureAgentEvidenceInput {
    pub id: String,
    pub mission_id: String,
    pub dispatch_id: String,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvidence {
    pub id: String,
    pub workspace_id: String,
    pub mission_id: String,
    pub run_id: String,
    pub task_id: String,
    pub dispatch_id: String,
    pub handoff_operation_id: Option<String>,
    pub text: String,
    pub content_hash: String,
    pub source: Option<String>,
    pub request_cursor: Option<String>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub warning: Option<String>,
    pub observed_task_status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyInput {
    pub operation_id: String,
    pub mission_id: String,
    pub message_id: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseWorkerInput {
    pub operation_id: String,
    pub mission_id: String,
    pub dispatch_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconnectInput {
    pub operation_id: String,
    pub mission_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadOutputInput {
    pub mission_id: String,
    pub dispatch_id: String,
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests;
