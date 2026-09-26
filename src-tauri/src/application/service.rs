use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    domain::{
        self, BranchPointer, ContextCompileRequest, ContextCompiler, ContextPolicy,
        ContextSourceKind, ContextWarning, DecisionMark, DecisionStatus, InclusionReason,
        MessageRole, ModelRun, ProviderProfile, RunDraft, RunFailure, RunStatus, Turn, ViewState,
        Workspace,
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, CredentialPlacement, DiscoveredModel,
        MessageRole as ProviderMessageRole, ProviderConnectionTester, ProviderDialect,
        ProviderError, ProviderGateway, ProviderInvocation, ProviderModelCatalog,
        ProviderModelCatalogKind, ProviderModelQuery, ProviderTarget, RunEvent, SessionCredential,
        Usage,
    },
    ports::{
        CheckpointOutcome, DecisionPacketWriter, PersistRunStart, RepositoryPort,
        RepositoryPortError, RunCheckpoint, RunFinish, RunPersistencePort, RunProviderProvenance,
    },
};

use super::*;

const DEFAULT_PROVIDER_ID: &str = "provider-local-ollama";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a careful technical reasoning partner. Make assumptions explicit and preserve competing options.";
const DEFAULT_MAX_CONTEXT_CHARS: usize = 100_000;
const DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS: u32 = 4_096;
const INTERNAL_DEFAULT_KEY: &str = "_thoughsflowIsDefault";

pub struct DefaultApplicationBackend {
    repository: Arc<dyn RepositoryPort>,
    run_persistence: Arc<dyn RunPersistencePort>,
    provider: Arc<dyn ProviderGateway>,
    connection_tester: Arc<dyn ProviderConnectionTester>,
    model_catalog: Arc<dyn ProviderModelCatalog>,
    compiler: ContextCompiler,
    run_registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    maintenance_registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    decision_packet_writer: Arc<dyn DecisionPacketWriter>,
}

impl DefaultApplicationBackend {
    pub fn new<R>(
        repository: Arc<R>,
        provider: Arc<dyn ProviderGateway>,
        connection_tester: Arc<dyn ProviderConnectionTester>,
        model_catalog: Arc<dyn ProviderModelCatalog>,
        decision_packet_writer: Arc<dyn DecisionPacketWriter>,
    ) -> Self
    where
        R: RepositoryPort + RunPersistencePort + 'static,
    {
        Self {
            repository: repository.clone(),
            run_persistence: repository,
            provider,
            connection_tester,
            model_catalog,
            compiler: ContextCompiler::new(ContextPolicy {
                compiler_version: domain::CONTEXT_COMPILER_VERSION.into(),
                max_chars: DEFAULT_MAX_CONTEXT_CHARS,
            }),
            run_registry: Arc::new(Mutex::new(HashMap::new())),
            maintenance_registry: Arc::new(Mutex::new(HashMap::new())),
            decision_packet_writer,
        }
    }

    pub async fn initialize(&self) -> AppResult<()> {
        let recovered_at = now_millis();
        self.repository
            .recover_interrupted_runs(recovered_at)
            .await
            .map_err(repository_port_error)?;
        self.repository
            .recover_interrupted_context_maintenance(recovered_at)
            .await
            .map_err(repository_port_error)?;
        self.ensure_default_provider().await
    }

    async fn checkpoint_for_existing_maintenance(
        &self,
        maintenance: &domain::ContextMaintenanceRun,
    ) -> AppResult<domain::ContextCheckpoint> {
        match maintenance.status {
            domain::ContextMaintenanceStatus::Completed => self
                .repository
                .get_context_checkpoint_for_maintenance(&maintenance.id)
                .await
                .map_err(repository_port_error)?
                .ok_or_else(|| {
                    AppError::internal(
                        "context_checkpoint_missing",
                        "Completed Context maintenance is missing its immutable checkpoint",
                    )
                    .with_details(json!({
                        "clientOperationId": &maintenance.id,
                    }))
                }),
            domain::ContextMaintenanceStatus::Queued
            | domain::ContextMaintenanceStatus::Running => Err(AppError::internal(
                "context_maintenance_in_progress",
                "The Context maintenance operation is already in progress",
            )
            .with_details(json!({
                "clientOperationId": &maintenance.id,
            }))),
            domain::ContextMaintenanceStatus::Failed
            | domain::ContextMaintenanceStatus::Cancelled
            | domain::ContextMaintenanceStatus::Conflicted => Err(AppError::validation(
                "context_maintenance_terminal",
                "The Context maintenance operation already reached a terminal non-success state; retry with a new clientOperationId",
            )
            .with_details(json!({
                "clientOperationId": &maintenance.id,
                "status": context_maintenance_status_name(maintenance.status),
                "error": &maintenance.error,
            }))),
        }
    }

    async fn ensure_default_provider(&self) -> AppResult<()> {
        if !self
            .repository
            .list_provider_profiles()
            .await
            .map_err(repository_port_error)?
            .is_empty()
        {
            return Ok(());
        }
        let now = now_millis();
        self.repository
            .save_provider_profile(ProviderProfile {
                id: DEFAULT_PROVIDER_ID.into(),
                provider_id: "ollama".into(),
                name: "Local Ollama".into(),
                dialect: domain::ProviderDialect::Ollama,
                base_url: "http://127.0.0.1:11434".into(),
                model: "qwen3".into(),
                parameters: BTreeMap::from([(INTERNAL_DEFAULT_KEY.into(), "true".into())]),
                created_at: now,
                updated_at: now,
            })
            .await
            .map_err(repository_port_error)?;
        Ok(())
    }

    async fn inspect_context_impl(&self, input: InspectContextInput) -> AppResult<ContextPreview> {
        let workspace = self
            .repository
            .get_workspace(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        self.inspect_context_from_data(input, workspace, profile, data)
    }

    fn inspect_context_from_data(
        &self,
        input: InspectContextInput,
        workspace: Workspace,
        profile: ProviderProfile,
        data: crate::ports::WorkspaceContextData,
    ) -> AppResult<ContextPreview> {
        let overrides = if data.draft.consumed_by_run_id.is_none()
            && data.draft.parent_run_id == input.parent_run_id
        {
            context_overrides_from_draft(&data.draft)?
        } else {
            domain::ContextOverrides::default()
        };
        let requested_branch_id = input.branch_id.as_deref().or_else(|| {
            (data.cursor.active_run_id == input.parent_run_id)
                .then_some(data.cursor.branch_pointer_id.as_deref())
                .flatten()
        });
        if let Some(branch_id) = requested_branch_id {
            let branch = data
                .branch_pointers
                .iter()
                .find(|branch| branch.id == branch_id)
                .ok_or_else(|| {
                    AppError::validation(
                        "branch_not_found",
                        format!("Branch `{branch_id}` is not in this workspace"),
                    )
                })?;
            if let Some(parent_run_id) = input.parent_run_id.as_deref() {
                let runs_by_id = data
                    .runs
                    .iter()
                    .cloned()
                    .map(|run| (run.id.clone(), run))
                    .collect::<HashMap<_, _>>();
                if !exact_lineage_ids(Some(&branch.head_run_id), &data.turns, &runs_by_id)
                    .contains(parent_run_id)
                {
                    return Err(AppError::validation(
                        "run_outside_branch",
                        "The preview parent Model Run is not on the requested branch",
                    ));
                }
            }
        }
        let eligible_checkpoint_ids =
            eligible_checkpoint_ids_for_branch(&data, requested_branch_id);
        let request = ContextCompileRequest {
            workspace_id: input.workspace_id.clone(),
            system_prompt: workspace.system_prompt,
            parent_run_id: input.parent_run_id.clone(),
            current_prompt: input.prompt,
            overrides,
            provider: Some(domain_provider_snapshot(&profile)?),
        };
        let actual = self
            .compiler
            .inspect(
                &data.graph,
                domain::ContextCompileInput::new(request)
                    .with_checkpoints(data.checkpoints.clone())
                    .with_eligible_checkpoint_ids(eligible_checkpoint_ids),
            )
            .map_err(domain_error)?;
        let items = actual
            .manifest
            .items
            .iter()
            .map(|item| {
                context_item_view(
                    item,
                    true,
                    item.inclusion_reason == InclusionReason::ExplicitPin,
                )
            })
            .collect::<Vec<_>>();
        let effective_identities = actual
            .manifest
            .items
            .iter()
            .map(context_item_identity)
            .collect::<BTreeSet<_>>();
        let excluded_identities = data
            .draft
            .items
            .iter()
            .filter(|item| item.operation == crate::ports::ContextOverrideOperation::Exclude)
            .map(|item| item.source_ref.stable_id())
            .collect::<BTreeSet<_>>();
        let raw_items = actual
            .raw_items
            .iter()
            .map(|item| {
                let included = effective_identities.contains(&context_item_identity(item));
                let mut view = context_item_view(item, included, false);
                if !included {
                    view.reason = if excluded_identities.contains(&item.source_ref.stable_id()) {
                        "Explicitly excluded from the next request".into()
                    } else {
                        "Preserved in raw history outside the effective checkpoint tail".into()
                    };
                }
                view
            })
            .collect();
        let applied_checkpoint = actual.applied_checkpoint.as_ref().and_then(|provenance| {
            data.checkpoints
                .iter()
                .find(|checkpoint| checkpoint.id == provenance.checkpoint_id)
                .map(context_checkpoint_view)
        });
        Ok(ContextPreview {
            hash: actual.preview_hash,
            estimated_tokens: chars_to_tokens(actual.estimated_chars),
            limit_tokens: chars_to_tokens(DEFAULT_MAX_CONTEXT_CHARS),
            blocked: actual
                .warnings
                .iter()
                .any(|warning| matches!(warning, ContextWarning::ExceedsLimit { .. })),
            warnings: actual.warnings.iter().map(context_warning).collect(),
            provider_profile_id: profile.id,
            provider_name: profile.name,
            model: profile.model,
            base_url: profile.base_url,
            items,
            raw_items,
            draft_version: data.draft.version,
            applied_checkpoint,
        })
    }

    async fn preview_context_transition_impl(
        &self,
        input: PreviewContextTransitionInput,
    ) -> AppResult<ContextPreview> {
        let workspace = self
            .repository
            .get_workspace(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if data.draft.version != input.draft_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_draft",
                    id: input.workspace_id.clone(),
                    expected: input.draft_version,
                    actual: data.draft.version,
                },
            ));
        }
        self.inspect_context_from_data(
            InspectContextInput {
                workspace_id: input.workspace_id,
                parent_run_id: input.parent_run_id,
                prompt: input.prompt,
                provider_profile_id: input.provider_profile_id,
                branch_id: input.branch_id,
            },
            workspace,
            profile,
            data,
        )
    }

    async fn get_context_tree_impl(
        &self,
        input: GetContextTreeInput,
    ) -> AppResult<ContextTreeProjection> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let eligible_checkpoint_ids =
            eligible_checkpoint_ids_for_branch(&data, data.cursor.branch_pointer_id.as_deref())
                .into_iter()
                .collect::<BTreeSet<_>>();
        let cursor = context_cursor_view(data.cursor);
        let draft_version = data.draft.version;
        let checkpoints = data
            .checkpoints
            .iter()
            .map(context_checkpoint_view)
            .collect();
        Ok(context_tree_projection(
            &input.workspace_id,
            &data.graph,
            cursor,
            draft_version,
            &data.branch_pointers,
            &eligible_checkpoint_ids,
            checkpoints,
        ))
    }

    async fn set_active_context_impl(
        &self,
        input: SetActiveContextInput,
    ) -> AppResult<ContextCursorView> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if let Some(run_id) = input.run_id.as_deref()
            && data.graph.run(run_id).is_none()
        {
            return Err(AppError::validation(
                "context_run_not_found",
                format!("Model Run `{run_id}` is not in this workspace"),
            ));
        }
        let branch_pointer_id = match (input.run_id.as_deref(), input.branch_id.as_deref()) {
            (None, Some(_)) => {
                return Err(AppError::validation(
                    "root_context_has_no_branch",
                    "The workspace root cannot carry a branch pointer",
                ));
            }
            (None, None) => None,
            (Some(run_id), Some(branch_id)) => {
                let branch = data
                    .branch_pointers
                    .iter()
                    .find(|branch| branch.id == branch_id)
                    .ok_or_else(|| {
                        AppError::validation(
                            "branch_not_found",
                            format!("Branch `{branch_id}` is not in this workspace"),
                        )
                    })?;
                let runs_by_id = data
                    .runs
                    .iter()
                    .cloned()
                    .map(|run| (run.id.clone(), run))
                    .collect::<HashMap<_, _>>();
                let branch_path =
                    exact_lineage_ids(Some(&branch.head_run_id), &data.turns, &runs_by_id);
                if !branch_path.contains(run_id) {
                    return Err(AppError::validation(
                        "run_outside_branch",
                        "The selected Model Run is not on the requested branch",
                    ));
                }
                Some(branch.id.clone())
            }
            // A Run can be shared by multiple branch paths. An omitted
            // BranchPointer is an intentional, precise cursor state; never
            // infer one from ordering or timestamps.
            (Some(_), None) => None,
        };
        let expected_draft_version = input.expected_draft_version;
        self.repository
            .set_context_cursor_and_rebase_draft(
                crate::ports::ContextCursorUpdate {
                    workspace_id: input.workspace_id,
                    active_run_id: input.run_id,
                    branch_pointer_id,
                    expected_version: input.expected_cursor_version,
                    updated_at: now_millis(),
                },
                expected_draft_version,
            )
            .await
            .map(context_cursor_view)
            .map_err(repository_port_error)
    }

    async fn rename_branch_impl(&self, input: RenameBranchInput) -> AppResult<ContextBranchView> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if !data
            .branch_pointers
            .iter()
            .any(|branch| branch.id == input.branch_id)
        {
            return Err(AppError::validation(
                "branch_not_found",
                format!("Branch `{}` is not in this workspace", input.branch_id),
            ));
        }
        let active_branch_id = data.cursor.branch_pointer_id;
        self.repository
            .rename_branch(
                &input.branch_id,
                input.name.trim(),
                input.expected_branch_version,
                now_millis(),
            )
            .await
            .map(|branch| context_branch_view(branch, active_branch_id.as_deref()))
            .map_err(repository_port_error)
    }

    async fn update_context_draft_impl(
        &self,
        input: UpdateContextDraftInput,
    ) -> AppResult<UpdateContextDraftResult> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if let Some(parent_run_id) = input.parent_run_id.as_deref()
            && data.graph.run(parent_run_id).is_none()
        {
            return Err(AppError::validation(
                "context_run_not_found",
                format!("Model Run `{parent_run_id}` is not in this workspace"),
            ));
        }
        let mut items = Vec::new();
        let mut content_blocks = BTreeMap::new();
        for item in input.items {
            if item.included && !item.pinned {
                continue;
            }
            let source_ref = context_source_ref_domain(item.source_ref);
            validate_context_source_ref(&data, &source_ref)?;
            if matches!(
                source_ref.kind,
                domain::ContextSourceRefKind::WorkspaceSystem
                    | domain::ContextSourceRefKind::CurrentPrompt
            ) {
                return Err(AppError::validation(
                    "mandatory_context_item",
                    "System and current-prompt Context items cannot be overridden",
                ));
            }
            let (operation, content_block_id, content_hash) = if item.pinned {
                let content_block_id = item.content_block_id.ok_or_else(|| {
                    AppError::validation(
                        "missing_content_block_identity",
                        "A pinned Context item must identify its exact Content Block",
                    )
                })?;
                let block = content_block_for_source(&data, &source_ref).ok_or_else(|| {
                    AppError::validation(
                        "content_block_not_found",
                        format!(
                            "Context source `{}` has no pinnable Content Block in this workspace",
                            source_ref.stable_id()
                        ),
                    )
                })?;
                validate_pinned_content_identity(&source_ref, &content_block_id, &block)?;
                content_blocks.insert(block.id.clone(), block.clone());
                (
                    crate::ports::ContextOverrideOperation::Pin,
                    Some(block.id.clone()),
                    Some(block.content_hash.clone()),
                )
            } else {
                (crate::ports::ContextOverrideOperation::Exclude, None, None)
            };
            items.push(crate::ports::ContextOverrideItem {
                position: items.len(),
                operation,
                source_ref,
                content_block_id,
                content_hash,
            });
        }
        let draft = self
            .repository
            .update_context_draft(crate::ports::ContextDraftUpdate {
                workspace_id: input.workspace_id,
                parent_run_id: input.parent_run_id,
                expected_version: input.expected_draft_version,
                content_blocks: content_blocks.into_values().collect(),
                items,
                updated_at: now_millis(),
            })
            .await
            .map_err(repository_port_error)?;
        Ok(UpdateContextDraftResult {
            draft_version: draft.version,
        })
    }

    async fn update_context_overrides_impl(
        &self,
        input: UpdateContextOverridesInput,
    ) -> AppResult<()> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let source_ref = legacy_context_source_ref(&data, &input.item_id)?;
        if matches!(
            source_ref.kind,
            domain::ContextSourceRefKind::WorkspaceSystem
                | domain::ContextSourceRefKind::CurrentPrompt
        ) {
            return Err(AppError::validation(
                "mandatory_context_item",
                "System and current-prompt Context items cannot be overridden",
            ));
        }
        let mut items = data
            .draft
            .items
            .iter()
            .filter(|item| item.source_ref != source_ref)
            .cloned()
            .collect::<Vec<_>>();
        let mut content_blocks = Vec::new();
        if input.pinned {
            let block = content_block_for_source(&data, &source_ref).ok_or_else(|| {
                AppError::validation(
                    "missing_content_block_identity",
                    "The legacy pin could not resolve an exact Content Block; refresh Context",
                )
            })?;
            content_blocks.push(block.clone());
            items.push(crate::ports::ContextOverrideItem {
                position: items.len(),
                operation: crate::ports::ContextOverrideOperation::Pin,
                source_ref,
                content_block_id: Some(block.id.clone()),
                content_hash: Some(block.content_hash.clone()),
            });
        } else if !input.included {
            items.push(crate::ports::ContextOverrideItem {
                position: items.len(),
                operation: crate::ports::ContextOverrideOperation::Exclude,
                source_ref,
                content_block_id: None,
                content_hash: None,
            });
        }
        for (position, item) in items.iter_mut().enumerate() {
            item.position = position;
        }
        self.repository
            .update_context_draft(crate::ports::ContextDraftUpdate {
                workspace_id: input.workspace_id,
                parent_run_id: input.parent_run_id,
                expected_version: data.draft.version,
                content_blocks,
                items,
                updated_at: now_millis(),
            })
            .await
            .map_err(repository_port_error)?;
        Ok(())
    }

    async fn create_context_checkpoint_impl(
        &self,
        input: CreateContextCheckpointInput,
    ) -> AppResult<ContextCheckpointView> {
        validate_manual_checkpoint_summary(&input.summary)?;
        let request_json = json!({
            "mode": "manual",
            "summary": &input.summary,
            "sourceRunIds": &input.source_run_ids,
            "firstKeptRunId": &input.first_kept_run_id,
            "branchId": &input.branch_id,
            "expectedBranchVersion": input.expected_branch_version,
            "expectedCursorVersion": input.expected_cursor_version,
        })
        .to_string();
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let maintenance_runs = self
            .repository
            .list_context_maintenance_runs(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if let Some(maintenance) = context_maintenance_replay(
            &maintenance_runs,
            &input.client_operation_id,
            checkpoint_kind_domain(input.kind),
            &request_json,
        )? {
            let checkpoint = self
                .checkpoint_for_existing_maintenance(maintenance)
                .await?;
            return Ok(context_checkpoint_view(&checkpoint));
        }
        if data.cursor.version != input.expected_cursor_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_cursor",
                    id: input.workspace_id,
                    expected: input.expected_cursor_version,
                    actual: data.cursor.version,
                },
            ));
        }
        let anchor_run_id = data.cursor.active_run_id.clone().ok_or_else(|| {
            AppError::validation(
                "checkpoint_requires_run",
                "A Context checkpoint cannot be attached to the workspace root",
            )
        })?;
        let kind = checkpoint_kind_domain(input.kind);
        let branch = validate_checkpoint_selection(
            &data,
            &input.branch_id,
            input.expected_branch_version,
            &anchor_run_id,
            kind,
            &input.source_run_ids,
            input.first_kept_run_id.as_deref(),
        )?;
        let source_hash = checkpoint_source_hash(&data, &input.source_run_ids)?;
        let now = now_millis();
        let maintenance_id = input.client_operation_id.clone();
        let guard = crate::ports::MaintenanceContextGuard {
            workspace_id: data.workspace_id.clone(),
            expected_cursor_version: input.expected_cursor_version,
            branch_pointer_id: Some(branch.id.clone()),
            expected_branch_version: Some(input.expected_branch_version),
            expected_draft_version: None,
        };
        let maintenance = domain::ContextMaintenanceRun {
            id: maintenance_id.clone(),
            workspace_id: data.workspace_id.clone(),
            kind,
            status: domain::ContextMaintenanceStatus::Running,
            branch_pointer_id: Some(branch.id.clone()),
            branch_revision: Some(branch.version),
            anchor_run_id: anchor_run_id.clone(),
            first_kept_run_id: input.first_kept_run_id.clone(),
            source_run_ids: input.source_run_ids.clone(),
            source_hash: source_hash.clone(),
            // A manual checkpoint records author-supplied text; no Provider
            // invocation exists from which provenance could be captured.
            provider: None,
            request_json,
            summary: None,
            error: None,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
        };
        let start = self
            .repository
            .start_context_maintenance(maintenance, guard.clone())
            .await
            .map_err(repository_port_error)?;
        if !start.started {
            let checkpoint = self.checkpoint_for_existing_maintenance(&start.run).await?;
            return Ok(context_checkpoint_view(&checkpoint));
        }
        let started = start.run;
        let finished_at = now_millis();
        let summary_hash = domain::sha256_hex(input.summary.as_bytes());
        let summary_content_block_id = format!("block-system-{summary_hash}");
        let summary_block = domain::ContentBlock {
            id: summary_content_block_id.clone(),
            workspace_id: data.workspace_id.clone(),
            role: MessageRole::System,
            content: input.summary.clone(),
            content_hash: summary_hash,
            created_at: finished_at,
        };
        let checkpoint = domain::ContextCheckpoint {
            id: input.client_operation_id,
            workspace_id: data.workspace_id,
            maintenance_run_id: maintenance_id,
            kind,
            branch_pointer_id: Some(branch.id),
            branch_revision: Some(branch.version),
            anchor_run_id,
            first_kept_run_id: input.first_kept_run_id,
            summary: input.summary.clone(),
            summary_content_block_id,
            source_run_ids: input.source_run_ids,
            source_hash,
            provider: None,
            created_at: finished_at,
        };
        let completed = domain::ContextMaintenanceRun {
            status: domain::ContextMaintenanceStatus::Completed,
            summary: Some(input.summary),
            finished_at: Some(finished_at),
            ..started
        };
        let stored_maintenance = self
            .repository
            .finish_context_maintenance(
                completed,
                Some(checkpoint.clone()),
                Some(summary_block),
                guard,
                None,
            )
            .await
            .map_err(repository_port_error)?;
        let persisted_checkpoint = self
            .checkpoint_for_existing_maintenance(&stored_maintenance)
            .await?;
        Ok(context_checkpoint_view(&persisted_checkpoint))
    }

    async fn summarize_and_set_active_context_impl(
        &self,
        input: SummarizeAndSetActiveContextInput,
        credentials: Arc<dyn SessionCredentialLookup>,
    ) -> AppResult<SummarizeAndSetActiveContextResult> {
        let request_json = json!({
            "mode": "provider_summary",
            "summaryPrompt": &input.summary_prompt,
            "sourceRunIds": &input.source_run_ids,
            "firstKeptRunId": &input.first_kept_run_id,
            "targetRunId": &input.target_run_id,
            "branchId": &input.branch_id,
            "expectedBranchVersion": input.expected_branch_version,
            "expectedCursorVersion": input.expected_cursor_version,
            "expectedDraftVersion": input.expected_draft_version,
            "providerProfileId": &input.provider_profile_id,
        })
        .to_string();
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let maintenance_runs = self
            .repository
            .list_context_maintenance_runs(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if let Some(maintenance) = context_maintenance_replay(
            &maintenance_runs,
            &input.client_operation_id,
            domain::ContextCheckpointKind::Compaction,
            &request_json,
        )? {
            let checkpoint = self
                .checkpoint_for_existing_maintenance(maintenance)
                .await?;
            let cursor = self
                .repository
                .get_context_cursor(&input.workspace_id)
                .await
                .map(context_cursor_view)
                .map_err(repository_port_error)?;
            return Ok(SummarizeAndSetActiveContextResult {
                cursor,
                checkpoint: Some(context_checkpoint_view(&checkpoint)),
            });
        }
        if data.cursor.version != input.expected_cursor_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_cursor",
                    id: input.workspace_id,
                    expected: input.expected_cursor_version,
                    actual: data.cursor.version,
                },
            ));
        }
        if data.draft.version != input.expected_draft_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_draft",
                    id: data.workspace_id,
                    expected: input.expected_draft_version,
                    actual: data.draft.version,
                },
            ));
        }
        let kind = domain::ContextCheckpointKind::Compaction;
        let branch = validate_checkpoint_selection(
            &data,
            &input.branch_id,
            input.expected_branch_version,
            &input.target_run_id,
            kind,
            &input.source_run_ids,
            input.first_kept_run_id.as_deref(),
        )?;
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let provider_snapshot = domain_provider_snapshot(&profile)?;
        let provider_target = provider_target(&provider_snapshot)?;
        let parameters = effective_provider_parameters(&profile)?;
        let messages =
            checkpoint_summary_messages(&data, &input.source_run_ids, &input.summary_prompt)?;
        let source_hash = checkpoint_source_hash(&data, &input.source_run_ids)?;
        let credential = credentials
            .credential_for(&profile.id)?
            .map(|credential| {
                credential
                    .as_str()
                    .map(|value| SessionCredential::new(value.to_owned()))
            })
            .transpose()?;
        let now = now_millis();
        let maintenance_id = input.client_operation_id.clone();
        let guard = crate::ports::MaintenanceContextGuard {
            workspace_id: data.workspace_id.clone(),
            expected_cursor_version: input.expected_cursor_version,
            branch_pointer_id: Some(branch.id.clone()),
            expected_branch_version: Some(input.expected_branch_version),
            expected_draft_version: Some(input.expected_draft_version),
        };
        let maintenance = domain::ContextMaintenanceRun {
            id: maintenance_id.clone(),
            workspace_id: data.workspace_id.clone(),
            kind,
            status: domain::ContextMaintenanceStatus::Running,
            branch_pointer_id: Some(branch.id.clone()),
            branch_revision: Some(branch.version),
            anchor_run_id: input.target_run_id.clone(),
            first_kept_run_id: input.first_kept_run_id.clone(),
            source_run_ids: input.source_run_ids.clone(),
            source_hash: source_hash.clone(),
            provider: Some(provider_snapshot.clone()),
            request_json,
            summary: None,
            error: None,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
        };
        let start = self
            .repository
            .start_context_maintenance(maintenance, guard.clone())
            .await
            .map_err(repository_port_error)?;
        if !start.started {
            let checkpoint = self.checkpoint_for_existing_maintenance(&start.run).await?;
            let cursor = self
                .repository
                .get_context_cursor(&data.workspace_id)
                .await
                .map(context_cursor_view)
                .map_err(repository_port_error)?;
            return Ok(SummarizeAndSetActiveContextResult {
                cursor,
                checkpoint: Some(context_checkpoint_view(&checkpoint)),
            });
        }
        let started = start.run;
        let invocation = ProviderInvocation {
            target: provider_target,
            credential,
            request: canonical_provider_request(
                maintenance_id.clone(),
                profile.model,
                messages,
                &parameters,
            ),
        };
        let cancellation = CancellationToken::new();
        self.maintenance_registry
            .lock()
            .map_err(|_| {
                AppError::internal(
                    "context_maintenance_registry_lock",
                    "Context maintenance registry is unavailable",
                )
            })?
            .insert(maintenance_id.clone(), cancellation.clone());
        let summary_result =
            collect_provider_summary(self.provider.clone(), invocation, cancellation).await;
        if let Ok(mut registry) = self.maintenance_registry.lock() {
            registry.remove(&maintenance_id);
        }
        let summary = match summary_result {
            Ok(summary) => summary,
            Err(error) => {
                let finished_at = now_millis();
                let terminal_status = if error.code == "context_maintenance_cancelled" {
                    domain::ContextMaintenanceStatus::Cancelled
                } else {
                    domain::ContextMaintenanceStatus::Failed
                };
                let failed = domain::ContextMaintenanceRun {
                    status: terminal_status,
                    error: Some(
                        json!({
                            "code": &error.code,
                            "message": &error.message,
                            "retryable": error.retryable,
                            "details": &error.details,
                        })
                        .to_string(),
                    ),
                    finished_at: Some(finished_at),
                    ..started
                };
                self.repository
                    .finish_context_maintenance(failed, None, None, guard, None)
                    .await
                    .map_err(repository_port_error)?;
                return Err(context_maintenance_provider_error(
                    error,
                    &maintenance_id,
                    terminal_status,
                ));
            }
        };
        let finished_at = now_millis();
        let summary_hash = domain::sha256_hex(summary.as_bytes());
        let summary_content_block_id = format!("block-system-{summary_hash}");
        let summary_block = domain::ContentBlock {
            id: summary_content_block_id.clone(),
            workspace_id: data.workspace_id.clone(),
            role: MessageRole::System,
            content: summary.clone(),
            content_hash: summary_hash,
            created_at: finished_at,
        };
        let checkpoint = domain::ContextCheckpoint {
            id: input.client_operation_id,
            workspace_id: data.workspace_id.clone(),
            maintenance_run_id: maintenance_id,
            kind,
            branch_pointer_id: Some(branch.id.clone()),
            branch_revision: Some(branch.version),
            anchor_run_id: input.target_run_id.clone(),
            first_kept_run_id: input.first_kept_run_id,
            summary: summary.clone(),
            summary_content_block_id,
            source_run_ids: input.source_run_ids,
            source_hash,
            provider: Some(provider_snapshot),
            created_at: finished_at,
        };
        let completed = domain::ContextMaintenanceRun {
            status: domain::ContextMaintenanceStatus::Completed,
            summary: Some(summary),
            finished_at: Some(finished_at),
            ..started
        };
        let stored_maintenance = self
            .repository
            .finish_context_maintenance(
                completed,
                Some(checkpoint.clone()),
                Some(summary_block),
                guard,
                Some(crate::ports::FinishContextMaintenanceContextUpdate {
                    active_run_id: Some(input.target_run_id),
                    branch_pointer_id: Some(branch.id),
                    updated_at: finished_at,
                }),
            )
            .await
            .map_err(repository_port_error)?;
        let persisted_checkpoint = self
            .checkpoint_for_existing_maintenance(&stored_maintenance)
            .await?;
        let cursor = self
            .repository
            .get_context_cursor(&data.workspace_id)
            .await
            .map(context_cursor_view)
            .map_err(repository_port_error)?;
        Ok(SummarizeAndSetActiveContextResult {
            cursor,
            checkpoint: Some(context_checkpoint_view(&persisted_checkpoint)),
        })
    }

    async fn cancel_context_maintenance_impl(&self, client_operation_id: &str) -> AppResult<()> {
        let cancellation = self
            .maintenance_registry
            .lock()
            .map_err(|_| {
                AppError::internal(
                    "context_maintenance_registry_lock",
                    "Context maintenance registry is unavailable",
                )
            })?
            .get(client_operation_id)
            .cloned()
            .ok_or_else(|| {
                AppError::validation(
                    "context_maintenance_not_running",
                    "The requested Context maintenance operation is not running",
                )
                .with_details(json!({
                    "clientOperationId": client_operation_id,
                }))
            })?;
        cancellation.cancel();
        Ok(())
    }
}

