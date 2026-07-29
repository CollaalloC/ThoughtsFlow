use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        BranchPointer, ContentBlock, ContextCheckpoint, ContextCheckpointKind,
        ContextCheckpointProvenance, ContextMaintenanceRun, ContextMaintenanceStatus,
        ContextManifest, ContextSnapshot, ContextSourceKind, ContextSourceRef,
        ContextSourceRefKind, ContextWarning, ConversationGraph, DecisionMark, DecisionStatus,
        InclusionReason, MessageRole, ModelRun, ProviderDialect, ProviderProfile, ProviderSnapshot,
        RunContextItem, RunDraft, RunFailure, RunStateSnapshot, RunStatus, RunUsage, Turn,
        ViewState, Workspace,
    },
    ports::{
        BranchCheckpointInheritance, BranchRevision, BranchRevisionChange, CheckpointOutcome,
        ContextCursor, ContextCursorUpdate, ContextDraft, ContextDraftUpdate,
        ContextMaintenanceStart, ContextOverrideItem, ContextOverrideOperation,
        FinishContextMaintenanceContextUpdate, MaintenanceContextGuard, PersistRunStart,
        PersistRunStartOutcome, RepositoryFuture, RepositoryPort, RepositoryPortError,
        RunCheckpoint as PortRunCheckpoint, RunFinish as PortRunFinish, RunPersistencePort,
        RunProviderProvenance, WorkspaceContextData,
    },
};

use super::*;

