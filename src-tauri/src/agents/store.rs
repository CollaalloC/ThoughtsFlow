use serde_json::Value;
use sqlx::{Row, SqlitePool};

use super::{
    AgentEvidence, AgentEvidencePage, AgentHandoff, AgentHandoffPage, AgentMission, AgentOperation,
    CaptureAgentEvidenceInput,
};
use crate::application::{AppError, AppResult};
use crate::infrastructure::sqlite::{SqliteRepository, get_context_cursor_in};

#[derive(Clone)]
pub(crate) struct AgentStore {
    pool: SqlitePool,
    pub(super) repository: SqliteRepository,
}

fn database(error: impl std::fmt::Display) -> AppError {
    AppError::internal("agent_storage", format!("Agent 工作台存储失败：{error}"))
}

const HISTORY_PAGE_SIZE: usize = 50;

fn history_cursor(kind: &str, scope: &str, row_id: i64) -> String {
    // This checksum binds an opaque navigation token to its collection. It is
    // not authorization: every query independently restricts its stored scope.
    let identity = serde_json::json!(["agent-history/1", kind, scope, row_id]).to_string();
    format!(
        "h1.{row_id}.{}",
        crate::domain::sha256_hex(identity.as_bytes())
    )
}

fn history_upper_bound(kind: &str, scope: &str, cursor: Option<&str>) -> AppResult<i64> {
    let Some(cursor) = cursor else {
        return Ok(i64::MAX);
    };
    let invalid = || {
        AppError::validation(
            "agent_history_cursor",
            "历史分页游标无效或不属于当前列表，请重新加载。",
        )
    };
    if cursor.len() > 128 {
        return Err(invalid());
    }
    let row_id = cursor
        .split('.')
        .nth(1)
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(invalid)?;
    if cursor != history_cursor(kind, scope, row_id) {
        return Err(invalid());
    }
    Ok(row_id - 1)
}

fn history_page<T: serde::de::DeserializeOwned>(
    kind: &str,
    scope: &str,
    mut rows: Vec<(i64, String)>,
) -> AppResult<(Vec<T>, Option<String>)> {
    let more = rows.len() > HISTORY_PAGE_SIZE;
    rows.truncate(HISTORY_PAGE_SIZE);
    let next_cursor =
        more.then(|| history_cursor(kind, scope, rows.last().expect("nonempty full page").0));
    let items = rows
        .into_iter()
        .map(|(_, summary)| serde_json::from_str(&summary).map_err(database))
        .collect::<AppResult<Vec<_>>>()?;
    Ok((items, next_cursor))
}

impl AgentStore {
    pub(crate) fn new(pool: SqlitePool, repository: SqliteRepository) -> Self {
        Self { pool, repository }
    }

    pub async fn recover(&self) -> AppResult<()> {
        sqlx::query("UPDATE agent_operation SET status = 'unknown', error = '应用退出时未取得最终回执。请检查 Orca；不会自动重发。' WHERE status = 'pending'")
            .execute(&self.pool).await.map_err(database)?;
        sqlx::query("UPDATE agent_mission SET body_json = json_set(body_json, '$.status', 'needs-attention', '$.error', '创建被中断，请检查 Orca；不会自动重建。') WHERE json_extract(body_json, '$.status') = 'creating'")
            .execute(&self.pool).await.map_err(database)?;
        Ok(())
    }

    pub async fn create_mission(&self, mission: &AgentMission) -> AppResult<()> {
        sqlx::query("INSERT INTO agent_mission(id, workspace_id, body_json) VALUES (?, ?, ?)")
            .bind(&mission.id)
            .bind(&mission.workspace_id)
            .bind(serde_json::to_string(mission).map_err(database)?)
            .execute(&self.pool)
            .await
            .map_err(database)?;
        Ok(())
    }

    pub async fn save_mission(&self, mission: &AgentMission) -> AppResult<()> {
        sqlx::query("UPDATE agent_mission SET body_json = ? WHERE id = ?")
            .bind(serde_json::to_string(mission).map_err(database)?)
            .bind(&mission.id)
            .execute(&self.pool)
            .await
            .map_err(database)?;
        Ok(())
    }