fn spawn_run(
    run_persistence: Arc<dyn RunPersistencePort>,
    provider: Arc<dyn ProviderGateway>,
    registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    invocation: ProviderInvocation,
    cancellation: CancellationToken,
    events: Arc<dyn RunEventSink>,
) {
    let run_id = invocation.request.run_id.clone();
    tokio::spawn(async move {
        let (sender, receiver) = mpsc::channel(128);
        let provider_cancellation = cancellation.clone();
        let persistence_cancellation = cancellation.clone();
        let provider_task = tokio::spawn(async move {
            provider
                .stream(invocation, provider_cancellation, sender)
                .await
        });
        run_event_loop(
            run_persistence.as_ref(),
            &run_id,
            receiver,
            events,
            provider_task,
            persistence_cancellation,
        )
        .await;
        if let Ok(mut registry) = registry.lock() {
            registry.remove(&run_id);
        }
    });
}

async fn run_event_loop(
    repository: &dyn RunPersistencePort,
    run_id: &str,
    mut receiver: mpsc::Receiver<RunEvent>,
    sink: Arc<dyn RunEventSink>,
    provider_task: tokio::task::JoinHandle<Result<(), crate::ports::provider::ProviderError>>,
    cancellation: CancellationToken,
) {
    let mut output = String::new();
    let mut reasoning = String::new();
    let mut pending_output = String::new();
    let mut pending_reasoning = String::new();
    let mut usage: Option<Usage> = None;
    let mut interval = tokio::time::interval(Duration::from_millis(40));
    let mut checkpoint_interval = tokio::time::interval(Duration::from_millis(400));
    let mut bytes_since_checkpoint = 0usize;
    let mut terminal = false;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                flush_deltas(run_id, &sink, &mut pending_output, &mut pending_reasoning);
            }
            _ = checkpoint_interval.tick(), if bytes_since_checkpoint > 0 => {
                match checkpoint(repository, run_id, &output, &reasoning, usage.as_ref()).await {
                    Ok(CheckpointOutcome::Saved) => {
                        bytes_since_checkpoint = 0;
                        send_checkpoint_saved(run_id, &sink, output.len());
                    }
                    Ok(CheckpointOutcome::SkippedTerminal(_)) => {
                        cancellation.cancel();
                        terminal = true;
                        break;
                    }
                    Err(error) => {
                        cancellation.cancel();
                        finalize_storage_failure(
                            repository, run_id, &output, &reasoning, usage.as_ref(), &sink, &error,
                        )
                        .await;
                        terminal = true;
                        break;
                    }
                }
            }
            event = receiver.recv() => {
                let Some(event) = event else { break };
                match event {
                    RunEvent::RunStarted { .. } => {
                        if let Err(error) = repository.mark_run_streaming(run_id, now_millis()).await {
                            cancellation.cancel();
                            finalize_storage_failure(
                                repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                &error,
                            )
                            .await;
                            terminal = true;
                            break;
                        }
                        let _ = sink.send(RunEventView::RunStarted {
                            api_version: CONTRACT_VERSION,
                            run_id: run_id.into(),
                            at: now_view(),
                        });
                    }
                    RunEvent::TextDelta { text } => {
                        bytes_since_checkpoint += text.len();
                        output.push_str(&text);
                        pending_output.push_str(&text);
                    }
                    RunEvent::ReasoningDelta { text } => {
                        bytes_since_checkpoint += text.len();
                        reasoning.push_str(&text);
                        pending_reasoning.push_str(&text);
                    }
                    RunEvent::UsageUpdated { usage: next } => {
                        usage = Some(next.clone());
                        let _ = sink.send(RunEventView::UsageUpdated {
                            api_version: CONTRACT_VERSION,
                            run_id: run_id.into(),
                            usage: usage_map(&next),
                            at: now_view(),
                        });
                    }
                    RunEvent::ProviderMetadata { request_id, model, created_at } => {
                        let mut metadata = BTreeMap::new();
                        if let Some(value) = request_id { metadata.insert("requestId".into(), Value::String(value)); }
                        if let Some(value) = model { metadata.insert("model".into(), Value::String(value)); }
                        if let Some(value) = created_at { metadata.insert("createdAt".into(), Value::String(value)); }
                        let _ = sink.send(RunEventView::ProviderMetadata {
                            api_version: CONTRACT_VERSION,
                            run_id: run_id.into(),
                            metadata,
                            at: now_view(),
                        });
                    }
                    RunEvent::RunCompleted { .. } => {
                        flush_deltas(run_id, &sink, &mut pending_output, &mut pending_reasoning);
                        match checkpoint(repository, run_id, &output, &reasoning, usage.as_ref()).await {
                            Ok(CheckpointOutcome::Saved) => {
                                send_checkpoint_saved(run_id, &sink, output.len())
                            }
                            Ok(CheckpointOutcome::SkippedTerminal(_)) => {
                                cancellation.cancel();
                                terminal = true;
                                break;
                            }
                            Err(error) => {
                                cancellation.cancel();
                                finalize_storage_failure(
                                    repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                    &error,
                                )
                                .await;
                                terminal = true;
                                break;
                            }
                        }
                        match persist_terminal_run(
                            repository,
                            run_id,
                            RunStatus::Completed,
                            &output,
                            &reasoning,
                            usage.as_ref(),
                            None,
                        ).await {
                            Ok(()) => {
                                let _ = sink.send(RunEventView::RunCompleted {
                                    api_version: CONTRACT_VERSION,
                                    run_id: run_id.into(),
                                    at: now_view(),
                                });
                            }
                            Err(error) => {
                                finalize_storage_failure(
                                    repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                    &error,
                                )
                                .await
                            }
                        }
                        terminal = true;
                        break;
                    }
                    RunEvent::RunFailed { code, message, retryable, status } => {
                        flush_deltas(run_id, &sink, &mut pending_output, &mut pending_reasoning);
                        match persist_terminal_run(
                            repository,
                            run_id,
                            RunStatus::Failed,
                            &output,
                            &reasoning,
                            usage.as_ref(),
                            Some(RunFailure {
                                code: code.clone(),
                                message: message.clone(),
                                retryable,
                                status,
                            }),
                        ).await {
                            Ok(()) => {
                                let _ = sink.send(RunEventView::RunFailed {
                                    api_version: CONTRACT_VERSION,
                                    run_id: run_id.into(),
                                    error: RunErrorView {
                                        code,
                                        message,
                                        retryable,
                                        status,
                                    },
                                    at: now_view(),
                                });
                            }
                            Err(error) => {
                                finalize_storage_failure(
                                    repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                    &error,
                                )
                                .await
                            }
                        }
                        terminal = true;
                        break;
                    }
                    RunEvent::RunCancelled => {
                        flush_deltas(run_id, &sink, &mut pending_output, &mut pending_reasoning);
                        match persist_terminal_run(
                            repository,
                            run_id,
                            RunStatus::Cancelled,
                            &output,
                            &reasoning,
                            usage.as_ref(),
                            None,
                        ).await {
                            Ok(()) => {
                                let _ = sink.send(RunEventView::RunCancelled {
                                    api_version: CONTRACT_VERSION,
                                    run_id: run_id.into(),
                                    at: now_view(),
                                });
                            }
                            Err(error) => {
                                finalize_storage_failure(
                                    repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                    &error,
                                )
                                .await
                            }
                        }
                        terminal = true;
                        break;
                    }
                }
                if bytes_since_checkpoint >= 4096 {
                    match checkpoint(repository, run_id, &output, &reasoning, usage.as_ref()).await {
                        Ok(CheckpointOutcome::Saved) => {
                            bytes_since_checkpoint = 0;
                            send_checkpoint_saved(run_id, &sink, output.len());
                        }
                        Ok(CheckpointOutcome::SkippedTerminal(_)) => {
                            cancellation.cancel();
                            terminal = true;
                            break;
                        }
                        Err(error) => {
                            cancellation.cancel();
                            finalize_storage_failure(
                                repository, run_id, &output, &reasoning, usage.as_ref(), &sink,
                                &error,
                            )
                            .await;
                            terminal = true;
                            break;
                        }
                    }
                }
            }
        }
    }
    drop(receiver);
    let provider_result = provider_task.await;
    if !terminal {
        let message = match provider_result {
            Ok(Ok(())) => "Provider stream ended without a terminal event".to_owned(),
            Ok(Err(error)) => error.to_string(),
            Err(error) => format!("Provider task failed: {error}"),
        };
        match persist_terminal_run(
            repository,
            run_id,
            RunStatus::Failed,
            &output,
            &reasoning,
            usage.as_ref(),
            Some(RunFailure {
                code: "provider_stream_ended".into(),
                message: message.clone(),
                retryable: true,
                status: None,
            }),
        )
        .await
        {
            Ok(()) => {
                let _ = sink.send(RunEventView::RunFailed {
                    api_version: CONTRACT_VERSION,
                    run_id: run_id.into(),
                    error: RunErrorView {
                        code: "provider_stream_ended".into(),
                        message,
                        retryable: true,
                        status: None,
                    },
                    at: now_view(),
                });
            }
            Err(error) => {
                finalize_storage_failure(
                    repository,
                    run_id,
                    &output,
                    &reasoning,
                    usage.as_ref(),
                    &sink,
                    &error,
                )
                .await
            }
        }
    }
}

fn flush_deltas(
    run_id: &str,
    sink: &Arc<dyn RunEventSink>,
    output: &mut String,
    reasoning: &mut String,
) {
    if !output.is_empty() {
        let _ = sink.send(RunEventView::TextDelta {
            api_version: CONTRACT_VERSION,
            run_id: run_id.into(),
            text: std::mem::take(output),
            at: now_view(),
        });
    }
    if !reasoning.is_empty() {
        let _ = sink.send(RunEventView::ReasoningDelta {
            api_version: CONTRACT_VERSION,
            run_id: run_id.into(),
            text: std::mem::take(reasoning),
            at: now_view(),
        });
    }
}

fn send_checkpoint_saved(run_id: &str, sink: &Arc<dyn RunEventSink>, output_bytes: usize) {
    let mut metadata = BTreeMap::new();
    metadata.insert("outputBytes".into(), json!(output_bytes));
    let _ = sink.send(RunEventView::CheckpointSaved {
        api_version: CONTRACT_VERSION,
        run_id: run_id.into(),
        metadata,
        at: now_view(),
    });
}

async fn finalize_storage_failure(
    repository: &dyn RunPersistencePort,
    run_id: &str,
    output: &str,
    reasoning: &str,
    usage: Option<&Usage>,
    sink: &Arc<dyn RunEventSink>,
    error: &RepositoryPortError,
) {
    let message = format!("Run output could not be persisted: {error}");
    let committed = persist_terminal_run(
        repository,
        run_id,
        RunStatus::Failed,
        output,
        reasoning,
        usage,
        Some(RunFailure {
            code: "storage_failure".into(),
            message: message.clone(),
            retryable: true,
            status: None,
        }),
    )
    .await
    .is_ok();
    let event = storage_failure_event(run_id, message, committed);
    let _ = sink.send(event);
}

fn storage_failure_event(run_id: &str, message: String, committed: bool) -> RunEventView {
    if committed {
        RunEventView::RunFailed {
            api_version: CONTRACT_VERSION,
            run_id: run_id.into(),
            error: RunErrorView {
                code: "storage_failure".into(),
                message,
                retryable: true,
                status: None,
            },
            at: now_view(),
        }
    } else {
        RunEventView::PersistenceFailed {
            api_version: CONTRACT_VERSION,
            run_id: run_id.into(),
            error: RunErrorView {
                code: "storage_failure_uncommitted".into(),
                message,
                retryable: false,
                status: None,
            },
            at: now_view(),
        }
    }
}

async fn checkpoint(
    repository: &dyn RunPersistencePort,
    run_id: &str,
    output: &str,
    reasoning: &str,
    usage: Option<&Usage>,
) -> Result<CheckpointOutcome, RepositoryPortError> {
    repository
        .checkpoint_run(
            run_id,
            RunCheckpoint {
                output_markdown: output.into(),
                reasoning_markdown: reasoning.into(),
                usage: usage.map(domain_usage),
                checkpointed_at: now_millis(),
            },
        )
        .await
}

fn domain_usage(usage: &Usage) -> domain::RunUsage {
    domain::RunUsage::new(
        usage.prompt_tokens.unwrap_or(0),
        usage.completion_tokens.unwrap_or(0),
    )
}

async fn persist_terminal_run(
    repository: &dyn RunPersistencePort,
    run_id: &str,
    status: RunStatus,
    output: &str,
    reasoning: &str,
    usage: Option<&Usage>,
    error: Option<RunFailure>,
) -> Result<(), RepositoryPortError> {
    repository
        .finish_run(
            run_id,
            RunFinish {
                status,
                output_markdown: output.into(),
                reasoning_markdown: reasoning.into(),
                error,
                usage: usage.map(domain_usage),
                finished_at: now_millis(),
            },
        )
        .await
}

fn exact_retry_turn_id(original: &ModelRun) -> &str {
    &original.turn_id
}

