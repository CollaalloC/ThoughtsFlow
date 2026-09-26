use serde_json::Value;
use sqlx::{Row, SqlitePool};

use super::{AgentMission, AgentOperation};
use crate::application::{AppError, AppResult};

#[derive(Clone)]
pub(crate) struct AgentStore {
    pool: SqlitePool,
}

fn database(error: impl std::fmt::Display) -> AppError {
    AppError::internal("agent_storage", format!("Agent 工作台存储失败：{error}"))
}

impl AgentStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