impl RepositoryPort for SqliteRepository {
    fn list_workspaces(&self, include_archived: bool) -> RepositoryFuture<'_, Vec<Workspace>> {
        let repository = self.clone();
        Box::pin(async move {
            SqliteRepository::list_workspaces(&repository, include_archived)
                .await
                .map(|records| records.into_iter().map(workspace_to_domain).collect())
                .map_err(port_error)
        })
    }

    fn get_workspace(&self, id: &str) -> RepositoryFuture<'_, Workspace> {
        let repository = self.clone();
        let id = id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_workspace(&repository, &id)
                .await
                .map(workspace_to_domain)
                .map_err(port_error)
        })
    }

    fn save_workspace(&self, workspace: Workspace) -> RepositoryFuture<'_, Workspace> {
        let repository = self.clone();
        Box::pin(async move {
            let record = workspace_to_record(&workspace);
            let stored = SqliteRepository::save_workspace(&repository, &record)
                .await
                .map_err(port_error)?;
            Ok(workspace_to_domain(stored))
        })
    }

    fn load_conversation_graph(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, ConversationGraph> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_workspace(&repository, &workspace_id)
                .await
                .map_err(port_error)?;
            let turns = SqliteRepository::list_turns(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(turn_to_domain)
                .collect();
            let runs = SqliteRepository::list_runs(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(run_to_domain)
                .collect::<Result<Vec<_>, _>>()?;
            let content_blocks = SqliteRepository::list_content_blocks(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(|record| content_block_to_domain(record, &workspace_id))
                .collect::<Result<Vec<_>, _>>()?;
            ConversationGraph::try_new(turns, runs, content_blocks)
                .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))
        })
    }

    fn get_run(&self, id: &str) -> RepositoryFuture<'_, ModelRun> {
        let repository = self.clone();
        let id = id.to_owned();
        Box::pin(async move {
            let record = SqliteRepository::get_run(&repository, &id)
                .await
                .map_err(port_error)?;
            run_to_domain(record)
        })
    }

    fn get_turn(&self, id: &str) -> RepositoryFuture<'_, Turn> {
        let repository = self.clone();
        let id = id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_turn(&repository, &id)
                .await
                .map(turn_to_domain)
                .map_err(port_error)
        })
    }

    fn list_turns(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<Turn>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_turns(&repository, &workspace_id)
                .await
                .map(|records| records.into_iter().map(turn_to_domain).collect())
                .map_err(port_error)
        })
    }

    fn list_runs_for_turn(&self, turn_id: &str) -> RepositoryFuture<'_, Vec<ModelRun>> {
        let repository = self.clone();
        let turn_id = turn_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_runs_for_turn(&repository, &turn_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(run_to_domain)
                .collect()
        })
    }

    fn list_run_provider_provenance(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<RunProviderProvenance>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_run_provider_provenance(&repository, &workspace_id)
                .await
                .map(|records| {
                    records
                        .into_iter()
                        .map(|record| RunProviderProvenance {
                            run_id: record.run_id,
                            provider_name: record.provider_name,
                            base_url: record.base_url,
                            model: record.model,
                        })
                        .collect()
                })
                .map_err(port_error)
        })
    }

    fn mark_run_connecting(&self, run_id: &str, at: i64) -> RepositoryFuture<'_, ()> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            SqliteRepository::mark_run_connecting(&repository, &run_id, at)
                .await
                .map_err(port_error)
        })
    }

    fn persist_run_start(
        &self,
        start: PersistRunStart,
    ) -> RepositoryFuture<'_, PersistRunStartOutcome> {
        let repository = self.clone();
        Box::pin(async move {
            let bundle = build_start_bundle(&repository, start).await?;
            let outcome = SqliteRepository::persist_run_start(&repository, &bundle)
                .await
                .map_err(port_error)?;
            run_start_outcome_to_port(outcome)
        })
    }

    fn recover_interrupted_runs(&self, recovered_at: i64) -> RepositoryFuture<'_, u64> {
        let repository = self.clone();
        Box::pin(async move {
            SqliteRepository::recover_interrupted_runs(&repository, recovered_at)
                .await
                .map_err(port_error)
        })
    }

    fn get_run_snapshot(&self, run_id: &str) -> RepositoryFuture<'_, ContextSnapshot> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            let receipt = SqliteRepository::get_run_receipt(&repository, &run_id)
                .await
                .map_err(port_error)?;
            let run = SqliteRepository::get_run(&repository, &run_id)
                .await
                .map_err(port_error)?;
            receipt_to_domain(receipt, &run)
        })
    }

    fn list_provider_profiles(&self) -> RepositoryFuture<'_, Vec<ProviderProfile>> {
        let repository = self.clone();
        Box::pin(async move {
            SqliteRepository::list_provider_profiles(&repository)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(provider_profile_to_domain)
                .collect()
        })
    }

    fn get_provider_profile(&self, id: &str) -> RepositoryFuture<'_, ProviderProfile> {
        let repository = self.clone();
        let id = id.to_owned();
        Box::pin(async move {
            let record = SqliteRepository::get_provider_profile(&repository, &id)
                .await
                .map_err(port_error)?;
            provider_profile_to_domain(record)
        })
    }

    fn save_provider_profile(
        &self,
        profile: ProviderProfile,
    ) -> RepositoryFuture<'_, ProviderProfile> {
        let repository = self.clone();
        Box::pin(async move {
            let record = provider_profile_to_record(&profile)?;
            let stored = SqliteRepository::save_provider_profile(&repository, &record)
                .await
                .map_err(port_error)?;
            provider_profile_to_domain(stored)
        })
    }

    fn save_branch_pointer(&self, pointer: BranchPointer) -> RepositoryFuture<'_, BranchPointer> {
        let repository = self.clone();
        Box::pin(async move {
            let stored = match SqliteRepository::get_branch_pointer(&repository, &pointer.id).await
            {
                Ok(existing) => {
                    if existing.workspace_id != pointer.workspace_id
                        || existing.name != pointer.name
                    {
                        return Err(RepositoryPortError::Conflict(format!(
                            "branch pointer `{}` identity cannot be changed",
                            pointer.id
                        )));
                    }
                    let desired_version = i64::try_from(pointer.version).map_err(|_| {
                        RepositoryPortError::InvalidData(
                            "branch version exceeds SQLite range".into(),
                        )
                    })?;
                    if desired_version != existing.version + 1 {
                        return Err(RepositoryPortError::Conflict(format!(
                            "branch pointer `{}` must advance from version {} to {}",
                            pointer.id,
                            existing.version,
                            existing.version + 1
                        )));
                    }
                    SqliteRepository::advance_branch_pointer(
                        &repository,
                        &pointer.id,
                        existing.version,
                        &pointer.head_run_id,
                        pointer.updated_at,
                    )
                    .await
                }
                Err(RepositoryError::NotFound { .. }) => {
                    if pointer.version != 0 {
                        return Err(RepositoryPortError::Conflict(
                            "a new branch pointer must start at version 0".into(),
                        ));
                    }
                    let record = BranchPointerRecord {
                        id: pointer.id.clone(),
                        workspace_id: pointer.workspace_id.clone(),
                        name: pointer.name.clone(),
                        head_run_id: pointer.head_run_id.clone(),
                        version: 0,
                        created_at: pointer.updated_at,
                        updated_at: pointer.updated_at,
                    };
                    SqliteRepository::create_branch_pointer(&repository, &record).await
                }
                Err(error) => Err(error),
            }
            .map_err(port_error)?;
            branch_pointer_to_domain(stored)
        })
    }

    fn list_branch_pointers(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<BranchPointer>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_branch_pointers(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(branch_pointer_to_domain)
                .collect()
        })
    }

    fn rename_branch(
        &self,
        branch_pointer_id: &str,
        name: &str,
        expected_version: u64,
        updated_at: i64,
    ) -> RepositoryFuture<'_, BranchPointer> {
        let repository = self.clone();
        let branch_pointer_id = branch_pointer_id.to_owned();
        let name = name.to_owned();
        Box::pin(async move {
            let expected_version = i64::try_from(expected_version).map_err(|_| {
                RepositoryPortError::InvalidData("branch version exceeds SQLite range".into())
            })?;
            let stored = SqliteRepository::rename_branch(
                &repository,
                &branch_pointer_id,
                &name,
                expected_version,
                updated_at,
            )
            .await
            .map_err(port_error)?;
            branch_pointer_to_domain(stored)
        })
    }

    fn get_context_cursor(&self, workspace_id: &str) -> RepositoryFuture<'_, ContextCursor> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_context_cursor(&repository, &workspace_id)
                .await
                .map_err(port_error)
                .and_then(context_cursor_to_port)
        })
    }

    fn set_context_cursor(
        &self,
        update: ContextCursorUpdate,
    ) -> RepositoryFuture<'_, ContextCursor> {
        let repository = self.clone();
        Box::pin(async move {
            let record = context_cursor_update_to_record(update)?;
            SqliteRepository::set_context_cursor(&repository, &record)
                .await
                .map_err(port_error)
                .and_then(context_cursor_to_port)
        })
    }

    fn set_context_cursor_and_rebase_draft(
        &self,
        update: ContextCursorUpdate,
        expected_draft_version: u64,
    ) -> RepositoryFuture<'_, ContextCursor> {
        let repository = self.clone();
        Box::pin(async move {
            let record = context_cursor_update_to_record(update)?;
            let expected_draft_version = i64::try_from(expected_draft_version).map_err(|_| {
                RepositoryPortError::InvalidData("draft version exceeds SQLite range".into())
            })?;
            SqliteRepository::set_context_cursor_and_rebase_draft(
                &repository,
                &record,
                expected_draft_version,
            )
            .await
            .map_err(port_error)
            .and_then(context_cursor_to_port)
        })
    }

    fn get_context_draft(&self, workspace_id: &str) -> RepositoryFuture<'_, ContextDraft> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_context_draft(&repository, &workspace_id)
                .await
                .map_err(port_error)
                .and_then(context_draft_to_port)
        })
    }

    fn update_context_draft(
        &self,
        update: ContextDraftUpdate,
    ) -> RepositoryFuture<'_, ContextDraft> {
        let repository = self.clone();
        Box::pin(async move {
            let record = context_draft_update_to_record(update)?;
            SqliteRepository::update_context_draft(&repository, &record)
                .await
                .map_err(port_error)
                .and_then(context_draft_to_port)
        })
    }

    fn load_workspace_context_data(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, WorkspaceContextData> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            let records =
                SqliteRepository::load_workspace_context_records(&repository, &workspace_id)
                    .await
                    .map_err(port_error)?;
            let turns = records
                .turns
                .into_iter()
                .map(turn_to_domain)
                .collect::<Vec<_>>();
            let runs = records
                .runs
                .into_iter()
                .map(run_to_domain)
                .collect::<Result<Vec<_>, _>>()?;
            let content_blocks = records
                .content_blocks
                .into_iter()
                .map(|record| content_block_to_domain(record, &workspace_id))
                .collect::<Result<Vec<_>, _>>()?;
            let graph = ConversationGraph::try_new(turns.clone(), runs.clone(), content_blocks)
                .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
            Ok(WorkspaceContextData {
                workspace_id,
                graph,
                turns,
                runs,
                run_provider_provenance: records
                    .run_provider_provenance
                    .into_iter()
                    .map(|record| RunProviderProvenance {
                        run_id: record.run_id,
                        provider_name: record.provider_name,
                        base_url: record.base_url,
                        model: record.model,
                    })
                    .collect(),
                cursor: context_cursor_to_port(records.cursor)?,
                branch_pointers: records
                    .branch_pointers
                    .into_iter()
                    .map(branch_pointer_to_domain)
                    .collect::<Result<Vec<_>, _>>()?,
                branch_revisions: records
                    .branch_revisions
                    .into_iter()
                    .map(branch_revision_to_port)
                    .collect::<Result<Vec<_>, _>>()?,
                branch_checkpoint_inheritance: records
                    .branch_checkpoint_inheritance
                    .into_iter()
                    .map(|record| BranchCheckpointInheritance {
                        workspace_id: record.workspace_id,
                        branch_pointer_id: record.branch_pointer_id,
                        checkpoint_id: record.checkpoint_id,
                        inherited_at: record.inherited_at,
                    })
                    .collect(),
                draft: context_draft_to_port(records.draft)?,
                checkpoints: records
                    .checkpoints
                    .into_iter()
                    .map(context_checkpoint_to_domain)
                    .collect::<Result<Vec<_>, _>>()?,
                view_states: records
                    .view_states
                    .into_iter()
                    .map(|record| ViewState {
                        workspace_id: record.workspace_id,
                        view_key: record.view_key,
                        state_json: record.state_json,
                        updated_at: record.updated_at,
                    })
                    .collect(),
            })
        })
    }

    fn start_context_maintenance(
        &self,
        run: ContextMaintenanceRun,
        guard: MaintenanceContextGuard,
    ) -> RepositoryFuture<'_, ContextMaintenanceStart> {
        let repository = self.clone();
        Box::pin(async move {
            let record = context_maintenance_to_record(&run, None)?;
            let guard = maintenance_guard_to_record(guard)?;
            SqliteRepository::start_context_maintenance(&repository, &record, &guard)
                .await
                .map_err(port_error)
                .and_then(|(run, started)| {
                    Ok(ContextMaintenanceStart {
                        run: context_maintenance_to_domain(run)?,
                        started,
                    })
                })
        })
    }

    fn get_context_maintenance_run(
        &self,
        maintenance_run_id: &str,
    ) -> RepositoryFuture<'_, ContextMaintenanceRun> {
        let repository = self.clone();
        let maintenance_run_id = maintenance_run_id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_context_maintenance_run(&repository, &maintenance_run_id)
                .await
                .map_err(port_error)
                .and_then(context_maintenance_to_domain)
        })
    }

    fn get_context_checkpoint_for_maintenance(
        &self,
        maintenance_run_id: &str,
    ) -> RepositoryFuture<'_, Option<ContextCheckpoint>> {
        let repository = self.clone();
        let maintenance_run_id = maintenance_run_id.to_owned();
        Box::pin(async move {
            SqliteRepository::get_context_checkpoint_for_maintenance(
                &repository,
                &maintenance_run_id,
            )
            .await
            .map_err(port_error)?
            .map(context_checkpoint_to_domain)
            .transpose()
        })
    }

    fn recover_interrupted_context_maintenance(
        &self,
        recovered_at: i64,
    ) -> RepositoryFuture<'_, u64> {
        let repository = self.clone();
        Box::pin(async move {
            SqliteRepository::recover_interrupted_context_maintenance(&repository, recovered_at)
                .await
                .map_err(port_error)
        })
    }

    fn finish_context_maintenance(
        &self,
        run: ContextMaintenanceRun,
        checkpoint: Option<ContextCheckpoint>,
        summary_block: Option<ContentBlock>,
        guard: MaintenanceContextGuard,
        context_update: Option<FinishContextMaintenanceContextUpdate>,
    ) -> RepositoryFuture<'_, ContextMaintenanceRun> {
        let repository = self.clone();
        Box::pin(async move {
            let checkpoint_record = checkpoint
                .as_ref()
                .map(context_checkpoint_to_record)
                .transpose()?;
            let run_record = context_maintenance_to_record(
                &run,
                checkpoint_record
                    .as_ref()
                    .map(|checkpoint| checkpoint.summary_block_id.as_str()),
            )?;
            let summary_record = summary_block
                .map(|block| {
                    if block.workspace_id != run.workspace_id {
                        return Err(RepositoryPortError::InvalidData(
                            "maintenance summary block belongs to another workspace".into(),
                        ));
                    }
                    Ok(ContentBlockRecord {
                        id: block.id,
                        role: role_to_str(block.role).into(),
                        content: block.content,
                        content_hash: block.content_hash,
                        created_at: block.created_at,
                    })
                })
                .transpose()?;
            let context_update = context_update
                .map(finish_maintenance_update_to_record)
                .transpose()?;
            let guard = maintenance_guard_to_record(guard)?;
            SqliteRepository::finish_context_maintenance(
                &repository,
                &run_record,
                checkpoint_record.as_ref(),
                summary_record.as_ref(),
                &guard,
                context_update.as_ref(),
            )
            .await
            .map_err(port_error)
            .and_then(context_maintenance_to_domain)
        })
    }

    fn list_context_maintenance_runs(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<ContextMaintenanceRun>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_context_maintenance_runs(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(context_maintenance_to_domain)
                .collect()
        })
    }

    fn list_context_checkpoints(
        &self,
        workspace_id: &str,
    ) -> RepositoryFuture<'_, Vec<ContextCheckpoint>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_context_checkpoints(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(context_checkpoint_to_domain)
                .collect()
        })
    }

    fn save_decision_mark(&self, mark: DecisionMark) -> RepositoryFuture<'_, DecisionMark> {
        let repository = self.clone();
        Box::pin(async move {
            let record = decision_mark_to_record(&mark);
            let stored = SqliteRepository::save_decision_mark(&repository, &record)
                .await
                .map_err(port_error)?;
            decision_mark_to_domain(stored)
        })
    }

    fn get_decision_mark(
        &self,
        workspace_id: &str,
        run_id: &str,
    ) -> RepositoryFuture<'_, DecisionMark> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            let record = SqliteRepository::get_decision_mark(&repository, &workspace_id, &run_id)
                .await
                .map_err(port_error)?;
            decision_mark_to_domain(record)
        })
    }

    fn list_decision_marks(&self, workspace_id: &str) -> RepositoryFuture<'_, Vec<DecisionMark>> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        Box::pin(async move {
            SqliteRepository::list_decision_marks(&repository, &workspace_id)
                .await
                .map_err(port_error)?
                .into_iter()
                .map(decision_mark_to_domain)
                .collect()
        })
    }

    fn save_view_state(&self, state: ViewState) -> RepositoryFuture<'_, ViewState> {
        let repository = self.clone();
        Box::pin(async move {
            let record = ViewStateRecord {
                workspace_id: state.workspace_id,
                view_key: state.view_key,
                state_json: state.state_json,
                updated_at: state.updated_at,
            };
            let stored = SqliteRepository::save_view_state(&repository, &record)
                .await
                .map_err(port_error)?;
            Ok(ViewState {
                workspace_id: stored.workspace_id,
                view_key: stored.view_key,
                state_json: stored.state_json,
                updated_at: stored.updated_at,
            })
        })
    }

    fn get_view_state(
        &self,
        workspace_id: &str,
        view_key: &str,
    ) -> RepositoryFuture<'_, ViewState> {
        let repository = self.clone();
        let workspace_id = workspace_id.to_owned();
        let view_key = view_key.to_owned();
        Box::pin(async move {
            let stored = SqliteRepository::get_view_state(&repository, &workspace_id, &view_key)
                .await
                .map_err(port_error)?;
            Ok(ViewState {
                workspace_id: stored.workspace_id,
                view_key: stored.view_key,
                state_json: stored.state_json,
                updated_at: stored.updated_at,
            })
        })
    }
}