    pub async fn mission(&self, id: &str) -> AppResult<Option<AgentMission>> {
        let body: Option<String> =
            sqlx::query_scalar("SELECT body_json FROM agent_mission WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(database)?;
        body.map(|body| serde_json::from_str(&body).map_err(database))
            .transpose()
    }

    pub async fn missions(&self, workspace: &str) -> AppResult<Vec<AgentMission>> {
        let bodies: Vec<String> = sqlx::query_scalar(
            "SELECT body_json FROM agent_mission WHERE workspace_id = ? ORDER BY rowid DESC",
        )
        .bind(workspace)
        .fetch_all(&self.pool)
        .await
        .map_err(database)?;
        bodies
            .into_iter()
            .map(|body| serde_json::from_str(&body).map_err(database))
            .collect()
    }

    #[cfg(test)]
    pub async fn save_binding(
        &self,
        id: &str,
        pane_key: Option<&str>,
        generation: Option<i64>,
    ) -> AppResult<()> {
        sqlx::query("UPDATE agent_mission SET coordinator_pane_key = COALESCE(?, coordinator_pane_key), consumer_generation = ? WHERE id = ?")
            .bind(pane_key).bind(generation).bind(id).execute(&self.pool).await.map_err(database)?;
        Ok(())
    }

    pub async fn binding(&self, id: &str) -> AppResult<(Option<String>, Option<i64>)> {
        sqlx::query_as(
            "SELECT coordinator_pane_key, consumer_generation FROM agent_mission WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(database)
    }

    pub async fn operation(&self, id: &str) -> AppResult<Option<(AgentOperation, Value)>> {
        let row = sqlx::query("SELECT * FROM agent_operation WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(database)?;
        row.map(|row| {
            let request =
                serde_json::from_str(row.get::<&str, _>("request_json")).map_err(database)?;
            Ok((read_operation(&row)?, request))
        })
        .transpose()
    }

    pub async fn operations(&self, mission: &str) -> AppResult<Vec<AgentOperation>> {
        let rows =
            sqlx::query("SELECT * FROM agent_operation WHERE mission_id = ? ORDER BY rowid DESC")
                .bind(mission)
                .fetch_all(&self.pool)
                .await
                .map_err(database)?;
        rows.iter().map(read_operation).collect()
    }

    pub(super) async fn handoffs(&self, mission: &str) -> AppResult<Vec<AgentHandoff>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT body_json FROM agent_handoff WHERE mission_id = ? ORDER BY rowid DESC",
        )
        .bind(mission)
        .fetch_all(&self.pool)
        .await
        .map_err(database)?;
        rows.into_iter()
            .map(|body| serde_json::from_str(&body).map_err(database))
            .collect()
    }

    pub(super) async fn evidence(
        &self,
        id: &str,
    ) -> AppResult<Option<(AgentEvidence, CaptureAgentEvidenceInput)>> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT body_json, request_json FROM agent_evidence WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(database)?;
        row.map(|(body, request)| {
            Ok((
                serde_json::from_str(&body).map_err(database)?,
                serde_json::from_str(&request).map_err(database)?,
            ))
        })
        .transpose()
    }

