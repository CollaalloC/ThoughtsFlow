use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        BranchPointer, ContentBlock, ContextManifest, ContextSnapshot, ContextSourceKind,
        ConversationGraph, DecisionMark, DecisionStatus, InclusionReason, MessageRole, ModelRun,
        ProviderDialect, ProviderProfile, ProviderSnapshot, RunContextItem, RunDraft, RunFailure,
        RunStateSnapshot, RunStatus, RunUsage, Turn, ViewState, Workspace,
    },
    ports::{
        CheckpointOutcome, PersistRunStart, RepositoryFuture, RepositoryPort, RepositoryPortError,
        RunCheckpoint as PortRunCheckpoint, RunFinish as PortRunFinish, RunPersistencePort,
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

    fn mark_run_connecting(&self, run_id: &str, at: i64) -> RepositoryFuture<'_, ()> {
        let repository = self.clone();
        let run_id = run_id.to_owned();
        Box::pin(async move {
            SqliteRepository::mark_run_connecting(&repository, &run_id, at)
                .await
                .map_err(port_error)
        })
    }

    fn persist_run_start(&self, start: PersistRunStart) -> RepositoryFuture<'_, ()> {
        let repository = self.clone();
        Box::pin(async move {
            let bundle = build_start_bundle(&repository, start).await?;
            SqliteRepository::persist_run_start(&repository, &bundle)
                .await
                .map_err(port_error)
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
            block.content_hash == item.content_hash
                && block.content == item.content
                && block.role == item.role
        }) {
            Some(block) => block.id.clone(),
            None => {
                let id = format!("block-{}-{}", role_to_str(item.role), item.content_hash);
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
            source_kind: source_kind_to_str(item.source_kind).into(),
            role: role_to_str(item.role).into(),
            content_block_id: block_id,
            inclusion_reason: inclusion_reason_to_str(item.inclusion_reason).into(),
        });
    }

    let state = start.run.state_snapshot();
    let provider_json = json!({
        "profile_id": start.snapshot.provider.profile_id,
        "provider_name": start.snapshot.provider.provider_name,
        "dialect": dialect_to_str(start.snapshot.provider.dialect),
        "base_url": start.snapshot.provider.base_url,
        "model": start.snapshot.provider.model,
        "parameters": start.snapshot.provider.parameters,
    })
    .to_string();
    let parameters_json = serde_json::to_string(&start.snapshot.provider.parameters)
        .map_err(|error| RepositoryPortError::InvalidData(error.to_string()))?;
    let request_json = json!({
        "messages": start.snapshot.manifest.items.iter().map(|item| json!({
            "role": role_to_str(item.role),
            "content": item.content,
        })).collect::<Vec<_>>(),
        "model": start.snapshot.provider.model,
        "parameters": start.snapshot.provider.parameters,
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
        warnings_json: "[]".into(),
        created_at: start.snapshot.created_at,
    };
    let snapshot = ContextSnapshotRecord {
        id: start.snapshot.id,
        run_id: start.run.id.clone(),
        manifest_id,
        workspace_id: workspace_id.clone(),
        provider_profile_id: non_empty(start.snapshot.provider.profile_id),
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
    let items = receipt
        .items
        .into_iter()
        .map(|item| {
            Ok(RunContextItem {
                position: usize::try_from(item.position).map_err(|_| {
                    RepositoryPortError::InvalidData("negative context position".into())
                })?,
                source_id: item.source_id,
                source_kind: source_kind_from_str(&item.source_kind)?,
                role: role_from_str(&item.role)?,
                content: item.content,
                content_hash: item.content_hash,
                inclusion_reason: inclusion_reason_from_str(&item.inclusion_reason)?,
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
        },
        provider: ProviderSnapshot {
            profile_id: receipt.snapshot.provider_profile_id.unwrap_or_default(),
            provider_name: receipt.snapshot.provider,
            dialect,
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
    }
}

fn dialect_from_str(value: &str) -> Result<ProviderDialect, RepositoryPortError> {
    match value {
        "openai_chat_completions" => Ok(ProviderDialect::OpenAiCompatible),
        "ollama_chat" => Ok(ProviderDialect::Ollama),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown provider dialect `{other}`"
        ))),
    }
}

fn source_kind_to_str(kind: ContextSourceKind) -> &'static str {
    match kind {
        ContextSourceKind::System => "system",
        ContextSourceKind::TurnPrompt => "turn_prompt",
        ContextSourceKind::ModelRun => "model_run",
        ContextSourceKind::Pinned => "pinned",
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn source_kind_from_str(value: &str) -> Result<ContextSourceKind, RepositoryPortError> {
    match value {
        "system" => Ok(ContextSourceKind::System),
        "turn_prompt" => Ok(ContextSourceKind::TurnPrompt),
        "model_run" => Ok(ContextSourceKind::ModelRun),
        "pinned" => Ok(ContextSourceKind::Pinned),
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
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

fn inclusion_reason_from_str(value: &str) -> Result<InclusionReason, RepositoryPortError> {
    match value {
        "system_policy" => Ok(InclusionReason::SystemPolicy),
        "exact_ancestor_path" => Ok(InclusionReason::ExactAncestorPath),
        "explicit_pin" => Ok(InclusionReason::ExplicitPin),
        "current_prompt" => Ok(InclusionReason::CurrentPrompt),
        other => Err(RepositoryPortError::InvalidData(format!(
            "unknown inclusion reason `{other}`"
        ))),
    }
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
        RepositoryError::InvalidInput(message) | RepositoryError::InvalidStoredValue(message) => {
            RepositoryPortError::InvalidData(message)
        }
        RepositoryError::Database(error) => RepositoryPortError::Unavailable(error.to_string()),
        RepositoryError::Migration(error) => RepositoryPortError::Unavailable(error.to_string()),
    }
}