impl RunPersistencePort for SqliteRepository {
    fn mark_run_streaming(&self, run_id: &str, at: i64) -> RepositoryFuture<'_, ()> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            SqliteRepository::mark_run_streaming(&repository, &run_id, at)
                .await
                .map_err(port_error)
        })
    }

    fn checkpoint_run(
        &self,
        run_id: &str,
        checkpoint: PortRunCheckpoint,
    ) -> RepositoryFuture<'_, CheckpointOutcome> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            let checkpoint = RunCheckpoint {
                output_markdown: checkpoint.output_markdown,
                reasoning_markdown: checkpoint.reasoning_markdown,
                usage_json: encode_usage(checkpoint.usage),
                checkpointed_at: checkpoint.checkpointed_at,
            };
            SqliteRepository::checkpoint_run(&repository, &run_id, &checkpoint)
                .await
                .map(|outcome| match outcome {
                    CheckpointWriteOutcome::Saved => CheckpointOutcome::Saved,
                    CheckpointWriteOutcome::SkippedTerminal(status) => {
                        CheckpointOutcome::SkippedTerminal(status_to_domain(status))
                    }
                })
                .map_err(port_error)
        })
    }

    fn finish_run(&self, run_id: &str, finish: PortRunFinish) -> RepositoryFuture<'_, ()> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            let finish = RunFinish {
                status: status_to_record(finish.status),
                output_markdown: finish.output_markdown,
                reasoning_markdown: finish.reasoning_markdown,
                usage_json: encode_usage(finish.usage),
                error_json: encode_failure(finish.error),
                finished_at: finish.finished_at,
            };
            SqliteRepository::finish_run(&repository, &run_id, &finish)
                .await
                .map_err(port_error)
        })
    }
}

