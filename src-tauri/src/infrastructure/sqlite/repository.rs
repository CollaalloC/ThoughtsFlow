use std::{
    collections::BTreeMap,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use sqlx::{
    Row, Sqlite, SqliteConnection, SqlitePool, Transaction,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};

use super::*;

static MIGRATOR: Migrator = sqlx::migrate!();

static CONTEXT_LOGICAL_READS: AtomicUsize = AtomicUsize::new(0);

/// The single controlled SQLite entry point. The pool intentionally contains
/// one connection in v1 so all writes are serialized without a second writer
/// protocol. WAL can be evaluated later from measured contention.
#[derive(Clone, Debug)]
pub struct SqliteRepository {
    pool: SqlitePool,
    count_context_reads: bool,
}

impl SqliteRepository {
    pub(crate) fn agent_store(&self) -> crate::agents::store::AgentStore {
        crate::agents::store::AgentStore::new(self.pool.clone())
    }

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

    /// Opens an isolated test database whose workspace context projection
    /// counts logical SELECTs. Ordinary production connections never count.
    #[doc(hidden)]
    pub async fn connect_in_memory_with_context_read_probe() -> RepositoryResult<Self> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let mut repository = Self::connect_with_pool_options(options, Self::pool_options()).await?;
        repository.count_context_reads = true;
        Ok(repository)
    }

    #[doc(hidden)]
    pub fn reset_context_read_count() {
        CONTEXT_LOGICAL_READS.store(0, Ordering::Relaxed);
    }

    #[doc(hidden)]
    pub fn context_read_count() -> usize {
        CONTEXT_LOGICAL_READS.load(Ordering::Relaxed)
    }

    fn record_context_read(&self) {
        if self.count_context_reads {
            CONTEXT_LOGICAL_READS.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg(test)]
    pub(crate) async fn acquire_test_connection(
        &self,
    ) -> Result<sqlx::pool::PoolConnection<Sqlite>, sqlx::Error> {
        self.pool.acquire().await
    }

    async fn connect_with(options: SqliteConnectOptions) -> RepositoryResult<Self> {
        Self::connect_with_pool_options(options, Self::pool_options()).await
    }

    fn pool_options() -> SqlitePoolOptions {
        SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
    }

    async fn connect_with_pool_options(
        options: SqliteConnectOptions,
        pool_options: SqlitePoolOptions,
    ) -> RepositoryResult<Self> {
        let pool = pool_options.connect_with(options).await?;
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
        Ok(Self {
            pool,
            count_context_reads: false,
        })
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
             (id, provider_id, name, dialect, protocol_dialect, base_url, default_model, parameters_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 provider_id = excluded.provider_id, name = excluded.name, \
                 dialect = excluded.dialect, protocol_dialect = excluded.protocol_dialect, \
                 base_url = excluded.base_url, default_model = excluded.default_model, \
                 parameters_json = excluded.parameters_json, updated_at = excluded.updated_at",
        )
        .bind(&profile.id)
        .bind(&profile.provider_id)
        .bind(&profile.name)
        .bind(legacy_profile_dialect(&profile.dialect))
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
            "SELECT id, provider_id, name, protocol_dialect AS dialect, base_url, default_model, parameters_json, created_at, updated_at \
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
            "SELECT id, provider_id, name, protocol_dialect AS dialect, base_url, default_model, parameters_json, created_at, updated_at \
             FROM provider_profile ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(provider_profile_from_row).collect()
    }

    /// Atomically stores everything needed to prove a model request before the
    /// application is allowed to contact a Provider. A failure rolls back the
    /// Turn, Run, immutable receipt and optional branch-head update together.
    pub async fn persist_run_start(
        &self,
        bundle: &RunStartBundle,
    ) -> RepositoryResult<RunStartOutcomeRecord> {
        validate_run_start_bundle(bundle)?;
        let mut transaction = self.pool.begin().await?;
        if let Some(update) = &bundle.context_update {
            validate_run_start_branch_guard(&mut transaction, &bundle.run.workspace_id, update)
                .await?;
        }

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
        let creates_branch = match (&bundle.branch_pointer, &bundle.context_update) {
            (Some(pointer), Some(update))
                if update.expected_branch_pointer_id.as_deref() != Some(pointer.id.as_str()) =>
            {
                let already_exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branch_pointer WHERE id = ?)")
                        .bind(&pointer.id)
                        .fetch_one(&mut *transaction)
                        .await?;
                if already_exists {
                    return Err(RepositoryError::Conflict(format!(
                        "historical fork cannot reuse existing branch pointer `{}`",
                        pointer.id
                    )));
                }
                true
            }
            _ => false,
        };
        if let Some(pointer) = &bundle.branch_pointer {
            persist_branch_pointer(&mut transaction, pointer).await?;
        }
        if creates_branch {
            let update = bundle
                .context_update
                .as_ref()
                .expect("a context-managed branch creation has an update");
            if let (Some(source_branch_id), Some(result_branch_id)) = (
                update.expected_branch_pointer_id.as_deref(),
                update.result_branch_pointer_id.as_deref(),
            ) {
                copy_branch_checkpoint_inheritance(
                    &mut transaction,
                    &bundle.run.workspace_id,
                    source_branch_id,
                    result_branch_id,
                    update.updated_at,
                )
                .await?;
            }
        }
        if let Some(update) = &bundle.context_update {
            apply_context_cursor_update(
                &mut transaction,
                &ContextCursorUpdateRecord {
                    workspace_id: bundle.run.workspace_id.clone(),
                    active_run_id: Some(bundle.run.id.clone()),
                    branch_pointer_id: update.result_branch_pointer_id.clone(),
                    expected_version: update.expected_cursor_version,
                    updated_at: update.updated_at,
                },
            )
            .await?;
            consume_context_draft(
                &mut transaction,
                &bundle.run.workspace_id,
                &bundle.run.turn_id,
                &bundle.run.id,
                update.expected_draft_version,
                update.updated_at,
            )
            .await?;
        }
        let cursor = get_context_cursor_in(&mut transaction, &bundle.run.workspace_id).await?;
        let draft_version =
            get_context_draft_version_in(&mut transaction, &bundle.run.workspace_id).await?;
        let branch_pointer = match bundle
            .context_update
            .as_ref()
            .and_then(|update| update.result_branch_pointer_id.as_deref())
        {
            Some(id) => Some(get_branch_pointer_in(&mut transaction, id).await?),
            None => None,
        };
        transaction.commit().await?;
        Ok(RunStartOutcomeRecord {
            cursor,
            draft_version,
            branch_pointer,
        })
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
        let mut connection = self.pool.acquire().await?;
        list_turns_on(&mut connection, workspace_id).await
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

    pub async fn list_run_provider_provenance(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<RunProviderProvenanceRecord>> {
        let rows = sqlx::query(
            "SELECT run_id, provider AS provider_name, base_url, model \
             FROM context_snapshot WHERE workspace_id = ? ORDER BY created_at, run_id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_provider_provenance_from_row).collect()
    }

    pub async fn list_content_blocks(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<ContentBlockRecord>> {
        let mut connection = self.pool.acquire().await?;
        list_content_blocks_on(&mut connection, workspace_id).await
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
                    canonical_hash, warnings_json, checkpoint_provenance_json, \
                    branch_summary_provenance_json, created_at \
             FROM context_manifest WHERE id = ?",
        )
        .bind(&snapshot.manifest_id)
        .fetch_one(&self.pool)
        .await?;
        let manifest = manifest_from_row(&manifest_row)?;

        let item_rows = sqlx::query(
            "SELECT i.position, i.source_id, i.source_ref_kind, i.source_ref_id, \
                    i.source_kind, i.role, i.content_block_id, b.content, b.content_hash, \
                    i.inclusion_reason, i.mandatory \
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
        let stored = get_branch_pointer_in(&mut transaction, &pointer.id).await?;
        transaction.commit().await?;
        Ok(stored)
    }

    pub async fn advance_branch_pointer(
        &self,
        id: &str,
        expected_version: i64,
        head_run_id: &str,
        updated_at: i64,
    ) -> RepositoryResult<BranchPointerRecord> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE branch_pointer SET head_run_id = ?, version = version + 1, updated_at = ? \
             WHERE id = ? AND version = ?",
        )
        .bind(head_run_id)
        .bind(updated_at)
        .bind(id)
        .bind(expected_version)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return match get_branch_pointer_in(&mut transaction, id).await {
                Err(RepositoryError::NotFound { .. }) => Err(not_found("branch pointer", id)),
                Ok(pointer) => Err(RepositoryError::Conflict(format!(
                    "branch pointer `{id}` is at version {}, expected {expected_version}",
                    pointer.version
                ))),
                Err(error) => Err(error),
            };
        }
        let stored = get_branch_pointer_in(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(stored)
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

    /// Returns the persisted cursor, or a deterministic read-only v4 fallback.
    /// A fallback is version 0 and is never materialized by this read.
    pub async fn get_context_cursor(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<ContextCursorRecord> {
        let row = sqlx::query(
            "WITH stored AS ( \
                 SELECT workspace_id, active_run_id, branch_pointer_id, version, updated_at \
                 FROM workspace_context_cursor WHERE workspace_id = ? \
             ), fallback_branch AS ( \
                 SELECT w.id AS workspace_id, b.head_run_id AS active_run_id, \
                        b.id AS branch_pointer_id, 0 AS version, b.updated_at AS updated_at \
                 FROM workspace w JOIN branch_pointer b ON b.workspace_id = w.id \
                 WHERE w.id = ? ORDER BY b.updated_at DESC, b.id DESC LIMIT 1 \
             ), fallback_run AS ( \
                 SELECT w.id AS workspace_id, r.id AS active_run_id, \
                        NULL AS branch_pointer_id, 0 AS version, r.created_at AS updated_at \
                 FROM workspace w JOIN model_run r ON r.workspace_id = w.id \
                 WHERE w.id = ? ORDER BY r.created_at DESC, r.id DESC LIMIT 1 \
             ) \
             SELECT * FROM stored \
             UNION ALL SELECT * FROM fallback_branch \
               WHERE NOT EXISTS (SELECT 1 FROM stored) \
             UNION ALL SELECT * FROM fallback_run \
               WHERE NOT EXISTS (SELECT 1 FROM stored) \
                 AND NOT EXISTS (SELECT 1 FROM fallback_branch) \
             UNION ALL \
             SELECT w.id, NULL, NULL, 0, w.updated_at FROM workspace w \
               WHERE w.id = ? \
                 AND NOT EXISTS (SELECT 1 FROM stored) \
                 AND NOT EXISTS (SELECT 1 FROM fallback_branch) \
                 AND NOT EXISTS (SELECT 1 FROM fallback_run) \
             LIMIT 1",
        )
        .bind(workspace_id)
        .bind(workspace_id)
        .bind(workspace_id)
        .bind(workspace_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("workspace", workspace_id))?;
        context_cursor_from_row(&row)
    }

    pub async fn set_context_cursor(
        &self,
        update: &ContextCursorUpdateRecord,
    ) -> RepositoryResult<ContextCursorRecord> {
        validate_context_cursor_update(update)?;
        let mut transaction = self.pool.begin().await?;
        apply_context_cursor_update(&mut transaction, update).await?;
        let cursor = get_context_cursor_in(&mut transaction, &update.workspace_id).await?;
        transaction.commit().await?;
        Ok(cursor)
    }

    pub async fn set_context_cursor_and_rebase_draft(
        &self,
        update: &ContextCursorUpdateRecord,
        expected_draft_version: i64,
    ) -> RepositoryResult<ContextCursorRecord> {
        validate_context_cursor_update(update)?;
        let draft_update = ContextDraftUpdateRecord {
            workspace_id: update.workspace_id.clone(),
            parent_run_id: update.active_run_id.clone(),
            expected_version: expected_draft_version,
            content_blocks: Vec::new(),
            items: Vec::new(),
            updated_at: update.updated_at,
        };
        validate_context_draft_update(&draft_update)?;

        let mut transaction = self.pool.begin().await?;
        apply_context_cursor_update(&mut transaction, update).await?;
        let cursor = get_context_cursor_in(&mut transaction, &update.workspace_id).await?;
        apply_context_draft_update(&mut transaction, &draft_update).await?;
        transaction.commit().await?;
        Ok(cursor)
    }

    pub async fn get_context_draft(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<ContextDraftRecord> {
        let mut transaction = self.pool.begin().await?;
        let cursor = get_context_cursor_in(&mut transaction, workspace_id).await?;
        let draft = load_context_draft_state_on(&mut transaction, workspace_id, &cursor).await?;
        transaction.commit().await?;
        Ok(draft)
    }

    pub async fn update_context_draft(
        &self,
        update: &ContextDraftUpdateRecord,
    ) -> RepositoryResult<ContextDraftRecord> {
        validate_context_draft_update(update)?;
        let mut transaction = self.pool.begin().await?;
        for block in &update.content_blocks {
            insert_content_block(&mut transaction, block).await?;
        }
        let version = apply_context_draft_update(&mut transaction, update).await?;
        let draft = ContextDraftRecord {
            workspace_id: update.workspace_id.clone(),
            parent_run_id: update.parent_run_id.clone(),
            version,
            items: update.items.clone(),
            consumed_by_run_id: None,
            updated_at: update.updated_at,
        };
        transaction.commit().await?;
        Ok(draft)
    }

    pub async fn start_context_maintenance(
        &self,
        run: &ContextMaintenanceRunRecord,
        guard: &MaintenanceContextGuardRecord,
    ) -> RepositoryResult<(ContextMaintenanceRunRecord, bool)> {
        validate_context_maintenance_start(run, guard)?;
        let mut transaction = self.pool.begin().await?;
        if let Some(existing) = find_context_maintenance_run_in(&mut transaction, &run.id).await? {
            validate_context_maintenance_start_identity(&existing, run)?;
            return Ok((existing, false));
        }
        let guard_error = validate_maintenance_guard(&mut transaction, guard, &run.anchor_run_id)
            .await
            .err();
        let mut stored = run.clone();
        if let Some(error) = guard_error.as_ref() {
            stored.status = "conflicted".into();
            stored.summary_block_id = None;
            stored.summary = None;
            stored.error_json = Some(version_conflict_json(error));
            stored.finished_at = Some(run.finished_at.unwrap_or(run.created_at));
        }
        insert_context_maintenance_run(&mut transaction, &stored).await?;
        transaction.commit().await?;
        if let Some(error) = guard_error {
            return Err(error);
        }
        Ok((stored, true))
    }

    pub async fn finish_context_maintenance(
        &self,
        run: &ContextMaintenanceRunRecord,
        checkpoint: Option<&ContextCheckpointRecord>,
        summary_block: Option<&ContentBlockRecord>,
        guard: &MaintenanceContextGuardRecord,
        context_update: Option<&FinishContextMaintenanceUpdateRecord>,
    ) -> RepositoryResult<ContextMaintenanceRunRecord> {
        validate_context_maintenance_finish(run, checkpoint, summary_block, guard, context_update)?;
        let mut transaction = self.pool.begin().await?;
        let existing = get_context_maintenance_run_in(&mut transaction, &run.id).await?;
        validate_context_maintenance_identity(&existing, run)?;
        if matches!(
            existing.status.as_str(),
            "completed" | "failed" | "cancelled" | "conflicted"
        ) {
            validate_terminal_maintenance_replay(
                &mut transaction,
                &existing,
                run,
                checkpoint,
                summary_block,
            )
            .await?;
            return Ok(existing);
        }

        if run.status == "completed" {
            if let Err(error) =
                validate_maintenance_guard(&mut transaction, guard, &run.anchor_run_id).await
            {
                mark_context_maintenance_conflicted(
                    &mut transaction,
                    &run.id,
                    run.finished_at.unwrap_or(run.created_at),
                    &error,
                )
                .await?;
                transaction.commit().await?;
                return Err(error);
            }

            let checkpoint = checkpoint.expect("validated completed maintenance has checkpoint");
            let summary_block =
                summary_block.expect("validated completed maintenance has summary block");
            insert_content_block(&mut transaction, summary_block).await?;
            finish_context_maintenance_row(&mut transaction, run, Some(&summary_block.id)).await?;
            let stored_checkpoint = insert_context_checkpoint(&mut transaction, checkpoint).await?;
            if let Some(branch_pointer_id) = stored_checkpoint.branch_pointer_id.as_deref() {
                insert_branch_checkpoint_inheritance(
                    &mut transaction,
                    &stored_checkpoint.workspace_id,
                    branch_pointer_id,
                    &stored_checkpoint.id,
                    stored_checkpoint.created_at,
                )
                .await?;
            }
            if let Some(update) = context_update {
                apply_context_cursor_update(
                    &mut transaction,
                    &ContextCursorUpdateRecord {
                        workspace_id: guard.workspace_id.clone(),
                        active_run_id: update.active_run_id.clone(),
                        branch_pointer_id: update.branch_pointer_id.clone(),
                        expected_version: guard.expected_cursor_version,
                        updated_at: update.updated_at,
                    },
                )
                .await?;
            }
        } else {
            finish_context_maintenance_row(&mut transaction, run, None).await?;
        }
        let stored = get_context_maintenance_run_in(&mut transaction, &run.id).await?;
        transaction.commit().await?;
        Ok(stored)
    }

    pub async fn get_context_maintenance_run(
        &self,
        id: &str,
    ) -> RepositoryResult<ContextMaintenanceRunRecord> {
        let row = sqlx::query(
            "SELECT m.id, m.workspace_id, m.kind, m.status, m.branch_pointer_id, \
                    m.branch_revision, m.anchor_run_id, m.first_kept_run_id, \
                    m.source_run_ids_json, m.source_hash, m.provider_snapshot_json, \
                    m.request_json, m.summary_block_id, b.content AS summary, m.error_json, \
                    m.created_at, m.started_at, m.finished_at \
             FROM context_maintenance_run m \
             LEFT JOIN content_block b ON b.id = m.summary_block_id \
             WHERE m.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("context maintenance run", id))?;
        context_maintenance_run_from_row(&row)
    }

    pub async fn get_context_checkpoint_for_maintenance(
        &self,
        maintenance_run_id: &str,
    ) -> RepositoryResult<Option<ContextCheckpointRecord>> {
        let row = sqlx::query(
            "SELECT c.id, c.workspace_id, c.maintenance_run_id, c.kind, \
                    c.branch_pointer_id, c.branch_revision, c.anchor_run_id, \
                    c.first_kept_run_id, c.summary_block_id, b.content AS summary, \
                    b.content_hash AS summary_content_hash, c.source_run_ids_json, \
                    c.source_hash, m.provider_snapshot_json, c.created_at \
             FROM context_checkpoint c \
             JOIN content_block b ON b.id = c.summary_block_id \
             JOIN context_maintenance_run m ON m.id = c.maintenance_run_id \
             WHERE c.maintenance_run_id = ?",
        )
        .bind(maintenance_run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(context_checkpoint_from_row).transpose()
    }

    pub async fn recover_interrupted_context_maintenance(
        &self,
        recovered_at: i64,
    ) -> RepositoryResult<u64> {
        let result = sqlx::query(
            "UPDATE context_maintenance_run \
             SET status = 'failed', \
                 error_json = '{\"code\":\"application_restarted\",\
                                  \"message\":\"Context maintenance was interrupted by application shutdown\"}', \
                 finished_at = MAX(created_at, ?) \
             WHERE status = 'running'",
        )
        .bind(recovered_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_context_maintenance_runs(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<ContextMaintenanceRunRecord>> {
        let rows = sqlx::query(
            "SELECT m.id, m.workspace_id, m.kind, m.status, m.branch_pointer_id, \
                    m.branch_revision, m.anchor_run_id, m.first_kept_run_id, \
                    m.source_run_ids_json, m.source_hash, m.provider_snapshot_json, \
                    m.request_json, m.summary_block_id, b.content AS summary, m.error_json, \
                    m.created_at, m.started_at, m.finished_at \
             FROM context_maintenance_run m \
             LEFT JOIN content_block b ON b.id = m.summary_block_id \
             WHERE m.workspace_id = ? ORDER BY m.created_at, m.id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(context_maintenance_run_from_row).collect()
    }

    pub async fn list_context_checkpoints(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<ContextCheckpointRecord>> {
        let mut connection = self.pool.acquire().await?;
        list_context_checkpoints_on(&mut connection, workspace_id).await
    }

    /// Eight bounded reads hydrate everything needed for a Run-level tree and
    /// compiler preview. No Receipt is fetched per node.
    pub async fn load_workspace_context_records(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<WorkspaceContextRecords> {
        let mut transaction = self.pool.begin().await?;

        self.record_context_read();
        let cursor = get_context_cursor_in(&mut transaction, workspace_id).await?; // 1
        self.record_context_read();
        let turns = list_turns_on(&mut transaction, workspace_id).await?; // 2
        self.record_context_read();
        let (runs, run_provider_provenance) =
            list_runs_with_provenance_on(&mut transaction, workspace_id).await?; // 3
        self.record_context_read();
        let content_blocks = list_content_blocks_on(&mut transaction, workspace_id).await?; // 4
        self.record_context_read();
        let (branch_pointers, branch_revisions, branch_checkpoint_inheritance) =
            load_branch_state_on(&mut transaction, workspace_id).await?; // 5
        self.record_context_read();
        let draft = load_context_draft_state_on(&mut transaction, workspace_id, &cursor).await?; // 6
        self.record_context_read();
        let checkpoints = list_context_checkpoints_on(&mut transaction, workspace_id).await?; // 7
        self.record_context_read();
        let view_states = list_view_states_on(&mut transaction, workspace_id).await?; // 8
        let records = WorkspaceContextRecords {
            turns,
            runs,
            content_blocks,
            run_provider_provenance,
            cursor,
            branch_pointers,
            branch_revisions,
            branch_checkpoint_inheritance,
            draft,
            checkpoints,
            view_states,
        };
        transaction.commit().await?;
        Ok(records)
    }

    pub async fn list_branch_revisions(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<BranchRevisionRecord>> {
        let rows = sqlx::query(
            "SELECT branch_pointer_id, workspace_id, revision, name, head_run_id, \
                    change_kind, created_at \
             FROM branch_revision WHERE workspace_id = ? \
             ORDER BY branch_pointer_id, revision",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(branch_revision_from_row).collect()
    }

    pub async fn list_branch_checkpoint_inheritance(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<BranchCheckpointInheritanceRecord>> {
        let rows = sqlx::query(
            "SELECT workspace_id, branch_pointer_id, checkpoint_id, inherited_at \
             FROM branch_checkpoint_inheritance WHERE workspace_id = ? \
             ORDER BY branch_pointer_id, checkpoint_id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(branch_checkpoint_inheritance_from_row)
            .collect()
    }

    pub async fn rename_branch(
        &self,
        branch_pointer_id: &str,
        name: &str,
        expected_version: i64,
        updated_at: i64,
    ) -> RepositoryResult<BranchPointerRecord> {
        if name.trim().is_empty() {
            return Err(RepositoryError::InvalidInput(
                "branch name must not be empty".into(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE branch_pointer \
             SET name = ?, version = version + 1, updated_at = ? \
             WHERE id = ? AND version = ?",
        )
        .bind(name)
        .bind(updated_at)
        .bind(branch_pointer_id)
        .bind(expected_version)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            let actual = get_branch_pointer_in(&mut transaction, branch_pointer_id)
                .await
                .map(|pointer| pointer.version)?;
            return Err(RepositoryError::VersionConflict {
                resource: "branch_pointer",
                id: branch_pointer_id.into(),
                expected: expected_version,
                actual,
            });
        }
        let stored = get_branch_pointer_in(&mut transaction, branch_pointer_id).await?;
        transaction.commit().await?;
        Ok(stored)
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

    pub async fn list_view_states(
        &self,
        workspace_id: &str,
    ) -> RepositoryResult<Vec<ViewStateRecord>> {
        let mut connection = self.pool.acquire().await?;
        list_view_states_on(&mut connection, workspace_id).await
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

async fn list_turns_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<Vec<TurnRecord>> {
    let rows = sqlx::query(
        "SELECT t.id, t.workspace_id, t.parent_run_id, t.prompt_block_id, \
                b.content AS prompt_markdown, t.title, t.created_at, t.deleted_at \
         FROM turn t JOIN content_block b ON b.id = t.prompt_block_id \
         WHERE t.workspace_id = ? AND t.deleted_at IS NULL \
         ORDER BY t.created_at, t.id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    rows.iter().map(turn_from_row).collect()
}

async fn list_runs_with_provenance_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<(Vec<ModelRunRecord>, Vec<RunProviderProvenanceRecord>)> {
    let rows = sqlx::query(
        "SELECT r.id, r.turn_id, r.workspace_id, r.provider_profile_id, r.model, \
                r.status, r.output_markdown, r.reasoning_markdown, \
                r.provider_snapshot_json, r.usage_json, r.error_json, r.created_at, \
                r.started_at, r.finished_at, r.checkpointed_at, \
                s.run_id AS provenance_run_id, s.provider AS provenance_provider_name, \
                s.base_url AS provenance_base_url, s.model AS provenance_model \
         FROM model_run r \
         LEFT JOIN context_snapshot s ON s.run_id = r.id \
         WHERE r.workspace_id = ? ORDER BY r.created_at, r.id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    let mut runs = Vec::with_capacity(rows.len());
    let mut provenance = Vec::with_capacity(rows.len());
    for row in &rows {
        runs.push(model_run_from_row(row)?);
        if let Some(run_id) = row.try_get::<Option<String>, _>("provenance_run_id")? {
            provenance.push(RunProviderProvenanceRecord {
                run_id,
                provider_name: row.try_get("provenance_provider_name")?,
                base_url: row.try_get("provenance_base_url")?,
                model: row.try_get("provenance_model")?,
            });
        }
    }
    Ok((runs, provenance))
}

async fn list_content_blocks_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<Vec<ContentBlockRecord>> {
    // Start from indexed workspace references, not the global block collection.
    // UNION preserves one copy when a block has several owners in this workspace.
    let rows = sqlx::query(
        "SELECT b.id, b.role, b.content, b.content_hash, b.created_at \
         FROM content_block b \
         JOIN ( \
             SELECT prompt_block_id AS id FROM turn WHERE workspace_id = ? \
             UNION \
             SELECT content_block_id AS id FROM context_manifest_item WHERE workspace_id = ? \
             UNION \
             SELECT summary_block_id AS id FROM context_checkpoint WHERE workspace_id = ? \
             UNION \
             SELECT content_block_id AS id FROM context_override_item WHERE workspace_id = ? \
         ) owned ON owned.id = b.id \
         ORDER BY b.created_at, b.id",
    )
    .bind(workspace_id)
    .bind(workspace_id)
    .bind(workspace_id)
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    rows.iter().map(content_block_from_row).collect()
}

async fn load_branch_state_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<(
    Vec<BranchPointerRecord>,
    Vec<BranchRevisionRecord>,
    Vec<BranchCheckpointInheritanceRecord>,
)> {
    let rows = sqlx::query(
        "SELECT b.id, b.workspace_id, b.name, b.head_run_id, b.version, \
                b.created_at, b.updated_at, r.revision AS revision_revision, \
                r.name AS revision_name, r.head_run_id AS revision_head_run_id, \
                r.change_kind AS revision_change_kind, r.created_at AS revision_created_at, \
                i.checkpoint_id AS inheritance_checkpoint_id, \
                i.inherited_at AS inheritance_inherited_at \
         FROM branch_pointer b \
         JOIN branch_revision r \
           ON r.branch_pointer_id = b.id AND r.workspace_id = b.workspace_id \
         LEFT JOIN branch_checkpoint_inheritance i \
           ON i.branch_pointer_id = b.id AND i.workspace_id = b.workspace_id \
         WHERE b.workspace_id = ? ORDER BY b.name COLLATE NOCASE, b.id, r.revision",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    let mut pointers = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    let mut inheritance = BTreeMap::new();
    for row in &rows {
        let pointer = branch_pointer_from_row(row)?;
        pointers
            .entry(pointer.id.clone())
            .or_insert(pointer.clone());
        let revision = BranchRevisionRecord {
            branch_pointer_id: pointer.id.clone(),
            workspace_id: pointer.workspace_id.clone(),
            revision: row.try_get("revision_revision")?,
            name: row.try_get("revision_name")?,
            head_run_id: row.try_get("revision_head_run_id")?,
            change_kind: row.try_get("revision_change_kind")?,
            created_at: row.try_get("revision_created_at")?,
        };
        revisions
            .entry((revision.branch_pointer_id.clone(), revision.revision))
            .or_insert(revision);
        if let Some(checkpoint_id) =
            row.try_get::<Option<String>, _>("inheritance_checkpoint_id")?
        {
            let evidence = BranchCheckpointInheritanceRecord {
                workspace_id: workspace_id.into(),
                branch_pointer_id: pointer.id.clone(),
                checkpoint_id,
                inherited_at: row.try_get("inheritance_inherited_at")?,
            };
            inheritance
                .entry((
                    evidence.branch_pointer_id.clone(),
                    evidence.checkpoint_id.clone(),
                ))
                .or_insert(evidence);
        }
    }
    Ok((
        pointers.into_values().collect(),
        revisions.into_values().collect(),
        inheritance.into_values().collect(),
    ))
}

async fn load_context_draft_state_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
    cursor: &ContextCursorRecord,
) -> RepositoryResult<ContextDraftRecord> {
    let rows = sqlx::query(
        "SELECT w.id AS workspace_id, d.parent_run_id, d.version, \
                d.consumed_by_run_id, COALESCE(d.updated_at, w.updated_at) AS updated_at, \
                o.position AS item_position, o.operation AS item_operation, \
                o.source_kind AS item_source_kind, o.source_id AS item_source_id, \
                o.content_block_id AS item_content_block_id, \
                o.content_hash AS item_content_hash, o.created_at AS item_created_at \
         FROM workspace w \
         LEFT JOIN context_draft d ON d.workspace_id = w.id \
         LEFT JOIN context_override_item o \
           ON o.workspace_id = d.workspace_id AND d.consumed_by_run_id IS NULL \
         WHERE w.id = ? ORDER BY o.position",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    let first = rows
        .first()
        .ok_or_else(|| not_found("workspace", workspace_id))?;
    let consumed_by_run_id: Option<String> = first.try_get("consumed_by_run_id")?;
    let persisted_parent: Option<String> = first.try_get("parent_run_id")?;
    let version: Option<i64> = first.try_get("version")?;
    let mut items = Vec::new();
    for row in &rows {
        let Some(position) = row.try_get::<Option<i64>, _>("item_position")? else {
            continue;
        };
        items.push(ContextOverrideItemRecord {
            workspace_id: workspace_id.into(),
            position,
            operation: row.try_get("item_operation")?,
            source_kind: row.try_get("item_source_kind")?,
            source_id: row.try_get("item_source_id")?,
            content_block_id: row.try_get("item_content_block_id")?,
            content_hash: row.try_get("item_content_hash")?,
            created_at: row.try_get("item_created_at")?,
        });
    }
    Ok(ContextDraftRecord {
        workspace_id: workspace_id.into(),
        parent_run_id: if version.is_none() || consumed_by_run_id.is_some() {
            cursor.active_run_id.clone()
        } else {
            persisted_parent
        },
        version: version.unwrap_or(0),
        items,
        consumed_by_run_id,
        updated_at: first.try_get("updated_at")?,
    })
}

async fn list_context_checkpoints_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<Vec<ContextCheckpointRecord>> {
    let rows = sqlx::query(
        "SELECT c.id, c.workspace_id, c.maintenance_run_id, c.kind, \
                c.branch_pointer_id, c.branch_revision, c.anchor_run_id, \
                c.first_kept_run_id, c.summary_block_id, b.content AS summary, \
                b.content_hash AS summary_content_hash, c.source_run_ids_json, \
                c.source_hash, m.provider_snapshot_json, c.created_at \
         FROM context_checkpoint c \
         JOIN content_block b ON b.id = c.summary_block_id \
         JOIN context_maintenance_run m ON m.id = c.maintenance_run_id \
         WHERE c.workspace_id = ? ORDER BY c.created_at, c.id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    rows.iter().map(context_checkpoint_from_row).collect()
}

async fn list_view_states_on(
    connection: &mut SqliteConnection,
    workspace_id: &str,
) -> RepositoryResult<Vec<ViewStateRecord>> {
    let rows = sqlx::query(
        "SELECT workspace_id, view_key, state_json, updated_at FROM view_state \
         WHERE workspace_id = ? ORDER BY view_key",
    )
    .bind(workspace_id)
    .fetch_all(&mut *connection)
    .await?;
    rows.iter().map(view_state_from_row).collect()
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
        let has_nonempty_source_ref_id = item
            .source_ref_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty());
        if item.source_ref_kind.trim().is_empty() || !has_nonempty_source_ref_id {
            return Err(RepositoryError::InvalidInput(
                "new context items require a paired nonempty typed source identity".into(),
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
    if let Some(update) = &bundle.context_update {
        if update.expected_cursor_version < 0 || update.expected_draft_version < 0 {
            return Err(RepositoryError::InvalidInput(
                "expected context versions cannot be negative".into(),
            ));
        }
        if update.result_branch_pointer_id.as_deref()
            != bundle
                .branch_pointer
                .as_ref()
                .map(|pointer| pointer.id.as_str())
        {
            return Err(RepositoryError::InvalidInput(
                "run-start cursor branch must match the atomically persisted branch pointer".into(),
            ));
        }
        if update.expected_branch_pointer_id.is_some() != update.expected_branch_version.is_some() {
            return Err(RepositoryError::InvalidInput(
                "run-start expected branch identity and version must be present together".into(),
            ));
        }
    }
    Ok(())
}

async fn validate_run_start_branch_guard(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
    update: &RunStartContextUpdateRecord,
) -> RepositoryResult<()> {
    let Some(branch_pointer_id) = update.expected_branch_pointer_id.as_deref() else {
        return Ok(());
    };
    let expected = update
        .expected_branch_version
        .expect("validated run-start branch guard is paired");
    let actual = sqlx::query_scalar::<_, i64>(
        "SELECT version FROM branch_pointer WHERE id = ? AND workspace_id = ?",
    )
    .bind(branch_pointer_id)
    .bind(workspace_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| not_found("branch pointer", branch_pointer_id))?;
    if actual == expected {
        Ok(())
    } else {
        Err(RepositoryError::VersionConflict {
            resource: "branch_pointer",
            id: branch_pointer_id.into(),
            expected,
            actual,
        })
    }
}

fn validate_context_cursor_update(update: &ContextCursorUpdateRecord) -> RepositoryResult<()> {
    if update.expected_version < 0 {
        return Err(RepositoryError::InvalidInput(
            "expected cursor version cannot be negative".into(),
        ));
    }
    if update.branch_pointer_id.is_some() && update.active_run_id.is_none() {
        return Err(RepositoryError::InvalidInput(
            "a cursor branch requires an active Run".into(),
        ));
    }
    Ok(())
}

async fn apply_context_cursor_update(
    transaction: &mut Transaction<'_, Sqlite>,
    update: &ContextCursorUpdateRecord,
) -> RepositoryResult<i64> {
    let result = if update.expected_version == 0 {
        sqlx::query(
            "INSERT INTO workspace_context_cursor \
             (workspace_id, active_run_id, branch_pointer_id, version, updated_at) \
             VALUES (?, ?, ?, 1, ?) ON CONFLICT(workspace_id) DO NOTHING",
        )
        .bind(&update.workspace_id)
        .bind(&update.active_run_id)
        .bind(&update.branch_pointer_id)
        .bind(update.updated_at)
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE workspace_context_cursor \
             SET active_run_id = ?, branch_pointer_id = ?, \
                 version = version + 1, updated_at = ? \
             WHERE workspace_id = ? AND version = ?",
        )
        .bind(&update.active_run_id)
        .bind(&update.branch_pointer_id)
        .bind(update.updated_at)
        .bind(&update.workspace_id)
        .bind(update.expected_version)
        .execute(&mut **transaction)
        .await?
    };
    if result.rows_affected() != 0 {
        return Ok(update.expected_version + 1);
    }

    let workspace_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workspace WHERE id = ?)")
            .bind(&update.workspace_id)
            .fetch_one(&mut **transaction)
            .await?;
    if !workspace_exists {
        return Err(not_found("workspace", &update.workspace_id));
    }
    let actual = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT version FROM workspace_context_cursor WHERE workspace_id = ?",
    )
    .bind(&update.workspace_id)
    .fetch_optional(&mut **transaction)
    .await?
    .flatten()
    .unwrap_or(0);
    Err(RepositoryError::VersionConflict {
        resource: "context_cursor",
        id: update.workspace_id.clone(),
        expected: update.expected_version,
        actual,
    })
}

fn validate_context_draft_update(update: &ContextDraftUpdateRecord) -> RepositoryResult<()> {
    if update.expected_version < 0 {
        return Err(RepositoryError::InvalidInput(
            "expected draft version cannot be negative".into(),
        ));
    }
    for (position, item) in update.items.iter().enumerate() {
        if item.workspace_id != update.workspace_id || item.position != position as i64 {
            return Err(RepositoryError::InvalidInput(
                "context override items must be contiguous and owned by the draft workspace".into(),
            ));
        }
        if (item.content_block_id.is_some()) != (item.content_hash.is_some()) {
            return Err(RepositoryError::InvalidInput(
                "context override content block and hash must be provided together".into(),
            ));
        }
        if item.operation == "pin" && item.content_block_id.is_none() {
            return Err(RepositoryError::InvalidInput(
                "a pinned override requires exact content identity".into(),
            ));
        }
    }
    Ok(())
}

async fn apply_context_draft_update(
    transaction: &mut Transaction<'_, Sqlite>,
    update: &ContextDraftUpdateRecord,
) -> RepositoryResult<i64> {
    let result = if update.expected_version == 0 {
        sqlx::query(
            "INSERT INTO context_draft \
             (workspace_id, parent_run_id, version, consumed_by_run_id, updated_at) \
             VALUES (?, ?, 1, NULL, ?) ON CONFLICT(workspace_id) DO NOTHING",
        )
        .bind(&update.workspace_id)
        .bind(&update.parent_run_id)
        .bind(update.updated_at)
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE context_draft \
             SET parent_run_id = ?, version = version + 1, \
                 consumed_by_run_id = NULL, updated_at = ? \
             WHERE workspace_id = ? AND version = ?",
        )
        .bind(&update.parent_run_id)
        .bind(update.updated_at)
        .bind(&update.workspace_id)
        .bind(update.expected_version)
        .execute(&mut **transaction)
        .await?
    };
    if result.rows_affected() == 0 {
        let workspace_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workspace WHERE id = ?)")
                .bind(&update.workspace_id)
                .fetch_one(&mut **transaction)
                .await?;
        if !workspace_exists {
            return Err(not_found("workspace", &update.workspace_id));
        }
        let actual = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM context_draft WHERE workspace_id = ?",
        )
        .bind(&update.workspace_id)
        .fetch_optional(&mut **transaction)
        .await?
        .unwrap_or(0);
        return Err(RepositoryError::VersionConflict {
            resource: "context_draft",
            id: update.workspace_id.clone(),
            expected: update.expected_version,
            actual,
        });
    }

    sqlx::query("DELETE FROM context_override_item WHERE workspace_id = ?")
        .bind(&update.workspace_id)
        .execute(&mut **transaction)
        .await?;
    for item in &update.items {
        insert_context_override_item(transaction, item).await?;
    }
    Ok(update.expected_version + 1)
}

async fn insert_context_override_item(
    transaction: &mut Transaction<'_, Sqlite>,
    item: &ContextOverrideItemRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO context_override_item \
         (workspace_id, position, operation, source_kind, source_id, \
          content_block_id, content_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item.workspace_id)
    .bind(item.position)
    .bind(&item.operation)
    .bind(&item.source_kind)
    .bind(&item.source_id)
    .bind(&item.content_block_id)
    .bind(&item.content_hash)
    .bind(item.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn consume_context_draft(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
    turn_id: &str,
    run_id: &str,
    expected_version: i64,
    updated_at: i64,
) -> RepositoryResult<i64> {
    let parent_run_id =
        sqlx::query_scalar::<_, Option<String>>("SELECT parent_run_id FROM turn WHERE id = ?")
            .bind(turn_id)
            .fetch_one(&mut **transaction)
            .await?;
    let result = if expected_version == 0 {
        sqlx::query(
            "INSERT INTO context_draft \
             (workspace_id, parent_run_id, version, consumed_by_run_id, updated_at) \
             VALUES (?, ?, 1, ?, ?) ON CONFLICT(workspace_id) DO NOTHING",
        )
        .bind(workspace_id)
        .bind(&parent_run_id)
        .bind(run_id)
        .bind(updated_at)
        .execute(&mut **transaction)
        .await?
    } else {
        sqlx::query(
            "UPDATE context_draft \
             SET version = version + 1, consumed_by_run_id = ?, updated_at = ? \
             WHERE workspace_id = ? AND version = ?",
        )
        .bind(run_id)
        .bind(updated_at)
        .bind(workspace_id)
        .bind(expected_version)
        .execute(&mut **transaction)
        .await?
    };
    if result.rows_affected() == 0 {
        let actual = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM context_draft WHERE workspace_id = ?",
        )
        .bind(workspace_id)
        .fetch_optional(&mut **transaction)
        .await?
        .unwrap_or(0);
        return Err(RepositoryError::VersionConflict {
            resource: "context_draft",
            id: workspace_id.into(),
            expected: expected_version,
            actual,
        });
    }
    sqlx::query("DELETE FROM context_override_item WHERE workspace_id = ?")
        .bind(workspace_id)
        .execute(&mut **transaction)
        .await?;
    Ok(expected_version + 1)
}

fn validate_context_maintenance_start(
    run: &ContextMaintenanceRunRecord,
    guard: &MaintenanceContextGuardRecord,
) -> RepositoryResult<()> {
    if run.workspace_id != guard.workspace_id {
        return Err(RepositoryError::InvalidInput(
            "maintenance guard and request must belong to the same workspace".into(),
        ));
    }
    if !matches!(run.status.as_str(), "queued" | "running") {
        return Err(RepositoryError::InvalidInput(
            "new context maintenance must be queued or running".into(),
        ));
    }
    validate_branch_evidence(
        run.branch_pointer_id.as_deref(),
        run.branch_revision,
        guard.branch_pointer_id.as_deref(),
        guard.expected_branch_version,
    )?;
    if run.status == "queued" && run.started_at.is_some()
        || run.status == "running" && run.started_at.is_none()
    {
        return Err(RepositoryError::InvalidInput(
            "maintenance start timestamp does not match its status".into(),
        ));
    }
    if run.summary_block_id.is_some()
        || run.summary.is_some()
        || run.error_json.is_some()
        || run.finished_at.is_some()
    {
        return Err(RepositoryError::InvalidInput(
            "new context maintenance cannot already contain a terminal result".into(),
        ));
    }
    Ok(())
}

fn validate_context_maintenance_finish(
    run: &ContextMaintenanceRunRecord,
    checkpoint: Option<&ContextCheckpointRecord>,
    summary_block: Option<&ContentBlockRecord>,
    guard: &MaintenanceContextGuardRecord,
    context_update: Option<&FinishContextMaintenanceUpdateRecord>,
) -> RepositoryResult<()> {
    if !matches!(
        run.status.as_str(),
        "completed" | "failed" | "cancelled" | "conflicted"
    ) || run.finished_at.is_none()
    {
        return Err(RepositoryError::InvalidInput(
            "maintenance finish requires a terminal status and timestamp".into(),
        ));
    }
    if run.status == "completed" {
        let checkpoint = checkpoint.ok_or_else(|| {
            RepositoryError::InvalidInput(
                "completed maintenance requires an immutable checkpoint".into(),
            )
        })?;
        let summary_block = summary_block.ok_or_else(|| {
            RepositoryError::InvalidInput(
                "completed maintenance requires a summary content block".into(),
            )
        })?;
        if checkpoint.workspace_id != run.workspace_id
            || checkpoint.maintenance_run_id != run.id
            || checkpoint.kind != run.kind
            || checkpoint.anchor_run_id != run.anchor_run_id
            || checkpoint.branch_pointer_id != run.branch_pointer_id
            || checkpoint.branch_revision != run.branch_revision
            || checkpoint.first_kept_run_id != run.first_kept_run_id
            || checkpoint.source_run_ids_json != run.source_run_ids_json
            || checkpoint.source_hash != run.source_hash
            || checkpoint.summary_block_id != summary_block.id
            || checkpoint.summary != summary_block.content
            || checkpoint.summary_content_hash != summary_block.content_hash
            || checkpoint.created_at != run.finished_at.expect("validated terminal timestamp")
            || run.summary.as_deref() != Some(summary_block.content.as_str())
            || run.error_json.is_some()
        {
            return Err(RepositoryError::InvalidInput(
                "checkpoint does not exactly match its maintenance evidence and summary block"
                    .into(),
            ));
        }
        validate_branch_evidence(
            run.branch_pointer_id.as_deref(),
            run.branch_revision,
            guard.branch_pointer_id.as_deref(),
            guard.expected_branch_version,
        )?;
        if guard.workspace_id != run.workspace_id {
            return Err(RepositoryError::InvalidInput(
                "maintenance finish guard belongs to another workspace".into(),
            ));
        }
    } else if checkpoint.is_some() || summary_block.is_some() || context_update.is_some() {
        return Err(RepositoryError::InvalidInput(
            "unsuccessful maintenance cannot activate a checkpoint or move the cursor".into(),
        ));
    }
    Ok(())
}

fn validate_branch_evidence(
    stored_branch_id: Option<&str>,
    stored_branch_version: Option<i64>,
    guarded_branch_id: Option<&str>,
    guarded_branch_version: Option<i64>,
) -> RepositoryResult<()> {
    if stored_branch_id.is_some() != stored_branch_version.is_some()
        || guarded_branch_id.is_some() != guarded_branch_version.is_some()
        || stored_branch_id != guarded_branch_id
        || stored_branch_version != guarded_branch_version
    {
        return Err(RepositoryError::InvalidInput(
            "branch identity and revision evidence must be present together and match".into(),
        ));
    }
    Ok(())
}

async fn validate_maintenance_guard(
    transaction: &mut Transaction<'_, Sqlite>,
    guard: &MaintenanceContextGuardRecord,
    anchor_run_id: &str,
) -> RepositoryResult<()> {
    let cursor = get_context_cursor_in(transaction, &guard.workspace_id).await?;
    if cursor.version != guard.expected_cursor_version
        || cursor.active_run_id.as_deref() != Some(anchor_run_id)
        || cursor.branch_pointer_id != guard.branch_pointer_id
    {
        return Err(RepositoryError::VersionConflict {
            resource: "context_cursor",
            id: guard.workspace_id.clone(),
            expected: guard.expected_cursor_version,
            actual: cursor.version,
        });
    }
    if let Some(expected_draft_version) = guard.expected_draft_version {
        let actual = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM context_draft WHERE workspace_id = ?",
        )
        .bind(&guard.workspace_id)
        .fetch_optional(&mut **transaction)
        .await?
        .unwrap_or(0);
        if actual != expected_draft_version {
            return Err(RepositoryError::VersionConflict {
                resource: "context_draft",
                id: guard.workspace_id.clone(),
                expected: expected_draft_version,
                actual,
            });
        }
    }
    match (
        guard.branch_pointer_id.as_deref(),
        guard.expected_branch_version,
    ) {
        (Some(branch_pointer_id), Some(expected_branch_version)) => {
            let actual = sqlx::query_scalar::<_, i64>(
                "SELECT version FROM branch_pointer WHERE id = ? AND workspace_id = ?",
            )
            .bind(branch_pointer_id)
            .bind(&guard.workspace_id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| not_found("branch pointer", branch_pointer_id))?;
            if actual != expected_branch_version {
                return Err(RepositoryError::VersionConflict {
                    resource: "branch_pointer",
                    id: branch_pointer_id.into(),
                    expected: expected_branch_version,
                    actual,
                });
            }
        }
        (None, None) => {}
        _ => {
            return Err(RepositoryError::InvalidInput(
                "maintenance branch guard must include both identity and version".into(),
            ));
        }
    }
    Ok(())
}

async fn get_context_cursor_in(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
) -> RepositoryResult<ContextCursorRecord> {
    let row = sqlx::query(
        "WITH stored AS ( \
             SELECT workspace_id, active_run_id, branch_pointer_id, version, updated_at \
             FROM workspace_context_cursor WHERE workspace_id = ? \
         ), fallback_branch AS ( \
             SELECT w.id AS workspace_id, b.head_run_id AS active_run_id, \
                    b.id AS branch_pointer_id, 0 AS version, b.updated_at AS updated_at \
             FROM workspace w JOIN branch_pointer b ON b.workspace_id = w.id \
             WHERE w.id = ? ORDER BY b.updated_at DESC, b.id DESC LIMIT 1 \
         ), fallback_run AS ( \
             SELECT w.id AS workspace_id, r.id AS active_run_id, \
                    NULL AS branch_pointer_id, 0 AS version, r.created_at AS updated_at \
             FROM workspace w JOIN model_run r ON r.workspace_id = w.id \
             WHERE w.id = ? ORDER BY r.created_at DESC, r.id DESC LIMIT 1 \
         ) \
         SELECT * FROM stored \
         UNION ALL SELECT * FROM fallback_branch \
           WHERE NOT EXISTS (SELECT 1 FROM stored) \
         UNION ALL SELECT * FROM fallback_run \
           WHERE NOT EXISTS (SELECT 1 FROM stored) \
             AND NOT EXISTS (SELECT 1 FROM fallback_branch) \
         UNION ALL SELECT w.id, NULL, NULL, 0, w.updated_at FROM workspace w \
           WHERE w.id = ? \
             AND NOT EXISTS (SELECT 1 FROM stored) \
             AND NOT EXISTS (SELECT 1 FROM fallback_branch) \
             AND NOT EXISTS (SELECT 1 FROM fallback_run) \
         LIMIT 1",
    )
    .bind(workspace_id)
    .bind(workspace_id)
    .bind(workspace_id)
    .bind(workspace_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| not_found("workspace", workspace_id))?;
    context_cursor_from_row(&row)
}

async fn get_context_draft_version_in(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
) -> RepositoryResult<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT version FROM context_draft WHERE workspace_id = ?")
            .bind(workspace_id)
            .fetch_optional(&mut **transaction)
            .await?
            .unwrap_or(0),
    )
}

async fn get_branch_pointer_in(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> RepositoryResult<BranchPointerRecord> {
    let row = sqlx::query(
        "SELECT id, workspace_id, name, head_run_id, version, created_at, updated_at \
         FROM branch_pointer WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| not_found("branch pointer", id))?;
    branch_pointer_from_row(&row)
}

async fn insert_context_maintenance_run(
    transaction: &mut Transaction<'_, Sqlite>,
    run: &ContextMaintenanceRunRecord,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO context_maintenance_run \
         (id, workspace_id, kind, anchor_run_id, branch_pointer_id, branch_revision, \
          first_kept_run_id, source_run_ids_json, source_hash, provider_snapshot_json, \
          request_json, status, summary_block_id, error_json, created_at, started_at, finished_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&run.id)
    .bind(&run.workspace_id)
    .bind(&run.kind)
    .bind(&run.anchor_run_id)
    .bind(&run.branch_pointer_id)
    .bind(run.branch_revision)
    .bind(&run.first_kept_run_id)
    .bind(&run.source_run_ids_json)
    .bind(&run.source_hash)
    .bind(&run.provider_snapshot_json)
    .bind(&run.request_json)
    .bind(&run.status)
    .bind(&run.summary_block_id)
    .bind(&run.error_json)
    .bind(run.created_at)
    .bind(run.started_at)
    .bind(run.finished_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn get_context_maintenance_run_in(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> RepositoryResult<ContextMaintenanceRunRecord> {
    find_context_maintenance_run_in(transaction, id)
        .await?
        .ok_or_else(|| not_found("context maintenance run", id))
}

async fn find_context_maintenance_run_in(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> RepositoryResult<Option<ContextMaintenanceRunRecord>> {
    let row = sqlx::query(
        "SELECT m.id, m.workspace_id, m.kind, m.status, m.branch_pointer_id, \
                m.branch_revision, m.anchor_run_id, m.first_kept_run_id, \
                m.source_run_ids_json, m.source_hash, m.provider_snapshot_json, \
                m.request_json, m.summary_block_id, b.content AS summary, m.error_json, \
                m.created_at, m.started_at, m.finished_at \
         FROM context_maintenance_run m \
         LEFT JOIN content_block b ON b.id = m.summary_block_id WHERE m.id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?;
    row.as_ref()
        .map(context_maintenance_run_from_row)
        .transpose()
}

fn validate_context_maintenance_identity(
    existing: &ContextMaintenanceRunRecord,
    finish: &ContextMaintenanceRunRecord,
) -> RepositoryResult<()> {
    if existing.id != finish.id
        || existing.workspace_id != finish.workspace_id
        || existing.kind != finish.kind
        || existing.branch_pointer_id != finish.branch_pointer_id
        || existing.branch_revision != finish.branch_revision
        || existing.anchor_run_id != finish.anchor_run_id
        || existing.first_kept_run_id != finish.first_kept_run_id
        || existing.source_run_ids_json != finish.source_run_ids_json
        || existing.source_hash != finish.source_hash
        || existing.provider_snapshot_json != finish.provider_snapshot_json
        || existing.request_json != finish.request_json
        || existing.created_at != finish.created_at
    {
        return Err(RepositoryError::Conflict(
            "maintenance finish does not match its immutable request".into(),
        ));
    }
    Ok(())
}

fn validate_context_maintenance_start_identity(
    existing: &ContextMaintenanceRunRecord,
    replay: &ContextMaintenanceRunRecord,
) -> RepositoryResult<()> {
    if existing.id != replay.id
        || existing.workspace_id != replay.workspace_id
        || existing.kind != replay.kind
        || existing.branch_pointer_id != replay.branch_pointer_id
        || existing.branch_revision != replay.branch_revision
        || existing.anchor_run_id != replay.anchor_run_id
        || existing.first_kept_run_id != replay.first_kept_run_id
        || existing.source_run_ids_json != replay.source_run_ids_json
        || existing.source_hash != replay.source_hash
        || existing.provider_snapshot_json != replay.provider_snapshot_json
        || existing.request_json != replay.request_json
    {
        return Err(RepositoryError::Conflict(
            "maintenance operation id was reused with a different request".into(),
        ));
    }
    Ok(())
}

async fn validate_terminal_maintenance_replay(
    transaction: &mut Transaction<'_, Sqlite>,
    existing: &ContextMaintenanceRunRecord,
    replay: &ContextMaintenanceRunRecord,
    checkpoint: Option<&ContextCheckpointRecord>,
    summary_block: Option<&ContentBlockRecord>,
) -> RepositoryResult<()> {
    if existing.status != replay.status
        || existing.summary_block_id != replay.summary_block_id
        || existing.summary != replay.summary
        || existing.error_json != replay.error_json
        || existing.finished_at != replay.finished_at
    {
        return Err(RepositoryError::Conflict(
            "maintenance operation id was reused with a different terminal result".into(),
        ));
    }
    if existing.status != "completed" {
        return Ok(());
    }

    let checkpoint = checkpoint.ok_or_else(|| {
        RepositoryError::InvalidInput(
            "completed maintenance replay requires its immutable checkpoint".into(),
        )
    })?;
    let summary_block = summary_block.ok_or_else(|| {
        RepositoryError::InvalidInput(
            "completed maintenance replay requires its exact summary block".into(),
        )
    })?;
    let stored_row = sqlx::query(
        "SELECT c.id, c.workspace_id, c.maintenance_run_id, c.kind, \
                c.branch_pointer_id, c.branch_revision, c.anchor_run_id, \
                c.first_kept_run_id, c.summary_block_id, b.content AS summary, \
                b.content_hash AS summary_content_hash, c.source_run_ids_json, \
                c.source_hash, m.provider_snapshot_json, c.created_at \
         FROM context_checkpoint c \
         JOIN content_block b ON b.id = c.summary_block_id \
         JOIN context_maintenance_run m ON m.id = c.maintenance_run_id \
         WHERE c.maintenance_run_id = ?",
    )
    .bind(&existing.id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| not_found("context checkpoint for maintenance", &existing.id))?;
    let stored = context_checkpoint_from_row(&stored_row)?;
    let mut normalized_checkpoint = checkpoint.clone();
    normalized_checkpoint.created_at = stored.created_at;
    if stored.created_at < checkpoint.created_at
        || stored != normalized_checkpoint
        || stored.summary_block_id != summary_block.id
        || stored.summary != summary_block.content
        || stored.summary_content_hash != summary_block.content_hash
    {
        return Err(RepositoryError::Conflict(
            "maintenance operation id was reused with different checkpoint evidence".into(),
        ));
    }
    Ok(())
}

async fn finish_context_maintenance_row(
    transaction: &mut Transaction<'_, Sqlite>,
    run: &ContextMaintenanceRunRecord,
    summary_block_id: Option<&str>,
) -> RepositoryResult<()> {
    let result = sqlx::query(
        "UPDATE context_maintenance_run \
         SET status = ?, summary_block_id = ?, error_json = ?, finished_at = ? \
         WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(&run.status)
    .bind(summary_block_id)
    .bind(&run.error_json)
    .bind(run.finished_at)
    .bind(&run.id)
    .execute(&mut **transaction)
    .await?;
    ensure_changed(
        result.rows_affected(),
        "active context maintenance run",
        &run.id,
    )
}

async fn mark_context_maintenance_conflicted(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
    finished_at: i64,
    error: &RepositoryError,
) -> RepositoryResult<()> {
    let result = sqlx::query(
        "UPDATE context_maintenance_run \
         SET status = 'conflicted', error_json = ?, finished_at = ? \
         WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(version_conflict_json(error))
    .bind(finished_at)
    .bind(id)
    .execute(&mut **transaction)
    .await?;
    ensure_changed(result.rows_affected(), "active context maintenance run", id)
}

async fn insert_context_checkpoint(
    transaction: &mut Transaction<'_, Sqlite>,
    checkpoint: &ContextCheckpointRecord,
) -> RepositoryResult<ContextCheckpointRecord> {
    let latest_created_at = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(created_at) FROM context_checkpoint WHERE workspace_id = ?",
    )
    .bind(&checkpoint.workspace_id)
    .fetch_one(&mut **transaction)
    .await?;
    let created_at = match latest_created_at {
        Some(latest) if checkpoint.created_at <= latest => {
            latest.checked_add(1).ok_or_else(|| {
                RepositoryError::InvalidStoredValue(
                    "context checkpoint timestamp exhausted SQLite integer range".into(),
                )
            })?
        }
        _ => checkpoint.created_at,
    };
    sqlx::query(
        "INSERT INTO context_checkpoint \
         (id, workspace_id, maintenance_run_id, kind, anchor_run_id, branch_pointer_id, \
          branch_revision, first_kept_run_id, summary_block_id, source_run_ids_json, \
          source_hash, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&checkpoint.id)
    .bind(&checkpoint.workspace_id)
    .bind(&checkpoint.maintenance_run_id)
    .bind(&checkpoint.kind)
    .bind(&checkpoint.anchor_run_id)
    .bind(&checkpoint.branch_pointer_id)
    .bind(checkpoint.branch_revision)
    .bind(&checkpoint.first_kept_run_id)
    .bind(&checkpoint.summary_block_id)
    .bind(&checkpoint.source_run_ids_json)
    .bind(&checkpoint.source_hash)
    .bind(created_at)
    .execute(&mut **transaction)
    .await?;
    let mut stored = checkpoint.clone();
    stored.created_at = created_at;
    Ok(stored)
}

async fn insert_branch_checkpoint_inheritance(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
    branch_pointer_id: &str,
    checkpoint_id: &str,
    inherited_at: i64,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO branch_checkpoint_inheritance \
         (workspace_id, branch_pointer_id, checkpoint_id, inherited_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(workspace_id)
    .bind(branch_pointer_id)
    .bind(checkpoint_id)
    .bind(inherited_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn copy_branch_checkpoint_inheritance(
    transaction: &mut Transaction<'_, Sqlite>,
    workspace_id: &str,
    source_branch_pointer_id: &str,
    result_branch_pointer_id: &str,
    inherited_at: i64,
) -> RepositoryResult<()> {
    sqlx::query(
        "INSERT INTO branch_checkpoint_inheritance \
         (workspace_id, branch_pointer_id, checkpoint_id, inherited_at) \
         SELECT workspace_id, ?, checkpoint_id, ? \
         FROM branch_checkpoint_inheritance \
         WHERE workspace_id = ? AND branch_pointer_id = ? \
         ORDER BY checkpoint_id",
    )
    .bind(result_branch_pointer_id)
    .bind(inherited_at)
    .bind(workspace_id)
    .bind(source_branch_pointer_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn version_conflict_json(error: &RepositoryError) -> String {
    match error {
        RepositoryError::VersionConflict {
            resource,
            id,
            expected,
            actual,
        } => serde_json::json!({
            "code": format!("{resource}_conflict"),
            "id": id,
            "expected": expected,
            "actual": actual,
        })
        .to_string(),
        other => serde_json::json!({
            "code": "context_maintenance_conflict",
            "message": other.to_string(),
        })
        .to_string(),
    }
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
         (id, workspace_id, compiler_version, strategy, estimated_chars, canonical_hash, \
          warnings_json, checkpoint_provenance_json, branch_summary_provenance_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&manifest.id)
    .bind(&manifest.workspace_id)
    .bind(&manifest.compiler_version)
    .bind(&manifest.strategy)
    .bind(manifest.estimated_chars)
    .bind(&manifest.canonical_hash)
    .bind(&manifest.warnings_json)
    .bind(&manifest.checkpoint_provenance_json)
    .bind(&manifest.branch_summary_provenance_json)
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
         (manifest_id, workspace_id, position, source_id, source_ref_kind, source_ref_id, \
          source_kind, role, content_block_id, inclusion_reason, mandatory) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&item.manifest_id)
    .bind(&item.workspace_id)
    .bind(item.position)
    .bind(&item.source_id)
    .bind(&item.source_ref_kind)
    .bind(&item.source_ref_id)
    .bind(&item.source_kind)
    .bind(&item.role)
    .bind(&item.content_block_id)
    .bind(&item.inclusion_reason)
    .bind(item.mandatory)
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

fn legacy_profile_dialect(dialect: &str) -> &str {
    match dialect {
        "ollama_chat" => "ollama_chat",
        _ => "openai_chat_completions",
    }
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

fn run_provider_provenance_from_row(
    row: &SqliteRow,
) -> RepositoryResult<RunProviderProvenanceRecord> {
    Ok(RunProviderProvenanceRecord {
        run_id: row.try_get("run_id")?,
        provider_name: row.try_get("provider_name")?,
        base_url: row.try_get("base_url")?,
        model: row.try_get("model")?,
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
        checkpoint_provenance_json: row.try_get("checkpoint_provenance_json")?,
        branch_summary_provenance_json: row.try_get("branch_summary_provenance_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn stored_context_item_from_row(row: &SqliteRow) -> RepositoryResult<StoredContextItem> {
    Ok(StoredContextItem {
        position: row.try_get("position")?,
        source_id: row.try_get("source_id")?,
        source_ref_kind: row.try_get("source_ref_kind")?,
        source_ref_id: row.try_get("source_ref_id")?,
        source_kind: row.try_get("source_kind")?,
        role: row.try_get("role")?,
        content_block_id: row.try_get("content_block_id")?,
        content: row.try_get("content")?,
        content_hash: row.try_get("content_hash")?,
        inclusion_reason: row.try_get("inclusion_reason")?,
        mandatory: row.try_get("mandatory")?,
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

fn context_cursor_from_row(row: &SqliteRow) -> RepositoryResult<ContextCursorRecord> {
    Ok(ContextCursorRecord {
        workspace_id: row.try_get("workspace_id")?,
        active_run_id: row.try_get("active_run_id")?,
        branch_pointer_id: row.try_get("branch_pointer_id")?,
        version: row.try_get("version")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn branch_revision_from_row(row: &SqliteRow) -> RepositoryResult<BranchRevisionRecord> {
    Ok(BranchRevisionRecord {
        branch_pointer_id: row.try_get("branch_pointer_id")?,
        workspace_id: row.try_get("workspace_id")?,
        revision: row.try_get("revision")?,
        name: row.try_get("name")?,
        head_run_id: row.try_get("head_run_id")?,
        change_kind: row.try_get("change_kind")?,
        created_at: row.try_get("created_at")?,
    })
}

fn context_maintenance_run_from_row(
    row: &SqliteRow,
) -> RepositoryResult<ContextMaintenanceRunRecord> {
    Ok(ContextMaintenanceRunRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        kind: row.try_get("kind")?,
        status: row.try_get("status")?,
        branch_pointer_id: row.try_get("branch_pointer_id")?,
        branch_revision: row.try_get("branch_revision")?,
        anchor_run_id: row.try_get("anchor_run_id")?,
        first_kept_run_id: row.try_get("first_kept_run_id")?,
        source_run_ids_json: row.try_get("source_run_ids_json")?,
        source_hash: row.try_get("source_hash")?,
        provider_snapshot_json: row.try_get("provider_snapshot_json")?,
        request_json: row.try_get("request_json")?,
        summary_block_id: row.try_get("summary_block_id")?,
        summary: row.try_get("summary")?,
        error_json: row.try_get("error_json")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

fn context_checkpoint_from_row(row: &SqliteRow) -> RepositoryResult<ContextCheckpointRecord> {
    Ok(ContextCheckpointRecord {
        id: row.try_get("id")?,
        workspace_id: row.try_get("workspace_id")?,
        maintenance_run_id: row.try_get("maintenance_run_id")?,
        kind: row.try_get("kind")?,
        branch_pointer_id: row.try_get("branch_pointer_id")?,
        branch_revision: row.try_get("branch_revision")?,
        anchor_run_id: row.try_get("anchor_run_id")?,
        first_kept_run_id: row.try_get("first_kept_run_id")?,
        summary_block_id: row.try_get("summary_block_id")?,
        summary: row.try_get("summary")?,
        summary_content_hash: row.try_get("summary_content_hash")?,
        source_run_ids_json: row.try_get("source_run_ids_json")?,
        source_hash: row.try_get("source_hash")?,
        provider_snapshot_json: row.try_get("provider_snapshot_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn branch_checkpoint_inheritance_from_row(
    row: &SqliteRow,
) -> RepositoryResult<BranchCheckpointInheritanceRecord> {
    Ok(BranchCheckpointInheritanceRecord {
        workspace_id: row.try_get("workspace_id")?,
        branch_pointer_id: row.try_get("branch_pointer_id")?,
        checkpoint_id: row.try_get("checkpoint_id")?,
        inherited_at: row.try_get("inherited_at")?,
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
