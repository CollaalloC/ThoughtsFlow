//! Local execution control: Orca owns workers; ThoughsFlow owns intent and receipts.
mod commands;
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