async fn build_start_bundle(
    repository: &SqliteRepository,
    start: PersistRunStart,
) -> Result<RunStartBundle, RepositoryPortError> {
    if start.snapshot.run_id != start.run.id {
        return Err(RepositoryPortError::InvalidData(
            "snapshot and Run identifiers differ".into(),
        ));
    }
    start
        .snapshot
        .provider
        .require_resolved_metadata()
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let provider_id = start
        .snapshot
        .provider
        .provider_id
        .clone()
        .ok_or_else(|| RepositoryPortError::InvalidData("provider id is unresolved".into()))?;
    let template_revision = start.snapshot.provider.template_revision.ok_or_else(|| {
        RepositoryPortError::InvalidData("Provider Template revision is unresolved".into())
    })?;
    let stream_protocol =
        start.snapshot.provider.stream_protocol.ok_or_else(|| {
            RepositoryPortError::InvalidData("stream protocol is unresolved".into())
        })?;
    let auth_placement = start.snapshot.provider.auth_placement.ok_or_else(|| {
        RepositoryPortError::InvalidData("authentication placement is unresolved".into())
    })?;

    let workspace_id = if let Some(turn) = &start.turn {
        turn.workspace_id.clone()
    } else {
        SqliteRepository::get_turn(repository, &start.run.turn_id)
            .await
            .map_err(port_error)?
            .workspace_id
    };
    if start
        .content_blocks
        .iter()
        .any(|block| block.workspace_id != workspace_id)
    {
        return Err(RepositoryPortError::InvalidData(
            "all content blocks in a Run start must belong to its workspace".into(),
        ));
    }

    let mut blocks = start.content_blocks;
    let prompt_block_id = if let Some(turn) = &start.turn {
        match blocks
            .iter()
            .find(|block| block.role == MessageRole::User && block.content == turn.prompt_markdown)
        {
            Some(block) => block.id.clone(),
            None => {
                let hash = sha256(&turn.prompt_markdown);
                let id = format!("block-user-{hash}");
                blocks.push(ContentBlock {
                    id: id.clone(),
                    workspace_id: workspace_id.clone(),
                    role: MessageRole::User,
                    content: turn.prompt_markdown.clone(),
                    content_hash: hash,
                    created_at: turn.created_at,
                });
                id
            }
        }
    } else {
        String::new()
    };

    let mut context_items = Vec::with_capacity(start.snapshot.manifest.items.len());
    for item in &start.snapshot.manifest.items {
        let block_id = match blocks.iter().find(|block| {
            ((!item.content_block_id.is_empty() && block.id == item.content_block_id)
                || item.content_block_id.is_empty())
                && block.content_hash == item.content_hash
                && block.content == item.content
                && block.role == item.role
        }) {
            Some(block) => block.id.clone(),
            None => {
                let id = if item.content_block_id.is_empty() {
                    format!("block-{}-{}", role_to_str(item.role), item.content_hash)
                } else {
                    item.content_block_id.clone()
                };
                blocks.push(ContentBlock {
                    id: id.clone(),
                    workspace_id: workspace_id.clone(),
                    role: item.role,
                    content: item.content.clone(),
                    content_hash: item.content_hash.clone(),
                    created_at: start.snapshot.created_at,
                });
                id
            }
        };
        context_items.push(RunContextItemRecord {
            manifest_id: format!("manifest:{}", start.snapshot.id),
            workspace_id: workspace_id.clone(),
            position: i64::try_from(item.position).map_err(|_| {
                RepositoryPortError::InvalidData("context position exceeds SQLite range".into())
            })?,
            source_id: item.source_id.clone(),
            source_ref_kind: source_ref_kind_to_str(item.source_ref.kind).into(),
            source_ref_id: item.source_ref.id.clone(),
            source_kind: source_kind_to_str(item.source_kind).into(),
            role: role_to_str(item.role).into(),
            content_block_id: block_id,
            inclusion_reason: inclusion_reason_to_str(item.inclusion_reason).into(),
            mandatory: item.mandatory,
        });
    }

    let state = start.run.state_snapshot();
    let effective_parameters = start
        .snapshot
        .provider
        .parameters
        .iter()
        .map(|(name, encoded)| {
            serde_json::from_str::<Value>(encoded)
                .map(|value| (name.clone(), value))
                .map_err(|error| {
                    RepositoryPortError::InvalidData(format!(
                        "Provider parameter `{name}` is not canonical JSON: {error}"
                    ))
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let provider_json = json!({
        "profile_id": start.snapshot.provider.profile_id,
        "provider_id": &provider_id,
        "template_revision": template_revision,
        "provider_name": start.snapshot.provider.provider_name,
        "dialect": dialect_to_str(start.snapshot.provider.dialect),
        "stream_protocol": stream_protocol_to_str(stream_protocol),
        "auth_placement": auth_placement_to_str(auth_placement),
        "auth_header_name": start.snapshot.provider.auth_header_name,
        "additional_headers": start.snapshot.provider.additional_headers,
        "base_url": start.snapshot.provider.base_url,
        "model": start.snapshot.provider.model,
        "parameters": effective_parameters,
    })
    .to_string();
    let parameters_json = serde_json::to_string(&effective_parameters)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let additional_headers_json =
        serde_json::to_string(&start.snapshot.provider.additional_headers)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let request_json = json!({
        "messages": start.snapshot.manifest.items.iter().map(|item| json!({
            "role": role_to_str(item.role),
            "content": item.content,
        })).collect::<Vec<_>>(),
        "model": start.snapshot.provider.model,
        "parameters": effective_parameters,
    })
    .to_string();
    let manifest_id = format!("manifest:{}", start.snapshot.id);
    let estimated_chars = i64::try_from(start.snapshot.manifest.estimated_chars).map_err(|_| {
        RepositoryPortError::InvalidData("context size exceeds SQLite range".into())
    })?;

    let turn = start.turn.map(|turn| TurnRecord {
        id: turn.id,
        workspace_id: turn.workspace_id,
        parent_run_id: turn.parent_run_id,
        prompt_block_id,
        prompt_markdown: turn.prompt_markdown,
        title: turn.title.unwrap_or_default(),
        created_at: turn.created_at,
        deleted_at: None,
    });
    let run = ModelRunRecord {
        id: start.run.id.clone(),
        turn_id: start.run.turn_id.clone(),
        workspace_id: workspace_id.clone(),
        provider_profile_id: start.run.provider_profile_id.clone(),
        model: start.run.model.clone(),
        status: status_to_record(state.status),
        output_markdown: state.output_markdown,
        reasoning_markdown: state.reasoning_markdown,
        provider_snapshot_json: provider_json,
        usage_json: encode_usage(state.usage),
        error_json: encode_failure(state.error),
        created_at: start.run.created_at,
        started_at: state.started_at,
        finished_at: state.finished_at,
        checkpointed_at: state.checkpointed_at,
    };
    let content_blocks = blocks
        .into_iter()
        .map(|block| ContentBlockRecord {
            id: block.id,
            role: role_to_str(block.role).into(),
            content: block.content,
            content_hash: block.content_hash,
            created_at: block.created_at,
        })
        .collect();
    let manifest = ContextManifestRecord {
        id: manifest_id.clone(),
        workspace_id: workspace_id.clone(),
        compiler_version: start.snapshot.manifest.compiler_version.clone(),
        strategy: "ancestor_path_with_pins".into(),
        estimated_chars,
        canonical_hash: start.snapshot.manifest.canonical_hash.clone(),
        warnings_json: encode_context_warnings(&start.snapshot.manifest.warnings)?,
        checkpoint_provenance_json: start
            .snapshot
            .manifest
            .checkpoint_provenance
            .as_ref()
            .map(encode_checkpoint_provenance)
            .transpose()?,
        branch_summary_provenance_json: encode_checkpoint_provenances(
            &start.snapshot.manifest.branch_summary_provenance,
        )?,
        created_at: start.snapshot.created_at,
    };
    let snapshot = ContextSnapshotRecord {
        id: start.snapshot.id,
        run_id: start.run.id.clone(),
        manifest_id,
        workspace_id: workspace_id.clone(),
        provider_profile_id: non_empty(start.snapshot.provider.profile_id),
        provider_id: Some(provider_id),
        template_revision: Some(i64::from(template_revision)),
        stream_protocol: Some(stream_protocol_to_str(stream_protocol).into()),
        auth_placement: Some(auth_placement_to_str(auth_placement).into()),
        auth_header_name: start.snapshot.provider.auth_header_name,
        additional_headers_json,
        provider: start.snapshot.provider.provider_name,
        model: start.snapshot.provider.model,
        base_url: start.snapshot.provider.base_url,
        parameters_json,
        request_json,
        canonical_hash: start.snapshot.manifest.canonical_hash,
        created_at: start.snapshot.created_at,
    };
    let branch_pointer = start
        .branch_pointer
        .map(|pointer| {
            Ok(BranchPointerRecord {
                id: pointer.id,
                workspace_id: pointer.workspace_id,
                name: pointer.name,
                head_run_id: pointer.head_run_id,
                version: i64::try_from(pointer.version).map_err(|_| {
                    RepositoryPortError::InvalidData("branch version exceeds SQLite range".into())
                })?,
                created_at: pointer.updated_at,
                updated_at: pointer.updated_at,
            })
        })
        .transpose()?;

    Ok(RunStartBundle {
        turn,
        run,
        content_blocks,
        manifest,
        context_items,
        snapshot,
        branch_pointer,
        context_update: start
            .context_update
            .map(|update| {
                Ok(RunStartContextUpdateRecord {
                    expected_cursor_version: i64::try_from(update.expected_cursor_version)
                        .map_err(|_| {
                            RepositoryPortError::InvalidData(
                                "cursor version exceeds SQLite range".into(),
                            )
                        })?,
                    expected_draft_version: i64::try_from(update.expected_draft_version).map_err(
                        |_| {
                            RepositoryPortError::InvalidData(
                                "draft version exceeds SQLite range".into(),
                            )
                        },
                    )?,
                    expected_branch_pointer_id: update.expected_branch_pointer_id,
                    expected_branch_version: update
                        .expected_branch_version
                        .map(i64::try_from)
                        .transpose()
                        .map_err(|_| {
                            RepositoryPortError::InvalidData(
                                "branch version exceeds SQLite range".into(),
                            )
                        })?,
                    result_branch_pointer_id: update.result_branch_pointer_id,
                    updated_at: update.updated_at,
                })
            })
            .transpose()?,
    })
}

fn receipt_to_domain(
    receipt: StoredRunReceipt,
    run: &ModelRunRecord,
) -> Result<ContextSnapshot, RepositoryPortError> {
    let provider_value: Value = serde_json::from_str(&run.provider_snapshot_json)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let dialect = dialect_from_str(
        provider_value
            .get("dialect")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RepositoryPortError::InvalidData("provider dialect is missing".into())
            })?,
    )?;
    let parameters = decode_parameters(&receipt.snapshot.parameters_json)?;
    let additional_headers = decode_parameters(&receipt.snapshot.additional_headers_json)?;
    let items = receipt
        .items
        .into_iter()
        .map(|item| {
            Ok(RunContextItem {
                position: usize::try_from(item.position).map_err(|_| {
                    RepositoryPortError::InvalidData("negative context position".into())
                })?,
                source_id: item.source_id.clone(),
                source_ref: source_ref_from_stored(&item)?,
                source_kind: source_kind_from_str(&item.source_kind)?,
                role: role_from_str(&item.role)?,
                content: item.content,
                content_block_id: item.content_block_id,
                content_hash: item.content_hash,
                inclusion_reason: inclusion_reason_from_str(&item.inclusion_reason)?,
                mandatory: item.mandatory,
            })
        })
        .collect::<Result<Vec<_>, RepositoryPortError>>()?;
    Ok(ContextSnapshot {
        id: receipt.snapshot.id,
        run_id: receipt.snapshot.run_id,
        manifest: ContextManifest {
            compiler_version: receipt.manifest.compiler_version,
            items,
            estimated_chars: usize::try_from(receipt.manifest.estimated_chars)
                .map_err(|_| RepositoryPortError::InvalidData("negative context size".into()))?,
            canonical_hash: receipt.manifest.canonical_hash,
            warnings: decode_context_warnings(&receipt.manifest.warnings_json)?,
            checkpoint_provenance: receipt
                .manifest
                .checkpoint_provenance_json
                .as_deref()
                .map(decode_checkpoint_provenance)
                .transpose()?,
            branch_summary_provenance: decode_checkpoint_provenances(
                &receipt.manifest.branch_summary_provenance_json,
            )?,
        },
        provider: ProviderSnapshot {
            profile_id: receipt.snapshot.provider_profile_id.unwrap_or_default(),
            provider_id: receipt.snapshot.provider_id,
            template_revision: receipt
                .snapshot
                .template_revision
                .map(|revision| {
                    u16::try_from(revision).map_err(|_| {
                        RepositoryPortError::InvalidData(
                            "invalid Provider Template revision".into(),
                        )
                    })
                })
                .transpose()?,
            provider_name: receipt.snapshot.provider,
            dialect,
            stream_protocol: receipt
                .snapshot
                .stream_protocol
                .as_deref()
                .map(stream_protocol_from_str)
                .transpose()?,
            auth_placement: receipt
                .snapshot
                .auth_placement
                .as_deref()
                .map(auth_placement_from_str)
                .transpose()?,
            auth_header_name: receipt.snapshot.auth_header_name,
            additional_headers,
            base_url: receipt.snapshot.base_url,
            model: receipt.snapshot.model,
            parameters,
        },
        created_at: receipt.snapshot.created_at,
    })
}

fn workspace_to_record(workspace: &Workspace) -> WorkspaceRecord {
    WorkspaceRecord {
        id: workspace.id.clone(),
        title: workspace.title.clone(),
        goal: workspace.goal.clone(),
        system_prompt: workspace.system_prompt.clone(),
        created_at: workspace.created_at,
        updated_at: workspace.updated_at,
        archived_at: workspace.archived_at,
    }
}

fn workspace_to_domain(record: WorkspaceRecord) -> Workspace {
    Workspace {
        id: record.id,
        title: record.title,
        goal: record.goal,
        system_prompt: record.system_prompt,
        created_at: record.created_at,
        updated_at: record.updated_at,
        archived_at: record.archived_at,
    }
}

fn turn_to_domain(record: TurnRecord) -> Turn {
    Turn {
        id: record.id,
        workspace_id: record.workspace_id,
        parent_run_id: record.parent_run_id,
        prompt_markdown: record.prompt_markdown,
        title: non_empty(record.title),
        created_at: record.created_at,
    }
}

fn run_to_domain(record: ModelRunRecord) -> Result<ModelRun, RepositoryPortError> {
    let usage = decode_usage(record.usage_json.as_deref())?;
    let error = decode_error(record.error_json.as_deref())?;
    ModelRun::rehydrate(
        RunDraft {
            id: record.id,
            turn_id: record.turn_id,
            provider_profile_id: record.provider_profile_id,
            model: record.model,
            created_at: record.created_at,
        },
        RunStateSnapshot {
            status: status_to_domain(record.status),
            output_markdown: record.output_markdown,
            reasoning_markdown: record.reasoning_markdown,
            error,
            usage,
            started_at: record.started_at,
            checkpointed_at: record.checkpointed_at,
            finished_at: record.finished_at,
        },
    )
    .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))
}

fn content_block_to_domain(
    record: ContentBlockRecord,
    workspace_id: &str,
) -> Result<ContentBlock, RepositoryPortError> {
    Ok(ContentBlock {
        id: record.id,
        workspace_id: workspace_id.into(),
        role: role_from_str(&record.role)?,
        content: record.content,
        content_hash: record.content_hash,
        created_at: record.created_at,
    })
}