fn content_blocks_for_manifest(
    workspace_id: &str,
    items: &[domain::RunContextItem],
    now: i64,
) -> Vec<domain::ContentBlock> {
    items
        .iter()
        .map(|item| {
            let id = if item.content_block_id.is_empty() {
                content_block_id(message_role_name(item.role), &item.content_hash)
            } else {
                item.content_block_id.clone()
            };
            (
                id.clone(),
                domain::ContentBlock {
                    id,
                    workspace_id: workspace_id.into(),
                    role: item.role,
                    content: item.content.clone(),
                    content_hash: item.content_hash.clone(),
                    created_at: now,
                },
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect()
}

fn content_block_id(role: &str, content_hash: &str) -> String {
    format!("block-{role}-{content_hash}")
}

fn workspace_view(record: Workspace) -> WorkspaceSummary {
    WorkspaceSummary {
        id: record.id,
        name: record.title,
        goal: record.goal,
        system_prompt: record.system_prompt,
        archived: record.archived_at.is_some(),
        created_at: timestamp_view(record.created_at),
        updated_at: timestamp_view(record.updated_at),
    }
}

fn run_view(record: &ModelRun, provenance: Option<&RunProviderProvenance>) -> AppResult<RunView> {
    let usage = record.usage().map(|usage| {
        BTreeMap::from([
            ("prompt_tokens".into(), usage.input_tokens),
            ("completion_tokens".into(), usage.output_tokens),
        ])
    });
    let error = record.failure().map(|failure| RunErrorView {
        code: failure.code.clone(),
        message: failure.message.clone(),
        retryable: failure.retryable,
        status: failure.status,
    });
    let state = record.state_snapshot();
    Ok(RunView {
        id: record.id.clone(),
        turn_id: record.turn_id.clone(),
        status: domain_run_status_view(record.status()),
        output: record.output_markdown().into(),
        reasoning: (!record.reasoning_markdown().is_empty())
            .then(|| record.reasoning_markdown().to_owned()),
        provider_profile_id: record.provider_profile_id.clone().unwrap_or_default(),
        provider_name: provenance
            .map(|provenance| provenance.provider_name.clone())
            .unwrap_or_else(|| "Unknown provider".into()),
        model: provenance
            .map(|provenance| provenance.model.clone())
            .unwrap_or_else(|| record.model.clone()),
        base_url: provenance
            .map(|provenance| provenance.base_url.clone())
            .unwrap_or_default(),
        created_at: timestamp_view(record.created_at),
        completed_at: state.finished_at.map(timestamp_view),
        usage,
        error,
    })
}

fn receipt_view(receipt: domain::ContextSnapshot) -> AppResult<RunSnapshotView> {
    let parameters = receipt
        .provider
        .parameters
        .iter()
        .map(|(key, value)| (key.clone(), parse_parameter_value(value)))
        .collect();
    Ok(RunSnapshotView {
        id: receipt.id,
        run_id: receipt.run_id,
        canonical_hash: receipt.manifest.canonical_hash,
        created_at: timestamp_view(receipt.created_at),
        provider_id: receipt.provider.provider_id,
        template_revision: receipt.provider.template_revision,
        provider_name: receipt.provider.provider_name,
        stream_protocol: receipt
            .provider
            .stream_protocol
            .map(|protocol| match protocol {
                domain::StreamProtocol::OpenAiSse => ProviderStreamProtocolView::OpenAiSse,
                domain::StreamProtocol::OllamaNdjson => ProviderStreamProtocolView::OllamaNdjson,
                domain::StreamProtocol::AnthropicSse => ProviderStreamProtocolView::AnthropicSse,
                domain::StreamProtocol::GoogleSse => ProviderStreamProtocolView::GoogleSse,
            }),
        auth_placement: receipt
            .provider
            .auth_placement
            .map(|placement| match placement {
                domain::AuthPlacement::None => ProviderAuthPlacementView::None,
                domain::AuthPlacement::BearerHeader => ProviderAuthPlacementView::BearerHeader,
                domain::AuthPlacement::ApiKeyHeader => ProviderAuthPlacementView::ApiKeyHeader,
                domain::AuthPlacement::QueryParam => ProviderAuthPlacementView::QueryParam,
            }),
        auth_header_name: receipt.provider.auth_header_name,
        additional_headers: receipt.provider.additional_headers,
        model: receipt.provider.model,
        base_url: receipt.provider.base_url,
        parameters,
        items: receipt
            .manifest
            .items
            .iter()
            .map(|item| context_item_view(item, true, false))
            .collect(),
    })
}

fn context_item_view(
    item: &domain::RunContextItem,
    included: bool,
    pinned: bool,
) -> ContextItemView {
    ContextItemView {
        id: item.source_ref.stable_id(),
        source_ref: ContextSourceRefView {
            kind: match item.source_ref.kind {
                domain::ContextSourceRefKind::WorkspaceSystem => {
                    ContextSourceKindView::WorkspaceSystem
                }
                domain::ContextSourceRefKind::TurnPrompt => ContextSourceKindView::TurnPrompt,
                domain::ContextSourceRefKind::ModelRun => ContextSourceKindView::ModelRun,
                domain::ContextSourceRefKind::ContentBlock => ContextSourceKindView::ContentBlock,
                domain::ContextSourceRefKind::CurrentPrompt => ContextSourceKindView::CurrentPrompt,
                domain::ContextSourceRefKind::CheckpointSummary => {
                    ContextSourceKindView::CheckpointSummary
                }
                domain::ContextSourceRefKind::BranchSummary => ContextSourceKindView::BranchSummary,
            },
            id: item.source_ref.id.clone(),
        },
        content_block_id: Some(item.content_block_id.clone()),
        content_hash: item.content_hash.clone(),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: domain_message_role_view(item.role),
        label: source_label(context_source_name(item.source_kind)),
        source: context_source_name(item.source_kind).into(),
        content: item.content.clone(),
        reason: inclusion_reason_name(item.inclusion_reason).into(),
        estimated_tokens: chars_to_tokens(item.content.chars().count()),
        included,
        pinned,
        mandatory: item.mandatory,
    }
}

fn provider_profile_view(record: ProviderProfile) -> AppResult<ProviderProfileView> {
    let mut parameters = record
        .parameters
        .iter()
        .map(|(key, value)| (key.clone(), parse_parameter_value(value)))
        .collect::<BTreeMap<_, _>>();
    let is_default = parameters
        .remove(INTERNAL_DEFAULT_KEY)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(ProviderProfileView {
        id: record.id,
        provider_id: record.provider_id,
        name: record.name,
        dialect: match record.dialect {
            domain::ProviderDialect::OpenAiCompatible => ProviderDialectView::OpenaiCompatible,
            domain::ProviderDialect::Ollama => ProviderDialectView::Ollama,
            domain::ProviderDialect::Anthropic => ProviderDialectView::Anthropic,
            domain::ProviderDialect::GoogleGenerativeAi => ProviderDialectView::GoogleGenerativeAi,
        },
        base_url: record.base_url,
        model: record.model,
        is_default,
        parameters: (!parameters.is_empty()).then_some(parameters),
    })
}

fn provider_template_view(record: &domain::ProviderTemplate) -> ProviderTemplateView {
    ProviderTemplateView {
        provider_id: record.provider_id.into(),
        revision: record.revision,
        display_name: record.display_name.into(),
        default_base_url: record.default_base_url.into(),
        protocol: ProtocolProfileView {
            stream_protocol: match record.protocol.stream_protocol {
                domain::StreamProtocol::OpenAiSse => ProviderStreamProtocolView::OpenAiSse,
                domain::StreamProtocol::OllamaNdjson => ProviderStreamProtocolView::OllamaNdjson,
                domain::StreamProtocol::AnthropicSse => ProviderStreamProtocolView::AnthropicSse,
                domain::StreamProtocol::GoogleSse => ProviderStreamProtocolView::GoogleSse,
            },
            auth_placement: match record.protocol.auth_placement {
                domain::AuthPlacement::None => ProviderAuthPlacementView::None,
                domain::AuthPlacement::BearerHeader => ProviderAuthPlacementView::BearerHeader,
                domain::AuthPlacement::ApiKeyHeader => ProviderAuthPlacementView::ApiKeyHeader,
                domain::AuthPlacement::QueryParam => ProviderAuthPlacementView::QueryParam,
            },
            auth_header_name: record.protocol.auth_header_name.map(str::to_owned),
            models_endpoint: record.protocol.models_endpoint.map(str::to_owned),
            requires_additional_headers: record.protocol.requires_additional_headers,
            additional_headers: record
                .protocol
                .additional_headers
                .iter()
                .map(|header| (header.name.into(), header.value.into()))
                .collect(),
        },
        runtime_available: record.runtime_available,
    }
}

fn domain_provider_snapshot(record: &ProviderProfile) -> AppResult<domain::ProviderSnapshot> {
    let (template, template_dialect) = runnable_template(&record.provider_id)?;
    if record.dialect != template_dialect {
        return Err(AppError::internal(
            "invalid_provider_profile",
            "Provider Profile dialect does not match its template",
        ));
    }
    let parameters = effective_provider_parameters(record)?.stored_values();
    Ok(domain::ProviderSnapshot {
        profile_id: record.id.clone(),
        provider_id: Some(record.provider_id.clone()),
        template_revision: Some(template.revision),
        provider_name: record.name.clone(),
        dialect: record.dialect,
        stream_protocol: Some(template.protocol.stream_protocol),
        auth_placement: Some(template.protocol.auth_placement),
        auth_header_name: template.protocol.auth_header_name.map(str::to_owned),
        additional_headers: template
            .protocol
            .additional_headers
            .iter()
            .map(|header| (header.name.into(), header.value.into()))
            .collect(),
        base_url: record.base_url.clone(),
        model: record.model.clone(),
        parameters,
    })
}

fn parse_parameter_value(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()))
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EffectiveProviderParameters {
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_output_tokens: Option<u32>,
    stop: Vec<String>,
}

impl EffectiveProviderParameters {
    fn stored_values(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        if let Some(value) = self.temperature {
            values.insert("temperature".into(), json!(value).to_string());
        }
        if let Some(value) = self.top_p {
            values.insert("top_p".into(), json!(value).to_string());
        }
        if let Some(value) = self.max_output_tokens {
            values.insert("max_output_tokens".into(), json!(value).to_string());
        }
        if !self.stop.is_empty() {
            values.insert("stop".into(), json!(self.stop).to_string());
        }
        values
    }
}

fn invalid_provider_parameter(name: &str, expectation: &str) -> AppError {
    AppError::validation(
        "invalid_provider_parameters",
        format!("Provider parameter `{name}` {expectation}"),
    )
    .with_details(json!({ "parameter": name }))
}

fn normalized_f32_parameter(name: &str, value: Value) -> AppResult<f32> {
    let value = value
        .as_f64()
        .ok_or_else(|| invalid_provider_parameter(name, "must be a JSON number"))?;
    let normalized = value as f32;
    if !normalized.is_finite() {
        return Err(invalid_provider_parameter(
            name,
            "must fit in a finite 32-bit floating-point value",
        ));
    }
    Ok(normalized)
}

fn normalize_provider_parameter_values(
    parameters: BTreeMap<String, Value>,
) -> AppResult<EffectiveProviderParameters> {
    let mut effective = EffectiveProviderParameters::default();
    for (name, value) in parameters {
        match name.as_str() {
            "temperature" => {
                effective.temperature = Some(normalized_f32_parameter(&name, value)?);
            }
            "top_p" => {
                effective.top_p = Some(normalized_f32_parameter(&name, value)?);
            }
            "max_output_tokens" => {
                effective.max_output_tokens = Some(
                    value
                        .as_u64()
                        .filter(|value| *value > 0)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| {
                            invalid_provider_parameter(
                                &name,
                                "must be a positive 32-bit JSON integer",
                            )
                        })?,
                );
            }
            "stop" => {
                let values = value.as_array().ok_or_else(|| {
                    invalid_provider_parameter(&name, "must be an array of strings")
                })?;
                effective.stop = values
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            invalid_provider_parameter(&name, "must be an array of strings")
                        })
                    })
                    .collect::<AppResult<_>>()?;
            }
            _ => {
                return Err(invalid_provider_parameter(
                    &name,
                    "is not supported; allowed parameters are temperature, top_p, max_output_tokens, and stop",
                ));
            }
        }
    }
    Ok(effective)
}

fn effective_provider_parameters(
    record: &ProviderProfile,
) -> AppResult<EffectiveProviderParameters> {
    let parameters = record
        .parameters
        .iter()
        .filter(|(name, _)| name.as_str() != INTERNAL_DEFAULT_KEY)
        .map(|(name, value)| {
            serde_json::from_str(value)
                .map(|value| (name.clone(), value))
                .map_err(|_| {
                    invalid_provider_parameter(name, "must contain a canonical JSON value")
                })
        })
        .collect::<AppResult<BTreeMap<_, _>>>()?;
    let mut effective = normalize_provider_parameter_values(parameters)?;
    if record.dialect == domain::ProviderDialect::Anthropic && effective.max_output_tokens.is_none()
    {
        effective.max_output_tokens = Some(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS);
    }
    Ok(effective)
}

fn decision_view(record: DecisionMark) -> AppResult<DecisionMarkView> {
    Ok(DecisionMarkView {
        id: record.id,
        workspace_id: record.workspace_id,
        run_id: record.run_id,
        status: match record.status {
            DecisionStatus::Adopted => DecisionStatusView::Accepted,
            DecisionStatus::Rejected => DecisionStatusView::Rejected,
            DecisionStatus::NeedsValidation => DecisionStatusView::ToVerify,
        },
        reason: record.reason,
        created_at: timestamp_view(record.created_at),
    })
}

fn context_cursor_view(cursor: crate::ports::ContextCursor) -> ContextCursorView {
    ContextCursorView {
        workspace_id: cursor.workspace_id,
        active_run_id: cursor.active_run_id,
        branch_id: cursor.branch_pointer_id,
        version: cursor.version,
        updated_at: timestamp_view(cursor.updated_at),
    }
}

fn context_branch_view(
    pointer: BranchPointer,
    active_branch_id: Option<&str>,
) -> ContextBranchView {
    ContextBranchView {
        is_active: active_branch_id == Some(pointer.id.as_str()),
        id: pointer.id,
        name: pointer.name,
        head_run_id: pointer.head_run_id,
        version: pointer.version,
    }
}

fn context_checkpoint_view(checkpoint: &domain::ContextCheckpoint) -> ContextCheckpointView {
    let provider = checkpoint
        .provider
        .as_ref()
        .map(context_checkpoint_provider_snapshot_view);
    ContextCheckpointView {
        id: checkpoint.id.clone(),
        workspace_id: checkpoint.workspace_id.clone(),
        branch_id: checkpoint.branch_pointer_id.clone(),
        branch_version: checkpoint.branch_revision,
        kind: match checkpoint.kind {
            domain::ContextCheckpointKind::Compaction => ContextCheckpointKindView::Compaction,
            domain::ContextCheckpointKind::BranchSummary => {
                ContextCheckpointKindView::BranchSummary
            }
        },
        anchor_run_id: Some(checkpoint.anchor_run_id.clone()),
        source_run_ids: checkpoint.source_run_ids.clone(),
        source_hash: checkpoint.source_hash.clone(),
        first_kept_run_id: checkpoint.first_kept_run_id.clone(),
        summary: checkpoint.summary.clone(),
        provider,
        status: ContextMaintenanceStatusView::Completed,
        created_at: timestamp_view(checkpoint.created_at),
    }
}

fn context_maintenance_replay<'a>(
    maintenance_runs: &'a [domain::ContextMaintenanceRun],
    client_operation_id: &str,
    expected_kind: domain::ContextCheckpointKind,
    expected_request_json: &str,
) -> AppResult<Option<&'a domain::ContextMaintenanceRun>> {
    let Some(maintenance) = maintenance_runs
        .iter()
        .find(|run| run.id == client_operation_id)
    else {
        return Ok(None);
    };
    if maintenance.kind != expected_kind || maintenance.request_json != expected_request_json {
        return Err(AppError::validation(
            "idempotency_key_reused",
            "clientOperationId was already used for a different Context maintenance request",
        )
        .with_details(json!({
            "clientOperationId": client_operation_id,
        })));
    }
    match maintenance.status {
        domain::ContextMaintenanceStatus::Completed => Ok(Some(maintenance)),
        domain::ContextMaintenanceStatus::Queued | domain::ContextMaintenanceStatus::Running => {
            Err(AppError::internal(
                "context_maintenance_in_progress",
                "The Context maintenance operation is already in progress",
            )
            .with_details(json!({
                "clientOperationId": client_operation_id,
            })))
        }
        domain::ContextMaintenanceStatus::Failed
        | domain::ContextMaintenanceStatus::Cancelled
        | domain::ContextMaintenanceStatus::Conflicted => Err(AppError::validation(
            "context_maintenance_terminal",
            "The Context maintenance operation already reached a terminal non-success state; retry with a new clientOperationId",
        )
        .with_details(json!({
            "clientOperationId": client_operation_id,
            "status": context_maintenance_status_name(maintenance.status),
            "error": &maintenance.error,
        }))),
    }
}

fn context_maintenance_status_name(status: domain::ContextMaintenanceStatus) -> &'static str {
    match status {
        domain::ContextMaintenanceStatus::Queued => "queued",
        domain::ContextMaintenanceStatus::Running => "running",
        domain::ContextMaintenanceStatus::Completed => "completed",
        domain::ContextMaintenanceStatus::Failed => "failed",
        domain::ContextMaintenanceStatus::Cancelled => "cancelled",
        domain::ContextMaintenanceStatus::Conflicted => "conflicted",
    }
}

fn context_source_ref_domain(source: ContextSourceRefView) -> domain::ContextSourceRef {
    domain::ContextSourceRef {
        kind: match source.kind {
            ContextSourceKindView::WorkspaceSystem => domain::ContextSourceRefKind::WorkspaceSystem,
            ContextSourceKindView::TurnPrompt => domain::ContextSourceRefKind::TurnPrompt,
            ContextSourceKindView::ModelRun => domain::ContextSourceRefKind::ModelRun,
            ContextSourceKindView::ContentBlock => domain::ContextSourceRefKind::ContentBlock,
            ContextSourceKindView::CurrentPrompt => domain::ContextSourceRefKind::CurrentPrompt,
            ContextSourceKindView::CheckpointSummary => {
                domain::ContextSourceRefKind::CheckpointSummary
            }
            ContextSourceKindView::BranchSummary => domain::ContextSourceRefKind::BranchSummary,
        },
        id: source.id,
    }
}

