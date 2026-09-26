use tauri::State;

use super::*;
use crate::{
    application::{AppResult, CONTRACT_VERSION},
    interface::ApiResponse,
};

fn response<T>(data: T) -> ApiResponse<T> {
    ApiResponse {
        api_version: CONTRACT_VERSION,
        data,
    }
}

#[tauri::command]
pub async fn agent_environment(
    state: State<'_, AgentService>,
) -> AppResult<ApiResponse<AgentEnvironment>> {
    state.environment().await.map(response)
}

#[tauri::command]
pub async fn agent_open_runtime(
    state: State<'_, AgentService>,
) -> AppResult<ApiResponse<AgentEnvironment>> {
    state.open_runtime().await.map(response)
}

#[tauri::command]
pub async fn agent_list_missions(
    state: State<'_, AgentService>,
    workspace_id: String,
) -> AppResult<ApiResponse<Vec<AgentMission>>> {
    state.list_missions(&workspace_id).await.map(response)
}

#[tauri::command]
pub async fn agent_create_mission(
    state: State<'_, AgentService>,
    input: CreateMissionInput,
) -> AppResult<ApiResponse<AgentMission>> {
    state.create_mission(input).await.map(response)
}

#[tauri::command]
pub async fn agent_snapshot(
    state: State<'_, AgentService>,
    mission_id: String,
) -> AppResult<ApiResponse<AgentSnapshot>> {
    state.snapshot(&mission_id).await.map(response)
}

#[tauri::command]
pub async fn agent_start_task(
    state: State<'_, AgentService>,
    input: StartTaskInput,
) -> AppResult<ApiResponse<AgentOperation>> {
    state.start_task(input).await.map(response)
}

#[tauri::command]
pub async fn agent_reply(
    state: State<'_, AgentService>,
    input: ReplyInput,
) -> AppResult<ApiResponse<AgentOperation>> {
    state.reply(input).await.map(response)
}

#[tauri::command]
pub async fn agent_release_worker(
    state: State<'_, AgentService>,
    input: ReleaseWorkerInput,
) -> AppResult<ApiResponse<AgentOperation>> {
    state.release(input).await.map(response)
}

#[tauri::command]
pub async fn agent_reconnect(
    state: State<'_, AgentService>,
    input: ReconnectInput,
) -> AppResult<ApiResponse<AgentOperation>> {
    state.reconnect(input).await.map(response)
}

#[tauri::command]
pub async fn agent_operations(
    state: State<'_, AgentService>,
    mission_id: String,
) -> AppResult<ApiResponse<Vec<AgentOperation>>> {
    state.operations(&mission_id).await.map(response)
}

#[tauri::command]
pub async fn agent_read_output(
    state: State<'_, AgentService>,
    input: ReadOutputInput,
) -> AppResult<ApiResponse<AgentOutput>> {
    state.read_output(input).await.map(response)
}