fn provider_profile_to_record(
    profile: &ProviderProfile,
) -> Result<ProviderProfileRecord, RepositoryPortError> {
    Ok(ProviderProfileRecord {
        id: profile.id.clone(),
        provider_id: profile.provider_id.clone(),
        name: profile.name.clone(),
        dialect: dialect_to_str(profile.dialect).into(),
        base_url: profile.base_url.clone(),
        default_model: profile.model.clone(),
        parameters_json: serde_json::to_string(&profile.parameters)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?,
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    })
}

fn provider_profile_to_domain(
    record: ProviderProfileRecord,
) -> Result<ProviderProfile, RepositoryPortError> {
    Ok(ProviderProfile {
        id: record.id,
        provider_id: record.provider_id,
        name: record.name,
        dialect: dialect_from_str(&record.dialect)?,
        base_url: record.base_url,
        model: record.default_model,
        parameters: decode_parameters(&record.parameters_json)?,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn branch_pointer_to_domain(
    record: BranchPointerRecord,
) -> Result<BranchPointer, RepositoryPortError> {
    Ok(BranchPointer {
        id: record.id,
        workspace_id: record.workspace_id,
        name: record.name,
        head_run_id: record.head_run_id,
        version: u64::try_from(record.version)
            .map_err(|_| RepositoryPortError::InvalidData("negative branch version".into()))?,
        updated_at: record.updated_at,
    })
}

fn context_cursor_to_port(
    record: ContextCursorRecord,
) -> Result<ContextCursor, RepositoryPortError> {
    Ok(ContextCursor {
        workspace_id: record.workspace_id,
        active_run_id: record.active_run_id,
        branch_pointer_id: record.branch_pointer_id,
        version: u64::try_from(record.version)
            .map_err(|_| RepositoryPortError::InvalidData("negative cursor version".into()))?,
        updated_at: record.updated_at,
    })
}

fn branch_revision_to_port(
    record: BranchRevisionRecord,
) -> Result<BranchRevision, RepositoryPortError> {
    Ok(BranchRevision {
        branch_pointer_id: record.branch_pointer_id,
        workspace_id: record.workspace_id,
        revision: u64::try_from(record.revision)
            .map_err(|_| RepositoryPortError::InvalidData("negative branch revision".into()))?,
        name: record.name,
        head_run_id: record.head_run_id,
        change: match record.change_kind.as_str() {
            "migration_baseline" => BranchRevisionChange::MigrationBaseline,
            "created" => BranchRevisionChange::Created,
            "advanced" => BranchRevisionChange::Advanced,
            "renamed" => BranchRevisionChange::Renamed,
            other => {
                return Err(RepositoryPortError::InvalidData(format!(
                    "unknown branch revision change `{other}`"
                )));
            }
        },
        created_at: record.created_at,
    })
}

fn context_cursor_update_to_record(
    update: ContextCursorUpdate,
) -> Result<ContextCursorUpdateRecord, RepositoryPortError> {
    Ok(ContextCursorUpdateRecord {
        workspace_id: update.workspace_id,
        active_run_id: update.active_run_id,
        branch_pointer_id: update.branch_pointer_id,
        expected_version: i64::try_from(update.expected_version).map_err(|_| {
            RepositoryPortError::InvalidData("cursor version exceeds SQLite range".into())
        })?,
        updated_at: update.updated_at,
    })
}

fn run_start_outcome_to_port(
    record: RunStartOutcomeRecord,
) -> Result<PersistRunStartOutcome, RepositoryPortError> {
    Ok(PersistRunStartOutcome {
        cursor: context_cursor_to_port(record.cursor)?,
        draft_version: u64::try_from(record.draft_version)
            .map_err(|_| RepositoryPortError::InvalidData("negative draft version".into()))?,
        branch_pointer: record
            .branch_pointer
            .map(branch_pointer_to_domain)
            .transpose()?,
    })
}

fn context_draft_to_port(record: ContextDraftRecord) -> Result<ContextDraft, RepositoryPortError> {
    Ok(ContextDraft {
        workspace_id: record.workspace_id,
        parent_run_id: record.parent_run_id,
        version: u64::try_from(record.version)
            .map_err(|_| RepositoryPortError::InvalidData("negative draft version".into()))?,
        items: record
            .items
            .into_iter()
            .map(context_override_item_to_port)
            .collect::<Result<Vec<_>, _>>()?,
        consumed_by_run_id: record.consumed_by_run_id,
        updated_at: record.updated_at,
    })
}

fn context_draft_update_to_record(
    update: ContextDraftUpdate,
) -> Result<ContextDraftUpdateRecord, RepositoryPortError> {
    let workspace_id = update.workspace_id;
    let updated_at = update.updated_at;
    let content_blocks = update
        .content_blocks
        .into_iter()
        .map(|block| {
            if block.workspace_id != workspace_id {
                return Err(RepositoryPortError::InvalidData(
                    "draft Content Block belongs to another workspace".into(),
                ));
            }
            Ok(ContentBlockRecord {
                id: block.id,
                role: role_to_str(block.role).into(),
                content: block.content,
                content_hash: block.content_hash,
                created_at: block.created_at,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let items = update
        .items
        .into_iter()
        .enumerate()
        .map(|(expected_position, item)| {
            if item.position != expected_position {
                return Err(RepositoryPortError::InvalidData(
                    "context override positions must be contiguous".into(),
                ));
            }
            Ok(ContextOverrideItemRecord {
                workspace_id: workspace_id.clone(),
                position: i64::try_from(item.position).map_err(|_| {
                    RepositoryPortError::InvalidData(
                        "context override position exceeds SQLite range".into(),
                    )
                })?,
                operation: match item.operation {
                    ContextOverrideOperation::Pin => "pin",
                    ContextOverrideOperation::Exclude => "exclude",
                }
                .into(),
                source_kind: source_ref_kind_to_str(item.source_ref.kind).into(),
                source_id: item.source_ref.id,
                content_block_id: item.content_block_id,
                content_hash: item.content_hash,
                created_at: updated_at,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ContextDraftUpdateRecord {
        workspace_id,
        parent_run_id: update.parent_run_id,
        expected_version: i64::try_from(update.expected_version).map_err(|_| {
            RepositoryPortError::InvalidData("draft version exceeds SQLite range".into())
        })?,
        content_blocks,
        items,
        updated_at,
    })
}

fn context_override_item_to_port(
    record: ContextOverrideItemRecord,
) -> Result<ContextOverrideItem, RepositoryPortError> {
    Ok(ContextOverrideItem {
        position: usize::try_from(record.position).map_err(|_| {
            RepositoryPortError::InvalidData("negative context override position".into())
        })?,
        operation: match record.operation.as_str() {
            "pin" => ContextOverrideOperation::Pin,
            "exclude" => ContextOverrideOperation::Exclude,
            other => {
                return Err(RepositoryPortError::InvalidData(format!(
                    "unknown context override operation `{other}`"
                )));
            }
        },
        source_ref: ContextSourceRef {
            kind: source_ref_kind_from_str(&record.source_kind)?,
            id: record.source_id,
        },
        content_block_id: record.content_block_id,
        content_hash: record.content_hash,
    })
}

fn maintenance_guard_to_record(
    guard: MaintenanceContextGuard,
) -> Result<MaintenanceContextGuardRecord, RepositoryPortError> {
    Ok(MaintenanceContextGuardRecord {
        workspace_id: guard.workspace_id,
        expected_cursor_version: i64::try_from(guard.expected_cursor_version).map_err(|_| {
            RepositoryPortError::InvalidData("cursor version exceeds SQLite range".into())
        })?,
        expected_draft_version: guard
            .expected_draft_version
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                RepositoryPortError::InvalidData("draft version exceeds SQLite range".into())
            })?,
        branch_pointer_id: guard.branch_pointer_id,
        expected_branch_version: guard
            .expected_branch_version
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                RepositoryPortError::InvalidData("branch version exceeds SQLite range".into())
            })?,
    })
}

fn finish_maintenance_update_to_record(
    update: FinishContextMaintenanceContextUpdate,
) -> Result<FinishContextMaintenanceUpdateRecord, RepositoryPortError> {
    Ok(FinishContextMaintenanceUpdateRecord {
        active_run_id: update.active_run_id,
        branch_pointer_id: update.branch_pointer_id,
        updated_at: update.updated_at,
    })
}

fn context_maintenance_to_record(
    run: &ContextMaintenanceRun,
    summary_block_id: Option<&str>,
) -> Result<ContextMaintenanceRunRecord, RepositoryPortError> {
    if run.branch_pointer_id.is_some() != run.branch_revision.is_some() {
        return Err(RepositoryPortError::InvalidData(
            "maintenance branch identity and revision must be present together".into(),
        ));
    }
    Ok(ContextMaintenanceRunRecord {
        id: run.id.clone(),
        workspace_id: run.workspace_id.clone(),
        kind: checkpoint_kind_to_str(run.kind).into(),
        status: maintenance_status_to_str(run.status).into(),
        branch_pointer_id: run.branch_pointer_id.clone(),
        branch_revision: run
            .branch_revision
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                RepositoryPortError::InvalidData("branch revision exceeds SQLite range".into())
            })?,
        anchor_run_id: run.anchor_run_id.clone(),
        first_kept_run_id: run.first_kept_run_id.clone(),
        source_run_ids_json: serde_json::to_string(&run.source_run_ids)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?,
        source_hash: run.source_hash.clone(),
        provider_snapshot_json: run
            .provider
            .as_ref()
            .map(encode_provider_snapshot)
            .transpose()?,
        request_json: run.request_json.clone(),
        summary_block_id: summary_block_id.map(str::to_owned),
        summary: run.summary.clone(),
        error_json: run
            .error
            .as_ref()
            .map(|message| json!({ "message": message }).to_string()),
        created_at: run.created_at,
        started_at: run.started_at,
        finished_at: run.finished_at,
    })
}

fn context_maintenance_to_domain(
    record: ContextMaintenanceRunRecord,
) -> Result<ContextMaintenanceRun, RepositoryPortError> {
    Ok(ContextMaintenanceRun {
        id: record.id,
        workspace_id: record.workspace_id,
        kind: checkpoint_kind_from_str(&record.kind)?,
        status: maintenance_status_from_str(&record.status)?,
        branch_pointer_id: record.branch_pointer_id,
        branch_revision: record
            .branch_revision
            .map(u64::try_from)
            .transpose()
            .map_err(|_| RepositoryPortError::InvalidData("negative branch revision".into()))?,
        anchor_run_id: record.anchor_run_id,
        first_kept_run_id: record.first_kept_run_id,
        source_run_ids: serde_json::from_str(&record.source_run_ids_json)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?,
        source_hash: record.source_hash,
        provider: record
            .provider_snapshot_json
            .as_deref()
            .map(decode_provider_snapshot)
            .transpose()?,
        request_json: record.request_json,
        summary: record.summary,
        error: record
            .error_json
            .as_deref()
            .map(decode_maintenance_error)
            .transpose()?,
        created_at: record.created_at,
        started_at: record.started_at,
        finished_at: record.finished_at,
    })
}

fn context_checkpoint_to_record(
    checkpoint: &ContextCheckpoint,
) -> Result<ContextCheckpointRecord, RepositoryPortError> {
    if checkpoint.branch_pointer_id.is_some() != checkpoint.branch_revision.is_some() {
        return Err(RepositoryPortError::InvalidData(
            "checkpoint branch identity and revision must be present together".into(),
        ));
    }
    Ok(ContextCheckpointRecord {
        id: checkpoint.id.clone(),
        workspace_id: checkpoint.workspace_id.clone(),
        maintenance_run_id: checkpoint.maintenance_run_id.clone(),
        kind: checkpoint_kind_to_str(checkpoint.kind).into(),
        branch_pointer_id: checkpoint.branch_pointer_id.clone(),
        branch_revision: checkpoint
            .branch_revision
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                RepositoryPortError::InvalidData("branch revision exceeds SQLite range".into())
            })?,
        anchor_run_id: checkpoint.anchor_run_id.clone(),
        first_kept_run_id: checkpoint.first_kept_run_id.clone(),
        summary_block_id: checkpoint.summary_content_block_id.clone(),
        summary: checkpoint.summary.clone(),
        summary_content_hash: sha256(&checkpoint.summary),
        source_run_ids_json: serde_json::to_string(&checkpoint.source_run_ids)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?,
        source_hash: checkpoint.source_hash.clone(),
        provider_snapshot_json: checkpoint
            .provider
            .as_ref()
            .map(encode_provider_snapshot)
            .transpose()?,
        created_at: checkpoint.created_at,
    })
}

fn context_checkpoint_to_domain(
    record: ContextCheckpointRecord,
) -> Result<ContextCheckpoint, RepositoryPortError> {
    Ok(ContextCheckpoint {
        id: record.id,
        workspace_id: record.workspace_id,
        maintenance_run_id: record.maintenance_run_id,
        kind: checkpoint_kind_from_str(&record.kind)?,
        branch_pointer_id: record.branch_pointer_id,
        branch_revision: record
            .branch_revision
            .map(u64::try_from)
            .transpose()
            .map_err(|_| RepositoryPortError::InvalidData("negative branch revision".into()))?,
        anchor_run_id: record.anchor_run_id,
        first_kept_run_id: record.first_kept_run_id,
        summary: record.summary,
        summary_content_block_id: record.summary_block_id,
        source_run_ids: serde_json::from_str(&record.source_run_ids_json)
            .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?,
        source_hash: record.source_hash,
        provider: record
            .provider_snapshot_json
            .as_deref()
            .map(decode_provider_snapshot)
            .transpose()?,
        created_at: record.created_at,
    })
}

fn maintenance_status_to_str(status: ContextMaintenanceStatus) -> &'static str {
    match status {
        ContextMaintenanceStatus::Queued => "queued",
        ContextMaintenanceStatus::Running => "running",
        ContextMaintenanceStatus::Completed => "completed",
        ContextMaintenanceStatus::Failed => "failed",
        ContextMaintenanceStatus::Cancelled => "cancelled",
        ContextMaintenanceStatus::Conflicted => "conflicted",
    }
}

fn maintenance_status_from_str(
    value: &str,
) -> Result<ContextMaintenanceStatus, RepositoryPortError> {
    match value {
        "queued" => Ok(ContextMaintenanceStatus::Queued),
        "running" => Ok(ContextMaintenanceStatus::Running),
        "completed" => Ok(ContextMaintenanceStatus::Completed),
        "failed" => Ok(ContextMaintenanceStatus::Failed),
        "cancelled" => Ok(ContextMaintenanceStatus::Cancelled),
        "conflicted" => Ok(ContextMaintenanceStatus::Conflicted),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown context maintenance status `{other}`"
        ))),
    }
}