    pub(super) async fn evidence_list(&self, workspace: &str) -> AppResult<Vec<AgentEvidence>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT body_json FROM agent_evidence WHERE workspace_id = ? ORDER BY rowid DESC",
        )
        .bind(workspace)
        .fetch_all(&self.pool)
        .await
        .map_err(database)?;
        rows.into_iter()
            .map(|body| serde_json::from_str(&body).map_err(database))
            .collect()
    }

    pub(super) async fn evidence_page(
        &self,
        workspace: &str,
        cursor: Option<&str>,
    ) -> AppResult<AgentEvidencePage> {
        let upper_bound = history_upper_bound("evidence", workspace, cursor)?;
        // Project before crossing SQLite/Rust: list readers never deserialize output text.
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT e.rowid, json_object(
                'id',e.id, 'workspaceId',e.workspace_id, 'missionId',e.mission_id,
                'runId',json_extract(e.body_json,'$.runId'),
                'taskId',json_extract(e.body_json,'$.taskId'),
                'dispatchId',json_extract(e.body_json,'$.dispatchId'),
                'handoffOperationId',json_extract(e.body_json,'$.handoffOperationId'),
                'title',NULLIF(json_extract(h.body_json,'$.preview.title'),''),
                'contentHash',json_extract(e.body_json,'$.contentHash'),
                'source',json_extract(e.body_json,'$.source'),
                'requestCursor',json_extract(e.body_json,'$.requestCursor'),
                'cursor',json_extract(e.body_json,'$.cursor'),
                'hasMore',json(CASE json_extract(e.body_json,'$.hasMore') WHEN 1 THEN 'true' ELSE 'false' END),
                'warning',json_extract(e.body_json,'$.warning'),
                'observedTaskStatus',json_extract(e.body_json,'$.observedTaskStatus'),
                'createdAt',json_extract(e.body_json,'$.createdAt'))
             FROM agent_evidence e
             LEFT JOIN agent_handoff h ON h.operation_id=json_extract(e.body_json,'$.handoffOperationId')
                AND h.workspace_id=e.workspace_id AND h.mission_id=e.mission_id
             WHERE e.workspace_id=? AND e.rowid<=? ORDER BY e.rowid DESC LIMIT 51"
        ).bind(workspace).bind(upper_bound).fetch_all(&self.pool).await.map_err(database)?;
        let (items, next_cursor) = history_page("evidence", workspace, rows)?;
        Ok(AgentEvidencePage { items, next_cursor })
    }

    pub(super) async fn handoff_page(
        &self,
        mission: &str,
        cursor: Option<&str>,
    ) -> AppResult<AgentHandoffPage> {
        let upper_bound = history_upper_bound("handoff", mission, cursor)?;
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT rowid, json_object(
                'operationId',operation_id, 'workspaceId',workspace_id, 'missionId',mission_id,
                'createdAt',json_extract(body_json,'$.createdAt'),
                'title',COALESCE(json_extract(body_json,'$.preview.title'),''),
                'hash',json_extract(body_json,'$.preview.hash'),
                'sourceRunIds',json_extract(body_json,'$.preview.sourceRunIds'),
                'evidenceIds',json_extract(body_json,'$.preview.evidenceIds'))
             FROM agent_handoff WHERE mission_id=? AND rowid<=? ORDER BY rowid DESC LIMIT 51",
        )
        .bind(mission)
        .bind(upper_bound)
        .fetch_all(&self.pool)
        .await
        .map_err(database)?;
        let (items, next_cursor) = history_page("handoff", mission, rows)?;
        Ok(AgentHandoffPage { items, next_cursor })
    }

    pub(super) async fn get_evidence(&self, workspace: &str, id: &str) -> AppResult<AgentEvidence> {
        let body: Option<String> = sqlx::query_scalar(
            "SELECT body_json FROM agent_evidence WHERE workspace_id=? AND id=?",
        )
        .bind(workspace)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?;
        let missing =
            || AppError::validation("agent_history_not_found", "此工作区中没有该输出证据。");
        let evidence: AgentEvidence =
            serde_json::from_str(&body.ok_or_else(missing)?).map_err(database)?;
        if evidence.workspace_id != workspace || evidence.id != id {
            return Err(missing());
        }
        Ok(evidence)
    }

    pub(super) async fn get_handoff(
        &self,
        mission: &str,
        operation: &str,
    ) -> AppResult<AgentHandoff> {
        let body: Option<String> = sqlx::query_scalar(
            "SELECT body_json FROM agent_handoff WHERE mission_id=? AND operation_id=?",
        )
        .bind(mission)
        .bind(operation)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?;
        let missing =
            || AppError::validation("agent_history_not_found", "此协作中没有该交接记录。");
        let handoff: AgentHandoff =
            serde_json::from_str(&body.ok_or_else(missing)?).map_err(database)?;
        if handoff.mission_id != mission || handoff.operation_id != operation {
            return Err(missing());
        }
        Ok(handoff)
    }

    pub(super) async fn handoff_for_dispatch(
        &self,
        mission: &str,
        task: &str,
        dispatch: &str,
    ) -> AppResult<Option<String>> {
        sqlx::query_scalar(
            "SELECT o.id FROM agent_operation o
            JOIN agent_handoff h ON h.operation_id=o.id AND h.mission_id=o.mission_id
            WHERE o.mission_id=? AND o.kind='start-task' AND o.status='succeeded'
                AND json_extract(o.receipt_json,'$.result.dispatchId')=?
                AND json_extract(o.receipt_json,'$.result.taskId')=?
            ORDER BY o.rowid DESC LIMIT 1",
        )
        .bind(mission)
        .bind(dispatch)
        .bind(task)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)
    }

    pub(super) async fn save_evidence(
        &self,
        input: &CaptureAgentEvidenceInput,
        evidence: AgentEvidence,
        raw: &Value,
    ) -> AppResult<AgentEvidence> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(database)?;
        let existing: Option<(String, String)> =
            sqlx::query_as("SELECT body_json, request_json FROM agent_evidence WHERE id = ?")
                .bind(&input.id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database)?;
        if let Some((body, request)) = existing {
            let original: CaptureAgentEvidenceInput =
                serde_json::from_str(&request).map_err(database)?;
            if &original != input {
                return Err(database("输出证据 ID 已用于不同的读取请求"));
            }
            return serde_json::from_str(&body).map_err(database);
        }
        sqlx::query("INSERT INTO agent_evidence(id,workspace_id,mission_id,request_json,body_json,raw_receipt_json) VALUES (?,?,?,?,?,?)")
            .bind(&evidence.id).bind(&evidence.workspace_id).bind(&evidence.mission_id)
            .bind(serde_json::to_string(input).map_err(database)?).bind(serde_json::to_string(&evidence).map_err(database)?)
            .bind(raw.to_string()).execute(&mut *transaction).await.map_err(database)?;
        transaction.commit().await.map_err(database)?;
        Ok(evidence)
    }

    /// Validate the local context and persist exactly what will be dispatched in one write transaction.
    pub(super) async fn begin_handoff(
        &self,
        id: &str,
        mission: &AgentMission,
        request: &Value,
        prepared: super::context::PreparedHandoff,
    ) -> AppResult<AgentOperation> {
        let preview = prepared.preview;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(database)?;
        let cursor = get_context_cursor_in(&mut transaction, &mission.workspace_id)
            .await
            .map_err(database)?;
        let draft: Option<i64> =
            sqlx::query_scalar("SELECT version FROM context_draft WHERE workspace_id = ?")
                .bind(&mission.workspace_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database)?;
        let branch: Option<i64> = if let Some(branch_id) = &preview.branch_id {
            sqlx::query_scalar(
                "SELECT version FROM branch_pointer WHERE id = ? AND workspace_id = ?",
            )
            .bind(branch_id)
            .bind(&mission.workspace_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database)?
        } else {
            None
        };
        let current_system: String =
            sqlx::query_scalar("SELECT system_prompt FROM workspace WHERE id = ?")
                .bind(&mission.workspace_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(database)?;
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM context_checkpoint WHERE workspace_id = ?), (SELECT COUNT(*) FROM branch_checkpoint_inheritance WHERE workspace_id = ?)")
            .bind(&mission.workspace_id).bind(&mission.workspace_id).fetch_one(&mut *transaction).await.map_err(database)?;
        if cursor.version as u64 != preview.cursor_version
            || cursor.active_run_id != preview.parent_run_id
            || cursor.branch_pointer_id != preview.branch_id
            || draft.unwrap_or(0) as u64 != preview.draft_version
            || branch.map(|v| v as u64) != preview.branch_version
            || current_system != prepared.system_prompt
            || counts.0 as usize != prepared.checkpoint_count
            || counts.1 as usize != prepared.inheritance_count
        {
            return Err(super::context::stale());
        }
        let operation = AgentOperation {
            id: id.into(),
            mission_id: mission.id.clone(),
            kind: "start-task".into(),
            status: "pending".into(),
            receipt: Value::Null,
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let handoff = AgentHandoff {
            operation_id: id.into(),
            workspace_id: mission.workspace_id.clone(),
            mission_id: mission.id.clone(),
            created_at: operation.created_at.clone(),
            preview,
        };
        sqlx::query("INSERT INTO agent_operation(id,mission_id,kind,status,request_json,created_at) VALUES (?,?,'start-task','pending',?,?)")
            .bind(id).bind(&mission.id).bind(request.to_string()).bind(&operation.created_at)
            .execute(&mut *transaction).await.map_err(database)?;
        sqlx::query("INSERT INTO agent_handoff(operation_id,workspace_id,mission_id,body_json) VALUES (?,?,?,?)")
            .bind(id).bind(&mission.workspace_id).bind(&mission.id).bind(serde_json::to_string(&handoff).map_err(database)?)
            .execute(&mut *transaction).await.map_err(database)?;
        transaction.commit().await.map_err(database)?;
        Ok(operation)
    }

    pub async fn replied_message_ids(
        &self,
        mission: &str,
    ) -> AppResult<std::collections::HashSet<String>> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT json_extract(request_json, '$.messageId') FROM agent_operation WHERE mission_id = ? AND kind = 'reply' AND status IN ('pending', 'succeeded', 'unknown') AND json_type(request_json, '$.messageId') = 'text'")
            .bind(mission).fetch_all(&self.pool).await.map_err(database)?;
        Ok(ids.into_iter().collect())
    }

    pub async fn unresolved_releases(
        &self,
        mission: &str,
    ) -> AppResult<std::collections::HashSet<String>> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT json_extract(request_json, '$.dispatchId') FROM agent_operation WHERE mission_id = ? AND kind = 'release' AND status IN ('pending', 'unknown') AND json_type(request_json, '$.dispatchId') = 'text'")
            .bind(mission).fetch_all(&self.pool).await.map_err(database)?;
        Ok(ids.into_iter().collect())
    }

    pub async fn begin(
        &self,
        id: &str,
        mission: &str,
        kind: &str,
        request: &Value,
    ) -> AppResult<AgentOperation> {
        let operation = AgentOperation {
            id: id.to_owned(),
            mission_id: mission.to_owned(),
            kind: kind.to_owned(),
            status: "pending".into(),
            receipt: Value::Null,
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        sqlx::query("INSERT INTO agent_operation(id, mission_id, kind, status, request_json, created_at) VALUES (?, ?, ?, 'pending', ?, ?)")
            .bind(id).bind(mission).bind(kind).bind(request.to_string()).bind(&operation.created_at)
            .execute(&self.pool).await.map_err(database)?;
        Ok(operation)
    }

    pub async fn save_operation(&self, operation: &AgentOperation) -> AppResult<()> {
        sqlx::query(
            "UPDATE agent_operation SET status = ?, receipt_json = ?, error = ? WHERE id = ?",
        )
        .bind(&operation.status)
        .bind(operation.receipt.to_string())
        .bind(&operation.error)
        .bind(&operation.id)
        .execute(&self.pool)
        .await
        .map_err(database)?;
        Ok(())
    }

    /// Preserve evidence before finalization, including when the following transaction fails.
    pub async fn record_receipt(&self, operation: &AgentOperation) -> AppResult<()> {
        let changed = sqlx::query("UPDATE agent_operation SET receipt_json = ? WHERE id = ? AND mission_id = ? AND status = 'pending'")
            .bind(operation.receipt.to_string()).bind(&operation.id).bind(&operation.mission_id)
            .execute(&self.pool).await.map_err(database)?.rows_affected();
        if changed != 1 {
            return Err(database("Agent 操作已结束，不能替换回执"));
        }
        Ok(())
    }

    /// One local commit publishes the outcome and the binding proved by that outcome.
    pub async fn finish_operation(
        &self,
        operation: &AgentOperation,
        mission: Option<&AgentMission>,
        binding: Option<(Option<&str>, Option<i64>)>,
    ) -> AppResult<()> {
        if mission.is_some_and(|mission| mission.id != operation.mission_id)
            || (binding.is_some() && mission.is_none())
        {
            return Err(database("Agent 操作和任务绑定不匹配"));
        }
        let mut transaction = self.pool.begin().await.map_err(database)?;
        let changed = sqlx::query("UPDATE agent_operation SET status = ?, receipt_json = ?, error = ? WHERE id = ? AND mission_id = ? AND status = 'pending'")
            .bind(&operation.status).bind(operation.receipt.to_string()).bind(&operation.error)
            .bind(&operation.id).bind(&operation.mission_id).execute(&mut *transaction).await.map_err(database)?.rows_affected();
        if changed != 1 {
            return Err(database("Agent 操作已结束，不能重复提交"));
        }
        if let Some(mission) = mission {
            sqlx::query("UPDATE agent_mission SET body_json = ? WHERE id = ?")
                .bind(serde_json::to_string(mission).map_err(database)?)
                .bind(&mission.id)
                .execute(&mut *transaction)
                .await
                .map_err(database)?;
            if let Some((pane_key, generation)) = binding {
                sqlx::query("UPDATE agent_mission SET coordinator_pane_key = COALESCE(?, coordinator_pane_key), consumer_generation = ? WHERE id = ?")
                    .bind(pane_key).bind(generation).bind(&mission.id).execute(&mut *transaction).await.map_err(database)?;
            }
        }
        transaction.commit().await.map_err(database)
    }
}

fn read_operation(row: &sqlx::sqlite::SqliteRow) -> AppResult<AgentOperation> {
    Ok(AgentOperation {
        id: row.get("id"),
        mission_id: row.get("mission_id"),
        kind: row.get("kind"),
        status: row.get("status"),
        receipt: serde_json::from_str(row.get("receipt_json")).map_err(database)?,
        error: row.get("error"),
        created_at: row.get("created_at"),
    })
}