fn context_overrides_from_draft(
    draft: &crate::ports::ContextDraft,
) -> AppResult<domain::ContextOverrides> {
    let pinned_sources = draft
        .items
        .iter()
        .filter(|item| item.operation == crate::ports::ContextOverrideOperation::Pin)
        .map(|item| {
            let content_block_id = item.content_block_id.clone().ok_or_else(|| {
                AppError::internal(
                    "context_draft_pin_identity_missing",
                    "Persisted Context pin is missing its immutable Content Block identity",
                )
                .with_details(json!({
                    "sourceRef": item.source_ref.stable_id(),
                    "draftVersion": draft.version,
                }))
            })?;
            let content_hash = item.content_hash.clone().ok_or_else(|| {
                AppError::internal(
                    "context_draft_pin_identity_missing",
                    "Persisted Context pin is missing its immutable content hash",
                )
                .with_details(json!({
                    "sourceRef": item.source_ref.stable_id(),
                    "draftVersion": draft.version,
                }))
            })?;
            Ok(domain::ContextPin {
                source_ref: item.source_ref.clone(),
                content_block_id,
                content_hash,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let excluded_source_ids = draft
        .items
        .iter()
        .filter(|item| item.operation == crate::ports::ContextOverrideOperation::Exclude)
        .map(|item| item.source_ref.stable_id())
        .collect();
    Ok(domain::ContextOverrides {
        pinned_sources,
        excluded_source_ids,
    })
}

fn validate_context_source_ref(
    data: &crate::ports::WorkspaceContextData,
    source: &domain::ContextSourceRef,
) -> AppResult<()> {
    let id = source.id.as_deref().ok_or_else(|| {
        AppError::validation(
            "missing_context_source_identity",
            "A persisted Context Draft item must have a stable source identity",
        )
    })?;
    let exists = match source.kind {
        domain::ContextSourceRefKind::WorkspaceSystem
        | domain::ContextSourceRefKind::CurrentPrompt => id == data.workspace_id,
        domain::ContextSourceRefKind::TurnPrompt => data.graph.turn(id).is_some(),
        domain::ContextSourceRefKind::ModelRun => data.graph.run(id).is_some(),
        domain::ContextSourceRefKind::ContentBlock => data.graph.content_block(id).is_some(),
        domain::ContextSourceRefKind::CheckpointSummary => {
            data.checkpoints.iter().any(|checkpoint| {
                checkpoint.id == id && checkpoint.kind == domain::ContextCheckpointKind::Compaction
            })
        }
        domain::ContextSourceRefKind::BranchSummary => data.checkpoints.iter().any(|checkpoint| {
            checkpoint.id == id && checkpoint.kind == domain::ContextCheckpointKind::BranchSummary
        }),
    };
    if !exists {
        return Err(AppError::validation(
            "context_source_not_found",
            format!(
                "Context source `{}` is not in workspace `{}`",
                source.stable_id(),
                data.workspace_id
            ),
        ));
    }
    Ok(())
}

fn legacy_context_source_ref(
    data: &crate::ports::WorkspaceContextData,
    item_id: &str,
) -> AppResult<domain::ContextSourceRef> {
    let prefixed = [
        (
            "workspace-system:",
            domain::ContextSourceRefKind::WorkspaceSystem,
        ),
        ("turn-prompt:", domain::ContextSourceRefKind::TurnPrompt),
        ("model-run:", domain::ContextSourceRefKind::ModelRun),
        ("content-block:", domain::ContextSourceRefKind::ContentBlock),
        (
            "current-prompt:",
            domain::ContextSourceRefKind::CurrentPrompt,
        ),
        (
            "checkpoint-summary:",
            domain::ContextSourceRefKind::CheckpointSummary,
        ),
        (
            "branch-summary:",
            domain::ContextSourceRefKind::BranchSummary,
        ),
    ]
    .into_iter()
    .find_map(|(prefix, kind)| {
        item_id
            .strip_prefix(prefix)
            .map(|id| domain::ContextSourceRef::new(kind, id))
    });
    let source = prefixed.unwrap_or_else(|| {
        if data.graph.run(item_id).is_some() {
            domain::ContextSourceRef::new(domain::ContextSourceRefKind::ModelRun, item_id)
        } else if data.graph.turn(item_id).is_some() {
            domain::ContextSourceRef::new(domain::ContextSourceRefKind::TurnPrompt, item_id)
        } else if data.graph.content_block(item_id).is_some() {
            domain::ContextSourceRef::new(domain::ContextSourceRefKind::ContentBlock, item_id)
        } else if item_id == format!("workspace:{}:system", data.workspace_id) {
            domain::ContextSourceRef::new(
                domain::ContextSourceRefKind::WorkspaceSystem,
                data.workspace_id.clone(),
            )
        } else {
            domain::ContextSourceRef {
                kind: domain::ContextSourceRefKind::CurrentPrompt,
                id: None,
            }
        }
    });
    validate_context_source_ref(data, &source)?;
    Ok(source)
}

fn content_block_for_source(
    data: &crate::ports::WorkspaceContextData,
    source: &domain::ContextSourceRef,
) -> Option<domain::ContentBlock> {
    let id = source.id.as_deref()?;
    match source.kind {
        domain::ContextSourceRefKind::ContentBlock => data.graph.content_block(id).cloned(),
        domain::ContextSourceRefKind::ModelRun => {
            let run = data.graph.run(id)?;
            let content = run.output_markdown().to_owned();
            let content_hash = domain::sha256_hex(content.as_bytes());
            let block_id = content_block_id("assistant", &content_hash);
            data.graph.content_block(&block_id).cloned().or_else(|| {
                Some(domain::ContentBlock {
                    id: block_id,
                    workspace_id: data.workspace_id.clone(),
                    role: MessageRole::Assistant,
                    content,
                    content_hash,
                    created_at: run.created_at,
                })
            })
        }
        domain::ContextSourceRefKind::TurnPrompt => {
            let turn = data.graph.turn(id)?;
            let content = turn.prompt_markdown.clone();
            let content_hash = domain::sha256_hex(content.as_bytes());
            let block_id = content_block_id("user", &content_hash);
            data.graph.content_block(&block_id).cloned().or_else(|| {
                Some(domain::ContentBlock {
                    id: block_id,
                    workspace_id: data.workspace_id.clone(),
                    role: MessageRole::User,
                    content,
                    content_hash,
                    created_at: turn.created_at,
                })
            })
        }
        domain::ContextSourceRefKind::CheckpointSummary
        | domain::ContextSourceRefKind::BranchSummary => {
            let checkpoint = data.checkpoints.iter().find(|checkpoint| {
                checkpoint.id == id
                    && matches!(
                        (source.kind, checkpoint.kind),
                        (
                            domain::ContextSourceRefKind::CheckpointSummary,
                            domain::ContextCheckpointKind::Compaction
                        ) | (
                            domain::ContextSourceRefKind::BranchSummary,
                            domain::ContextCheckpointKind::BranchSummary
                        )
                    )
            })?;
            data.graph
                .content_block(&checkpoint.summary_content_block_id)
                .cloned()
                .or_else(|| {
                    Some(domain::ContentBlock {
                        id: checkpoint.summary_content_block_id.clone(),
                        workspace_id: data.workspace_id.clone(),
                        role: MessageRole::System,
                        content: checkpoint.summary.clone(),
                        content_hash: domain::sha256_hex(checkpoint.summary.as_bytes()),
                        created_at: checkpoint.created_at,
                    })
                })
        }
        domain::ContextSourceRefKind::WorkspaceSystem
        | domain::ContextSourceRefKind::CurrentPrompt => None,
    }
}

fn validate_pinned_content_identity(
    source: &domain::ContextSourceRef,
    requested_content_block_id: &str,
    block: &domain::ContentBlock,
) -> AppResult<()> {
    if block.id != requested_content_block_id {
        return Err(AppError::validation(
            "context_content_identity_mismatch",
            format!(
                "Context source `{}` does not own Content Block `{requested_content_block_id}`",
                source.stable_id()
            ),
        ));
    }
    Ok(())
}

fn checkpoint_kind_domain(kind: ContextCheckpointKindView) -> domain::ContextCheckpointKind {
    match kind {
        ContextCheckpointKindView::Compaction => domain::ContextCheckpointKind::Compaction,
        ContextCheckpointKindView::BranchSummary => domain::ContextCheckpointKind::BranchSummary,
    }
}

fn checkpoint_source_hash(
    data: &crate::ports::WorkspaceContextData,
    source_run_ids: &[String],
) -> AppResult<String> {
    let evidence = source_run_ids
        .iter()
        .map(|run_id| {
            let run = data.graph.run(run_id).ok_or_else(|| {
                AppError::validation(
                    "checkpoint_source_not_found",
                    format!("Model Run `{run_id}` is not in this workspace"),
                )
            })?;
            ensure_checkpoint_source_completed(run, run_id)?;
            let turn = data.graph.turn(&run.turn_id).ok_or_else(|| {
                AppError::internal(
                    "checkpoint_turn_missing",
                    "A validated Context graph is missing a Run's Turn",
                )
            })?;
            Ok(json!({
                "runId": &run.id,
                "turnId": &turn.id,
                "parentRunId": &turn.parent_run_id,
                "prompt": &turn.prompt_markdown,
                "answer": run.output_markdown(),
            }))
        })
        .collect::<AppResult<Vec<_>>>()?;
    serde_json::to_vec(&evidence)
        .map(|encoded| domain::sha256_hex(&encoded))
        .map_err(|error| AppError::internal("checkpoint_hash_failed", error.to_string()))
}

fn validate_manual_checkpoint_summary(summary: &str) -> AppResult<()> {
    if summary.trim().is_empty() {
        return Err(AppError::validation(
            "empty_context_summary",
            "A manual Context checkpoint requires a non-empty summary",
        )
        .with_details(json!({ "mode": "manual" })));
    }
    Ok(())
}

fn validated_checkpoint_source<'a>(
    runs_by_id: &'a HashMap<String, ModelRun>,
    run_id: &str,
) -> AppResult<&'a ModelRun> {
    let run = runs_by_id.get(run_id).ok_or_else(|| {
        AppError::validation(
            "checkpoint_source_not_found",
            format!("Model Run `{run_id}` is not in this workspace"),
        )
    })?;
    ensure_checkpoint_source_completed(run, run_id)?;
    Ok(run)
}

fn ensure_checkpoint_source_completed(run: &ModelRun, run_id: &str) -> AppResult<()> {
    if run.status() != RunStatus::Completed {
        return Err(AppError::validation(
            "checkpoint_source_not_completed",
            format!("Model Run `{run_id}` is not completed and cannot be checkpoint evidence"),
        )
        .with_details(json!({
            "runId": run_id,
            "status": run_status_machine_code(run.status()),
        })));
    }
    Ok(())
}

fn run_status_machine_code(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Connecting => "connecting",
        RunStatus::Streaming => "streaming",
        RunStatus::Completed => "completed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
    }
}

fn validate_checkpoint_selection(
    data: &crate::ports::WorkspaceContextData,
    branch_id: &str,
    expected_branch_version: u64,
    anchor_run_id: &str,
    kind: domain::ContextCheckpointKind,
    source_run_ids: &[String],
    first_kept_run_id: Option<&str>,
) -> AppResult<BranchPointer> {
    let branch = data
        .branch_pointers
        .iter()
        .find(|branch| branch.id == branch_id)
        .cloned()
        .ok_or_else(|| {
            AppError::validation(
                "branch_not_found",
                format!("Branch `{branch_id}` is not in this workspace"),
            )
        })?;
    if branch.version != expected_branch_version {
        return Err(repository_port_error(
            RepositoryPortError::VersionConflict {
                resource: "branch_pointer",
                id: branch.id,
                expected: expected_branch_version,
                actual: branch.version,
            },
        ));
    }
    if kind == domain::ContextCheckpointKind::BranchSummary
        && let Some(first_kept_run_id) = first_kept_run_id
    {
        return Err(AppError::validation(
            "unexpected_checkpoint_boundary",
            "A branch-summary checkpoint cannot carry a compaction kept boundary",
        )
        .with_details(json!({
            "kind": "branch-summary",
            "firstKeptRunId": first_kept_run_id,
        })));
    }
    let runs_by_id = data
        .runs
        .iter()
        .cloned()
        .map(|run| (run.id.clone(), run))
        .collect::<HashMap<_, _>>();
    let branch_path = exact_lineage_ids(Some(&branch.head_run_id), &data.turns, &runs_by_id);
    if !branch_path.contains(anchor_run_id) {
        return Err(AppError::validation(
            "run_outside_branch",
            "The checkpoint anchor is not on the requested branch",
        ));
    }
    let branch_summary_anchor_path = (kind == domain::ContextCheckpointKind::BranchSummary)
        .then(|| exact_lineage_ids(Some(anchor_run_id), &data.turns, &runs_by_id));
    for run_id in source_run_ids {
        validated_checkpoint_source(&runs_by_id, run_id)?;
        if !branch_path.contains(run_id) {
            return Err(AppError::validation(
                "checkpoint_source_outside_branch",
                format!("Model Run `{run_id}` is not on the requested branch"),
            ));
        }
        if branch_summary_anchor_path
            .as_ref()
            .is_some_and(|anchor_path| !anchor_path.contains(run_id))
        {
            return Err(AppError::validation(
                "invalid_checkpoint_source_range",
                "Branch-summary sources must be on the root-to-anchor Run path",
            ));
        }
    }
    if kind == domain::ContextCheckpointKind::Compaction {
        let mut reverse = Vec::new();
        let turn_by_id = data
            .turns
            .iter()
            .map(|turn| (turn.id.as_str(), turn))
            .collect::<HashMap<_, _>>();
        let mut cursor = Some(anchor_run_id);
        while let Some(run_id) = cursor {
            let run = runs_by_id.get(run_id).ok_or_else(|| {
                AppError::validation(
                    "checkpoint_anchor_not_found",
                    format!("Model Run `{run_id}` is not in this workspace"),
                )
            })?;
            reverse.push(run.id.clone());
            cursor = turn_by_id
                .get(run.turn_id.as_str())
                .and_then(|turn| turn.parent_run_id.as_deref());
        }
        reverse.reverse();
        let kept = first_kept_run_id.ok_or_else(|| {
            AppError::validation(
                "missing_checkpoint_boundary",
                "A compaction checkpoint must identify the first kept Run",
            )
        })?;
        let boundary = reverse
            .iter()
            .position(|run_id| run_id == kept)
            .ok_or_else(|| {
                AppError::validation(
                    "checkpoint_boundary_outside_path",
                    "The first kept Run is not on the checkpoint anchor path",
                )
            })?;
        if reverse[..boundary] != source_run_ids[..] {
            return Err(AppError::validation(
                "invalid_checkpoint_source_range",
                "Compaction sources must be the exact root-to-boundary Run prefix",
            ));
        }
    } else {
        for pair in source_run_ids.windows(2) {
            let next = runs_by_id
                .get(&pair[1])
                .and_then(|run| data.graph.turn(&run.turn_id))
                .and_then(|turn| turn.parent_run_id.as_deref());
            if next != Some(pair[0].as_str()) {
                return Err(AppError::validation(
                    "invalid_checkpoint_source_range",
                    "Branch-summary sources must form one ordered parent-to-child Run path",
                ));
            }
        }
    }
    Ok(branch)
}

fn context_diff_item(item: &domain::RunContextItem) -> ContextDiffItemView {
    ContextDiffItemView {
        id: context_item_identity(item),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: domain_message_role_view(item.role),
        source: context_source_name(item.source_kind).into(),
        preview: item.content.chars().take(180).collect(),
    }
}

fn context_receipt_checkpoint_provenance(
    manifest: &domain::ContextManifest,
    checkpoints: &[domain::ContextCheckpoint],
) -> Vec<ContextCheckpointProvenanceView> {
    manifest
        .checkpoint_provenance
        .iter()
        .chain(manifest.branch_summary_provenance.iter())
        .map(|provenance| {
            let provider = checkpoints
                .iter()
                .find(|checkpoint| checkpoint.id == provenance.checkpoint_id)
                .and_then(|checkpoint| checkpoint.provider.as_ref())
                .map(context_checkpoint_provider_snapshot_view);
            ContextCheckpointProvenanceView {
                checkpoint_id: provenance.checkpoint_id.clone(),
                maintenance_run_id: provenance.maintenance_run_id.clone(),
                kind: match provenance.kind {
                    domain::ContextCheckpointKind::Compaction => {
                        ContextCheckpointKindView::Compaction
                    }
                    domain::ContextCheckpointKind::BranchSummary => {
                        ContextCheckpointKindView::BranchSummary
                    }
                },
                branch_id: provenance.branch_pointer_id.clone(),
                branch_version: provenance.branch_revision,
                anchor_run_id: provenance.anchor_run_id.clone(),
                first_kept_run_id: provenance.first_kept_run_id.clone(),
                summary_content_block_id: provenance.summary_content_block_id.clone(),
                source_run_ids: provenance.source_run_ids.clone(),
                source_hash: provenance.source_hash.clone(),
                provider,
            }
        })
        .collect()
}

fn context_checkpoint_provider_snapshot_view(
    provider: &domain::ProviderSnapshot,
) -> ContextCheckpointProviderSnapshotView {
    ContextCheckpointProviderSnapshotView {
        profile_id: provider.profile_id.clone(),
        provider_id: provider.provider_id.clone(),
        template_revision: provider.template_revision,
        provider_name: provider.provider_name.clone(),
        dialect: match provider.dialect {
            domain::ProviderDialect::OpenAiCompatible => ProviderDialectView::OpenaiCompatible,
            domain::ProviderDialect::Ollama => ProviderDialectView::Ollama,
            domain::ProviderDialect::Anthropic => ProviderDialectView::Anthropic,
            domain::ProviderDialect::GoogleGenerativeAi => ProviderDialectView::GoogleGenerativeAi,
        },
        stream_protocol: provider.stream_protocol.map(|protocol| match protocol {
            domain::StreamProtocol::OpenAiSse => ProviderStreamProtocolView::OpenAiSse,
            domain::StreamProtocol::OllamaNdjson => ProviderStreamProtocolView::OllamaNdjson,
            domain::StreamProtocol::AnthropicSse => ProviderStreamProtocolView::AnthropicSse,
            domain::StreamProtocol::GoogleSse => ProviderStreamProtocolView::GoogleSse,
        }),
        auth_placement: provider.auth_placement.map(|placement| match placement {
            domain::AuthPlacement::None => ProviderAuthPlacementView::None,
            domain::AuthPlacement::BearerHeader => ProviderAuthPlacementView::BearerHeader,
            domain::AuthPlacement::ApiKeyHeader => ProviderAuthPlacementView::ApiKeyHeader,
            domain::AuthPlacement::QueryParam => ProviderAuthPlacementView::QueryParam,
        }),
        auth_header_name: provider.auth_header_name.clone(),
        additional_headers: provider.additional_headers.clone(),
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        parameters: provider.parameters.clone(),
    }
}

fn context_item_identity(item: &domain::RunContextItem) -> String {
    format!(
        "{}:{}:{}",
        item.source_ref.stable_id(),
        item.content_block_id,
        item.content_hash,
    )
}

fn eligible_checkpoint_ids_for_branch(
    data: &crate::ports::WorkspaceContextData,
    branch_pointer_id: Option<&str>,
) -> Vec<String> {
    let Some(branch_pointer_id) = branch_pointer_id else {
        return Vec::new();
    };
    data.branch_checkpoint_inheritance
        .iter()
        .filter(|visibility| visibility.branch_pointer_id == branch_pointer_id)
        .map(|visibility| visibility.checkpoint_id.clone())
        .collect()
}

fn exact_lineage_ids(
    current_run_id: Option<&str>,
    turns: &[Turn],
    runs: &HashMap<String, ModelRun>,
) -> BTreeSet<String> {
    let turn_by_id = turns
        .iter()
        .map(|turn| (turn.id.as_str(), turn))
        .collect::<HashMap<_, _>>();
    let mut lineage = BTreeSet::new();
    let mut cursor = current_run_id;
    while let Some(run_id) = cursor {
        if !lineage.insert(run_id.to_owned()) {
            break;
        }
        cursor = runs
            .get(run_id)
            .and_then(|run| turn_by_id.get(run.turn_id.as_str()))
            .and_then(|turn| turn.parent_run_id.as_deref());
    }
    lineage
}

fn route_edge_is_on_lineage(
    source_run_id: &str,
    target_runs: &[ModelRun],
    lineage: &BTreeSet<String>,
) -> bool {
    lineage.contains(source_run_id)
        && target_runs
            .iter()
            .any(|run| lineage.contains(run.id.as_str()))
}

fn context_tree_projection(
    workspace_id: &str,
    graph: &domain::ConversationGraph,
    cursor: ContextCursorView,
    draft_version: u64,
    branch_pointers: &[BranchPointer],
    eligible_checkpoint_ids: &BTreeSet<String>,
    checkpoints: Vec<ContextCheckpointView>,
) -> ContextTreeProjection {
    let turns = graph.turns().cloned().collect::<Vec<_>>();
    let runs_by_id = graph
        .runs()
        .cloned()
        .map(|run| (run.id.clone(), run))
        .collect::<HashMap<_, _>>();
    let active_path = exact_lineage_ids(cursor.active_run_id.as_deref(), &turns, &runs_by_id);
    let branch_paths = branch_pointers
        .iter()
        .map(|branch| {
            (
                branch.id.as_str(),
                exact_lineage_ids(Some(&branch.head_run_id), &turns, &runs_by_id),
            )
        })
        .collect::<Vec<_>>();
    let checkpoint_ids_by_anchor = checkpoints
        .iter()
        .filter(|checkpoint| {
            checkpoint.branch_id.is_none() || eligible_checkpoint_ids.contains(&checkpoint.id)
        })
        .filter_map(|checkpoint| {
            checkpoint
                .anchor_run_id
                .as_deref()
                .map(|run_id| (run_id, checkpoint.id.as_str()))
        })
        .fold(
            HashMap::<&str, Vec<String>>::new(),
            |mut by_anchor, (run_id, checkpoint_id)| {
                by_anchor
                    .entry(run_id)
                    .or_default()
                    .push(checkpoint_id.to_owned());
                by_anchor
            },
        );
    let mut runs = graph.runs().collect::<Vec<_>>();
    runs.sort_by_key(|run| (run.created_at, run.id.as_str()));
    let nodes = runs
        .iter()
        .filter_map(|run| {
            let turn = graph.turn(&run.turn_id)?;
            Some(ContextTreeRunNodeView {
                run_id: run.id.clone(),
                turn_id: turn.id.clone(),
                parent_run_id: turn.parent_run_id.clone(),
                prompt: turn.prompt_markdown.clone(),
                title: turn
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| turn.prompt_markdown.chars().take(80).collect()),
                output_preview: run.output_markdown().chars().take(180).collect(),
                model: run.model.clone(),
                status: domain_run_status_view(run.status()),
                created_at: timestamp_view(run.created_at),
                can_continue: domain::run_is_usable_as_parent(run),
                is_active: cursor.active_run_id.as_deref() == Some(run.id.as_str()),
                is_on_active_path: active_path.contains(&run.id),
                branch_ids: branch_paths
                    .iter()
                    .filter(|(_, path)| path.contains(&run.id))
                    .map(|(branch_id, _)| (*branch_id).to_owned())
                    .collect(),
                checkpoint_ids: checkpoint_ids_by_anchor
                    .get(run.id.as_str())
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let edges = nodes
        .iter()
        .map(|node| ContextTreeEdgeView {
            id: format!(
                "{}->{}",
                node.parent_run_id.as_deref().unwrap_or("root"),
                node.run_id
            ),
            source_run_id: node.parent_run_id.clone(),
            target_run_id: node.run_id.clone(),
            is_on_active_path: active_path.contains(&node.run_id)
                && node
                    .parent_run_id
                    .as_ref()
                    .is_none_or(|parent| active_path.contains(parent)),
        })
        .collect();
    let branches = branch_pointers
        .iter()
        .map(|branch| ContextBranchView {
            id: branch.id.clone(),
            name: branch.name.clone(),
            head_run_id: branch.head_run_id.clone(),
            version: branch.version,
            is_active: cursor.branch_id.as_deref() == Some(branch.id.as_str()),
        })
        .collect();
    ContextTreeProjection {
        workspace_id: workspace_id.into(),
        root_id: format!("workspace-root:{workspace_id}"),
        draft_version,
        cursor,
        nodes,
        edges,
        branches,
        checkpoints,
    }
}

fn effective_route_run_id(
    requested_run_id: Option<&str>,
    runs: &HashMap<String, ModelRun>,
    branch_pointers: &[BranchPointer],
) -> Option<String> {
    requested_run_id
        .filter(|run_id| runs.contains_key(*run_id))
        .map(str::to_owned)
        .or_else(|| {
            branch_pointers
                .iter()
                .filter(|pointer| runs.contains_key(&pointer.head_run_id))
                .max_by_key(|pointer| (pointer.updated_at, pointer.version))
                .map(|pointer| pointer.head_run_id.clone())
        })
}

fn decision_problem<'a>(
    workspace_goal: &'a str,
    mut marked_turn_prompts: impl Iterator<Item = &'a str>,
    fallback_prompt: Option<&'a str>,
) -> &'a str {
    let goal = workspace_goal.trim();
    if !goal.is_empty() && goal != DEFAULT_SYSTEM_PROMPT && goal != "尚未设置工作区目标" {
        return workspace_goal;
    }
    marked_turn_prompts
        .next()
        .or(fallback_prompt)
        .unwrap_or(workspace_goal)
}

fn decision_packet_checkpoint_provenance(provenance: &[ContextCheckpointProvenanceView]) -> String {
    if provenance.is_empty() {
        return "_Checkpoint provenance:_ none.\n\n".into();
    }
    let mut markdown = String::from("**Checkpoint provenance**\n\n");
    for checkpoint in provenance {
        let kind = match checkpoint.kind {
            ContextCheckpointKindView::Compaction => "compaction",
            ContextCheckpointKindView::BranchSummary => "branch-summary",
        };
        let branch = checkpoint
            .branch_id
            .as_deref()
            .map(|id| {
                format!(
                    "`{id}` @ revision {}",
                    checkpoint
                        .branch_version
                        .map_or_else(|| "unknown".into(), |version| version.to_string())
                )
            })
            .unwrap_or_else(|| "legacy / no branch evidence".into());
        let first_kept = checkpoint.first_kept_run_id.as_deref().unwrap_or("none");
        let source_runs = checkpoint
            .source_run_ids
            .iter()
            .map(|run_id| format!("`{run_id}`"))
            .collect::<Vec<_>>()
            .join(", ");
        markdown.push_str(&format!(
            "- **{kind}** checkpoint `{}` (maintenance `{}`)\n  - Anchor: `{}`; first kept Run: `{}`; branch: {branch}\n  - Sources: {source_runs}\n  - Source hash: `{}`; summary Content Block: `{}`\n",
            checkpoint.checkpoint_id,
            checkpoint.maintenance_run_id,
            checkpoint.anchor_run_id,
            first_kept,
            checkpoint.source_hash,
            checkpoint.summary_content_block_id,
        ));
        if let Some(provider) = &checkpoint.provider {
            let snapshot =
                serde_json::to_string(provider).unwrap_or_else(|_| "\"unavailable\"".into());
            markdown.push_str(&format!("  - Provider snapshot: `{snapshot}`\n"));
        } else {
            markdown.push_str("  - Provider snapshot: `unavailable`\n");
        }
    }
    markdown.push('\n');
    markdown
}

fn decision_status_domain(status: DecisionStatusView) -> DecisionStatus {
    match status {
        DecisionStatusView::Accepted => DecisionStatus::Adopted,
        DecisionStatusView::Rejected => DecisionStatus::Rejected,
        DecisionStatusView::ToVerify => DecisionStatus::NeedsValidation,
    }
}

fn provider_dialect(value: domain::ProviderDialect) -> ProviderDialect {
    match value {
        domain::ProviderDialect::OpenAiCompatible => ProviderDialect::OpenAiChatCompletions,
        domain::ProviderDialect::Ollama => ProviderDialect::OllamaChat,
        domain::ProviderDialect::Anthropic => ProviderDialect::AnthropicMessages,
        domain::ProviderDialect::GoogleGenerativeAi => ProviderDialect::GoogleGenerativeAi,
    }
}

fn provider_target(snapshot: &domain::ProviderSnapshot) -> AppResult<ProviderTarget> {
    snapshot.require_resolved_metadata().map_err(domain_error)?;
    let dialect = provider_dialect(snapshot.dialect);
    let stream_protocol = snapshot.stream_protocol.ok_or_else(|| {
        AppError::internal(
            "unresolved_provider_snapshot",
            "Provider snapshot stream protocol is unresolved",
        )
    })?;
    let template_dialect = match stream_protocol {
        domain::StreamProtocol::OpenAiSse => ProviderDialect::OpenAiChatCompletions,
        domain::StreamProtocol::OllamaNdjson => ProviderDialect::OllamaChat,
        domain::StreamProtocol::AnthropicSse => ProviderDialect::AnthropicMessages,
        domain::StreamProtocol::GoogleSse => ProviderDialect::GoogleGenerativeAi,
    };
    if dialect != template_dialect {
        return Err(AppError::internal(
            "invalid_provider_profile",
            "Provider Profile dialect does not match its template",
        ));
    }
    let auth_placement = snapshot.auth_placement.ok_or_else(|| {
        AppError::internal(
            "unresolved_provider_snapshot",
            "Provider snapshot authentication placement is unresolved",
        )
    })?;
    let credential_placement = match auth_placement {
        domain::AuthPlacement::None => CredentialPlacement::None,
        domain::AuthPlacement::BearerHeader => CredentialPlacement::BearerHeader,
        domain::AuthPlacement::ApiKeyHeader => {
            let header_name = snapshot.auth_header_name.as_deref().ok_or_else(|| {
                AppError::internal(
                    "invalid_provider_template",
                    "API key header placement requires a header name",
                )
            })?;
            CredentialPlacement::Header(header_name.into())
        }
        domain::AuthPlacement::QueryParam => {
            return Err(AppError::validation(
                "provider_auth_unavailable",
                "Query-string Provider credentials are not supported",
            ));
        }
    };
    Ok(ProviderTarget {
        dialect,
        base_url: snapshot.base_url.clone(),
        credential_placement,
        additional_headers: snapshot.additional_headers.clone(),
    })
}

#[derive(Debug)]
enum ProviderModelSource {
    SavedProfile(String),
    Draft(ProviderModelDraftInput),
}

fn provider_model_source(input: ListProviderModelsInput) -> AppResult<ProviderModelSource> {
    match (input.provider_profile_id, input.draft) {
        (Some(provider_profile_id), None) if !provider_profile_id.trim().is_empty() => {
            Ok(ProviderModelSource::SavedProfile(provider_profile_id))
        }
        (None, Some(draft))
            if !draft.provider_id.trim().is_empty() && !draft.base_url.trim().is_empty() =>
        {
            Ok(ProviderModelSource::Draft(draft))
        }
        (Some(_), None) => Err(AppError::validation(
            "missing_required_field",
            "providerProfileId must not be empty",
        )),
        (None, Some(_)) => Err(AppError::validation(
            "missing_required_field",
            "Draft providerId and baseUrl must not be empty",
        )),
        _ => Err(AppError::validation(
            "invalid_provider_model_source",
            "Choose exactly one Provider Profile or draft Provider target",
        )),
    }
}

/// Resolves model discovery exclusively from the Rust-owned Provider Template.
/// This intentionally does not call `runnable_template`: Anthropic and Google
/// can expose model metadata before their streaming dialect is enabled.
fn provider_model_query(provider_id: &str, base_url: &str) -> AppResult<ProviderModelQuery> {
    let template = domain::provider_template(provider_id).ok_or_else(|| {
        AppError::validation(
            "unknown_provider_template",
            format!("Unknown Provider Template `{provider_id}`"),
        )
    })?;
    let catalog = match template.model_catalog {
        domain::ProviderModelCatalogStrategy::RemoteAnthropic => {
            ProviderModelCatalogKind::Anthropic
        }
        domain::ProviderModelCatalogStrategy::RemoteOpenAi => ProviderModelCatalogKind::OpenAi,
        domain::ProviderModelCatalogStrategy::RemoteOllama => ProviderModelCatalogKind::Ollama,
        domain::ProviderModelCatalogStrategy::RemoteGoogle => ProviderModelCatalogKind::Google,
        domain::ProviderModelCatalogStrategy::Unsupported => {
            return Err(AppError::validation(
                "provider_model_discovery_unsupported",
                format!(
                    "{} automatic model discovery is not implemented; enter a model ID manually",
                    template.display_name
                ),
            ));
        }
    };
    let credential_placement = match template.protocol.auth_placement {
        domain::AuthPlacement::None => CredentialPlacement::None,
        domain::AuthPlacement::BearerHeader => CredentialPlacement::BearerHeader,
        domain::AuthPlacement::ApiKeyHeader => {
            let header_name = template.protocol.auth_header_name.ok_or_else(|| {
                AppError::internal(
                    "invalid_provider_template",
                    "API key header placement requires a header name",
                )
            })?;
            CredentialPlacement::Header(header_name.into())
        }
        domain::AuthPlacement::QueryParam => {
            return Err(AppError::validation(
                "provider_auth_unavailable",
                "Query-string Provider credentials are not supported",
            ));
        }
    };
    let dialect = match catalog {
        ProviderModelCatalogKind::Ollama => ProviderDialect::OllamaChat,
        ProviderModelCatalogKind::OpenAi => ProviderDialect::OpenAiChatCompletions,
        ProviderModelCatalogKind::Anthropic => ProviderDialect::AnthropicMessages,
        ProviderModelCatalogKind::Google => ProviderDialect::GoogleGenerativeAi,
    };
    Ok(ProviderModelQuery {
        target: ProviderTarget {
            dialect,
            base_url: base_url.into(),
            credential_placement,
            additional_headers: template
                .protocol
                .additional_headers
                .iter()
                .map(|header| (header.name.into(), header.value.into()))
                .collect(),
        },
        catalog,
    })
}

fn model_info_view(model: DiscoveredModel) -> ModelInfoView {
    ModelInfoView {
        id: model.id,
        display_name: model.display_name,
        context_window: model.context_window,
        supports_tools: model.supports_tools,
    }
}

fn runnable_template(
    provider_id: &str,
) -> AppResult<(&'static domain::ProviderTemplate, domain::ProviderDialect)> {
    let template = domain::provider_template(provider_id).ok_or_else(|| {
        AppError::validation(
            "unknown_provider_template",
            format!("Unknown Provider Template `{provider_id}`"),
        )
    })?;
    if !template.runtime_available {
        return Err(AppError::validation(
            "provider_protocol_unavailable",
            format!(
                "{} streaming support is not available yet",
                template.display_name
            ),
        ));
    }
    let dialect = match template.protocol.stream_protocol {
        domain::StreamProtocol::OpenAiSse => domain::ProviderDialect::OpenAiCompatible,
        domain::StreamProtocol::OllamaNdjson => domain::ProviderDialect::Ollama,
        domain::StreamProtocol::AnthropicSse => domain::ProviderDialect::Anthropic,
        domain::StreamProtocol::GoogleSse => domain::ProviderDialect::GoogleGenerativeAi,
    };
    Ok((template, dialect))
}

fn provider_message_role(role: MessageRole) -> ProviderMessageRole {
    match role {
        MessageRole::System => ProviderMessageRole::System,
        MessageRole::User => ProviderMessageRole::User,
        MessageRole::Assistant => ProviderMessageRole::Assistant,
    }
}

fn domain_message_role_view(role: MessageRole) -> MessageRoleView {
    match role {
        MessageRole::System => MessageRoleView::System,
        MessageRole::User => MessageRoleView::User,
        MessageRole::Assistant => MessageRoleView::Assistant,
    }
}

fn message_role_name(role: MessageRole) -> &'static str {
    match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    }
}