fn encode_provider_snapshot(provider: &ProviderSnapshot) -> Result<String, RepositoryPortError> {
    let parameters = provider
        .parameters
        .iter()
        .map(|(name, encoded)| {
            serde_json::from_str::<Value>(encoded)
                .map(|value| (name.clone(), value))
                .map_err(|error| {
                    RepositoryPortError::InvalidData(format!(
                        "Provider parameter `{name}` is not canonical JSON: {error}"
                    ))
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(json!({
        "profile_id": provider.profile_id,
        "provider_id": provider.provider_id,
        "template_revision": provider.template_revision,
        "provider_name": provider.provider_name,
        "dialect": dialect_to_str(provider.dialect),
        "stream_protocol": provider.stream_protocol.map(stream_protocol_to_str),
        "auth_placement": provider.auth_placement.map(auth_placement_to_str),
        "auth_header_name": provider.auth_header_name,
        "additional_headers": provider.additional_headers,
        "base_url": provider.base_url,
        "model": provider.model,
        "parameters": parameters,
    })
    .to_string())
}

fn decode_provider_snapshot(value: &str) -> Result<ProviderSnapshot, RepositoryPortError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let parameters = serde_json::to_string(
        &value
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let additional_headers = serde_json::to_string(
        &value
            .get("additional_headers")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    Ok(ProviderSnapshot {
        profile_id: required_json_string(&value, "profile_id")?,
        provider_id: optional_json_string(&value, "provider_id")?,
        template_revision: value
            .get("template_revision")
            .and_then(Value::as_u64)
            .map(u16::try_from)
            .transpose()
            .map_err(|_| {
                RepositoryPortError::InvalidData("invalid Provider Template revision".into())
            })?,
        provider_name: required_json_string(&value, "provider_name")?,
        dialect: dialect_from_str(&required_json_string(&value, "dialect")?)?,
        stream_protocol: optional_json_string(&value, "stream_protocol")?
            .as_deref()
            .map(stream_protocol_from_str)
            .transpose()?,
        auth_placement: optional_json_string(&value, "auth_placement")?
            .as_deref()
            .map(auth_placement_from_str)
            .transpose()?,
        auth_header_name: optional_json_string(&value, "auth_header_name")?,
        additional_headers: decode_parameters(&additional_headers)?,
        base_url: required_json_string(&value, "base_url")?,
        model: required_json_string(&value, "model")?,
        parameters: decode_parameters(&parameters)?,
    })
}

fn decode_maintenance_error(value: &str) -> Result<String, RepositoryPortError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    Ok(value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string()))
}

fn decision_mark_to_record(mark: &DecisionMark) -> DecisionMarkRecord {
    DecisionMarkRecord {
        id: mark.id.clone(),
        workspace_id: mark.workspace_id.clone(),
        run_id: mark.run_id.clone(),
        status: match mark.status {
            DecisionStatus::Adopted => "adopted",
            DecisionStatus::Rejected => "rejected",
            DecisionStatus::NeedsValidation => "needs_validation",
        }
        .into(),
        reason: mark.reason.clone(),
        created_at: mark.created_at,
        updated_at: mark.updated_at,
    }
}

fn decision_mark_to_domain(
    record: DecisionMarkRecord,
) -> Result<DecisionMark, RepositoryPortError> {
    let status = match record.status.as_str() {
        "adopted" => DecisionStatus::Adopted,
        "rejected" => DecisionStatus::Rejected,
        "needs_validation" => DecisionStatus::NeedsValidation,
        other => {
            return Err(RepositoryPortError::InvalidData(format!(
                "unknown decision status `{other}`"
            )));
        }
    };
    Ok(DecisionMark {
        id: record.id,
        workspace_id: record.workspace_id,
        run_id: record.run_id,
        status,
        reason: record.reason,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn status_to_record(status: RunStatus) -> RunStatusRecord {
    match status {
        RunStatus::Queued => RunStatusRecord::Queued,
        RunStatus::Connecting => RunStatusRecord::Connecting,
        RunStatus::Streaming => RunStatusRecord::Streaming,
        RunStatus::Completed => RunStatusRecord::Completed,
        RunStatus::Cancelled => RunStatusRecord::Cancelled,
        RunStatus::Failed => RunStatusRecord::Failed,
        RunStatus::Interrupted => RunStatusRecord::Interrupted,
    }
}

fn status_to_domain(status: RunStatusRecord) -> RunStatus {
    match status {
        RunStatusRecord::Queued => RunStatus::Queued,
        RunStatusRecord::Connecting => RunStatus::Connecting,
        RunStatusRecord::Streaming => RunStatus::Streaming,
        RunStatusRecord::Completed => RunStatus::Completed,
        RunStatusRecord::Cancelled => RunStatus::Cancelled,
        RunStatusRecord::Failed => RunStatus::Failed,
        RunStatusRecord::Interrupted => RunStatus::Interrupted,
    }
}

fn encode_usage(usage: Option<RunUsage>) -> Option<String> {
    usage.map(|usage| {
        json!({
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
        })
        .to_string()
    })
}

fn decode_usage(value: Option<&str>) -> Result<Option<RunUsage>, RepositoryPortError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let input_tokens = value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = value
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(Some(RunUsage::new(input_tokens, output_tokens)))
}

fn decode_error(value: Option<&str>) -> Result<Option<RunFailure>, RepositoryPortError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| value.to_string());
    Ok(Some(RunFailure {
        code: value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("run_failed")
            .into(),
        message,
        retryable: value
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        status: value
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok()),
    }))
}

fn encode_failure(failure: Option<RunFailure>) -> Option<String> {
    failure.map(|failure| {
        json!({
            "code": failure.code,
            "message": failure.message,
            "retryable": failure.retryable,
            "status": failure.status,
        })
        .to_string()
    })
}

fn decode_parameters(value: &str) -> Result<BTreeMap<String, String>, RepositoryPortError> {
    let values = serde_json::from_str::<BTreeMap<String, Value>>(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    Ok(values
        .into_iter()
        .map(|(key, value)| {
            let encoded = match value {
                Value::String(value) => value,
                value => value.to_string(),
            };
            (key, encoded)
        })
        .collect())
}

fn role_to_str(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn role_from_str(value: &str) -> Result<MessageRole, RepositoryPortError> {
    match value {
        "system" => Ok(MessageRole::System),
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown message role `{other}`"
        ))),
    }
}

fn dialect_to_str(dialect: ProviderDialect) -> &'static str {
    match dialect {
        ProviderDialect::OpenAiCompatible => "openai_chat_completions",
        ProviderDialect::Ollama => "ollama_chat",
        ProviderDialect::Anthropic => "anthropic_messages",
        ProviderDialect::GoogleGenerativeAi => "google_generative_ai",
    }
}

