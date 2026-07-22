use std::{path::Path, time::Duration};

use sqlx::{
    Row, Sqlite, SqlitePool, Transaction,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};

use super::*;

static MIGRATOR: Migrator = sqlx::migrate!();

/// The single controlled SQLite entry point. The pool intentionally contains
/// one connection in v1 so all writes are serialized without a second writer
/// protocol. WAL can be evaluated later from measured contention.
#[derive(Clone, Debug)]
pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub async fn connect(path: impl AsRef<Path>) -> RepositoryResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_with(options).await
    }

    pub async fn connect_in_memory() -> RepositoryResult<Self> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        Self::connect_with(options).await
    }

    async fn connect_with(options: SqliteConnectOptions) -> RepositoryResult<Self> {
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        // Do not rely solely on connect options: verify the live connection
        // used by the pool has the safety setting enabled.
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await?;
        if foreign_keys != 1 {
            return Err(RepositoryError::InvalidStoredValue(
                "SQLite foreign-key enforcement is disabled".into(),
            ));
        }
        Ok(Self { pool })
    }

    pub async fn schema_info(&self) -> RepositoryResult<SchemaInfo> {
        let version = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1",
        )
        .fetch_one(&self.pool)
        .await?
        .unwrap_or(0);
        let strict_tables = sqlx::query_scalar::<_, String>(
            "SELECT name FROM pragma_table_list \
             WHERE schema = 'main' AND strict = 1 AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(SchemaInfo {
            version,
            strict_tables,
        })
    }

    pub async fn create_workspace(
        &self,
        workspace: &WorkspaceRecord,
    ) -> RepositoryResult<WorkspaceRecord> {
        sqlx::query(
            "INSERT INTO workspace \
             (id, title, goal, system_prompt, created_at, updated_at, archived_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&workspace.id)
        .bind(&workspace.title)
        .bind(&workspace.goal)
        .bind(&workspace.system_prompt)
        .bind(workspace.created_at)
        .bind(workspace.updated_at)
        .bind(workspace.archived_at)
        .execute(&self.pool)
        .await?;
        self.get_workspace(&workspace.id).await
    }

    pub async fn save_workspace(
        &self,
        workspace: &WorkspaceRecord,
    ) -> RepositoryResult<WorkspaceRecord> {
        let row = sqlx::query(
            "INSERT INTO workspace \
             (id, title, goal, system_prompt, created_at, updated_at, archived_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 title = excluded.title, goal = excluded.goal, \
                 system_prompt = excluded.system_prompt, updated_at = excluded.updated_at, \
                 archived_at = excluded.archived_at \
             RETURNING id, title, goal, system_prompt, created_at, updated_at, archived_at",
        )
        .bind(&workspace.id)
        .bind(&workspace.title)
        .bind(&workspace.goal)
        .bind(&workspace.system_prompt)
        .bind(workspace.created_at)
        .bind(workspace.updated_at)
        .bind(workspace.archived_at)
        .fetch_one(&self.pool)
        .await?;
        workspace_from_row(&row)
    }

    pub async fn get_workspace(&self, id: &str) -> RepositoryResult<WorkspaceRecord> {
        let row = sqlx::query(
            "SELECT id, title, goal, system_prompt, created_at, updated_at, archived_at \
             FROM workspace WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("workspace", id))?;
        workspace_from_row(&row)
    }

    pub async fn list_workspaces(
        &self,
        include_archived: bool,
    ) -> RepositoryResult<Vec<WorkspaceRecord>> {
        let rows = sqlx::query(
            "SELECT id, title, goal, system_prompt, created_at, updated_at, archived_at \
             FROM workspace WHERE (? OR archived_at IS NULL) \
             ORDER BY updated_at DESC, id",
        )
        .bind(include_archived)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(workspace_from_row).collect()
    }

    pub async fn update_workspace(
        &self,
        id: &str,
        title: &str,
        goal: &str,
        system_prompt: &str,
        archived_at: Option<i64>,
        updated_at: i64,
    ) -> RepositoryResult<WorkspaceRecord> {
        let result = sqlx::query(
            "UPDATE workspace SET title = ?, goal = ?, system_prompt = ?, archived_at = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(title)
        .bind(goal)
        .bind(system_prompt)
        .bind(archived_at)
        .bind(updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        ensure_changed(result.rows_affected(), "workspace", id)?;
        self.get_workspace(id).await
    }

    pub async fn save_provider_profile(
        &self,
        profile: &ProviderProfileRecord,
    ) -> RepositoryResult<ProviderProfileRecord> {
        sqlx::query(
            "INSERT INTO provider_profile \
             (id, provider_id, name, dialect, base_url, default_model, parameters_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 provider_id = excluded.provider_id, name = excluded.name, dialect = excluded.dialect, \
                 base_url = excluded.base_url, default_model = excluded.default_model, \
                 parameters_json = excluded.parameters_json, updated_at = excluded.updated_at",
        )
        .bind(&profile.id)
        .bind(&profile.provider_id)
        .bind(&profile.name)
        .bind(&profile.dialect)
        .bind(&profile.base_url)
        .bind(&profile.default_model)
        .bind(&profile.parameters_json)
        .bind(profile.created_at)
        .bind(profile.updated_at)
        .execute(&self.pool)
        .await?;
        self.get_provider_profile(&profile.id).await
    }

    pub async fn get_provider_profile(&self, id: &str) -> RepositoryResult<ProviderProfileRecord> {
        let row = sqlx::query(
            "SELECT id, provider_id, name, dialect, base_url, default_model, parameters_json, created_at, updated_at \
             FROM provider_profile WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("provider profile", id))?;
        provider_profile_from_row(&row)
    }

    pub async fn list_provider_profiles(&self) -> RepositoryResult<Vec<ProviderProfileRecord>> {
        let rows = sqlx::query(
            "SELECT id, provider_id, name, dialect, base_url, default_model, parameters_json, created_at, updated_at \
             FROM provider_profile ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(provider_profile_from_row).collect()
    }

    /// Atomically stores everything needed to prove a model request before the
    /// application is allowed to contact a Provider. A failure rolls back the
    /// Turn, Run, immutable receipt and optional branch-head update together.
    pub async fn persist_run_start(&self, bundle: &RunStartBundle) -> RepositoryResult<()> {
        validate_run_start_bundle(bundle)?;
        let mut transaction = self.pool.begin().await?;

        for block in &bundle.content_blocks {
            insert_content_block(&mut transaction, block).await?;
        }

        if let Some(turn) = &bundle.turn {
            sqlx::query(
                "INSERT INTO turn \
                 (id, workspace_id, parent_run_id, prompt_block_id, title, created_at, deleted_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&turn.id)
            .bind(&turn.workspace_id)
            .bind(&turn.parent_run_id)
            .bind(&turn.prompt_block_id)
            .bind(&turn.title)
            .bind(turn.created_at)
            .bind(turn.deleted_at)
            .execute(&mut *transaction)
            .await?;
        } else {
            let existing_workspace =
                sqlx::query_scalar::<_, String>("SELECT workspace_id FROM turn WHERE id = ?")
                    .bind(&bundle.run.turn_id)
                    .fetch_optional(&mut *transaction)
                    .await?
                    .ok_or_else(|| not_found("turn", &bundle.run.turn_id))?;
            if existing_workspace != bundle.run.workspace_id {
                return Err(RepositoryError::InvalidInput(
                    "retry Run and existing Turn must belong to the same workspace".into(),
                ));
            }
        }

        insert_model_run(&mut transaction, &bundle.run).await?;
        insert_manifest(&mut transaction, &bundle.manifest).await?;
        for item in &bundle.context_items {
            insert_manifest_item(&mut transaction, item).await?;
        }
        insert_snapshot(&mut transaction, &bundle.snapshot).await?;
        if let Some(pointer) = &bundle.branch_pointer {
            persist_branch_pointer(&mut transaction, pointer).await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    pub async fn get_turn(&self, id: &str) -> RepositoryResult<TurnRecord> {
        let row = sqlx::query(
            "SELECT t.id, t.workspace_id, t.parent_run_id, t.prompt_block_id, \
                    b.content AS prompt_markdown, t.title, t.created_at, t.deleted_at \
             FROM turn t JOIN content_block b ON b.id = t.prompt_block_id \
             WHERE t.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("turn", id))?;
        turn_from_row(&row)
    }

    pub async fn list_turns(&self, workspace_id: &str) -> RepositoryResult<Vec<TurnRecord>> {
        let rows = sqlx::query(
            "SELECT t.id, t.workspace_id, t.parent_run_id, t.prompt_block_id, \
                    b.content AS prompt_markdown, t.title, t.created_at, t.deleted_at \
             FROM turn t JOIN content_block b ON b.id = t.prompt_block_id \
             WHERE t.workspace_id = ? AND t.deleted_at IS NULL \
             ORDER BY t.created_at, t.id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(turn_from_row).collect()
    }

    pub async fn get_run(&self, id: &str) -> RepositoryResult<ModelRunRecord> {
        let row = sqlx::query(
            "SELECT id, turn_id, workspace_id, provider_profile_id, model, status, \
                    output_markdown, reasoning_markdown, provider_snapshot_json, \
                    usage_json, error_json, created_at, started_at, finished_at, checkpointed_at \
             FROM model_run WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("model run", id))?;
        model_run_from_row(&row)
    }

    pub async fn list_runs_for_turn(&self, turn_id: &str) -> RepositoryResult<Vec<ModelRunRecord>> {
        let rows = sqlx::query(
            "SELECT id, turn_id, workspace_id, provider_profile_id, model, status, \
                    output_markdown, reasoning_markdown, provider_snapshot_json, \
                    usage_json, error_json, created_at, started_at, finished_at, checkpointed_at \
             FROM model_run WHERE turn_id = ? ORDER BY created_at, id",
        )
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(model_run_from_row).collect()
    }

    pub async fn list_runs(&self, workspace_id: &str) -> RepositoryResult<Vec<ModelRunRecord>> {
        let rows = sqlx::query(
            "SELECT id, turn_id, workspace_id, provider_profile_id, model, status, \
                    output_markdown, reasoning_markdown, provider_snapshot_json, \
                    usage_json, error_json, created_at, started_at, finished_at, checkpointed_at \
             FROM model_run WHERE workspace_id = ? ORDER BY created_at, id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(model_run_from_row).collect()
    }

    pub async fn list_content_blocks(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<ContentBlockRecord>> {
        let rows = sqlx::query(
            "SELECT b.id, b.role, b.content, b.content_hash, b.created_at \
             FROM content_block b \
             WHERE EXISTS ( \
                 SELECT 1 FROM turn t \
                 WHERE t.workspace_id = ? AND t.prompt_block_id = b.id \
             ) OR EXISTS ( \
                 SELECT 1 FROM context_manifest_item i \
                 JOIN context_manifest m ON m.id = i.manifest_id \
                 WHERE m.workspace_id = ? AND i.content_block_id = b.id \
             ) \
             ORDER BY b.created_at, b.id",
        )
        .bind(workspace_id)
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(content_block_from_row).collect()
    }

    pub async fn mark_run_connecting(&self, id: &str, started_at: i64) -> RepositoryResult<()> {
        let result = sqlx::query(
            "UPDATE model_run SET status = 'connecting', started_at = ? \
             WHERE id = ? AND status = 'queued'",
        )
        .bind(started_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.ensure_transition_applied(id, result.rows_affected(), "queued", "connecting")
            .await
    }

    pub async fn mark_run_streaming(&self, id: &str, started_at: i64) -> RepositoryResult<()> {
        let result = sqlx::query(
            "UPDATE model_run SET status = 'streaming', started_at = COALESCE(started_at, ?) \
             WHERE id = ? AND status = 'connecting'",
        )
        .bind(started_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.ensure_transition_applied(id, result.rows_affected(), "connecting", "streaming")
            .await
    }

    /// Persists full accumulated buffers. This makes retries after an uncertain
    /// acknowledgement idempotent and avoids one database row per token.
    pub async fn checkpoint_run(
        &self,
        id: &str,
        checkpoint: &RunCheckpoint,
    ) -> RepositoryResult<CheckpointWriteOutcome> {
        let result = sqlx::query(
            "UPDATE model_run SET output_markdown = ?, reasoning_markdown = ?, \
                    usage_json = ?, checkpointed_at = ? \
             WHERE id = ? AND status = 'streaming'",
        )
        .bind(&checkpoint.output_markdown)
        .bind(&checkpoint.reasoning_markdown)
        .bind(&checkpoint.usage_json)
        .bind(checkpoint.checkpointed_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            match self.get_run(id).await {
                Err(RepositoryError::NotFound { .. }) => Err(not_found("model run", id)),
                Ok(run) if run.status.is_terminal() => {
                    Ok(CheckpointWriteOutcome::SkippedTerminal(run.status))
                }
                Ok(run) => Err(RepositoryError::Conflict(format!(
                    "cannot checkpoint Run `{id}` while it is `{}`",
                    run.status.as_str()
                ))),
                Err(error) => Err(error),
            }
        } else {
            Ok(CheckpointWriteOutcome::Saved)
        }
    }

    pub async fn finish_run(&self, id: &str, finish: &RunFinish) -> RepositoryResult<()> {
        if !finish.status.is_terminal() {
            return Err(RepositoryError::InvalidInput(
                "finish_run requires a terminal status".into(),
            ));
        }

        let result = sqlx::query(
            "UPDATE model_run SET status = ?, output_markdown = ?, reasoning_markdown = ?, \
                    usage_json = ?, error_json = ?, finished_at = ?, checkpointed_at = ? \
             WHERE id = ? AND ( \
                 (? = 'completed' AND status = 'streaming') OR \
                 (? IN ('cancelled', 'failed', 'interrupted') \
                     AND status IN ('queued', 'connecting', 'streaming')) \
             )",
        )
        .bind(finish.status.as_str())
        .bind(&finish.output_markdown)
        .bind(&finish.reasoning_markdown)
        .bind(&finish.usage_json)
        .bind(&finish.error_json)
        .bind(finish.finished_at)
        .bind(finish.finished_at)
        .bind(id)
        .bind(finish.status.as_str())
        .bind(finish.status.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            match self.get_run(id).await {
                Err(RepositoryError::NotFound { .. }) => Err(not_found("model run", id)),
                Ok(current) => Err(RepositoryError::Conflict(format!(
                    "cannot finish Run `{id}` from `{}` as `{}`",
                    current.status.as_str(),
                    finish.status.as_str()
                ))),
                Err(error) => Err(error),
            }
        } else {
            Ok(())
        }
    }

    /// Marks only externally-started unfinished Runs as interrupted. Queued
    /// Runs were never sent and remain retryable after restart.
    pub async fn recover_interrupted_runs(&self, recovered_at: i64) -> RepositoryResult<u64> {
        let result = sqlx::query(
            "UPDATE model_run SET status = 'interrupted', finished_at = ?, \
                    error_json = COALESCE(error_json, ?) \
             WHERE status IN ('connecting', 'streaming')",
        )
        .bind(recovered_at)
        .bind(
            r#"{"code":"application_restarted","message":"Run was interrupted by application shutdown"}"#,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn get_run_receipt(&self, run_id: &str) -> RepositoryResult<StoredRunReceipt> {
        let snapshot_row = sqlx::query(
            "SELECT id, run_id, manifest_id, workspace_id, provider_profile_id, provider_id, \
                    template_revision, stream_protocol, auth_placement, auth_header_name, \
                    additional_headers_json, provider, model, base_url, parameters_json, \
                    request_json, canonical_hash, created_at \
             FROM context_snapshot WHERE run_id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("context snapshot for Run", run_id))?;
        let snapshot = snapshot_from_row(&snapshot_row)?;

        let manifest_row = sqlx::query(
            "SELECT id, workspace_id, compiler_version, strategy, estimated_chars, \
                    canonical_hash, warnings_json, created_at \
             FROM context_manifest WHERE id = ?",
        )
        .bind(&snapshot.manifest_id)
        .fetch_one(&self.pool)
        .await?;
        let manifest = manifest_from_row(&manifest_row)?;

        let item_rows = sqlx::query(
            "SELECT i.position, i.source_id, i.source_kind, i.role, i.content_block_id, \
                    b.content, b.content_hash, i.inclusion_reason \
             FROM context_manifest_item i \
             JOIN content_block b ON b.id = i.content_block_id \
             WHERE i.manifest_id = ? ORDER BY i.position",
        )
        .bind(&snapshot.manifest_id)
        .fetch_all(&self.pool)
        .await?;
        let items = item_rows
            .iter()
            .map(stored_context_item_from_row)
            .collect::<RepositoryResult<Vec<_>>>()?;

        Ok(StoredRunReceipt {
            snapshot,
            manifest,
            items,
        })
    }

    pub async fn create_branch_pointer(
        &self,
        pointer: &BranchPointerRecord,
    ) -> RepositoryResult<BranchPointerRecord> {
        let mut transaction = self.pool.begin().await?;
        insert_branch_pointer(&mut transaction, pointer).await?;
        transaction.commit().await?;
        self.get_branch_pointer(&pointer.id).await
    }

    pub async fn advance_branch_pointer(
        &self,
        id: &str,
        expected_version: i64,
        head_run_id: &str,
        updated_at: i64,
    ) -> RepositoryResult<BranchPointerRecord> {
        let result = sqlx::query(
            "UPDATE branch_pointer SET head_run_id = ?, version = version + 1, updated_at = ? \
             WHERE id = ? AND version = ?",
        )
        .bind(head_run_id)
        .bind(updated_at)
        .bind(id)
        .bind(expected_version)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            match self.get_branch_pointer(id).await {
                Err(RepositoryError::NotFound { .. }) => Err(not_found("branch pointer", id)),
                Ok(pointer) => Err(RepositoryError::Conflict(format!(
                    "branch pointer `{id}` is at version {}, expected {expected_version}",
                    pointer.version
                ))),
                Err(error) => Err(error),
            }
        } else {
            self.get_branch_pointer(id).await
        }
    }

    pub async fn get_branch_pointer(&self, id: &str) -> RepositoryResult<BranchPointerRecord> {
        let row = sqlx::query(
            "SELECT id, workspace_id, name, head_run_id, version, created_at, updated_at \
             FROM branch_pointer WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("branch pointer", id))?;
        branch_pointer_from_row(&row)
    }

    pub async fn list_branch_pointers(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<BranchPointerRecord>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, name, head_run_id, version, created_at, updated_at \
             FROM branch_pointer WHERE workspace_id = ? ORDER BY name COLLATE NOCASE, id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(branch_pointer_from_row).collect()
    }

    pub async fn save_decision_mark(
        &self,
        mark: &DecisionMarkRecord,
    ) -> RepositoryResult<DecisionMarkRecord> {
        sqlx::query(
            "INSERT INTO decision_mark \
             (id, workspace_id, run_id, status, reason, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(workspace_id, run_id) DO UPDATE SET \
                 status = excluded.status, reason = excluded.reason, updated_at = excluded.updated_at",
        )
        .bind(&mark.id)
        .bind(&mark.workspace_id)
        .bind(&mark.run_id)
        .bind(&mark.status)
        .bind(&mark.reason)
        .bind(mark.created_at)
        .bind(mark.updated_at)
        .execute(&self.pool)
        .await?;
        self.get_decision_mark(&mark.workspace_id, &mark.run_id)
            .await
    }

    pub async fn get_decision_mark(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> RepositoryResult<DecisionMarkRecord> {
        let row = sqlx::query(
            "SELECT id, workspace_id, run_id, status, reason, created_at, updated_at \
             FROM decision_mark WHERE workspace_id = ? AND run_id = ?",
        )
        .bind(workspace_id)
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("decision mark for Run", run_id))?;
        decision_mark_from_row(&row)
    }

    pub async fn list_decision_marks(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<DecisionMarkRecord>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, run_id, status, reason, created_at, updated_at \
             FROM decision_mark WHERE workspace_id = ? ORDER BY updated_at DESC, id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(decision_mark_from_row).collect()
    }

    pub async fn save_view_state(
        &self,
        state: &ViewStateRecord,
    ) -> RepositoryResult<ViewStateRecord> {
        sqlx::query(
            "INSERT INTO view_state (workspace_id, view_key, state_json, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(workspace_id, view_key) DO UPDATE SET \
                 state_json = excluded.state_json, updated_at = excluded.updated_at",
        )
        .bind(&state.workspace_id)
        .bind(&state.view_key)
        .bind(&state.state_json)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await?;
        self.get_view_state(&state.workspace_id, &state.view_key)
            .await
    }

    pub async fn get_view_state(
        &self,
        workspace_id: &str,
        view_key: &str,
    ) -> RepositoryResult<ViewStateRecord> {
        let row = sqlx::query(
            "SELECT workspace_id, view_key, state_json, updated_at FROM view_state \
             WHERE workspace_id = ? AND view_key = ?",
        )
        .bind(workspace_id)
        .bind(view_key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("view state", view_key))?;
        view_state_from_row(&row)
    }

    async fn ensure_transition_applied(
        &self,
        id: &str,
        rows_affected: u64,
        expected: &str,
        next: &str,
    ) -> RepositoryResult<()> {
        if rows_affected != 0 {
            return Ok(());
        }
        match self.get_run(id).await {
            Err(RepositoryError::NotFound { .. }) => Err(not_found("model run", id)),
            Ok(run) => Err(RepositoryError::Conflict(format!(
                "cannot transition Run `{id}` from `{}`; expected `{expected}` before `{next}`",
                run.status.as_str()
            ))),
            Err(error) => Err(error),
        }
    }
}

fn validate_run_start_bundle(bundle: &RunStartBundle) -> RepositoryResult<()> {
    if bundle.run.status != RunStatusRecord::Queued {
        return Err(RepositoryError::InvalidInput(
            "a new model Run must start in queued status".into(),
        ));
    }
    if bundle.run.workspace_id != bundle.manifest.workspace_id
        || bundle.run.workspace_id != bundle.snapshot.workspace_id
    {
        return Err(RepositoryError::InvalidInput(
            "Run, manifest and snapshot must belong to the same workspace".into(),
        ));
    }
    if bundle.snapshot.run_id != bundle.run.id || bundle.snapshot.manifest_id != bundle.manifest.id
    {
        return Err(RepositoryError::InvalidInput(
            "snapshot must identify the Run and manifest in its start bundle".into(),
        ));
    }
    if bundle.snapshot.canonical_hash != bundle.manifest.canonical_hash {
        return Err(RepositoryError::InvalidInput(
            "snapshot and manifest canonical hashes differ".into(),
        ));
    }
    if bundle.snapshot.provider_id.is_none()
        || bundle.snapshot.template_revision.is_none()
        || bundle.snapshot.stream_protocol.is_none()
        || bundle.snapshot.auth_placement.is_none()
    {
        return Err(RepositoryError::InvalidInput(
            "new context snapshots require resolved Provider Template metadata".into(),
        ));
    }
    if let Some(turn) = &bundle.turn {
        if turn.id != bundle.run.turn_id || turn.workspace_id != bundle.run.workspace_id {
            return Err(RepositoryError::InvalidInput(
                "new Turn and Run identifiers do not agree".into(),
            ));
        }
        if !bundle
            .content_blocks
            .iter()
            .any(|block| block.id == turn.prompt_block_id && block.content == turn.prompt_markdown)
        {
            return Err(RepositoryError::InvalidInput(
                "new Turn prompt must be present as an identical content block".into(),
            ));
        }
    }
    for (expected_position, item) in bundle.context_items.iter().enumerate() {
        if item.manifest_id != bundle.manifest.id
            || item.workspace_id != bundle.manifest.workspace_id
            || item.position != expected_position as i64
        {
            return Err(RepositoryError::InvalidInput(
                "context items must be ordered, contiguous, and owned by the bundle manifest"
                    .into(),
            ));
        }
    }
    if let Some(pointer) = &bundle.branch_pointer {
        if pointer.workspace_id != bundle.run.workspace_id || pointer.head_run_id != bundle.run.id {
            return Err(RepositoryError::InvalidInput(
                "new branch pointer must point to the Run in its workspace".into(),
            ));
        }
    }
    Ok(())
}

async fn insert_content_block(
    transaction: &mut Transaction<'_, Sqlite>,
    block: &ContentBlockRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO content_block (id, role, content, content_hash, created_at) \
         VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(&block.id)
    .bind(&block.role)
    .bind(&block.content)
    .bind(&block.content_hash)
    .bind(block.created_at)
    .execute(&mut **transaction)
    .await?;

    let row = sqlx::query(
        "SELECT id, role, content, content_hash, created_at FROM content_block WHERE id = ?",
    )
    .bind(&block.id)
    .fetch_one(&mut **transaction)
    .await?;
    let stored = content_block_from_row(&row)?;
    if stored.id != block.id
        || stored.role != block.role
        || stored.content != block.content
        || stored.content_hash != block.content_hash
    {
        return Err(RepositoryError::Conflict(format!(
            "content block id `{}` already contains different immutable content",
            block.id
        )));
    }
    Ok(())
}

async fn insert_model_run(
    transaction: &mut Transaction<'_, Sqlite>,
    run: &ModelRunRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO model_run \
         (id, turn_id, workspace_id, provider_profile_id, model, status, output_markdown, \
          reasoning_markdown, provider_snapshot_json, usage_json, error_json, created_at, \
          started_at, finished_at, checkpointed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&run.id)
    .bind(&run.turn_id)
    .bind(&run.workspace_id)
    .bind(&run.provider_profile_id)
    .bind(&run.model)
    .bind(run.status.as_str())
    .bind(&run.output_markdown)
    .bind(&run.reasoning_markdown)
    .bind(&run.provider_snapshot_json)
    .bind(&run.usage_json)
    .bind(&run.error_json)
    .bind(run.created_at)
    .bind(run.started_at)
    .bind(run.finished_at)
    .bind(run.checkpointed_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_manifest(
    transaction: &mut Transaction<'_, Sqlite>,
    manifest: &ContextManifestRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO context_manifest \
         (id, workspace_id, compiler_version, strategy, estimated_chars, canonical_hash, warnings_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&manifest.id)
    .bind(&manifest.workspace_id)
    .bind(&manifest.compiler_version)
    .bind(&manifest.strategy)
    .bind(manifest.estimated_chars)
    .bind(&manifest.canonical_hash)
    .bind(&manifest.warnings_json)
    .bind(manifest.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_manifest_item(
    transaction: &mut Transaction<'_, Sqlite>,
    item: &RunContextItemRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO context_manifest_item \
         (manifest_id, workspace_id, position, source_id, source_kind, role, content_block_id, inclusion_reason) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item.manifest_id)
    .bind(&item.workspace_id)
    .bind(item.position)
    .bind(&item.source_id)
    .bind(&item.source_kind)
    .bind(&item.role)
    .bind(&item.content_block_id)
    .bind(&item.inclusion_reason)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    snapshot: &ContextSnapshotRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO context_snapshot \
         (id, run_id, manifest_id, workspace_id, provider_profile_id, provider_id, \
          template_revision, stream_protocol, auth_placement, auth_header_name, \
          additional_headers_json, provider, model, base_url, parameters_json, request_json, \
          canonical_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&snapshot.id)
    .bind(&snapshot.run_id)
    .bind(&snapshot.manifest_id)
    .bind(&snapshot.workspace_id)
    .bind(&snapshot.provider_profile_id)
    .bind(&snapshot.provider_id)
    .bind(snapshot.template_revision)
    .bind(&snapshot.stream_protocol)
    .bind(&snapshot.auth_placement)
    .bind(&snapshot.auth_header_name)
    .bind(&snapshot.additional_headers_json)
    .bind(&snapshot.provider)
    .bind(&snapshot.model)
    .bind(&snapshot.base_url)
    .bind(&snapshot.parameters_json)
    .bind(&snapshot.request_json)
    .bind(&snapshot.canonical_hash)
    .bind(snapshot.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn insert_branch_pointer(
    transaction: &mut Transaction<'_, Sqlite>,
    pointer: &BranchPointerRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO branch_pointer \
         (id, workspace_id, name, head_run_id, version, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&pointer.id)
    .bind(&pointer.workspace_id)
    .bind(&pointer.name)
    .bind(&pointer.head_run_id)
    .bind(pointer.version)
    .bind(pointer.created_at)
    .bind(pointer.updated_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn persist_branch_pointer(
    transaction: &mut Transaction<'_, Sqlite>,
    pointer: &BranchPointerRecord,
) -> RepositoryResult<()> {
    let result = sqlx::query(
        "INSERT INTO branch_pointer \
         (id, workspace_id, name, head_run_id, version, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET \
             head_run_id = excluded.head_run_id, \
             version = excluded.version, \
             updated_at = excluded.updated_at \
         WHERE branch_pointer.workspace_id = excluded.workspace_id \
           AND branch_pointer.name = excluded.name \
           AND excluded.version = branch_pointer.version + 1",
    )
    .bind(&pointer.id)
    .bind(&pointer.workspace_id)
    .bind(&pointer.name)
    .bind(&pointer.head_run_id)
    .bind(pointer.version)
    .bind(pointer.created_at)
    .bind(pointer.updated_at)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 0 {
        Err(RepositoryError::Conflict(format!(
            "branch pointer `{}` identity or version does not match its stored value",
            pointer.id
        )))
    } else {
        Ok(())
    }
}

fn not_found(entity: &'static str, id: &str) -> RepositoryError {
    RepositoryError::NotFound {
        entity,
        id: id.into(),
    }
}

fn ensure_changed(rows_affected: u64, entity: &'static str, id: &str) -> RepositoryResult<()> {
    if rows_affected == 0 {
        Err(not_found(entity, id))
    } else {
        Ok(())
    }
}

fn workspace_from_row(row: &SqliteRow) -> RepositoryResult<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        goal: row.try_get("goal")?,
        system_prompt: row.try_get("system_prompt")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        archived_at: row.try_get("archived_at")?,
    })
}

fn provider_profile_from_row(row: &SqliteRow) -> RepositoryResult<ProviderProfileRecord> {
    Ok(ProviderProfileRecord {
        id: row.try_get("id")?,
        provider_id: row.try_get("provider_id")?,
        name: row.try_get("name")?,
        dialect: row.try_get("dialect")?,
        base_url: row.try_get("base_url")?,
        default_model: row.try_get("default_model")?,
        parameters_json: row.try_get("parameters_json")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn content_block_from_row(row: &SqliteRow) -> RepositoryResult<ContentBlockRecord> {
    Ok(ContentBlockRecord {
        id: row.try_get("id")?,
        role: row.try_get("role")?,
        content: row.try_get("content")?,
        content_hash: row.try_get("content_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

fn turn_from_row(row: &SqliteRow) -> RepositoryResult<TurnRecord> {
    Ok(TurnRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        parent_run_id: row.try_get("parent_run_id")?,
        prompt_block_id: row.try_get("prompt_block_id")?,
        prompt_markdown: row.try_get("prompt_markdown")?,
        title: row.try_get("title")?,
        created_at: row.try_get("created_at")?,
        deleted_at: row.try_get("deleted_at")?,
    })
}

fn model_run_from_row(row: &SqliteRow) -> RepositoryResult<ModelRunRecord> {
    let status: String = row.try_get("status")?;
    Ok(ModelRunRecord {
        id: row.try_get("id")?,
        turn_id: row.try_get("turn_id")?,
        workspace_id: row.try_get("workspace_id")?,
        provider_profile_id: row.try_get("provider_profile_id")?,
        model: row.try_get("model")?,
        status: RunStatusRecord::from_stored(&status)?,
        output_markdown: row.try_get("output_markdown")?,
        reasoning_markdown: row.try_get("reasoning_markdown")?,
        provider_snapshot_json: row.try_get("provider_snapshot_json")?,
        usage_json: row.try_get("usage_json")?,
        error_json: row.try_get("error_json")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        checkpointed_at: row.try_get("checkpointed_at")?,
    })
}

fn snapshot_from_row(row: &SqliteRow) -> RepositoryResult<ContextSnapshotRecord> {
    Ok(ContextSnapshotRecord {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        manifest_id: row.try_get("manifest_id")?,
        workspace_id: row.try_get("workspace_id")?,
        provider_profile_id: row.try_get("provider_profile_id")?,
        provider_id: row.try_get("provider_id")?,
        template_revision: row.try_get("template_revision")?,
        stream_protocol: row.try_get("stream_protocol")?,
        auth_placement: row.try_get("auth_placement")?,
        auth_header_name: row.try_get("auth_header_name")?,
        additional_headers_json: row.try_get("additional_headers_json")?,
        provider: row.try_get("provider")?,
        model: row.try_get("model")?,
        base_url: row.try_get("base_url")?,
        parameters_json: row.try_get("parameters_json")?,
        request_json: row.try_get("request_json")?,
        canonical_hash: row.try_get("canonical_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

fn manifest_from_row(row: &SqliteRow) -> RepositoryResult<ContextManifestRecord> {
    Ok(ContextManifestRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        compiler_version: row.try_get("compiler_version")?,
        strategy: row.try_get("strategy")?,
        estimated_chars: row.try_get("estimated_chars")?,
        canonical_hash: row.try_get("canonical_hash")?,
        warnings_json: row.try_get("warnings_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn stored_context_item_from_row(row: &SqliteRow) -> RepositoryResult<StoredContextItem> {
    Ok(StoredContextItem {
        position: row.try_get("position")?,
        source_id: row.try_get("source_id")?,
        source_kind: row.try_get("source_kind")?,
        role: row.try_get("role")?,
        content_block_id: row.try_get("content_block_id")?,
        content: row.try_get("content")?,
        content_hash: row.try_get("content_hash")?,
        inclusion_reason: row.try_get("inclusion_reason")?,
    })
}

fn branch_pointer_from_row(row: &SqliteRow) -> RepositoryResult<BranchPointerRecord> {
    Ok(BranchPointerRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        name: row.try_get("name")?,
        head_run_id: row.try_get("head_run_id")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn decision_mark_from_row(row: &SqliteRow) -> RepositoryResult<DecisionMarkRecord> {
    Ok(DecisionMarkRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        run_id: row.try_get("run_id")?,
        status: row.try_get("status")?,
        reason: row.try_get("reason")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn view_state_from_row(row: &SqliteRow) -> RepositoryResult<ViewStateRecord> {
    Ok(ViewStateRecord {
        workspace_id: row.try_get("workspace_id")?,
        view_key: row.try_get("view_key")?,
        state_json: row.try_get("state_json")?,
        updated_at: row.try_get("updated_at")?,
    })
}