fn context_source_name(source: ContextSourceKind) -> &'static str {
    match source {
        ContextSourceKind::System => "system",
        ContextSourceKind::TurnPrompt => "turn_prompt",
        ContextSourceKind::ModelRun => "model_run",
        ContextSourceKind::Pinned => "pinned",
        ContextSourceKind::CompactionSummary => "compaction_summary",
        ContextSourceKind::BranchSummary => "branch_summary",
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn inclusion_reason_name(reason: InclusionReason) -> &'static str {
    match reason {
        InclusionReason::SystemPolicy => "system_policy",
        InclusionReason::ExactAncestorPath => "exact_ancestor_path",
        InclusionReason::ExplicitPin => "explicit_pin",
        InclusionReason::LatestCompaction => "latest_compaction",
        InclusionReason::BranchSummary => "branch_summary",
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

fn source_label(source: &str) -> String {
    match source {
        "system" => "System prompt",
        "turn_prompt" => "Ancestor prompt",
        "model_run" => "Exact ancestor answer",
        "pinned" => "Pinned context",
        "compaction_summary" => "Compaction summary",
        "branch_summary" => "Branch summary",
        "current_prompt" => "Current prompt",
        _ => "Context",
    }
    .into()
}

fn context_warning(warning: &ContextWarning) -> String {
    match warning {
        ContextWarning::ExcludedPinnedSource(id) => {
            format!("Pinned source {id} is excluded and was not sent")
        }
        ContextWarning::DuplicatePinnedSource(id) => {
            format!("Pinned source {id} appeared more than once")
        }
        ContextWarning::ExceedsLimit {
            estimated_chars,
            max_chars,
        } => format!("Context has {estimated_chars} characters; limit is {max_chars}"),
    }
}

fn domain_run_status_view(status: RunStatus) -> RunStatusView {
    match status {
        RunStatus::Queued => RunStatusView::Pending,
        RunStatus::Connecting => RunStatusView::Connecting,
        RunStatus::Streaming => RunStatusView::Streaming,
        RunStatus::Completed => RunStatusView::Completed,
        RunStatus::Cancelled => RunStatusView::Cancelled,
        RunStatus::Failed => RunStatusView::Failed,
        RunStatus::Interrupted => RunStatusView::Interrupted,
    }
}

fn usage_map(usage: &Usage) -> BTreeMap<String, u64> {
    let mut values = BTreeMap::new();
    if let Some(value) = usage.prompt_tokens {
        values.insert("inputTokens".into(), value);
    }
    if let Some(value) = usage.completion_tokens {
        values.insert("outputTokens".into(), value);
    }
    if let Some(value) = usage.total_tokens {
        values.insert("totalTokens".into(), value);
    }
    values
}

fn canonical_provider_request(
    run_id: String,
    model: String,
    messages: Vec<CanonicalMessage>,
    parameters: &EffectiveProviderParameters,
) -> CanonicalRequest {
    CanonicalRequest {
        run_id,
        model,
        messages,
        temperature: parameters.temperature,
        top_p: parameters.top_p,
        max_output_tokens: parameters.max_output_tokens,
        stop: parameters.stop.clone(),
    }
}

fn checkpoint_summary_messages(
    data: &crate::ports::WorkspaceContextData,
    source_run_ids: &[String],
    summary_prompt: &str,
) -> AppResult<Vec<CanonicalMessage>> {
    let mut messages = vec![CanonicalMessage {
        role: ProviderMessageRole::System,
        content: "Create an auditable, loss-aware Context checkpoint summary. Preserve decisions, constraints, unresolved questions, and exact identifiers. Do not invent facts.".into(),
    }];
    for run_id in source_run_ids {
        let run = data.graph.run(run_id).ok_or_else(|| {
            AppError::validation(
                "checkpoint_source_not_found",
                format!("Model Run `{run_id}` is not in this workspace"),
            )
        })?;
        let turn = data.graph.turn(&run.turn_id).ok_or_else(|| {
            AppError::internal(
                "checkpoint_turn_missing",
                "A validated Context graph is missing a Run's Turn",
            )
        })?;
        messages.push(CanonicalMessage {
            role: ProviderMessageRole::User,
            content: format!("[Turn {}]\n{}", turn.id, turn.prompt_markdown),
        });
        messages.push(CanonicalMessage {
            role: ProviderMessageRole::Assistant,
            content: format!("[Model Run {}]\n{}", run.id, run.output_markdown()),
        });
    }
    messages.push(CanonicalMessage {
        role: ProviderMessageRole::User,
        content: summary_prompt.into(),
    });
    Ok(messages)
}

async fn collect_provider_summary(
    provider: Arc<dyn ProviderGateway>,
    invocation: ProviderInvocation,
    cancellation: CancellationToken,
) -> AppResult<String> {
    let (sender, mut receiver) = mpsc::channel(128);
    let provider_cancellation = cancellation.clone();
    let provider_task = tokio::spawn(async move {
        provider
            .stream(invocation, provider_cancellation, sender)
            .await
    });
    let mut summary = String::new();
    let mut terminal: Option<AppResult<()>> = None;
    loop {
        let event = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                break;
            }
            event = receiver.recv() => event,
        };
        let Some(event) = event else {
            break;
        };
        match event {
            RunEvent::TextDelta { text } => summary.push_str(&text),
            RunEvent::RunCompleted { .. } => {
                terminal = Some(Ok(()));
                break;
            }
            RunEvent::RunFailed {
                code,
                message,
                retryable,
                status,
            } => {
                terminal = Some(Err(AppError {
                    code,
                    message,
                    retryable,
                    details: status
                        .map(|status| json!({ "status": status }))
                        .unwrap_or(Value::Null),
                }));
                break;
            }
            RunEvent::RunCancelled => {
                terminal = Some(Err(context_maintenance_cancelled_error()));
                break;
            }
            RunEvent::RunStarted { .. }
            | RunEvent::ReasoningDelta { .. }
            | RunEvent::UsageUpdated { .. }
            | RunEvent::ProviderMetadata { .. } => {}
        }
    }
    drop(receiver);
    if cancellation.is_cancelled() {
        provider_task.abort();
        let _ = provider_task.await;
        return Err(context_maintenance_cancelled_error());
    }
    let provider_result = provider_task.await.map_err(|error| {
        AppError::internal(
            "context_maintenance_join_failed",
            format!("Context summary task could not be joined: {error}"),
        )
    })?;
    if let Some(Err(error)) = terminal.as_ref() {
        return Err(error.clone());
    }
    provider_result.map_err(provider_port_error)?;
    if terminal.is_none() {
        return Err(AppError::internal(
            "context_maintenance_incomplete",
            "Provider summary stream ended without a terminal event",
        ));
    }
    if summary.trim().is_empty() {
        return Err(AppError::validation(
            "empty_context_summary",
            "Provider returned an empty Context summary",
        ));
    }
    Ok(summary)
}

fn context_maintenance_cancelled_error() -> AppError {
    AppError::validation(
        "context_maintenance_cancelled",
        "Context summary generation was cancelled",
    )
}

fn context_maintenance_provider_error(
    mut error: AppError,
    client_operation_id: &str,
    status: domain::ContextMaintenanceStatus,
) -> AppError {
    let provider_details = std::mem::replace(&mut error.details, Value::Null);
    error.details = json!({
        "clientOperationId": client_operation_id,
        "maintenanceStatus": context_maintenance_status_name(status),
        "providerDetails": provider_details,
    });
    error
}

fn chars_to_tokens(characters: usize) -> u64 {
    u64::try_from(characters.div_ceil(4)).unwrap_or(u64::MAX)
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn now_view() -> String {
    Utc::now().to_rfc3339()
}

fn timestamp_view(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn repository_port_error(error: RepositoryPortError) -> AppError {
    match error {
        RepositoryPortError::NotFound { entity, id } => {
            AppError::validation("not_found", format!("{entity} {id} was not found"))
        }
        RepositoryPortError::Conflict(message) => AppError::validation("conflict", message),
        RepositoryPortError::VersionConflict {
            resource,
            id,
            expected,
            actual,
        } => AppError::validation(
            match resource {
                "context_cursor" => "context_cursor_conflict",
                "context_draft" => "context_draft_conflict",
                "branch_pointer" => "branch_version_conflict",
                _ => "version_conflict",
            },
            format!("{resource} version changed for `{id}`: expected {expected}, actual {actual}"),
        )
        .with_details(json!({
            "resource": resource,
            "id": id,
            "expectedVersion": expected,
            "actualVersion": actual,
        })),
        RepositoryPortError::InvalidData(message) => {
            AppError::validation("invalid_repository_data", message)
        }
        RepositoryPortError::Unavailable(message) => {
            AppError::internal("repository_error", message)
        }
    }
}

fn provider_port_error(error: ProviderError) -> AppError {
    AppError {
        code: error.code().into(),
        message: error.to_string(),
        retryable: error.retryable(),
        details: error
            .status()
            .map(|status| json!({ "status": status }))
            .unwrap_or(Value::Null),
    }
}

fn domain_error(error: domain::DomainError) -> AppError {
    match error {
        domain::DomainError::MissingCheckpointVisibilityEvidence { checkpoint_id } => {
            AppError::validation(
                "checkpoint_visibility_missing",
                "Branch-scoped Context checkpoint visibility could not be proven",
            )
            .with_details(json!({ "checkpointId": checkpoint_id }))
        }
        domain::DomainError::InvalidCheckpointBranchEvidence { checkpoint_id } => {
            AppError::validation(
                "invalid_checkpoint_branch_evidence",
                "Context checkpoint branch evidence is incomplete",
            )
            .with_details(json!({ "checkpointId": checkpoint_id }))
        }
        other => AppError::validation("domain_invariant", other.to_string()),
    }
}

impl ApplicationBackend for DefaultApplicationBackend {
    fn list_workspaces(&self, include_archived: bool) -> AppFuture<'_, Vec<WorkspaceSummary>> {
        Box::pin(async move {
            self.repository
                .list_workspaces(include_archived)
                .await
                .map_err(repository_port_error)
                .map(|records| records.into_iter().map(workspace_view).collect())
        })
    }

    fn create_workspace(&self, input: CreateWorkspaceInput) -> AppFuture<'_, WorkspaceSummary> {
        Box::pin(async move {
            let now = now_millis();
            self.repository
                .save_workspace(Workspace {
                    id: Uuid::new_v4().to_string(),
                    title: input.name,
                    goal: input.goal,
                    system_prompt: if input
                        .system_prompt
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                    {
                        DEFAULT_SYSTEM_PROMPT.into()
                    } else {
                        input.system_prompt.unwrap_or_default()
                    },
                    created_at: now,
                    updated_at: now,
                    archived_at: None,
                })
                .await
                .map(workspace_view)
                .map_err(repository_port_error)
        })
    }

    fn open_workspace(&self, workspace_id: EntityId) -> AppFuture<'_, WorkspaceDetail> {
        Box::pin(async move { self.open_workspace_impl(&workspace_id).await })
    }

    fn update_workspace(&self, input: UpdateWorkspaceInput) -> AppFuture<'_, WorkspaceSummary> {
        Box::pin(async move {
            let current = self
                .repository
                .get_workspace(&input.id)
                .await
                .map_err(repository_port_error)?;
            let archived_at = match input.archived {
                Some(true) => current.archived_at.or_else(|| Some(now_millis())),
                Some(false) => None,
                None => current.archived_at,
            };
            self.repository
                .save_workspace(Workspace {
                    id: current.id,
                    title: input.name.unwrap_or(current.title),
                    goal: input.goal.unwrap_or(current.goal),
                    system_prompt: input.system_prompt.unwrap_or(current.system_prompt),
                    created_at: current.created_at,
                    updated_at: now_millis(),
                    archived_at,
                })
                .await
                .map(workspace_view)
                .map_err(repository_port_error)
        })
    }

    fn inspect_context(&self, input: InspectContextInput) -> AppFuture<'_, ContextPreview> {
        Box::pin(async move { self.inspect_context_impl(input).await })
    }

    fn preview_context_transition(
        &self,
        input: PreviewContextTransitionInput,
    ) -> AppFuture<'_, ContextPreview> {
        Box::pin(async move { self.preview_context_transition_impl(input).await })
    }

    fn get_context_tree(&self, input: GetContextTreeInput) -> AppFuture<'_, ContextTreeProjection> {
        Box::pin(async move { self.get_context_tree_impl(input).await })
    }

    fn set_active_context(&self, input: SetActiveContextInput) -> AppFuture<'_, ContextCursorView> {
        Box::pin(async move { self.set_active_context_impl(input).await })
    }

    fn rename_branch(&self, input: RenameBranchInput) -> AppFuture<'_, ContextBranchView> {
        Box::pin(async move { self.rename_branch_impl(input).await })
    }

    fn update_context_draft(
        &self,
        input: UpdateContextDraftInput,
    ) -> AppFuture<'_, UpdateContextDraftResult> {
        Box::pin(async move { self.update_context_draft_impl(input).await })
    }

    fn create_context_checkpoint(
        &self,
        input: CreateContextCheckpointInput,
    ) -> AppFuture<'_, ContextCheckpointView> {
        Box::pin(async move { self.create_context_checkpoint_impl(input).await })
    }

    fn summarize_and_set_active_context(
        &self,
        input: SummarizeAndSetActiveContextInput,
        credentials: Arc<dyn SessionCredentialLookup>,
    ) -> AppFuture<'_, SummarizeAndSetActiveContextResult> {
        Box::pin(async move {
            self.summarize_and_set_active_context_impl(input, credentials)
                .await
        })
    }

    fn cancel_context_maintenance(&self, client_operation_id: EntityId) -> AppFuture<'_, ()> {
        Box::pin(async move {
            self.cancel_context_maintenance_impl(&client_operation_id)
                .await
        })
    }

    fn create_turn_and_start_run(
        &self,
        input: CreateTurnAndStartRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppFuture<'_, RunHandle> {
        Box::pin(async move { self.start_new_run(input, credentials, events).await })
    }

    fn retry_run(
        &self,
        input: RetryRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppFuture<'_, RunHandle> {
        Box::pin(async move { self.retry_run_impl(input, credentials, events).await })
    }

    fn cancel_run(&self, run_id: EntityId) -> AppFuture<'_, ()> {
        Box::pin(async move {
            let registry = self.run_registry.lock().map_err(|_| {
                AppError::internal("run_registry_lock", "Run registry is unavailable")
            })?;
            let cancellation = registry.get(&run_id).ok_or_else(|| {
                AppError::validation("run_not_active", "The selected Run is not active")
            })?;
            cancellation.cancel();
            Ok(())
        })
    }

    fn get_run_snapshot(&self, run_id: EntityId) -> AppFuture<'_, RunSnapshotView> {
        Box::pin(async move {
            self.repository
                .get_run_snapshot(&run_id)
                .await
                .map_err(repository_port_error)
                .and_then(receipt_view)
        })
    }

    fn update_context_overrides(&self, input: UpdateContextOverridesInput) -> AppFuture<'_, ()> {
        Box::pin(async move { self.update_context_overrides_impl(input).await })
    }

    fn get_route_projection(
        &self,
        input: GetRouteProjectionInput,
    ) -> AppFuture<'_, RouteProjection> {
        Box::pin(async move { self.route_projection_impl(input).await })
    }

    fn update_view_state(&self, input: UpdateViewStateInput) -> AppFuture<'_, ()> {
        Box::pin(async move {
            self.repository
                .save_view_state(ViewState {
                    workspace_id: input.workspace_id,
                    view_key: format!("route-node:{}", input.turn_id),
                    state_json: json!({
                        "x": input.x,
                        "y": input.y,
                        "collapsed": input.collapsed
                    })
                    .to_string(),
                    updated_at: now_millis(),
                })
                .await
                .map_err(repository_port_error)?;
            Ok(())
        })
    }

    fn compare_runs(&self, input: CompareRunsInput) -> AppFuture<'_, CompareRunsResult> {
        Box::pin(async move { self.compare_runs_impl(input).await })
    }

    fn mark_decision(&self, input: MarkDecisionInput) -> AppFuture<'_, DecisionMarkView> {
        Box::pin(async move { self.mark_decision_impl(input).await })
    }

    fn export_decision_packet(
        &self,
        input: ExportDecisionPacketInput,
    ) -> AppFuture<'_, ExportResult> {
        Box::pin(async move { self.export_decision_packet_impl(input).await })
    }

    fn list_provider_profiles(&self) -> AppFuture<'_, Vec<ProviderProfileView>> {
        Box::pin(async move {
            self.repository
                .list_provider_profiles()
                .await
                .map_err(repository_port_error)
                .and_then(|records| records.into_iter().map(provider_profile_view).collect())
        })
    }

    fn list_provider_templates(&self) -> AppFuture<'_, Vec<ProviderTemplateView>> {
        Box::pin(async move {
            Ok(domain::provider_templates()
                .iter()
                .map(provider_template_view)
                .collect())
        })
    }

    fn list_provider_models(
        &self,
        input: ListProviderModelsInput,
        credential: Option<SessionCredential>,
    ) -> AppFuture<'_, Vec<ModelInfoView>> {
        Box::pin(async move { self.list_provider_models_impl(input, credential).await })
    }

    fn save_provider_profile(
        &self,
        input: SaveProviderProfileInput,
    ) -> AppFuture<'_, ProviderProfileView> {
        Box::pin(async move { self.save_provider_profile_impl(input).await })
    }

    fn test_provider_connection(
        &self,
        input: TestProviderConnectionInput,
        credential: Option<SessionCredentialValue>,
    ) -> AppFuture<'_, ProviderConnectionResult> {
        Box::pin(async move { self.test_provider_connection_impl(input, credential).await })
    }
}

impl DefaultApplicationBackend {
    async fn list_provider_models_impl(
        &self,
        input: ListProviderModelsInput,
        credential: Option<SessionCredential>,
    ) -> AppResult<Vec<ModelInfoView>> {
        let (provider_id, base_url) = match provider_model_source(input)? {
            ProviderModelSource::SavedProfile(provider_profile_id) => {
                let profile = self
                    .repository
                    .get_provider_profile(&provider_profile_id)
                    .await
                    .map_err(repository_port_error)?;
                (profile.provider_id, profile.base_url)
            }
            ProviderModelSource::Draft(draft) => (draft.provider_id, draft.base_url),
        };

        let query = provider_model_query(&provider_id, &base_url)?;
        self.model_catalog
            .list_models(query, credential)
            .await
            .map_err(provider_port_error)
            .map(|models| models.into_iter().map(model_info_view).collect())
    }

    async fn open_workspace_impl(&self, workspace_id: &str) -> AppResult<WorkspaceDetail> {
        let workspace = self
            .repository
            .get_workspace(workspace_id)
            .await
            .map_err(repository_port_error)?;
        let data = self
            .repository
            .load_workspace_context_data(workspace_id)
            .await
            .map_err(repository_port_error)?;
        let run_provenance = data
            .run_provider_provenance
            .iter()
            .cloned()
            .map(|provenance| (provenance.run_id.clone(), provenance))
            .collect::<HashMap<_, _>>();
        let runs_by_id = data
            .runs
            .iter()
            .cloned()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();
        let lineage = exact_lineage_ids(
            data.cursor.active_run_id.as_deref(),
            &data.turns,
            &runs_by_id,
        );
        let mut runs_by_turn = data.runs.into_iter().fold(
            BTreeMap::<String, Vec<ModelRun>>::new(),
            |mut by_turn, run| {
                by_turn.entry(run.turn_id.clone()).or_default().push(run);
                by_turn
            },
        );
        let mut selected_run_ids = BTreeMap::new();
        let mut turn_views = Vec::with_capacity(data.turns.len());
        for turn in data.turns {
            let run_records = runs_by_turn.remove(&turn.id).unwrap_or_default();
            let views = run_records
                .iter()
                .map(|run| run_view(run, run_provenance.get(&run.id)))
                .collect::<AppResult<Vec<_>>>()?;
            if let Some(selected) = run_records
                .iter()
                .find(|run| lineage.contains(&run.id))
                .or_else(|| {
                    run_records
                        .iter()
                        .rev()
                        .find(|run| run.status() == RunStatus::Completed)
                })
                .or_else(|| run_records.last())
            {
                selected_run_ids.insert(turn.id.clone(), selected.id.clone());
            }
            turn_views.push(TurnView {
                id: turn.id,
                workspace_id: turn.workspace_id,
                parent_run_id: turn.parent_run_id,
                prompt: turn.prompt_markdown,
                title: turn.title,
                created_at: timestamp_view(turn.created_at),
                runs: views,
            });
        }
        let adjacent_branches = data
            .branch_pointers
            .iter()
            .cloned()
            .map(|pointer| AdjacentBranchView {
                run_id: pointer.head_run_id,
                label: pointer.name,
            })
            .collect();
        let decision_marks = self
            .repository
            .list_decision_marks(workspace_id)
            .await
            .map_err(repository_port_error)?
            .into_iter()
            .map(decision_view)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(WorkspaceDetail {
            workspace: workspace_view(workspace),
            turns: turn_views,
            selected_run_ids,
            adjacent_branches,
            decision_marks,
            context_cursor: context_cursor_view(data.cursor),
        })
    }