fn dialect_from_str(value: &str) -> Result<ProviderDialect, RepositoryPortError> {
    match value {
        "openai_chat_completions" => Ok(ProviderDialect::OpenAiCompatible),
        "ollama_chat" => Ok(ProviderDialect::Ollama),
        "anthropic_messages" => Ok(ProviderDialect::Anthropic),
        "google_generative_ai" => Ok(ProviderDialect::GoogleGenerativeAi),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown provider dialect `{other}`"
        ))),
    }
}

fn stream_protocol_to_str(value: crate::domain::StreamProtocol) -> &'static str {
    match value {
        crate::domain::StreamProtocol::OpenAiSse => "openai_sse",
        crate::domain::StreamProtocol::OllamaNdjson => "ollama_ndjson",
        crate::domain::StreamProtocol::AnthropicSse => "anthropic_sse",
        crate::domain::StreamProtocol::GoogleSse => "google_sse",
    }
}

fn stream_protocol_from_str(
    value: &str,
) -> Result<crate::domain::StreamProtocol, RepositoryPortError> {
    match value {
        "openai_sse" => Ok(crate::domain::StreamProtocol::OpenAiSse),
        "ollama_ndjson" => Ok(crate::domain::StreamProtocol::OllamaNdjson),
        "anthropic_sse" => Ok(crate::domain::StreamProtocol::AnthropicSse),
        "google_sse" => Ok(crate::domain::StreamProtocol::GoogleSse),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown Provider stream protocol `{other}`"
        ))),
    }
}

fn auth_placement_to_str(value: crate::domain::AuthPlacement) -> &'static str {
    match value {
        crate::domain::AuthPlacement::None => "none",
        crate::domain::AuthPlacement::BearerHeader => "bearer_header",
        crate::domain::AuthPlacement::ApiKeyHeader => "api_key_header",
        crate::domain::AuthPlacement::QueryParam => "query_param",
    }
}

fn auth_placement_from_str(
    value: &str,
) -> Result<crate::domain::AuthPlacement, RepositoryPortError> {
    match value {
        "none" => Ok(crate::domain::AuthPlacement::None),
        "bearer_header" => Ok(crate::domain::AuthPlacement::BearerHeader),
        "api_key_header" => Ok(crate::domain::AuthPlacement::ApiKeyHeader),
        "query_param" => Ok(crate::domain::AuthPlacement::QueryParam),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown Provider auth placement `{other}`"
        ))),
    }
}

fn source_kind_to_str(kind: ContextSourceKind) -> &'static str {
    match kind {
        ContextSourceKind::System => "system",
        ContextSourceKind::TurnPrompt => "turn_prompt",
        ContextSourceKind::ModelRun => "model_run",
        ContextSourceKind::Pinned => "pinned",
        ContextSourceKind::CompactionSummary => "compaction_summary",
        ContextSourceKind::BranchSummary => "branch_summary",
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn source_kind_from_str(value: &str) -> Result<ContextSourceKind, RepositoryPortError> {
    match value {
        "system" => Ok(ContextSourceKind::System),
        "turn_prompt" => Ok(ContextSourceKind::TurnPrompt),
        "model_run" => Ok(ContextSourceKind::ModelRun),
        "pinned" => Ok(ContextSourceKind::Pinned),
        "compaction_summary" => Ok(ContextSourceKind::CompactionSummary),
        "branch_summary" => Ok(ContextSourceKind::BranchSummary),
        "current_prompt" => Ok(ContextSourceKind::CurrentPrompt),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown context source kind `{other}`"
        ))),
    }
}

fn inclusion_reason_to_str(reason: InclusionReason) -> &'static str {
    match reason {
        InclusionReason::SystemPolicy => "system_policy",
        InclusionReason::ExactAncestorPath => "exact_ancestor_path",
        InclusionReason::ExplicitPin => "explicit_pin",
        InclusionReason::LatestCompaction => "latest_compaction",
        InclusionReason::BranchSummary => "branch_summary",
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

fn inclusion_reason_from_str(value: &str) -> Result<InclusionReason, RepositoryPortError> {
    match value {
        "system_policy" => Ok(InclusionReason::SystemPolicy),
        "exact_ancestor_path" => Ok(InclusionReason::ExactAncestorPath),
        "explicit_pin" => Ok(InclusionReason::ExplicitPin),
        "latest_compaction" => Ok(InclusionReason::LatestCompaction),
        "branch_summary" => Ok(InclusionReason::BranchSummary),
        "current_prompt" => Ok(InclusionReason::CurrentPrompt),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown inclusion reason `{other}`"
        ))),
    }
}

fn source_ref_kind_to_str(kind: ContextSourceRefKind) -> &'static str {
    match kind {
        ContextSourceRefKind::WorkspaceSystem => "workspace_system",
        ContextSourceRefKind::TurnPrompt => "turn_prompt",
        ContextSourceRefKind::ModelRun => "model_run",
        ContextSourceRefKind::ContentBlock => "content_block",
        ContextSourceRefKind::CurrentPrompt => "current_prompt",
        ContextSourceRefKind::CheckpointSummary => "checkpoint_summary",
        ContextSourceRefKind::BranchSummary => "branch_summary",
    }
}

fn source_ref_kind_from_str(value: &str) -> Result<ContextSourceRefKind, RepositoryPortError> {
    match value {
        "workspace_system" => Ok(ContextSourceRefKind::WorkspaceSystem),
        "turn_prompt" => Ok(ContextSourceRefKind::TurnPrompt),
        "model_run" => Ok(ContextSourceRefKind::ModelRun),
        "content_block" => Ok(ContextSourceRefKind::ContentBlock),
        "current_prompt" => Ok(ContextSourceRefKind::CurrentPrompt),
        "checkpoint_summary" => Ok(ContextSourceRefKind::CheckpointSummary),
        "branch_summary" => Ok(ContextSourceRefKind::BranchSummary),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown context source reference kind `{other}`"
        ))),
    }
}

fn source_ref_from_stored(
    item: &StoredContextItem,
) -> Result<ContextSourceRef, RepositoryPortError> {
    if let Some(kind) = item.source_ref_kind.as_deref() {
        return Ok(ContextSourceRef {
            kind: source_ref_kind_from_str(kind)?,
            id: item.source_ref_id.clone(),
        });
    }

    // v1-v4 receipts used source_kind/source_id as a single overloaded
    // identity. Rehydrate deterministically without rewriting that evidence.
    let (kind, id) = match item.source_kind.as_str() {
        "system" => (
            ContextSourceRefKind::WorkspaceSystem,
            item.source_id.clone(),
        ),
        "turn_prompt" => (ContextSourceRefKind::TurnPrompt, item.source_id.clone()),
        "model_run" => (ContextSourceRefKind::ModelRun, item.source_id.clone()),
        "pinned" => (
            ContextSourceRefKind::ContentBlock,
            Some(item.content_block_id.clone()),
        ),
        "compaction_summary" => (
            ContextSourceRefKind::CheckpointSummary,
            item.source_id.clone(),
        ),
        "branch_summary" => (ContextSourceRefKind::BranchSummary, item.source_id.clone()),
        "current_prompt" => (ContextSourceRefKind::CurrentPrompt, None),
        other => {
            return Err(RepositoryPortError::InvalidData(format!(
                "unknown legacy context source kind `{other}`"
            )));
        }
    };
    Ok(ContextSourceRef { kind, id })
}

fn encode_context_warnings(warnings: &[ContextWarning]) -> Result<String, RepositoryPortError> {
    let values = warnings
        .iter()
        .map(|warning| match warning {
            ContextWarning::ExcludedPinnedSource(source) => json!({
                "code": "excluded_pinned_source",
                "source": source,
            }),
            ContextWarning::DuplicatePinnedSource(source) => json!({
                "code": "duplicate_pinned_source",
                "source": source,
            }),
            ContextWarning::ExceedsLimit {
                estimated_chars,
                max_chars,
            } => json!({
                "code": "exceeds_limit",
                "estimated_chars": estimated_chars,
                "max_chars": max_chars,
            }),
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))
}

fn decode_context_warnings(value: &str) -> Result<Vec<ContextWarning>, RepositoryPortError> {
    let values: Vec<Value> = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    values
        .into_iter()
        .map(|value| {
            let code = value.get("code").and_then(Value::as_str).ok_or_else(|| {
                RepositoryPortError::InvalidData("context warning code is missing".into())
            })?;
            match code {
                "excluded_pinned_source" => Ok(ContextWarning::ExcludedPinnedSource(
                    required_json_string(&value, "source")?,
                )),
                "duplicate_pinned_source" => Ok(ContextWarning::DuplicatePinnedSource(
                    required_json_string(&value, "source")?,
                )),
                "exceeds_limit" => Ok(ContextWarning::ExceedsLimit {
                    estimated_chars: required_json_u64(&value, "estimated_chars")?
                        .try_into()
                        .map_err(|_| {
                            RepositoryPortError::InvalidData(
                                "warning estimated chars exceeds platform size".into(),
                            )
                        })?,
                    max_chars: required_json_u64(&value, "max_chars")?.try_into().map_err(
                        |_| {
                            RepositoryPortError::InvalidData(
                                "warning max chars exceeds platform size".into(),
                            )
                        },
                    )?,
                }),
                other => Err(RepositoryPortError::InvalidData(format!(
                    "unknown context warning `{other}`"
                ))),
            }
        })
        .collect()
}

fn encode_checkpoint_provenance(
    provenance: &ContextCheckpointProvenance,
) -> Result<String, RepositoryPortError> {
    serde_json::to_string(&checkpoint_provenance_value(provenance))
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))
}

fn encode_checkpoint_provenances(
    provenances: &[ContextCheckpointProvenance],
) -> Result<String, RepositoryPortError> {
    serde_json::to_string(
        &provenances
            .iter()
            .map(checkpoint_provenance_value)
            .collect::<Vec<_>>(),
    )
    .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))
}

fn checkpoint_provenance_value(provenance: &ContextCheckpointProvenance) -> Value {
    json!({
        "checkpoint_id": provenance.checkpoint_id,
        "maintenance_run_id": provenance.maintenance_run_id,
        "kind": checkpoint_kind_to_str(provenance.kind),
        "branch_pointer_id": provenance.branch_pointer_id,
        "branch_revision": provenance.branch_revision,
        "anchor_run_id": provenance.anchor_run_id,
        "first_kept_run_id": provenance.first_kept_run_id,
        "summary_content_block_id": provenance.summary_content_block_id,
        "source_run_ids": provenance.source_run_ids,
        "source_hash": provenance.source_hash,
    })
}

fn decode_checkpoint_provenance(
    value: &str,
) -> Result<ContextCheckpointProvenance, RepositoryPortError> {
    let value: Value = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    checkpoint_provenance_from_value(&value)
}

fn decode_checkpoint_provenances(
    value: &str,
) -> Result<Vec<ContextCheckpointProvenance>, RepositoryPortError> {
    let values: Vec<Value> = serde_json::from_str(value)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    values
        .iter()
        .map(checkpoint_provenance_from_value)
        .collect()
}

fn checkpoint_provenance_from_value(
    value: &Value,
) -> Result<ContextCheckpointProvenance, RepositoryPortError> {
    let branch_pointer_id = optional_json_string(value, "branch_pointer_id")?;
    let branch_revision = value.get("branch_revision").and_then(Value::as_u64);
    if branch_pointer_id.is_some() != branch_revision.is_some() {
        return Err(RepositoryPortError::InvalidData(
            "checkpoint branch identity and revision must be present together".into(),
        ));
    }
    Ok(ContextCheckpointProvenance {
        checkpoint_id: required_json_string(value, "checkpoint_id")?,
        maintenance_run_id: required_json_string(value, "maintenance_run_id")?,
        kind: checkpoint_kind_from_str(&required_json_string(value, "kind")?)?,
        branch_pointer_id,
        branch_revision,
        anchor_run_id: required_json_string(value, "anchor_run_id")?,
        first_kept_run_id: optional_json_string(value, "first_kept_run_id")?,
        summary_content_block_id: required_json_string(value, "summary_content_block_id")?,
        source_run_ids: required_json_strings(value, "source_run_ids")?,
        source_hash: required_json_string(value, "source_hash")?,
    })
}

fn checkpoint_kind_to_str(kind: ContextCheckpointKind) -> &'static str {
    match kind {
        ContextCheckpointKind::Compaction => "compaction",
        ContextCheckpointKind::BranchSummary => "branch_summary",
    }
}

fn checkpoint_kind_from_str(value: &str) -> Result<ContextCheckpointKind, RepositoryPortError> {
    match value {
        "compaction" => Ok(ContextCheckpointKind::Compaction),
        "branch_summary" => Ok(ContextCheckpointKind::BranchSummary),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown context checkpoint kind `{other}`"
        ))),
    }
}

fn required_json_string(value: &Value, field: &str) -> Result<String, RepositoryPortError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RepositoryPortError::InvalidData(format!("JSON field `{field}` is missing")))
}

fn optional_json_string(value: &Value, field: &str) -> Result<Option<String>, RepositoryPortError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(RepositoryPortError::InvalidData(format!(
            "JSON field `{field}` is not a string"
        ))),
    }
}

fn required_json_u64(value: &Value, field: &str) -> Result<u64, RepositoryPortError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| RepositoryPortError::InvalidData(format!("JSON field `{field}` is missing")))
}

fn required_json_strings(value: &Value, field: &str) -> Result<Vec<String>, RepositoryPortError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            RepositoryPortError::InvalidData(format!("JSON field `{field}` is missing"))
        })?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                RepositoryPortError::InvalidData(format!(
                    "JSON field `{field}` contains a non-string"
                ))
            })
        })
        .collect()
}

fn sha256(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn port_error(error: RepositoryError) -> RepositoryPortError {
    match error {
        RepositoryError::NotFound { entity, id } => RepositoryPortError::NotFound { entity, id },
        RepositoryError::Conflict(message) => RepositoryPortError::Conflict(message),
        RepositoryError::VersionConflict {
            resource,
            id,
            expected,
            actual,
        } => {
            let expected = u64::try_from(expected).unwrap_or_default();
            let actual = u64::try_from(actual).unwrap_or_default();
            RepositoryPortError::VersionConflict {
                resource,
                id,
                expected,
                actual,
            }
        }
        RepositoryError::InvalidInput(message) | RepositoryError::InvalidStoredValue(message) => {
            RepositoryPortError::InvalidData(message)
        }
        RepositoryError::Database(error) => RepositoryPortError::Unavailable(error.to_string()),
        RepositoryError::Migration(error) => RepositoryPortError::Unavailable(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_receipt_keeps_provider_metadata_unresolved_and_parameters_intact() {
        let receipt = StoredRunReceipt {
            snapshot: ContextSnapshotRecord {
                id: "snapshot-legacy".into(),
                run_id: "run-legacy".into(),
                manifest_id: "manifest-legacy".into(),
                workspace_id: "workspace-legacy".into(),
                provider_profile_id: Some("profile-legacy".into()),
                provider_id: None,
                template_revision: None,
                stream_protocol: None,
                auth_placement: None,
                auth_header_name: None,
                additional_headers_json: "{}".into(),
                provider: "Legacy Ollama".into(),
                model: "qwen3".into(),
                base_url: "http://127.0.0.1:11434".into(),
                parameters_json: r#"{"temperature":0.7}"#.into(),
                request_json: "{}".into(),
                canonical_hash: "hash-legacy".into(),
                created_at: 1,
            },
            manifest: ContextManifestRecord {
                id: "manifest-legacy".into(),
                workspace_id: "workspace-legacy".into(),
                compiler_version: "1".into(),
                strategy: "ancestor_path_with_pins".into(),
                estimated_chars: 0,
                canonical_hash: "hash-legacy".into(),
                warnings_json: "[]".into(),
                checkpoint_provenance_json: None,
                branch_summary_provenance_json: "[]".into(),
                created_at: 1,
            },
            items: Vec::new(),
        };
        let run = ModelRunRecord {
            id: "run-legacy".into(),
            turn_id: "turn-legacy".into(),
            workspace_id: "workspace-legacy".into(),
            provider_profile_id: Some("profile-legacy".into()),
            model: "qwen3".into(),
            status: RunStatusRecord::Completed,
            output_markdown: "answer".into(),
            reasoning_markdown: String::new(),
            provider_snapshot_json: r#"{"dialect":"ollama_chat"}"#.into(),
            usage_json: None,
            error_json: None,
            created_at: 1,
            started_at: Some(1),
            finished_at: Some(2),
            checkpointed_at: Some(2),
        };

        let restored = receipt_to_domain(receipt, &run).unwrap();

        assert_eq!(restored.provider.provider_id, None);
        assert_eq!(restored.provider.template_revision, None);
        assert_eq!(restored.provider.stream_protocol, None);
        assert_eq!(restored.provider.auth_placement, None);
        assert_eq!(
            restored
                .provider
                .parameters
                .get("temperature")
                .map(String::as_str),
            Some("0.7")
        );
    }
}