    async fn start_new_run(
        &self,
        input: CreateTurnAndStartRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppResult<RunHandle> {
        let turn_id = Uuid::new_v4().to_string();
        self.prepare_and_launch(
            input.workspace_id,
            Some(turn_id),
            None,
            false,
            input.parent_run_id.clone(),
            input.parent_run_id,
            input.prompt,
            input.provider_profile_id,
            input.preview_hash,
            input.branch_id,
            input.expected_cursor_version,
            input.expected_branch_version,
            input.expected_draft_version,
            credentials,
            events,
        )
        .await
    }

    async fn retry_run_impl(
        &self,
        input: RetryRunInput,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppResult<RunHandle> {
        let original = self
            .repository
            .get_run(&input.run_id)
            .await
            .map_err(repository_port_error)?;
        let turn = self
            .repository
            .get_turn(exact_retry_turn_id(&original))
            .await
            .map_err(repository_port_error)?;
        self.prepare_and_launch(
            turn.workspace_id,
            None,
            Some(turn.id),
            true,
            Some(original.id.clone()),
            turn.parent_run_id,
            turn.prompt_markdown,
            input.provider_profile_id,
            input.preview_hash,
            input.branch_id,
            input.expected_cursor_version,
            input.expected_branch_version,
            input.expected_draft_version,
            credentials,
            events,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_and_launch(
        &self,
        workspace_id: String,
        new_turn_id: Option<String>,
        retry_turn_id: Option<String>,
        is_retry: bool,
        branch_source_run_id: Option<String>,
        parent_run_id: Option<String>,
        prompt: String,
        provider_profile_id: String,
        preview_hash: String,
        requested_branch_id: Option<String>,
        expected_cursor_version: u64,
        expected_branch_version: Option<u64>,
        expected_draft_version: u64,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppResult<RunHandle> {
        let workspace = self
            .repository
            .get_workspace(&workspace_id)
            .await
            .map_err(repository_port_error)?;
        let profile = self
            .repository
            .get_provider_profile(&provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let resolved_provider = domain_provider_snapshot(&profile)?;
        let target = provider_target(&resolved_provider)?;
        let data = self
            .repository
            .load_workspace_context_data(&workspace_id)
            .await
            .map_err(repository_port_error)?;
        if data.cursor.version != expected_cursor_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_cursor",
                    id: workspace_id,
                    expected: expected_cursor_version,
                    actual: data.cursor.version,
                },
            ));
        }
        if data.draft.version != expected_draft_version {
            return Err(repository_port_error(
                RepositoryPortError::VersionConflict {
                    resource: "context_draft",
                    id: workspace_id,
                    expected: expected_draft_version,
                    actual: data.draft.version,
                },
            ));
        }
        if !data.draft.items.is_empty() && data.draft.parent_run_id != parent_run_id {
            return Err(AppError::validation(
                "context_draft_parent_changed",
                "The persisted Context Draft belongs to a different active path",
            ));
        }
        let runs_by_id = data
            .runs
            .iter()
            .cloned()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();
        let selected_branch_id = requested_branch_id.clone();
        let selected_branch = selected_branch_id
            .as_deref()
            .map(|branch_id| {
                data.branch_pointers
                    .iter()
                    .find(|branch| branch.id == branch_id)
                    .cloned()
                    .ok_or_else(|| {
                        AppError::validation(
                            "branch_not_found",
                            format!("Branch `{branch_id}` is not in this workspace"),
                        )
                    })
            })
            .transpose()?;
        if let (Some(source_run_id), Some(branch)) =
            (branch_source_run_id.as_deref(), selected_branch.as_ref())
        {
            let branch_path =
                exact_lineage_ids(Some(&branch.head_run_id), &data.turns, &runs_by_id);
            if !branch_path.contains(source_run_id) {
                return Err(AppError::validation(
                    "run_outside_branch",
                    "The selected source Model Run is not on the requested branch",
                ));
            }
        }
        match (&selected_branch, expected_branch_version) {
            (Some(branch), Some(expected)) if branch.version != expected => {
                return Err(repository_port_error(
                    RepositoryPortError::VersionConflict {
                        resource: "branch_pointer",
                        id: branch.id.clone(),
                        expected,
                        actual: branch.version,
                    },
                ));
            }
            (Some(_), None) => {
                return Err(AppError::validation(
                    "missing_branch_version",
                    "Starting from a branch requires its expected version",
                ));
            }
            (None, Some(_)) => {
                return Err(AppError::validation(
                    "unexpected_branch_version",
                    "A root Context has no branch version",
                ));
            }
            _ => {}
        }
        let overrides = if data.draft.consumed_by_run_id.is_none()
            && data.draft.parent_run_id == parent_run_id
        {
            context_overrides_from_draft(&data.draft)?
        } else {
            domain::ContextOverrides::default()
        };
        let eligible_checkpoint_ids = eligible_checkpoint_ids_for_branch(
            &data,
            selected_branch.as_ref().map(|branch| branch.id.as_str()),
        );
        let compiled = self
            .compiler
            .compile(
                &data.graph,
                domain::ContextCompileInput::new(ContextCompileRequest {
                    workspace_id: workspace_id.clone(),
                    system_prompt: workspace.system_prompt,
                    parent_run_id: parent_run_id.clone(),
                    current_prompt: prompt.clone(),
                    overrides,
                    provider: Some(resolved_provider.clone()),
                })
                .with_checkpoints(data.checkpoints)
                .with_eligible_checkpoint_ids(eligible_checkpoint_ids),
                &preview_hash,
            )
            .map_err(domain_error)?;

        let now = now_millis();
        let run_id = Uuid::new_v4().to_string();
        let turn_id = match &new_turn_id {
            Some(turn_id) => turn_id.clone(),
            None => retry_turn_id.ok_or_else(|| {
                AppError::internal(
                    "retry_turn_missing",
                    "A retry must identify its exact original Turn",
                )
            })?,
        };
        let snapshot_id = Uuid::new_v4().to_string();
        let parameters = effective_provider_parameters(&profile)?;
        let content_blocks =
            content_blocks_for_manifest(&workspace_id, &compiled.manifest.items, now);
        let turn = new_turn_id.map(|id| Turn {
            id,
            workspace_id: workspace_id.clone(),
            parent_run_id: parent_run_id.clone(),
            prompt_markdown: prompt.clone(),
            title: Some(prompt.chars().take(80).collect()),
            created_at: now,
        });
        let run = ModelRun::queued(RunDraft {
            id: run_id.clone(),
            turn_id: turn_id.clone(),
            provider_profile_id: Some(profile.id.clone()),
            model: profile.model.clone(),
            created_at: now,
        });
        let snapshot = domain::ContextSnapshot {
            id: snapshot_id,
            run_id: run_id.clone(),
            manifest: compiled.manifest.clone(),
            provider: resolved_provider.clone(),
            created_at: now,
        };
        let branch_pointer = match selected_branch.as_ref() {
            Some(branch)
                if !is_retry && parent_run_id.as_deref() == Some(branch.head_run_id.as_str()) =>
            {
                BranchPointer {
                    id: branch.id.clone(),
                    workspace_id: branch.workspace_id.clone(),
                    name: branch.name.clone(),
                    head_run_id: run_id.clone(),
                    version: branch.version.saturating_add(1),
                    updated_at: now,
                }
            }
            _ => BranchPointer {
                id: Uuid::new_v4().to_string(),
                workspace_id: workspace_id.clone(),
                name: format!(
                    "{} {}",
                    if is_retry { "Retry" } else { "Route" },
                    &turn_id[..8.min(turn_id.len())]
                ),
                head_run_id: run_id.clone(),
                version: 0,
                updated_at: now,
            },
        };
        let result_branch_id = branch_pointer.id.clone();
        let persisted = self
            .repository
            .persist_run_start(PersistRunStart {
                turn,
                run,
                content_blocks,
                snapshot,
                branch_pointer: Some(branch_pointer),
                context_update: Some(crate::ports::RunStartContextUpdate {
                    expected_cursor_version,
                    expected_draft_version,
                    expected_branch_pointer_id: selected_branch
                        .as_ref()
                        .map(|branch| branch.id.clone()),
                    expected_branch_version,
                    result_branch_pointer_id: Some(result_branch_id),
                    updated_at: now,
                }),
            })
            .await
            .map_err(repository_port_error)?;

        let credential = credentials
            .credential_for(&profile.id)?
            .map(|credential| {
                credential
                    .as_str()
                    .map(|value| SessionCredential::new(value.to_owned()))
            })
            .transpose()?;
        let request = canonical_provider_request(
            run_id.clone(),
            profile.model,
            compiled
                .messages
                .into_iter()
                .map(|message| CanonicalMessage {
                    role: provider_message_role(message.role),
                    content: message.content,
                })
                .collect(),
            &parameters,
        );
        let invocation = ProviderInvocation {
            target,
            credential,
            request,
        };
        let cancellation = CancellationToken::new();
        self.run_registry
            .lock()
            .map_err(|_| AppError::internal("run_registry_lock", "Run registry is unavailable"))?
            .insert(run_id.clone(), cancellation.clone());
        if let Err(error) = self
            .repository
            .mark_run_connecting(&run_id, now_millis())
            .await
        {
            if let Ok(mut registry) = self.run_registry.lock() {
                registry.remove(&run_id);
            }
            return Err(repository_port_error(error));
        }
        spawn_run(
            self.run_persistence.clone(),
            self.provider.clone(),
            self.run_registry.clone(),
            invocation,
            cancellation,
            events,
        );
        let active_branch = persisted.branch_pointer.ok_or_else(|| {
            AppError::internal(
                "run_branch_missing",
                "The committed Model Run did not return its branch revision",
            )
        })?;
        Ok(RunHandle {
            turn_id,
            run_id,
            cursor_version: persisted.cursor.version,
            draft_version: persisted.draft_version,
            branch_id: active_branch.id,
            branch_version: active_branch.version,
        })
    }

    async fn route_projection_impl(
        &self,
        input: GetRouteProjectionInput,
    ) -> AppResult<RouteProjection> {
        let data = self
            .repository
            .load_workspace_context_data(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let decisions = self
            .repository
            .list_decision_marks(&input.workspace_id)
            .await
            .map_err(repository_port_error)?
            .into_iter()
            .map(|mark| (mark.run_id.clone(), mark))
            .collect::<HashMap<_, _>>();
        let mut all_runs = HashMap::new();
        let mut runs_by_turn = BTreeMap::<String, Vec<ModelRun>>::new();
        for run in data.runs {
            all_runs.insert(run.id.clone(), run.clone());
            runs_by_turn
                .entry(run.turn_id.clone())
                .or_default()
                .push(run);
        }
        let effective_current_run_id = data
            .cursor
            .active_run_id
            .filter(|run_id| all_runs.contains_key(run_id))
            .or_else(|| effective_route_run_id(None, &all_runs, &data.branch_pointers));
        let lineage =
            exact_lineage_ids(effective_current_run_id.as_deref(), &data.turns, &all_runs);
        let view_states = data
            .view_states
            .into_iter()
            .filter_map(|record| {
                serde_json::from_str::<Value>(&record.state_json)
                    .ok()
                    .map(|state| (record.view_key, state))
            })
            .collect::<HashMap<_, _>>();
        let mut nodes = Vec::with_capacity(data.turns.len());
        let mut edges = Vec::new();
        for (index, turn) in data.turns.iter().enumerate() {
            let runs = runs_by_turn.get(&turn.id).map(Vec::as_slice).unwrap_or(&[]);
            let selected = effective_current_run_id
                .as_deref()
                .and_then(|current| runs.iter().find(|run| run.id == current))
                .or_else(|| runs.iter().find(|run| lineage.contains(&run.id)))
                .or_else(|| {
                    runs.iter()
                        .rev()
                        .find(|run| run.status() == RunStatus::Completed)
                })
                .or_else(|| runs.last());
            let state = view_states.get(&format!("route-node:{}", turn.id));
            let x = state
                .and_then(|value| value.get("x"))
                .and_then(Value::as_f64)
                .unwrap_or((index % 4) as f64 * 320.0);
            let y = state
                .and_then(|value| value.get("y"))
                .and_then(Value::as_f64)
                .unwrap_or((index / 4) as f64 * 220.0);
            let is_current = effective_current_run_id
                .as_deref()
                .map(|id| runs.iter().any(|run| run.id == id))
                .unwrap_or(false);
            let is_on_current_lineage = runs.iter().any(|run| lineage.contains(&run.id));
            nodes.push(RouteNodeView {
                id: format!("turn:{}", turn.id),
                turn_id: turn.id.clone(),
                title: turn
                    .title
                    .clone()
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| turn.prompt_markdown.chars().take(48).collect()),
                summary: turn.prompt_markdown.chars().take(140).collect(),
                status: selected
                    .map(|run| domain_run_status_view(run.status()))
                    .unwrap_or(RunStatusView::Pending),
                x,
                y,
                is_current,
                is_on_current_lineage,
                runs: runs
                    .iter()
                    .map(|run| RouteRunPortView {
                        run_id: run.id.clone(),
                        label: if decisions.contains_key(&run.id) {
                            "Decision".into()
                        } else {
                            format!("Run {}", &run.id[..8.min(run.id.len())])
                        },
                        model: run.model.clone(),
                        status: domain_run_status_view(run.status()),
                        can_branch: run.status() == RunStatus::Completed
                            || (!run.output_markdown().is_empty()
                                && matches!(
                                    run.status(),
                                    RunStatus::Cancelled
                                        | RunStatus::Failed
                                        | RunStatus::Interrupted
                                )),
                    })
                    .collect(),
            });
            if let Some(source_run_id) = &turn.parent_run_id {
                edges.push(RouteEdgeView {
                    id: format!("{}->{}", source_run_id, turn.id),
                    source_run_id: source_run_id.clone(),
                    target_turn_id: turn.id.clone(),
                    is_on_current_lineage: route_edge_is_on_lineage(source_run_id, runs, &lineage),
                });
            }
        }
        Ok(RouteProjection {
            workspace_id: input.workspace_id,
            nodes,
            edges,
        })
    }

    async fn compare_runs_impl(&self, input: CompareRunsInput) -> AppResult<CompareRunsResult> {
        let left = self
            .repository
            .get_run(&input.left_run_id)
            .await
            .map_err(repository_port_error)?;
        let right = self
            .repository
            .get_run(&input.right_run_id)
            .await
            .map_err(repository_port_error)?;
        let left_receipt = self
            .repository
            .get_run_snapshot(&left.id)
            .await
            .map_err(repository_port_error)?;
        let right_receipt = self
            .repository
            .get_run_snapshot(&right.id)
            .await
            .map_err(repository_port_error)?;
        let left_turn = self
            .repository
            .get_turn(&left.turn_id)
            .await
            .map_err(repository_port_error)?;
        let right_turn = self
            .repository
            .get_turn(&right.turn_id)
            .await
            .map_err(repository_port_error)?;
        let mut checkpoints = self
            .repository
            .list_context_checkpoints(&left_turn.workspace_id)
            .await
            .map_err(repository_port_error)?;
        if right_turn.workspace_id != left_turn.workspace_id {
            checkpoints.extend(
                self.repository
                    .list_context_checkpoints(&right_turn.workspace_id)
                    .await
                    .map_err(repository_port_error)?,
            );
        }
        let left_ids = left_receipt
            .manifest
            .items
            .iter()
            .map(context_item_identity)
            .collect::<BTreeSet<_>>();
        let right_ids = right_receipt
            .manifest
            .items
            .iter()
            .map(context_item_identity)
            .collect::<BTreeSet<_>>();
        let only_left = left_receipt
            .manifest
            .items
            .iter()
            .filter(|item| !right_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        let only_right = right_receipt
            .manifest
            .items
            .iter()
            .filter(|item| !left_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        let shared = left_receipt
            .manifest
            .items
            .iter()
            .filter(|item| right_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        Ok(CompareRunsResult {
            left: ComparableRunView {
                run_id: left.id.clone(),
                model: left.model.clone(),
                status: domain_run_status_view(left.status()),
            },
            right: ComparableRunView {
                run_id: right.id.clone(),
                model: right.model.clone(),
                status: domain_run_status_view(right.status()),
            },
            answer: AnswerComparison {
                left_markdown: left.output_markdown().into(),
                right_markdown: right.output_markdown().into(),
            },
            context_diff: ContextDiffView {
                only_left,
                only_right,
                shared,
                left_checkpoint_provenance: context_receipt_checkpoint_provenance(
                    &left_receipt.manifest,
                    &checkpoints,
                ),
                right_checkpoint_provenance: context_receipt_checkpoint_provenance(
                    &right_receipt.manifest,
                    &checkpoints,
                ),
            },
        })
    }

    async fn mark_decision_impl(&self, input: MarkDecisionInput) -> AppResult<DecisionMarkView> {
        let now = now_millis();
        let existing = self
            .repository
            .get_decision_mark(&input.workspace_id, &input.run_id)
            .await
            .ok();
        self.repository
            .save_decision_mark(DecisionMark {
                id: existing
                    .as_ref()
                    .map(|mark| mark.id.clone())
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                workspace_id: input.workspace_id,
                run_id: input.run_id,
                status: decision_status_domain(input.status),
                reason: input.reason,
                created_at: existing.map(|mark| mark.created_at).unwrap_or(now),
                updated_at: now,
            })
            .await
            .map_err(repository_port_error)
            .and_then(decision_view)
    }

    async fn export_decision_packet_impl(
        &self,
        input: ExportDecisionPacketInput,
    ) -> AppResult<ExportResult> {
        let detail = self.open_workspace_impl(&input.workspace_id).await?;
        let checkpoints = self
            .repository
            .list_context_checkpoints(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
        let marked_turn_prompts = [
            DecisionStatusView::Accepted,
            DecisionStatusView::Rejected,
            DecisionStatusView::ToVerify,
        ]
        .into_iter()
        .flat_map(|status| {
            detail
                .decision_marks
                .iter()
                .filter(move |mark| mark.status == status)
        })
        .filter_map(|mark| {
            detail
                .turns
                .iter()
                .find(|turn| turn.runs.iter().any(|run| run.id == mark.run_id))
                .map(|turn| turn.prompt.as_str())
        });
        let problem = decision_problem(
            &detail.workspace.goal,
            marked_turn_prompts,
            detail.turns.first().map(|turn| turn.prompt.as_str()),
        );
        let mut markdown = format!(
            "# Decision Packet: {}\n\n## Problem\n\n{}\n\n",
            detail.workspace.name, problem
        );
        for heading in [
            DecisionStatusView::Accepted,
            DecisionStatusView::Rejected,
            DecisionStatusView::ToVerify,
        ] {
            let title = match heading {
                DecisionStatusView::Accepted => "Adopted routes",
                DecisionStatusView::Rejected => "Rejected routes",
                DecisionStatusView::ToVerify => "Open questions / needs validation",
            };
            markdown.push_str(&format!("## {title}\n\n"));
            let mut found = false;
            for mark in detail
                .decision_marks
                .iter()
                .filter(|mark| mark.status == heading)
            {
                found = true;
                let run = self
                    .repository
                    .get_run(&mark.run_id)
                    .await
                    .map_err(repository_port_error)?;
                let receipt = self
                    .repository
                    .get_run_snapshot(&mark.run_id)
                    .await
                    .map_err(repository_port_error)?;
                let checkpoint_provenance =
                    context_receipt_checkpoint_provenance(&receipt.manifest, &checkpoints);
                markdown.push_str(&format!(
                    "### {} · {}\n\n**Rationale:** {}\n\n{}\n\n_Context receipt:_ `{}` · {} ordered items · {}\n\n",
                    run.model,
                    mark.run_id,
                    mark.reason,
                    run.output_markdown(),
                    receipt.manifest.canonical_hash,
                    receipt.manifest.items.len(),
                    receipt.provider.base_url,
                ));
                markdown.push_str(&decision_packet_checkpoint_provenance(
                    &checkpoint_provenance,
                ));
            }
            if !found {
                markdown.push_str("_None._\n\n");
            }
        }
        let exported = self
            .decision_packet_writer
            .write(&input.workspace_id, &markdown)
            .map_err(|error| AppError::internal("filesystem_error", error.to_string()))?;
        Ok(ExportResult {
            path: exported.path,
            bytes_written: exported.bytes_written,
        })
    }

    async fn save_provider_profile_impl(
        &self,
        input: SaveProviderProfileInput,
    ) -> AppResult<ProviderProfileView> {
        let now = now_millis();
        let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing = self.repository.get_provider_profile(&id).await.ok();
        let (_, dialect) = runnable_template(&input.provider_id)?;
        let effective_parameters =
            normalize_provider_parameter_values(input.parameters.unwrap_or_default())?;
        let mut parameters = effective_parameters.stored_values();
        parameters.insert(INTERNAL_DEFAULT_KEY.into(), input.is_default.to_string());
        let record = self
            .repository
            .save_provider_profile(ProviderProfile {
                id,
                provider_id: input.provider_id,
                name: input.name,
                dialect,
                base_url: input.base_url,
                model: input.model,
                parameters,
                created_at: existing.map(|profile| profile.created_at).unwrap_or(now),
                updated_at: now,
            })
            .await
            .map_err(repository_port_error)?;
        provider_profile_view(record)
    }

    async fn test_provider_connection_impl(
        &self,
        input: TestProviderConnectionInput,
        credential: Option<SessionCredentialValue>,
    ) -> AppResult<ProviderConnectionResult> {
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let credential = credential
            .map(|value| {
                value
                    .as_str()
                    .map(|secret| SessionCredential::new(secret.to_owned()))
            })
            .transpose()?;
        // Connection testing is a metadata-only catalog probe. A compatible
        // chat protocol does not imply that the provider has a /models API.
        provider_model_query(&profile.provider_id, &profile.base_url)?;
        let resolved_provider = domain_provider_snapshot(&profile)?;
        let target = provider_target(&resolved_provider)?;
        let response = self
            .connection_tester
            .test(target, credential)
            .await
            .map_err(provider_port_error)?;
        Ok(ProviderConnectionResult {
            ok: response.ok,
            message: if response.ok {
                format!("Connected to {}", profile.name)
            } else {
                format!("Provider returned HTTP {}", response.http_status)
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RunStateSnapshot;
    use crate::infrastructure::{
        filesystem::LocalDecisionPacketWriter,
        provider::ReqwestProviderGateway,
        sqlite::{
            BranchPointerRecord, ContentBlockRecord, ContextCursorUpdateRecord,
            ContextDraftUpdateRecord, ContextManifestRecord, ContextOverrideItemRecord,
            ContextSnapshotRecord, ModelRunRecord, RunContextItemRecord,
            RunFinish as StoredRunFinish, RunStartBundle, RunStatusRecord, SqliteRepository,
            TurnRecord, WorkspaceRecord,
        },
    };

    struct CancellationOnlyProvider;

    impl ProviderGateway for CancellationOnlyProvider {
        fn stream<'a>(
            &'a self,
            _invocation: ProviderInvocation,
            cancellation: CancellationToken,
            events: mpsc::Sender<RunEvent>,
        ) -> crate::ports::provider::ProviderFuture<'a> {
            Box::pin(async move {
                let _events = events;
                cancellation.cancelled().await;
                Ok(())
            })
        }
    }

    fn provider_summary_invocation() -> ProviderInvocation {
        ProviderInvocation {
            target: ProviderTarget {
                dialect: ProviderDialect::OpenAiChatCompletions,
                base_url: "http://127.0.0.1:1/v1".into(),
                credential_placement: CredentialPlacement::None,
                additional_headers: BTreeMap::new(),
            },
            credential: None,
            request: CanonicalRequest {
                run_id: "maintenance-1".into(),
                model: "summary-model".into(),
                messages: Vec::new(),
                temperature: None,
                top_p: None,
                max_output_tokens: None,
                stop: Vec::new(),
            },
        }
    }

    fn context_navigation_run_bundle(
        workspace_id: &str,
        turn_id: &str,
        run_id: &str,
        prompt: &str,
        created_at: i64,
    ) -> RunStartBundle {
        let prompt_hash = domain::sha256_hex(prompt.as_bytes());
        let prompt_block_id = format!("block-user-{prompt_hash}");
        let manifest_id = format!("manifest-{run_id}");
        let canonical_hash = format!("canonical-{run_id}");
        RunStartBundle {
            turn: Some(TurnRecord {
                id: turn_id.into(),
                workspace_id: workspace_id.into(),
                parent_run_id: None,
                prompt_block_id: prompt_block_id.clone(),
                prompt_markdown: prompt.into(),
                title: prompt.into(),
                created_at,
                deleted_at: None,
            }),
            run: ModelRunRecord {
                id: run_id.into(),
                turn_id: turn_id.into(),
                workspace_id: workspace_id.into(),
                provider_profile_id: Some("provider-local-ollama".into()),
                model: "test-model".into(),
                status: RunStatusRecord::Queued,
                output_markdown: String::new(),
                reasoning_markdown: String::new(),
                provider_snapshot_json: r#"{"dialect":"ollama_chat"}"#.into(),
                usage_json: None,
                error_json: None,
                created_at,
                started_at: None,
                finished_at: None,
                checkpointed_at: None,
            },
            content_blocks: vec![ContentBlockRecord {
                id: prompt_block_id.clone(),
                role: "user".into(),
                content: prompt.into(),
                content_hash: prompt_hash,
                created_at,
            }],
            manifest: ContextManifestRecord {
                id: manifest_id.clone(),
                workspace_id: workspace_id.into(),
                compiler_version: "4".into(),
                strategy: "exact_path".into(),
                estimated_chars: i64::try_from(prompt.len()).unwrap(),
                canonical_hash: canonical_hash.clone(),
                warnings_json: "[]".into(),
                checkpoint_provenance_json: None,
                branch_summary_provenance_json: "[]".into(),
                created_at,
            },
            context_items: Vec::<RunContextItemRecord>::new(),
            snapshot: ContextSnapshotRecord {
                id: format!("snapshot-{run_id}"),
                run_id: run_id.into(),
                manifest_id,
                workspace_id: workspace_id.into(),
                provider_profile_id: Some("provider-local-ollama".into()),
                provider_id: Some("ollama".into()),
                template_revision: Some(1),
                stream_protocol: Some("ollama_ndjson".into()),
                auth_placement: Some("none".into()),
                auth_header_name: None,
                additional_headers_json: "{}".into(),
                provider: "Local Ollama".into(),
                model: "test-model".into(),
                base_url: "http://127.0.0.1:11434".into(),
                parameters_json: "{}".into(),
                request_json: "{}".into(),
                canonical_hash,
                created_at,
            },
            branch_pointer: None,
            context_update: None,
        }
    }

    async fn complete_context_navigation_run(
        repository: &SqliteRepository,
        run_id: &str,
        output: &str,
        finished_at: i64,
    ) {
        repository
            .mark_run_connecting(run_id, finished_at - 2)
            .await
            .unwrap();
        repository
            .mark_run_streaming(run_id, finished_at - 1)
            .await
            .unwrap();
        repository
            .finish_run(
                run_id,
                &StoredRunFinish {
                    status: RunStatusRecord::Completed,
                    output_markdown: output.into(),
                    reasoning_markdown: String::new(),
                    usage_json: None,
                    error_json: None,
                    finished_at,
                },
            )
            .await
            .unwrap();
    }

    fn run_with_status(id: &str, turn_id: &str, status: RunStatus) -> ModelRun {
        ModelRun::rehydrate(
            RunDraft {
                id: id.into(),
                turn_id: turn_id.into(),
                provider_profile_id: Some("provider-1".into()),
                model: "model".into(),
                created_at: 1,
            },
            RunStateSnapshot {
                status,
                output_markdown: "same answer shape".into(),
                reasoning_markdown: String::new(),
                error: (status == RunStatus::Failed).then(|| RunFailure::message("failed")),
                usage: None,
                started_at: Some(2),
                finished_at: Some(3),
                checkpointed_at: Some(3),
            },
        )
        .unwrap()
    }

    fn run(id: &str, turn_id: &str) -> ModelRun {
        run_with_status(id, turn_id, RunStatus::Completed)
    }

    fn checkpoint_source_with_status(status: RunStatus) -> ModelRun {
        ModelRun::rehydrate(
            RunDraft {
                id: "checkpoint-source".into(),
                turn_id: "checkpoint-source-turn".into(),
                provider_profile_id: Some("provider-1".into()),
                model: "model".into(),
                created_at: 1,
            },
            RunStateSnapshot {
                status,
                output_markdown: if status != RunStatus::Queued {
                    "checkpoint evidence"
                } else {
                    ""
                }
                .into(),
                reasoning_markdown: String::new(),
                error: (status == RunStatus::Failed).then(|| RunFailure::message("failed")),
                usage: None,
                started_at: (status != RunStatus::Queued).then_some(2),
                finished_at: status.is_terminal().then_some(3),
                checkpointed_at: (status != RunStatus::Queued).then_some(3),
            },
        )
        .unwrap()
    }

    fn checkpoint_selection_data(status: RunStatus) -> crate::ports::WorkspaceContextData {
        let turns = vec![Turn::root(
            "checkpoint-source-turn",
            "workspace-1",
            "source prompt",
            2,
        )];
        let runs = vec![checkpoint_source_with_status(status)];
        let graph =
            domain::ConversationGraph::try_new(turns.clone(), runs.clone(), Vec::new()).unwrap();
        crate::ports::WorkspaceContextData {
            workspace_id: "workspace-1".into(),
            graph,
            turns,
            runs,
            run_provider_provenance: Vec::new(),
            cursor: crate::ports::ContextCursor {
                workspace_id: "workspace-1".into(),
                active_run_id: Some("checkpoint-source".into()),
                branch_pointer_id: Some("branch-main".into()),
                version: 1,
                updated_at: 1,
            },
            branch_pointers: vec![BranchPointer {
                id: "branch-main".into(),
                workspace_id: "workspace-1".into(),
                name: "Main".into(),
                head_run_id: "checkpoint-source".into(),
                version: 1,
                updated_at: 1,
            }],
            branch_revisions: Vec::new(),
            branch_checkpoint_inheritance: Vec::new(),
            draft: crate::ports::ContextDraft {
                workspace_id: "workspace-1".into(),
                parent_run_id: Some("checkpoint-source".into()),
                version: 1,
                items: Vec::new(),
                consumed_by_run_id: None,
                updated_at: 1,
            },
            checkpoints: Vec::new(),
            view_states: Vec::new(),
        }
    }

    #[tokio::test]
    async fn switching_context_atomically_rebases_the_next_send_draft() {
        const WORKSPACE_ID: &str = "workspace-context-rebase";
        let repository = Arc::new(SqliteRepository::connect_in_memory().await.unwrap());
        repository
            .create_workspace(&WorkspaceRecord {
                id: WORKSPACE_ID.into(),
                title: "Context rebase".into(),
                goal: "Keep navigation and draft atomic".into(),
                system_prompt: "System".into(),
                created_at: 1,
                updated_at: 1,
                archived_at: None,
            })
            .await
            .unwrap();
        let export_root = tempfile::tempdir().unwrap();
        let provider = Arc::new(ReqwestProviderGateway::with_defaults().unwrap());
        let backend = DefaultApplicationBackend::new(
            repository.clone(),
            provider.clone(),
            provider.clone(),
            provider,
            Arc::new(LocalDecisionPacketWriter::new(
                export_root.path().to_path_buf(),
            )),
        );
        backend.initialize().await.unwrap();
        repository
            .persist_run_start(&context_navigation_run_bundle(
                WORKSPACE_ID,
                "turn-a",
                "run-a",
                "Prompt A",
                10,
            ))
            .await
            .unwrap();
        repository
            .persist_run_start(&context_navigation_run_bundle(
                WORKSPACE_ID,
                "turn-b",
                "run-b",
                "Prompt B",
                20,
            ))
            .await
            .unwrap();
        let pinned_hash = domain::sha256_hex(b"Prompt A");
        repository
            .update_context_draft(&ContextDraftUpdateRecord {
                workspace_id: WORKSPACE_ID.into(),
                parent_run_id: Some("run-b".into()),
                expected_version: 0,
                content_blocks: Vec::new(),
                items: vec![ContextOverrideItemRecord {
                    workspace_id: WORKSPACE_ID.into(),
                    position: 0,
                    operation: "pin".into(),
                    source_kind: "content_block".into(),
                    source_id: Some(format!("block-user-{pinned_hash}")),
                    content_block_id: Some(format!("block-user-{pinned_hash}")),
                    content_hash: Some(pinned_hash),
                    created_at: 21,
                }],
                updated_at: 21,
            })
            .await
            .unwrap();

        let before = backend
            .get_context_tree(GetContextTreeInput {
                workspace_id: WORKSPACE_ID.into(),
            })
            .await
            .unwrap();
        assert_eq!(before.cursor.active_run_id.as_deref(), Some("run-b"));
        assert_eq!(before.cursor.version, 0);
        assert_eq!(before.draft_version, 1);

        let switched = backend
            .set_active_context(SetActiveContextInput {
                workspace_id: WORKSPACE_ID.into(),
                run_id: Some("run-a".into()),
                branch_id: None,
                expected_cursor_version: 0,
                expected_draft_version: 1,
            })
            .await
            .unwrap();
        assert_eq!(switched.active_run_id.as_deref(), Some("run-a"));
        assert_eq!(switched.version, 1);

        let after = backend
            .get_context_tree(GetContextTreeInput {
                workspace_id: WORKSPACE_ID.into(),
            })
            .await
            .unwrap();
        assert_eq!(after.cursor.active_run_id.as_deref(), Some("run-a"));
        assert_eq!(after.cursor.version, 1);
        assert_eq!(after.draft_version, 2);
        let rebased = repository.get_context_draft(WORKSPACE_ID).await.unwrap();
        assert_eq!(rebased.parent_run_id.as_deref(), Some("run-a"));
        assert!(rebased.items.is_empty());

        let stale = backend
            .set_active_context(SetActiveContextInput {
                workspace_id: WORKSPACE_ID.into(),
                run_id: Some("run-b".into()),
                branch_id: None,
                expected_cursor_version: 1,
                expected_draft_version: 1,
            })
            .await
            .expect_err("stale draft CAS must roll back the cursor switch");
        assert_eq!(stale.code, "context_draft_conflict");
        let unchanged = backend
            .get_context_tree(GetContextTreeInput {
                workspace_id: WORKSPACE_ID.into(),
            })
            .await
            .unwrap();
        assert_eq!(unchanged.cursor.active_run_id.as_deref(), Some("run-a"));
        assert_eq!(unchanged.cursor.version, 1);
        assert_eq!(unchanged.draft_version, 2);
    }

    #[tokio::test]
    async fn repeated_manual_compaction_applies_only_the_later_checkpoint_and_preserves_audit_data()
    {
        const WORKSPACE_ID: &str = "workspace-repeated-compaction";
        const BRANCH_ID: &str = "branch-repeated-compaction";
        const FIRST_CHECKPOINT_ID: &str = "11111111-1111-4111-8111-111111111111";
        const SECOND_CHECKPOINT_ID: &str = "22222222-2222-4222-8222-222222222222";

        let database_root = tempfile::tempdir().unwrap();
        let database_path = database_root.path().join("repeated-compaction.sqlite");
        let repository = Arc::new(SqliteRepository::connect(&database_path).await.unwrap());
        repository
            .create_workspace(&WorkspaceRecord {
                id: WORKSPACE_ID.into(),
                title: "Repeated compaction".into(),
                goal: "Keep only the latest effective summary".into(),
                system_prompt: "System baseline".into(),
                created_at: 1,
                updated_at: 1,
                archived_at: None,
            })
            .await
            .unwrap();
        let export_root = tempfile::tempdir().unwrap();
        let provider = Arc::new(ReqwestProviderGateway::with_defaults().unwrap());
        let backend = DefaultApplicationBackend::new(
            repository.clone(),
            provider.clone(),
            provider.clone(),
            provider,
            Arc::new(LocalDecisionPacketWriter::new(
                export_root.path().to_path_buf(),
            )),
        );
        backend.initialize().await.unwrap();

        let mut root = context_navigation_run_bundle(
            WORKSPACE_ID,
            "turn-compaction-root",
            "run-compaction-root",
            "Root question",
            10,
        );
        root.branch_pointer = Some(BranchPointerRecord {
            id: BRANCH_ID.into(),
            workspace_id: WORKSPACE_ID.into(),
            name: "Main".into(),
            head_run_id: "run-compaction-root".into(),
            version: 0,
            created_at: 10,
            updated_at: 10,
        });
        repository.persist_run_start(&root).await.unwrap();
        complete_context_navigation_run(
            repository.as_ref(),
            "run-compaction-root",
            "Root answer",
            13,
        )
        .await;

        let mut leaf = context_navigation_run_bundle(
            WORKSPACE_ID,
            "turn-compaction-leaf",
            "run-compaction-leaf",
            "Leaf question",
            20,
        );
        leaf.turn.as_mut().unwrap().parent_run_id = Some("run-compaction-root".into());
        leaf.branch_pointer = Some(BranchPointerRecord {
            id: BRANCH_ID.into(),
            workspace_id: WORKSPACE_ID.into(),
            name: "Main".into(),
            head_run_id: "run-compaction-leaf".into(),
            version: 1,
            created_at: 10,
            updated_at: 20,
        });
        repository.persist_run_start(&leaf).await.unwrap();
        complete_context_navigation_run(
            repository.as_ref(),
            "run-compaction-leaf",
            "Leaf answer",
            23,
        )
        .await;
        repository
            .set_context_cursor(&ContextCursorUpdateRecord {
                workspace_id: WORKSPACE_ID.into(),
                active_run_id: Some("run-compaction-leaf".into()),
                branch_pointer_id: Some(BRANCH_ID.into()),
                expected_version: 0,
                updated_at: 24,
            })
            .await
            .unwrap();

        let seed_id = "00000000-0000-4000-8000-000000000000";
        let seed_created_at = now_millis() + 60_000;
        let seed_summary = "Clock-normalization seed";
        let seed_summary_hash = domain::sha256_hex(seed_summary.as_bytes());
        let seed_summary_block_id = format!("block-system-{seed_summary_hash}");
        let seed_source_run_ids = vec!["run-compaction-root".into()];
        let seed_data =
            RepositoryPort::load_workspace_context_data(repository.as_ref(), WORKSPACE_ID)
                .await
                .unwrap();
        let seed_source_hash = checkpoint_source_hash(&seed_data, &seed_source_run_ids).unwrap();
        let seed_guard = crate::ports::MaintenanceContextGuard {
            workspace_id: WORKSPACE_ID.into(),
            expected_cursor_version: 1,
            branch_pointer_id: Some(BRANCH_ID.into()),
            expected_branch_version: Some(1),
            expected_draft_version: None,
        };
        let seed_start = RepositoryPort::start_context_maintenance(
            repository.as_ref(),
            domain::ContextMaintenanceRun {
                id: seed_id.into(),
                workspace_id: WORKSPACE_ID.into(),
                kind: domain::ContextCheckpointKind::Compaction,
                status: domain::ContextMaintenanceStatus::Running,
                branch_pointer_id: Some(BRANCH_ID.into()),
                branch_revision: Some(1),
                anchor_run_id: "run-compaction-leaf".into(),
                first_kept_run_id: Some("run-compaction-leaf".into()),
                source_run_ids: seed_source_run_ids.clone(),
                source_hash: seed_source_hash.clone(),
                provider: None,
                request_json: r#"{"mode":"test-clock-seed"}"#.into(),
                summary: None,
                error: None,
                created_at: seed_created_at,
                started_at: Some(seed_created_at),
                finished_at: None,
            },
            seed_guard.clone(),
        )
        .await
        .unwrap();
        RepositoryPort::finish_context_maintenance(
            repository.as_ref(),
            domain::ContextMaintenanceRun {
                status: domain::ContextMaintenanceStatus::Completed,
                summary: Some(seed_summary.into()),
                finished_at: Some(seed_created_at),
                ..seed_start.run
            },
            Some(domain::ContextCheckpoint {
                id: seed_id.into(),
                workspace_id: WORKSPACE_ID.into(),
                maintenance_run_id: seed_id.into(),
                kind: domain::ContextCheckpointKind::Compaction,
                branch_pointer_id: Some(BRANCH_ID.into()),
                branch_revision: Some(1),
                anchor_run_id: "run-compaction-leaf".into(),
                first_kept_run_id: Some("run-compaction-leaf".into()),
                summary: seed_summary.into(),
                summary_content_block_id: seed_summary_block_id.clone(),
                source_run_ids: seed_source_run_ids,
                source_hash: seed_source_hash,
                provider: None,
                created_at: seed_created_at,
            }),
            Some(domain::ContentBlock {
                id: seed_summary_block_id,
                workspace_id: WORKSPACE_ID.into(),
                role: MessageRole::System,
                content: seed_summary.into(),
                content_hash: seed_summary_hash,
                created_at: seed_created_at,
            }),
            seed_guard,
            None,
        )
        .await
        .unwrap();

        let receipt_before = backend
            .get_run_snapshot("run-compaction-leaf".into())
            .await
            .unwrap();
        let stored_receipt_before = repository
            .get_run_receipt("run-compaction-leaf")
            .await
            .unwrap();
        let first_response = backend
            .create_context_checkpoint(CreateContextCheckpointInput {
                client_operation_id: FIRST_CHECKPOINT_ID.into(),
                workspace_id: WORKSPACE_ID.into(),
                branch_id: BRANCH_ID.into(),
                kind: ContextCheckpointKindView::Compaction,
                source_run_ids: vec!["run-compaction-root".into()],
                first_kept_run_id: Some("run-compaction-leaf".into()),
                summary: "Obsolete manual summary".into(),
                expected_cursor_version: 1,
                expected_branch_version: 1,
            })
            .await
            .unwrap();
        let second_input = CreateContextCheckpointInput {
            client_operation_id: SECOND_CHECKPOINT_ID.into(),
            workspace_id: WORKSPACE_ID.into(),
            branch_id: BRANCH_ID.into(),
            kind: ContextCheckpointKindView::Compaction,
            source_run_ids: vec!["run-compaction-root".into()],
            first_kept_run_id: Some("run-compaction-leaf".into()),
            summary: "Replacement manual summary".into(),
            expected_cursor_version: 1,
            expected_branch_version: 1,
        };
        let second_response = backend
            .create_context_checkpoint(second_input.clone())
            .await
            .unwrap();
        let replayed_second_response = backend
            .create_context_checkpoint(second_input)
            .await
            .unwrap();

        let preview = backend
            .inspect_context(InspectContextInput {
                workspace_id: WORKSPACE_ID.into(),
                parent_run_id: Some("run-compaction-leaf".into()),
                prompt: "Next question".into(),
                provider_profile_id: "provider-local-ollama".into(),
                branch_id: Some(BRANCH_ID.into()),
            })
            .await
            .unwrap();
        assert_eq!(
            preview
                .items
                .iter()
                .map(|item| item.content.as_str())
                .collect::<Vec<_>>(),
            vec![
                "System baseline",
                "Replacement manual summary",
                "Leaf question",
                "Leaf answer",
                "Next question",
            ]
        );
        assert_eq!(
            preview
                .applied_checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.id.as_str()),
            Some(SECOND_CHECKPOINT_ID)
        );
        assert!(
            preview
                .items
                .iter()
                .all(|item| item.content != "Obsolete manual summary"
                    && item.content != "Root question"
                    && item.content != "Root answer"),
            "the old summary and compacted source path must not reach effective context"
        );

        let tree = backend
            .get_context_tree(GetContextTreeInput {
                workspace_id: WORKSPACE_ID.into(),
            })
            .await
            .unwrap();
        let first = tree
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == FIRST_CHECKPOINT_ID)
            .expect("the superseded checkpoint remains queryable");
        let second = tree
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == SECOND_CHECKPOINT_ID)
            .expect("the applied checkpoint remains queryable");
        assert_eq!(first.summary, "Obsolete manual summary");
        assert_eq!(second.summary, "Replacement manual summary");
        assert_eq!(&first_response, first);
        assert_eq!(&second_response, second);
        assert_eq!(
            replayed_second_response, second_response,
            "the first response, refresh, and idempotent replay expose one persisted timestamp"
        );
        assert!(
            first.created_at < second.created_at,
            "commit order must be represented by strictly increasing persisted time"
        );

        let receipt_after = backend
            .get_run_snapshot("run-compaction-leaf".into())
            .await
            .unwrap();
        let stored_receipt_after = repository
            .get_run_receipt("run-compaction-leaf")
            .await
            .unwrap();
        assert_eq!(
            receipt_after, receipt_before,
            "checkpoint maintenance must never rewrite an existing Context Receipt"
        );
        assert_eq!(
            stored_receipt_after, stored_receipt_before,
            "every persisted Receipt field and item must remain byte-for-byte stable"
        );

        drop(backend);
        drop(repository);

        let reopened_repository =
            Arc::new(SqliteRepository::connect(&database_path).await.unwrap());
        let reopened_provider = Arc::new(ReqwestProviderGateway::with_defaults().unwrap());
        let reopened_backend = DefaultApplicationBackend::new(
            reopened_repository.clone(),
            reopened_provider.clone(),
            reopened_provider.clone(),
            reopened_provider,
            Arc::new(LocalDecisionPacketWriter::new(
                export_root.path().to_path_buf(),
            )),
        );
        reopened_backend.initialize().await.unwrap();
        let reopened_preview = reopened_backend
            .inspect_context(InspectContextInput {
                workspace_id: WORKSPACE_ID.into(),
                parent_run_id: Some("run-compaction-leaf".into()),
                prompt: "Next question".into(),
                provider_profile_id: "provider-local-ollama".into(),
                branch_id: Some(BRANCH_ID.into()),
            })
            .await
            .unwrap();
        assert_eq!(
            reopened_preview.items, preview.items,
            "restart must rebuild the same effective Context from persisted evidence"
        );
        assert_eq!(
            reopened_preview
                .applied_checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.id.as_str()),
            Some(SECOND_CHECKPOINT_ID)
        );
        let reopened_tree = reopened_backend
            .get_context_tree(GetContextTreeInput {
                workspace_id: WORKSPACE_ID.into(),
            })
            .await
            .unwrap();
        assert!(
            reopened_tree
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.id == FIRST_CHECKPOINT_ID),
            "the superseded checkpoint remains auditable after restart"
        );
        assert_eq!(
            reopened_backend
                .get_run_snapshot("run-compaction-leaf".into())
                .await
                .unwrap(),
            receipt_before,
            "the complete immutable Receipt survives checkpointing and restart"
        );
        assert_eq!(
            reopened_repository
                .get_run_receipt("run-compaction-leaf")
                .await
                .unwrap(),
            stored_receipt_before,
            "the stored Receipt survives checkpointing and reconnect unchanged"
        );
    }

    fn run_provider_provenance(
        run_id: &str,
        provider_name: &str,
        base_url: &str,
        model: &str,
    ) -> RunProviderProvenance {
        RunProviderProvenance {
            run_id: run_id.into(),
            provider_name: provider_name.into(),
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    fn context_maintenance(
        id: &str,
        status: domain::ContextMaintenanceStatus,
        request_json: &str,
    ) -> domain::ContextMaintenanceRun {
        domain::ContextMaintenanceRun {
            id: id.into(),
            workspace_id: "workspace-1".into(),
            kind: domain::ContextCheckpointKind::Compaction,
            status,
            branch_pointer_id: Some("branch-1".into()),
            branch_revision: Some(2),
            anchor_run_id: "run-2".into(),
            first_kept_run_id: Some("run-2".into()),
            source_run_ids: vec!["run-1".into()],
            source_hash: "source-hash".into(),
            provider: None,
            request_json: request_json.into(),
            summary: (status == domain::ContextMaintenanceStatus::Completed)
                .then(|| "summary".into()),
            error: None,
            created_at: 1,
            started_at: Some(1),
            finished_at: (status == domain::ContextMaintenanceStatus::Completed).then_some(2),
        }
    }

    fn context_checkpoint(maintenance_run_id: &str) -> domain::ContextCheckpoint {
        domain::ContextCheckpoint {
            id: maintenance_run_id.into(),
            workspace_id: "workspace-1".into(),
            maintenance_run_id: maintenance_run_id.into(),
            kind: domain::ContextCheckpointKind::Compaction,
            branch_pointer_id: Some("branch-1".into()),
            branch_revision: Some(2),
            anchor_run_id: "run-2".into(),
            first_kept_run_id: Some("run-2".into()),
            summary: "summary".into(),
            summary_content_block_id: "block-summary".into(),
            source_run_ids: vec!["run-1".into()],
            source_hash: "source-hash".into(),
            provider: None,
            created_at: 2,
        }
    }

    #[tokio::test]
    async fn provider_summary_cancellation_does_not_wait_for_a_terminal_event() {
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let task = tokio::spawn(collect_provider_summary(
            Arc::new(CancellationOnlyProvider),
            provider_summary_invocation(),
            cancellation,
        ));
        tokio::task::yield_now().await;
        cancel.cancel();

        let error = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancellation must promptly stop summary collection")
            .expect("summary collection task must join")
            .expect_err("cancellation cannot produce a summary");
        assert_eq!(error.code, "context_maintenance_cancelled");
    }

    #[test]
    fn terminal_maintenance_errors_identify_when_a_new_operation_id_is_required() {
        let error = context_maintenance_provider_error(
            AppError::internal("provider_timeout", "Provider timed out")
                .with_details(json!({ "status": 504 })),
            "maintenance-1",
            domain::ContextMaintenanceStatus::Failed,
        );

        assert_eq!(error.code, "provider_timeout");
        assert_eq!(
            error.details,
            json!({
                "clientOperationId": "maintenance-1",
                "maintenanceStatus": "failed",
                "providerDetails": { "status": 504 },
            })
        );
    }

    #[test]
    fn failed_run_view_keeps_immutable_provider_identity_after_profile_changes() {
        let failed = run_with_status("run-failed", "turn-1", RunStatus::Failed);
        let historical = run_provider_provenance(
            &failed.id,
            "Original provider",
            "https://original.example.com/v1",
            "original-model",
        );
        let view = run_view(&failed, Some(&historical)).expect("a failed Run remains displayable");

        assert_eq!(view.provider_name, "Original provider");
        assert_eq!(view.base_url, "https://original.example.com/v1");
        assert_eq!(view.model, "original-model");
        assert_eq!(view.status, RunStatusView::Failed);
    }

    #[test]
    fn legacy_run_without_provenance_does_not_invent_mutable_provider_identity() {
        let failed = run_with_status("run-legacy", "turn-1", RunStatus::Failed);

        let view = run_view(&failed, None).expect("legacy fallback remains usable");

        assert_eq!(view.provider_name, "Unknown provider");
        assert!(view.base_url.is_empty());
        assert_eq!(view.model, failed.model);
    }

    #[test]
    fn retry_uses_the_exact_original_turn_when_prompts_are_duplicates() {
        // These Runs may belong to Turns with identical parent/prompt content.
        // Their exact persisted Run identity remains unambiguous.
        let first = run("run-1", "turn-1");
        let second = run("run-2", "turn-2");

        assert_eq!(exact_retry_turn_id(&first), "turn-1");
        assert_eq!(exact_retry_turn_id(&second), "turn-2");
    }

    #[test]
    fn identical_text_in_different_roles_has_distinct_content_block_ids() {
        let hash = domain::sha256_hex(b"same text");
        let items = [
            domain::RunContextItem {
                position: 0,
                source_id: Some("prompt".into()),
                source_ref: domain::ContextSourceRef::new(
                    domain::ContextSourceRefKind::TurnPrompt,
                    "prompt",
                ),
                source_kind: ContextSourceKind::TurnPrompt,
                role: MessageRole::User,
                content: "same text".into(),
                content_block_id: content_block_id("user", &hash),
                content_hash: hash.clone(),
                inclusion_reason: InclusionReason::ExactAncestorPath,
                mandatory: false,
            },
            domain::RunContextItem {
                position: 1,
                source_id: Some("answer".into()),
                source_ref: domain::ContextSourceRef::new(
                    domain::ContextSourceRefKind::ModelRun,
                    "answer",
                ),
                source_kind: ContextSourceKind::ModelRun,
                role: MessageRole::Assistant,
                content: "same text".into(),
                content_block_id: content_block_id("assistant", &hash),
                content_hash: hash.clone(),
                inclusion_reason: InclusionReason::ExactAncestorPath,
                mandatory: false,
            },
        ];
        let blocks = content_blocks_for_manifest("workspace-1", &items, 1);

        assert_eq!(blocks.len(), 2);
        assert!(
            blocks
                .iter()
                .any(|block| block.id == content_block_id("user", &hash))
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.id == content_block_id("assistant", &hash))
        );
    }

    #[test]
    fn pinned_context_keeps_its_exact_content_block_identity() {
        let hash = domain::sha256_hex(b"pinned evidence");
        let item = domain::RunContextItem {
            position: 0,
            source_id: Some("pin-1".into()),
            source_ref: domain::ContextSourceRef::new(
                domain::ContextSourceRefKind::ContentBlock,
                "pin-1",
            ),
            source_kind: ContextSourceKind::Pinned,
            role: MessageRole::Assistant,
            content: "pinned evidence".into(),
            content_block_id: "pin-1".into(),
            content_hash: hash,
            inclusion_reason: InclusionReason::ExplicitPin,
            mandatory: false,
        };

        let blocks = content_blocks_for_manifest("workspace-1", &[item], 1);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "pin-1");
    }

    #[test]
    fn fresh_completed_leaf_answer_materializes_a_stable_pinnable_content_block() {
        let data = checkpoint_selection_data(RunStatus::Completed);
        let source = domain::ContextSourceRef::new(
            domain::ContextSourceRefKind::ModelRun,
            "checkpoint-source",
        );

        let block = content_block_for_source(&data, &source)
            .expect("a completed leaf answer is pinnable before any child Receipt references it");
        let expected_hash = domain::sha256_hex(b"checkpoint evidence");

        assert_eq!(block.workspace_id, "workspace-1");
        assert_eq!(block.role, MessageRole::Assistant);
        assert_eq!(block.content, "checkpoint evidence");
        assert_eq!(block.content_hash, expected_hash);
        assert_eq!(block.id, content_block_id("assistant", &expected_hash));
        assert!(
            data.graph.content_block(&block.id).is_none(),
            "the draft transaction, not preview inspection, persists a fresh leaf block"
        );
    }

    #[test]
    fn typed_pin_rejects_a_content_block_owned_by_another_source() {
        let data = checkpoint_selection_data(RunStatus::Completed);
        let source = domain::ContextSourceRef::new(
            domain::ContextSourceRefKind::ModelRun,
            "checkpoint-source",
        );
        let block = content_block_for_source(&data, &source).unwrap();

        let error = validate_pinned_content_identity(&source, "block-from-another-source", &block)
            .expect_err("typed pin identity cannot be mixed across sources");

        assert_eq!(error.code, "context_content_identity_mismatch");
        assert!(error.message.contains("model-run:checkpoint-source"));
    }

    #[test]
    fn persisted_typed_pins_keep_source_block_and_hash_as_one_identity() {
        let content_hash = domain::sha256_hex(b"same answer");
        let content_block_id = content_block_id("assistant", &content_hash);
        let draft = crate::ports::ContextDraft {
            workspace_id: "workspace-1".into(),
            parent_run_id: Some("run-2".into()),
            version: 3,
            items: ["run-1", "run-2"]
                .into_iter()
                .enumerate()
                .map(|(position, run_id)| crate::ports::ContextOverrideItem {
                    position,
                    operation: crate::ports::ContextOverrideOperation::Pin,
                    source_ref: domain::ContextSourceRef::new(
                        domain::ContextSourceRefKind::ModelRun,
                        run_id,
                    ),
                    content_block_id: Some(content_block_id.clone()),
                    content_hash: Some(content_hash.clone()),
                })
                .collect(),
            consumed_by_run_id: None,
            updated_at: 1,
        };

        let overrides = context_overrides_from_draft(&draft).unwrap();

        assert_eq!(overrides.pinned_sources.len(), 2);
        assert_eq!(
            overrides.pinned_sources[0].source_ref.stable_id(),
            "model-run:run-1"
        );
        assert_eq!(
            overrides.pinned_sources[1].source_ref.stable_id(),
            "model-run:run-2"
        );
        assert_eq!(
            overrides.pinned_sources[1].content_block_id,
            content_block_id
        );
        assert_eq!(overrides.pinned_sources[1].content_hash, content_hash);
    }

    #[test]
    fn storage_failure_is_never_reported_as_a_successful_terminal_state() {
        assert!(matches!(
            storage_failure_event("run-1", "disk full".into(), true),
            RunEventView::RunFailed {
                error: RunErrorView { code, .. },
                ..
            } if code == "storage_failure"
        ));
        assert!(matches!(
            storage_failure_event("run-1", "disk full".into(), false),
            RunEventView::PersistenceFailed {
                error: RunErrorView { code, .. },
                ..
            } if code == "storage_failure_uncommitted"
        ));
    }

    #[test]
    fn route_projection_falls_back_to_the_latest_persisted_branch_head() {
        let mut runs = HashMap::new();
        let failed = run_with_status("run-failed", "turn-root", RunStatus::Failed);
        runs.insert(failed.id.clone(), failed);
        let pointers = vec![BranchPointer {
            id: "branch-main".into(),
            workspace_id: "workspace-1".into(),
            name: "Main".into(),
            head_run_id: "run-failed".into(),
            version: 1,
            updated_at: 2,
        }];

        assert_eq!(
            effective_route_run_id(None, &runs, &pointers).as_deref(),
            Some("run-failed")
        );
    }

    #[test]
    fn sibling_edge_is_not_highlighted_just_because_its_source_is_on_the_active_path() {
        let source = run("run-root", "turn-root");
        let active_child = run("run-active", "turn-active");
        let sibling_child = run("run-sibling", "turn-sibling");
        let lineage = BTreeSet::from([source.id.clone(), active_child.id.clone()]);

        assert!(route_edge_is_on_lineage(
            &source.id,
            std::slice::from_ref(&active_child),
            &lineage,
        ));
        assert!(!route_edge_is_on_lineage(
            &source.id,
            std::slice::from_ref(&sibling_child),
            &lineage,
        ));
    }

    #[test]
    fn context_tree_projects_each_model_run_and_only_the_exact_active_path() {
        let root_turn = Turn {
            id: "turn-root".into(),
            workspace_id: "workspace-1".into(),
            parent_run_id: None,
            prompt_markdown: "Root question".into(),
            title: Some("Root".into()),
            created_at: 1,
        };
        let active_turn = Turn {
            id: "turn-active".into(),
            workspace_id: "workspace-1".into(),
            parent_run_id: Some("run-root-a".into()),
            prompt_markdown: "Active child".into(),
            title: None,
            created_at: 2,
        };
        let sibling_turn = Turn {
            id: "turn-sibling".into(),
            workspace_id: "workspace-1".into(),
            parent_run_id: Some("run-root-a".into()),
            prompt_markdown: "Sibling child".into(),
            title: None,
            created_at: 3,
        };
        let graph = domain::ConversationGraph::try_new(
            vec![root_turn, active_turn, sibling_turn],
            vec![
                run("run-root-a", "turn-root"),
                run("run-root-b", "turn-root"),
                run("run-active", "turn-active"),
                run("run-sibling", "turn-sibling"),
            ],
            vec![],
        )
        .expect("fixture graph is valid");
        let cursor = ContextCursorView {
            workspace_id: "workspace-1".into(),
            active_run_id: Some("run-active".into()),
            branch_id: Some("branch-main".into()),
            version: 3,
            updated_at: "2026-07-28T00:00:00Z".into(),
        };
        let branches = [BranchPointer {
            id: "branch-main".into(),
            workspace_id: "workspace-1".into(),
            name: "Main".into(),
            head_run_id: "run-active".into(),
            version: 2,
            updated_at: 3,
        }];
        let mut main_checkpoint = context_checkpoint("checkpoint-main");
        main_checkpoint.anchor_run_id = "run-active".into();
        main_checkpoint.branch_pointer_id = Some("branch-main".into());
        let mut sibling_checkpoint = context_checkpoint("checkpoint-sibling");
        sibling_checkpoint.anchor_run_id = "run-active".into();
        sibling_checkpoint.branch_pointer_id = Some("branch-sibling".into());
        let mut branchless_checkpoint = context_checkpoint("checkpoint-branchless");
        branchless_checkpoint.anchor_run_id = "run-active".into();
        branchless_checkpoint.branch_pointer_id = None;
        let checkpoints = vec![
            context_checkpoint_view(&main_checkpoint),
            context_checkpoint_view(&sibling_checkpoint),
            context_checkpoint_view(&branchless_checkpoint),
        ];

        let projection = context_tree_projection(
            "workspace-1",
            &graph,
            cursor,
            7,
            &branches,
            &BTreeSet::from(["checkpoint-main".into()]),
            checkpoints,
        );

        assert_eq!(projection.draft_version, 7);
        assert_eq!(
            projection.nodes.len(),
            4,
            "Run attempts remain separate nodes"
        );
        assert!(
            projection
                .nodes
                .iter()
                .find(|node| node.run_id == "run-root-a")
                .is_some_and(|node| node.is_on_active_path)
        );
        assert!(
            projection
                .nodes
                .iter()
                .find(|node| node.run_id == "run-root-b")
                .is_some_and(|node| !node.is_on_active_path)
        );
        assert_eq!(
            projection
                .nodes
                .iter()
                .find(|node| node.run_id == "run-active")
                .map(|node| node.checkpoint_ids.as_slice()),
            Some(["checkpoint-main".into(), "checkpoint-branchless".into()].as_slice()),
            "node markers expose only branchless or explicitly inherited checkpoints"
        );
        assert_eq!(
            projection.checkpoints.len(),
            3,
            "the audit catalog remains complete even when a marker is branch-ineligible"
        );
        assert!(
            projection
                .edges
                .iter()
                .find(|edge| edge.target_run_id == "run-sibling")
                .is_some_and(|edge| !edge.is_on_active_path)
        );
    }

    #[test]
    fn context_maintenance_replay_is_independent_of_a_stale_checkpoint_catalog() {
        let request = r#"{"mode":"provider_summary"}"#;
        let completed = context_maintenance(
            "operation-1",
            domain::ContextMaintenanceStatus::Completed,
            request,
        );
        let completed_runs = [completed];

        let replayed = context_maintenance_replay(
            &completed_runs,
            "operation-1",
            domain::ContextCheckpointKind::Compaction,
            request,
        )
        .expect("a completed operation is replayable")
        .expect("the completed maintenance identity is returned");
        assert_eq!(replayed.id, "operation-1");

        let running = context_maintenance(
            "operation-2",
            domain::ContextMaintenanceStatus::Running,
            request,
        );
        let error = context_maintenance_replay(
            &[running],
            "operation-2",
            domain::ContextCheckpointKind::Compaction,
            request,
        )
        .expect_err("an in-flight operation must not be started a second time");
        assert_eq!(error.code, "context_maintenance_in_progress");
        assert!(error.retryable);
    }

    #[test]
    fn checkpoint_view_does_not_invent_provider_provenance_for_manual_summary() {
        let checkpoint = context_checkpoint("manual-operation");
        assert!(checkpoint.provider.is_none());

        let view = context_checkpoint_view(&checkpoint);

        assert!(view.provider.is_none());
        let value = serde_json::to_value(view).unwrap();
        assert_eq!(value["provider"], Value::Null);
        assert!(value.get("providerProfileId").is_none());
        assert!(value.get("providerName").is_none());
        assert!(value.get("model").is_none());
    }

    #[test]
    fn context_maintenance_rejects_reusing_an_operation_id_for_changed_input() {
        let completed = context_maintenance(
            "operation-1",
            domain::ContextMaintenanceStatus::Completed,
            r#"{"mode":"manual","summary":"first"}"#,
        );

        let error = context_maintenance_replay(
            &[completed],
            "operation-1",
            domain::ContextCheckpointKind::Compaction,
            r#"{"mode":"manual","summary":"changed"}"#,
        )
        .expect_err("an idempotency key cannot name two logical operations");
        assert_eq!(error.code, "idempotency_key_reused");
    }

    #[test]
    fn checkpoint_sources_must_be_completed_runs() {
        for (status, machine_status) in [
            (RunStatus::Queued, "queued"),
            (RunStatus::Streaming, "streaming"),
            (RunStatus::Failed, "failed"),
            (RunStatus::Cancelled, "cancelled"),
        ] {
            let data = checkpoint_selection_data(status);

            let error = validate_checkpoint_selection(
                &data,
                "branch-main",
                1,
                "checkpoint-source",
                domain::ContextCheckpointKind::BranchSummary,
                &["checkpoint-source".into()],
                None,
            )
            .expect_err("only completed Runs may become checkpoint evidence");

            assert_eq!(error.code, "checkpoint_source_not_completed");
            assert!(!error.retryable);
            assert_eq!(
                error.details,
                json!({
                    "runId": "checkpoint-source",
                    "status": machine_status,
                })
            );
        }

        let completed = checkpoint_selection_data(RunStatus::Completed);
        assert_eq!(
            validate_checkpoint_selection(
                &completed,
                "branch-main",
                1,
                "checkpoint-source",
                domain::ContextCheckpointKind::BranchSummary,
                &["checkpoint-source".into()],
                None,
            )
            .expect("completed evidence remains eligible")
            .id,
            "branch-main"
        );
    }

    #[test]
    fn branch_summary_rejects_a_compaction_only_kept_boundary() {
        let data = checkpoint_selection_data(RunStatus::Completed);

        let error = validate_checkpoint_selection(
            &data,
            "branch-main",
            1,
            "checkpoint-source",
            domain::ContextCheckpointKind::BranchSummary,
            &["checkpoint-source".into()],
            Some("checkpoint-source"),
        )
        .expect_err("branch summaries never carry a first-kept Run boundary");

        assert_eq!(error.code, "unexpected_checkpoint_boundary");
        assert_eq!(
            error.details,
            json!({
                "kind": "branch-summary",
                "firstKeptRunId": "checkpoint-source",
            })
        );
    }

    #[test]
    fn branch_summary_rejects_sources_after_a_historical_anchor() {
        let turns = vec![
            Turn::root("turn-root", "workspace-1", "root", 1),
            Turn::branch("turn-a", "workspace-1", "run-root", "A", 2),
            Turn::branch("turn-b", "workspace-1", "run-a", "B", 3),
            Turn::branch("turn-c", "workspace-1", "run-b", "C", 4),
        ];
        let runs = vec![
            run("run-root", "turn-root"),
            run("run-a", "turn-a"),
            run("run-b", "turn-b"),
            run("run-c", "turn-c"),
        ];
        let mut data = checkpoint_selection_data(RunStatus::Completed);
        data.graph =
            domain::ConversationGraph::try_new(turns.clone(), runs.clone(), Vec::new()).unwrap();
        data.turns = turns;
        data.runs = runs;
        data.cursor.active_run_id = Some("run-a".into());
        data.draft.parent_run_id = Some("run-a".into());
        data.branch_pointers[0].head_run_id = "run-c".into();

        for sources in [
            vec!["run-b".into()],
            vec!["run-c".into()],
            vec!["run-b".into(), "run-c".into()],
        ] {
            let error = validate_checkpoint_selection(
                &data,
                "branch-main",
                1,
                "run-a",
                domain::ContextCheckpointKind::BranchSummary,
                &sources,
                None,
            )
            .expect_err("descendants of a historical anchor are not checkpoint evidence");
            assert_eq!(error.code, "invalid_checkpoint_source_range");
        }

        for sources in [
            vec!["run-root".into()],
            vec!["run-root".into(), "run-a".into()],
        ] {
            validate_checkpoint_selection(
                &data,
                "branch-main",
                1,
                "run-a",
                domain::ContextCheckpointKind::BranchSummary,
                &sources,
                None,
            )
            .expect("ordered ancestor evidence remains valid");
        }
    }

    #[test]
    fn manual_checkpoint_summary_must_not_be_blank() {
        for summary in ["", " ", "\n\t"] {
            let error = validate_manual_checkpoint_summary(summary)
                .expect_err("blank manual summaries must be rejected before persistence");
            assert_eq!(error.code, "empty_context_summary");
            assert!(!error.retryable);
            assert_eq!(error.details, json!({ "mode": "manual" }));
        }

        validate_manual_checkpoint_summary("Evidence-backed summary")
            .expect("a non-empty manual summary remains valid");
    }

    #[test]
    fn decision_packet_problem_falls_back_to_the_marked_turn_prompt() {
        assert_eq!(
            decision_problem(
                "尚未设置工作区目标",
                ["比较两个可审查的技术结论"].into_iter(),
                Some("备用问题")
            ),
            "比较两个可审查的技术结论"
        );
        assert_eq!(
            decision_problem(
                "降低迁移风险",
                ["比较两个可审查的技术结论"].into_iter(),
                None
            ),
            "降低迁移风险"
        );
    }

    #[test]
    fn decision_packet_renders_checkpoint_sources_boundary_hash_and_provider_snapshot() {
        let provider = context_checkpoint_provider_snapshot_view(&domain::ProviderSnapshot {
            profile_id: "profile-1".into(),
            provider_id: Some("openai".into()),
            template_revision: Some(4),
            provider_name: "Review provider".into(),
            dialect: domain::ProviderDialect::OpenAiCompatible,
            stream_protocol: Some(domain::StreamProtocol::OpenAiSse),
            auth_placement: Some(domain::AuthPlacement::BearerHeader),
            auth_header_name: Some("Authorization".into()),
            additional_headers: BTreeMap::new(),
            base_url: "https://models.example.com/v1".into(),
            model: "review-model".into(),
            parameters: BTreeMap::from([("temperature".into(), "0".into())]),
        });
        let markdown = decision_packet_checkpoint_provenance(&[ContextCheckpointProvenanceView {
            checkpoint_id: "checkpoint-1".into(),
            maintenance_run_id: "maintenance-1".into(),
            kind: ContextCheckpointKindView::Compaction,
            branch_id: Some("branch-main".into()),
            branch_version: Some(7),
            anchor_run_id: "run-3".into(),
            first_kept_run_id: Some("run-2".into()),
            summary_content_block_id: "block-summary".into(),
            source_run_ids: vec!["run-0".into(), "run-1".into()],
            source_hash: "source-hash".into(),
            provider: Some(provider),
        }]);

        for evidence in [
            "checkpoint-1",
            "maintenance-1",
            "run-0",
            "run-1",
            "run-2",
            "source-hash",
            "branch-main",
            "\"profileId\":\"profile-1\"",
            "\"model\":\"review-model\"",
        ] {
            assert!(
                markdown.contains(evidence),
                "Decision Packet omitted {evidence}: {markdown}"
            );
        }
    }

    #[test]
    fn rust_authority_enables_implemented_protocols_and_rejects_unavailable_templates() {
        assert_eq!(
            runnable_template("missing")
                .expect_err("unknown templates must be rejected")
                .code,
            "unknown_provider_template"
        );
        assert_eq!(
            runnable_template("azure-openai")
                .expect_err("templates cannot run before their protocol exists")
                .code,
            "provider_protocol_unavailable"
        );
        assert_eq!(
            runnable_template("openrouter").unwrap().1,
            domain::ProviderDialect::OpenAiCompatible
        );

        assert_eq!(
            runnable_template("anthropic").unwrap().1,
            domain::ProviderDialect::Anthropic
        );
        assert_eq!(
            runnable_template("google").unwrap().1,
            domain::ProviderDialect::GoogleGenerativeAi
        );

        let anthropic_profile = ProviderProfile {
            id: "profile-anthropic".into(),
            provider_id: "anthropic".into(),
            name: "Anthropic".into(),
            dialect: domain::ProviderDialect::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            model: "claude".into(),
            parameters: BTreeMap::new(),
            created_at: 1,
            updated_at: 1,
        };
        let snapshot = domain_provider_snapshot(&anthropic_profile)
            .expect("an implemented protocol must resolve to a frozen Provider snapshot");
        assert_eq!(
            snapshot.stream_protocol,
            Some(domain::StreamProtocol::AnthropicSse)
        );
        assert_eq!(snapshot.template_revision, Some(3));
    }

    #[test]
    fn provider_model_source_requires_exactly_one_profile_or_draft() {
        let neither = provider_model_source(ListProviderModelsInput {
            provider_profile_id: None,
            draft: None,
        })
        .expect_err("a discovery source is required");
        assert_eq!(neither.code, "invalid_provider_model_source");

        let both = provider_model_source(ListProviderModelsInput {
            provider_profile_id: Some("profile-1".into()),
            draft: Some(ProviderModelDraftInput {
                provider_id: "openai".into(),
                base_url: "https://api.openai.com/v1".into(),
            }),
        })
        .expect_err("profile and draft are mutually exclusive");
        assert_eq!(both.code, "invalid_provider_model_source");

        assert!(matches!(
            provider_model_source(ListProviderModelsInput {
                provider_profile_id: None,
                draft: Some(ProviderModelDraftInput {
                    provider_id: "google".into(),
                    base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
                }),
            })
            .expect("a complete draft is valid"),
            ProviderModelSource::Draft(_)
        ));
    }

    #[test]
    fn model_catalog_resolution_keeps_discovery_and_runtime_protocols_typed() {
        let anthropic = provider_model_query("anthropic", "https://api.anthropic.com")
            .expect("Anthropic models are discovered from the authenticated API");
        assert_eq!(anthropic.catalog, ProviderModelCatalogKind::Anthropic);
        assert_eq!(
            anthropic.target.credential_placement,
            CredentialPlacement::Header("x-api-key".into())
        );
        assert_eq!(
            anthropic
                .target
                .additional_headers
                .get("anthropic-version")
                .map(String::as_str),
            Some("2023-06-01")
        );
        assert_eq!(
            runnable_template("anthropic").unwrap().1,
            domain::ProviderDialect::Anthropic
        );

        let google =
            provider_model_query("google", "https://generativelanguage.googleapis.com/v1beta")
                .expect("Google model metadata is remotely discoverable");
        assert_eq!(google.catalog, ProviderModelCatalogKind::Google);
        assert_eq!(
            google.target.credential_placement,
            CredentialPlacement::Header("x-goog-api-key".into())
        );
        assert_eq!(google.target.dialect, ProviderDialect::GoogleGenerativeAi);
        assert_eq!(
            runnable_template("google").unwrap().1,
            domain::ProviderDialect::GoogleGenerativeAi
        );
    }

    #[test]
    fn model_catalog_resolution_rejects_unknown_and_azure_templates_explicitly() {
        assert_eq!(
            provider_model_query("missing", "https://models.example.com")
                .expect_err("unknown templates have no authoritative catalog")
                .code,
            "unknown_provider_template"
        );
        assert_eq!(
            provider_model_query(
                "azure-openai",
                "https://resource.openai.azure.com/openai/v1",
            )
            .expect_err("Azure deployments do not expose a portable model catalog")
            .code,
            "provider_model_discovery_unsupported"
        );
    }

    #[test]
    fn manual_catalog_templates_allow_chat_but_reject_metadata_probes() {
        for provider_id in ["qwen-beijing", "qwen-singapore", "zai"] {
            let (template, dialect) = runnable_template(provider_id).unwrap();
            assert_eq!(dialect, domain::ProviderDialect::OpenAiCompatible);
            assert_eq!(template.protocol.models_endpoint, None);
            assert_eq!(
                provider_model_query(provider_id, template.default_base_url)
                    .unwrap_err()
                    .code,
                "provider_model_discovery_unsupported"
            );
        }
        let gateway = provider_model_query("omp-gateway", "http://127.0.0.1:4000/v1").unwrap();
        assert_eq!(gateway.catalog, ProviderModelCatalogKind::OpenAi);
        assert_eq!(
            gateway.target.credential_placement,
            CredentialPlacement::BearerHeader
        );
    }

    #[test]
    fn provider_parameters_reject_unknown_names_and_invalid_shapes() {
        let invalid = [
            BTreeMap::from([("unknown".into(), json!(true))]),
            BTreeMap::from([("temperature".into(), json!("0.7"))]),
            BTreeMap::from([("top_p".into(), json!(false))]),
            BTreeMap::from([("max_output_tokens".into(), json!(0))]),
            BTreeMap::from([("max_output_tokens".into(), json!(1.5))]),
            BTreeMap::from([("max_output_tokens".into(), json!(u64::from(u32::MAX) + 1))]),
            BTreeMap::from([("stop".into(), json!(["END", 2]))]),
            BTreeMap::from([(INTERNAL_DEFAULT_KEY.into(), json!(true))]),
        ];

        for parameters in invalid {
            let error = normalize_provider_parameter_values(parameters)
                .expect_err("invalid Provider parameters must be rejected");
            assert_eq!(error.code, "invalid_provider_parameters");
            assert!(!error.retryable);
        }

        let persisted = ProviderProfile {
            id: "profile-legacy".into(),
            provider_id: "openai".into(),
            name: "Legacy".into(),
            dialect: domain::ProviderDialect::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".into(),
            model: "model-1".into(),
            parameters: BTreeMap::from([("temperature".into(), "not-json".into())]),
            created_at: 1,
            updated_at: 1,
        };
        assert_eq!(
            domain_provider_snapshot(&persisted)
                .expect_err("invalid persisted values must not reach a Receipt or request")
                .code,
            "invalid_provider_parameters"
        );
    }

    #[test]
    fn normalized_provider_parameters_are_the_exact_canonical_request_values() {
        let effective = normalize_provider_parameter_values(BTreeMap::from([
            ("temperature".into(), json!(0.123_456_789_f64)),
            ("top_p".into(), json!(0.8)),
            ("max_output_tokens".into(), json!(4096)),
            ("stop".into(), json!(["END", "STOP"])),
        ]))
        .unwrap();
        let stored = effective.stored_values();
        let mut profile_parameters = stored.clone();
        profile_parameters.insert(INTERNAL_DEFAULT_KEY.into(), "true".into());
        let profile = ProviderProfile {
            id: "profile-1".into(),
            provider_id: "openai".into(),
            name: "OpenAI".into(),
            dialect: domain::ProviderDialect::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".into(),
            model: "model-1".into(),
            parameters: profile_parameters,
            created_at: 1,
            updated_at: 1,
        };
        let receipt_provider = domain_provider_snapshot(&profile).unwrap();
        let persisted_effective = effective_provider_parameters(&profile).unwrap();
        let request = canonical_provider_request(
            "run-1".into(),
            "model-1".into(),
            vec![CanonicalMessage {
                role: ProviderMessageRole::User,
                content: "question".into(),
            }],
            &persisted_effective,
        );

        assert_eq!(request.temperature, Some(0.123_456_79_f32));
        assert_eq!(request.top_p, Some(0.8_f32));
        assert_eq!(request.max_output_tokens, Some(4096));
        assert_eq!(request.stop, ["END", "STOP"]);
        assert_eq!(receipt_provider.parameters, stored);
        assert_eq!(
            stored.get("temperature"),
            Some(&json!(request.temperature.unwrap()).to_string())
        );
        assert_eq!(
            stored.get("top_p"),
            Some(&json!(request.top_p.unwrap()).to_string())
        );
        assert_eq!(stored.get("max_output_tokens"), Some(&"4096".into()));
        assert_eq!(stored.get("stop"), Some(&r#"["END","STOP"]"#.into()));
    }

    #[test]
    fn anthropic_default_max_tokens_is_frozen_before_receipt_hash_and_request() {
        let profile = ProviderProfile {
            id: "profile-anthropic".into(),
            provider_id: "anthropic".into(),
            name: "Anthropic".into(),
            dialect: domain::ProviderDialect::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-5".into(),
            parameters: BTreeMap::new(),
            created_at: 1,
            updated_at: 1,
        };
        let provider = domain_provider_snapshot(&profile).unwrap();
        assert_eq!(
            provider.parameters.get("max_output_tokens"),
            Some(&DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS.to_string())
        );

        let effective = effective_provider_parameters(&profile).unwrap();
        let request = canonical_provider_request(
            "run-anthropic".into(),
            profile.model.clone(),
            vec![CanonicalMessage {
                role: ProviderMessageRole::User,
                content: "question".into(),
            }],
            &effective,
        );
        assert_eq!(
            request.max_output_tokens,
            Some(DEFAULT_ANTHROPIC_MAX_OUTPUT_TOKENS)
        );

        let graph = domain::ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
        let compiler = ContextCompiler::new(ContextPolicy::default());
        let preview = |provider| {
            compiler
                .inspect(
                    &graph,
                    ContextCompileRequest {
                        workspace_id: "workspace".into(),
                        system_prompt: "policy".into(),
                        parent_run_id: None,
                        current_prompt: "question".into(),
                        overrides: domain::ContextOverrides::default(),
                        provider: Some(provider),
                    },
                )
                .unwrap()
        };
        let default_hash = preview(provider.clone()).preview_hash;
        let mut explicit_provider = provider;
        explicit_provider
            .parameters
            .insert("max_output_tokens".into(), "8192".into());
        assert_ne!(default_hash, preview(explicit_provider).preview_hash);
    }

    #[test]
    fn empty_stop_list_is_normalized_to_an_absent_effective_parameter() {
        let effective =
            normalize_provider_parameter_values(BTreeMap::from([("stop".into(), json!([]))]))
                .unwrap();

        assert!(effective.stop.is_empty());
        assert!(!effective.stored_values().contains_key("stop"));
    }
}
