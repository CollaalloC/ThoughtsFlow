use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    domain::{
        self, ContextCompileRequest, ContextCompiler, ContextPolicy, ContextSourceKind,
        ContextWarning, ConversationGraph, InclusionReason, MessageRole, ModelRun, RunDraft,
        RunStateSnapshot, RunStatus,
    },
    infrastructure::{
        provider::{ReqwestProviderGateway, validate_base_url},
        sqlite::{
            BranchPointerRecord, ContentBlockRecord, ContextManifestRecord, ContextSnapshotRecord,
            DecisionMarkRecord, ModelRunRecord, ProviderProfileRecord, RepositoryError,
            RunCheckpoint, RunContextItemRecord, RunFinish, RunStartBundle, RunStatusRecord,
            SqliteRepository, StoredContextItem, StoredRunReceipt, TurnRecord, ViewStateRecord,
            WorkspaceRecord,
        },
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, MessageRole as ProviderMessageRole, ProviderDialect,
        ProviderGateway, ProviderInvocation, ProviderTarget, RunEvent, SessionCredential, Usage,
    },
};

use super::*;

const DEFAULT_PROVIDER_ID: &str = "provider-local-ollama";
const DEFAULT_SYSTEM_PROMPT: &str = "You are a careful technical reasoning partner. Make assumptions explicit and preserve competing options.";
const DEFAULT_MAX_CONTEXT_CHARS: usize = 100_000;
const INTERNAL_DEFAULT_KEY: &str = "_thoughsflowIsDefault";

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct OverrideKey {
    workspace_id: String,
    parent_run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
struct OverrideItem {
    included: bool,
    pinned: bool,
}

pub struct DefaultApplicationBackend {
    repository: SqliteRepository,
    provider: Arc<dyn ProviderGateway>,
    connection_client: reqwest::Client,
    compiler: ContextCompiler,
    run_registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    context_overrides: Arc<Mutex<HashMap<OverrideKey, HashMap<String, OverrideItem>>>>,
    export_root: PathBuf,
}

impl DefaultApplicationBackend {
    pub async fn initialize(database_path: PathBuf, export_root: PathBuf) -> AppResult<Self> {
        let repository = SqliteRepository::connect(database_path)
            .await
            .map_err(repository_error)?;
        repository
            .recover_interrupted_runs(now_millis())
            .await
            .map_err(repository_error)?;

        let connection_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent("ThoughsFlow/0.1")
            .build()
            .map_err(|error| AppError::internal("http_client_failed", error.to_string()))?;
        let provider = Arc::new(
            ReqwestProviderGateway::with_defaults()
                .map_err(|error| AppError::internal(error.code(), error.to_string()))?,
        );
        let backend = Self {
            repository,
            provider,
            connection_client,
            compiler: ContextCompiler::new(ContextPolicy {
                compiler_version: domain::CONTEXT_COMPILER_VERSION.into(),
                max_chars: DEFAULT_MAX_CONTEXT_CHARS,
            }),
            run_registry: Arc::new(Mutex::new(HashMap::new())),
            context_overrides: Arc::new(Mutex::new(HashMap::new())),
            export_root,
        };
        backend.ensure_default_provider().await?;
        Ok(backend)
    }

    async fn ensure_default_provider(&self) -> AppResult<()> {
        if !self
            .repository
            .list_provider_profiles()
            .await
            .map_err(repository_error)?
            .is_empty()
        {
            return Ok(());
        }
        let now = now_millis();
        self.repository
            .save_provider_profile(&ProviderProfileRecord {
                id: DEFAULT_PROVIDER_ID.into(),
                name: "Local Ollama".into(),
                dialect: "ollama_chat".into(),
                base_url: "http://127.0.0.1:11434".into(),
                default_model: "qwen3".into(),
                parameters_json: json!({ INTERNAL_DEFAULT_KEY: true }).to_string(),
                created_at: now,
                updated_at: now,
            })
            .await
            .map_err(repository_error)?;
        Ok(())
    }

    fn override_key(workspace_id: &str, parent_run_id: Option<&str>) -> OverrideKey {
        OverrideKey {
            workspace_id: workspace_id.into(),
            parent_run_id: parent_run_id.map(str::to_owned),
        }
    }

    fn overrides_for(&self, key: &OverrideKey) -> AppResult<HashMap<String, OverrideItem>> {
        self.context_overrides
            .lock()
            .map_err(|_| {
                AppError::internal("context_override_lock", "Context overrides are unavailable")
            })
            .map(|overrides| overrides.get(key).cloned().unwrap_or_default())
    }

    fn compiler_overrides(items: &HashMap<String, OverrideItem>) -> domain::ContextOverrides {
        domain::ContextOverrides {
            pinned_source_ids: items
                .iter()
                .filter_map(|(id, item)| item.pinned.then_some(id.clone()))
                .collect(),
            excluded_source_ids: items
                .iter()
                .filter_map(|(id, item)| (!item.included).then_some(id.clone()))
                .collect(),
        }
    }

    async fn inspect_context_impl(&self, input: InspectContextInput) -> AppResult<ContextPreview> {
        let workspace = self
            .repository
            .get_workspace(&input.workspace_id)
            .await
            .map_err(repository_error)?;
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_error)?;
        let graph = load_graph(&self.repository, &input.workspace_id).await?;
        let key = Self::override_key(&input.workspace_id, input.parent_run_id.as_deref());
        let override_items = self.overrides_for(&key)?;
        let request = ContextCompileRequest {
            workspace_id: input.workspace_id.clone(),
            system_prompt: workspace.system_prompt,
            parent_run_id: input.parent_run_id.clone(),
            current_prompt: input.prompt,
            overrides: Self::compiler_overrides(&override_items),
            provider: Some(domain_provider_snapshot(&profile)?),
        };
        let actual = self
            .compiler
            .inspect(&graph, request.clone())
            .map_err(domain_error)?;
        let base = self
            .compiler
            .inspect(
                &graph,
                ContextCompileRequest {
                    overrides: domain::ContextOverrides::default(),
                    ..request
                },
            )
            .map_err(domain_error)?;
        let mut items = actual
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
        for item in base.manifest.items.iter().filter(|item| {
            item.source_id
                .as_ref()
                .and_then(|id| override_items.get(id))
                .map(|override_item| !override_item.included)
                .unwrap_or(false)
        }) {
            let mut excluded = context_item_view(item, false, false);
            excluded.ordinal = u32::try_from(items.len()).unwrap_or(u32::MAX);
            excluded.reason = "Explicitly excluded from the next request".into();
            items.push(excluded);
        }
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
            model: profile.default_model,
            base_url: profile.base_url,
            items,
        })
    }
}

fn spawn_run(
    repository: SqliteRepository,
    provider: Arc<dyn ProviderGateway>,
    registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    invocation: ProviderInvocation,
    cancellation: CancellationToken,
    events: Arc<dyn RunEventSink>,
) {
    let run_id = invocation.request.run_id.clone();
    tauri::async_runtime::spawn(async move {
        let (sender, receiver) = mpsc::channel(128);
        let provider_cancellation = cancellation.clone();
        let persistence_cancellation = cancellation.clone();
        let provider_task = tauri::async_runtime::spawn(async move {
            provider
                .stream(invocation, provider_cancellation, sender)
                .await
        });
        run_event_loop(
            &repository,
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
    repository: &SqliteRepository,
    run_id: &str,
    mut receiver: mpsc::Receiver<RunEvent>,
    sink: Arc<dyn RunEventSink>,
    provider_task: tauri::async_runtime::JoinHandle<
        Result<(), crate::ports::provider::ProviderError>,
    >,
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
                    Ok(()) => {
                        bytes_since_checkpoint = 0;
                        send_checkpoint_saved(run_id, &sink, output.len());
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
                            Ok(()) => send_checkpoint_saved(run_id, &sink, output.len()),
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
                        let finish = RunFinish {
                            status: RunStatusRecord::Completed,
                            output_markdown: output.clone(),
                            reasoning_markdown: reasoning.clone(),
                            usage_json: usage.as_ref().and_then(|value| serde_json::to_string(value).ok()),
                            error_json: None,
                            finished_at: now_millis(),
                        };
                        match repository.finish_run(run_id, &finish).await {
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
                        let error_json = json!({
                            "code": code,
                            "message": message,
                            "retryable": retryable,
                            "status": status,
                        })
                        .to_string();
                        let finish = RunFinish {
                            status: RunStatusRecord::Failed,
                            output_markdown: output.clone(),
                            reasoning_markdown: reasoning.clone(),
                            usage_json: usage.as_ref().and_then(|value| serde_json::to_string(value).ok()),
                            error_json: Some(error_json),
                            finished_at: now_millis(),
                        };
                        match repository.finish_run(run_id, &finish).await {
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
                        let finish = RunFinish {
                            status: RunStatusRecord::Cancelled,
                            output_markdown: output.clone(),
                            reasoning_markdown: reasoning.clone(),
                            usage_json: usage.as_ref().and_then(|value| serde_json::to_string(value).ok()),
                            error_json: None,
                            finished_at: now_millis(),
                        };
                        match repository.finish_run(run_id, &finish).await {
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
                        Ok(()) => {
                            bytes_since_checkpoint = 0;
                            send_checkpoint_saved(run_id, &sink, output.len());
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
        let finish = RunFinish {
            status: RunStatusRecord::Failed,
            output_markdown: output,
            reasoning_markdown: reasoning,
            usage_json: usage
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok()),
            error_json: Some(json!({"code":"provider_stream_ended","message":message}).to_string()),
            finished_at: now_millis(),
        };
        match repository.finish_run(run_id, &finish).await {
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
                    &finish.output_markdown,
                    &finish.reasoning_markdown,
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
    repository: &SqliteRepository,
    run_id: &str,
    output: &str,
    reasoning: &str,
    usage: Option<&Usage>,
    sink: &Arc<dyn RunEventSink>,
    error: &RepositoryError,
) {
    let message = format!("Run output could not be persisted: {error}");
    let finish = RunFinish {
        status: RunStatusRecord::Failed,
        output_markdown: output.into(),
        reasoning_markdown: reasoning.into(),
        usage_json: usage.and_then(|value| serde_json::to_string(value).ok()),
        error_json: Some(json!({"code":"storage_failure","message":message}).to_string()),
        finished_at: now_millis(),
    };
    let committed = repository.finish_run(run_id, &finish).await.is_ok();
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
    repository: &SqliteRepository,
    run_id: &str,
    output: &str,
    reasoning: &str,
    usage: Option<&Usage>,
) -> Result<(), RepositoryError> {
    repository
        .checkpoint_run(
            run_id,
            &RunCheckpoint {
                output_markdown: output.into(),
                reasoning_markdown: reasoning.into(),
                usage_json: usage.and_then(|value| serde_json::to_string(value).ok()),
                checkpointed_at: now_millis(),
            },
        )
        .await
}

async fn load_graph(
    repository: &SqliteRepository,
    workspace_id: &str,
) -> AppResult<ConversationGraph> {
    let turn_records = repository
        .list_turns(workspace_id)
        .await
        .map_err(repository_error)?;
    let mut turns = Vec::with_capacity(turn_records.len());
    let mut runs = Vec::new();
    let mut blocks = Vec::new();
    for turn in turn_records {
        blocks.push(domain::ContentBlock {
            id: turn.id.clone(),
            workspace_id: turn.workspace_id.clone(),
            role: MessageRole::User,
            content_hash: domain::sha256_hex(turn.prompt_markdown.as_bytes()),
            content: turn.prompt_markdown.clone(),
            created_at: turn.created_at,
        });
        turns.push(domain::Turn {
            id: turn.id.clone(),
            workspace_id: turn.workspace_id,
            parent_run_id: turn.parent_run_id,
            prompt_markdown: turn.prompt_markdown,
            title: (!turn.title.is_empty()).then_some(turn.title),
            created_at: turn.created_at,
        });
        for record in repository
            .list_runs_for_turn(&turn.id)
            .await
            .map_err(repository_error)?
        {
            let run = domain_run(&record)?;
            blocks.push(domain::ContentBlock {
                id: record.id.clone(),
                workspace_id: record.workspace_id,
                role: MessageRole::Assistant,
                content_hash: domain::sha256_hex(record.output_markdown.as_bytes()),
                content: record.output_markdown,
                created_at: record.created_at,
            });
            runs.push(run);
        }
    }
    ConversationGraph::try_new(turns, runs, blocks).map_err(domain_error)
}

fn domain_run(record: &ModelRunRecord) -> AppResult<ModelRun> {
    let usage = record
        .usage_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Usage>(json).ok())
        .map(|usage| {
            domain::RunUsage::new(
                usage.prompt_tokens.unwrap_or(0),
                usage.completion_tokens.unwrap_or(0),
            )
        });
    let error = record.error_json.as_deref().map(|value| {
        serde_json::from_str::<Value>(value)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| value.to_owned())
    });
    ModelRun::rehydrate(
        RunDraft {
            id: record.id.clone(),
            turn_id: record.turn_id.clone(),
            provider_profile_id: record.provider_profile_id.clone(),
            model: record.model.clone(),
            created_at: record.created_at,
        },
        RunStateSnapshot {
            status: domain_run_status(record.status),
            output_markdown: record.output_markdown.clone(),
            reasoning_markdown: record.reasoning_markdown.clone(),
            error,
            usage,
            started_at: record.started_at,
            checkpointed_at: record.checkpointed_at,
            finished_at: record.finished_at,
        },
    )
    .map_err(domain_error)
}

fn exact_retry_turn_id(original: &ModelRunRecord) -> &str {
    &original.turn_id
}

fn content_blocks_for_manifest(
    items: &[domain::RunContextItem],
    now: i64,
) -> Vec<ContentBlockRecord> {
    items
        .iter()
        .map(|item| {
            let id = content_block_id(message_role_name(item.role), &item.content_hash);
            (
                id.clone(),
                ContentBlockRecord {
                    id,
                    role: message_role_name(item.role).into(),
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

fn workspace_view(record: WorkspaceRecord) -> WorkspaceSummary {
    WorkspaceSummary {
        id: record.id,
        name: record.title,
        goal: record.system_prompt,
        archived: record.archived_at.is_some(),
        created_at: timestamp_view(record.created_at),
        updated_at: timestamp_view(record.updated_at),
    }
}

fn run_view(
    record: &ModelRunRecord,
    profile: Option<&ProviderProfileRecord>,
) -> AppResult<RunView> {
    let provider_snapshot =
        serde_json::from_str::<Value>(&record.provider_snapshot_json).map_err(json_error)?;
    let usage = record
        .usage_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Usage>(value).ok())
        .map(|usage| usage_map(&usage));
    let error = record.error_json.as_deref().map(|value| {
        let parsed = serde_json::from_str::<Value>(value).unwrap_or(Value::Null);
        RunErrorView {
            code: parsed
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("run_failed")
                .into(),
            message: parsed
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(value)
                .into(),
            retryable: parsed
                .get("retryable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            status: parsed
                .get("status")
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok()),
        }
    });
    Ok(RunView {
        id: record.id.clone(),
        turn_id: record.turn_id.clone(),
        status: run_status_view(record.status),
        output: record.output_markdown.clone(),
        reasoning: (!record.reasoning_markdown.is_empty())
            .then_some(record.reasoning_markdown.clone()),
        provider_profile_id: record.provider_profile_id.clone().unwrap_or_default(),
        provider_name: profile
            .map(|profile| profile.name.clone())
            .or_else(|| {
                provider_snapshot
                    .get("providerName")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Unknown provider".into()),
        model: record.model.clone(),
        base_url: profile
            .map(|profile| profile.base_url.clone())
            .or_else(|| {
                provider_snapshot
                    .get("baseUrl")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default(),
        created_at: timestamp_view(record.created_at),
        completed_at: record.finished_at.map(timestamp_view),
        usage,
        error,
    })
}

fn receipt_view(receipt: StoredRunReceipt) -> AppResult<RunSnapshotView> {
    let parameters =
        serde_json::from_str::<BTreeMap<String, Value>>(&receipt.snapshot.parameters_json)
            .map_err(json_error)?;
    Ok(RunSnapshotView {
        id: receipt.snapshot.id,
        run_id: receipt.snapshot.run_id,
        canonical_hash: receipt.snapshot.canonical_hash,
        created_at: timestamp_view(receipt.snapshot.created_at),
        provider_name: receipt.snapshot.provider,
        model: receipt.snapshot.model,
        base_url: receipt.snapshot.base_url,
        parameters,
        items: receipt
            .items
            .iter()
            .map(|item| stored_context_item_view(item, false))
            .collect::<AppResult<Vec<_>>>()?,
    })
}

fn stored_context_item_view(item: &StoredContextItem, pinned: bool) -> AppResult<ContextItemView> {
    Ok(ContextItemView {
        id: item
            .source_id
            .clone()
            .unwrap_or_else(|| format!("manifest-item-{}", item.position)),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: message_role_view(&item.role)?,
        label: source_label(&item.source_kind),
        source: item.source_kind.clone(),
        content: item.content.clone(),
        reason: item.inclusion_reason.clone(),
        estimated_tokens: chars_to_tokens(item.content.chars().count()),
        included: true,
        pinned,
    })
}

fn context_item_view(
    item: &domain::RunContextItem,
    included: bool,
    pinned: bool,
) -> ContextItemView {
    ContextItemView {
        id: item
            .source_id
            .clone()
            .unwrap_or_else(|| format!("current-prompt-{}", item.position)),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: domain_message_role_view(item.role),
        label: source_label(context_source_name(item.source_kind)),
        source: context_source_name(item.source_kind).into(),
        content: item.content.clone(),
        reason: inclusion_reason_name(item.inclusion_reason).into(),
        estimated_tokens: chars_to_tokens(item.content.chars().count()),
        included,
        pinned,
    }
}

fn provider_profile_view(record: ProviderProfileRecord) -> AppResult<ProviderProfileView> {
    let mut parameters = serde_json::from_str::<BTreeMap<String, Value>>(&record.parameters_json)
        .map_err(json_error)?;
    let is_default = parameters
        .remove(INTERNAL_DEFAULT_KEY)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(ProviderProfileView {
        id: record.id,
        name: record.name,
        dialect: match record.dialect.as_str() {
            "openai_chat_completions" => ProviderDialectView::OpenaiCompatible,
            "ollama_chat" => ProviderDialectView::Ollama,
            other => {
                return Err(AppError::internal(
                    "invalid_provider_dialect",
                    format!("Unknown persisted provider dialect: {other}"),
                ));
            }
        },
        base_url: record.base_url,
        model: record.default_model,
        is_default,
        parameters: (!parameters.is_empty()).then_some(parameters),
    })
}

fn domain_provider_snapshot(record: &ProviderProfileRecord) -> AppResult<domain::ProviderSnapshot> {
    let parameters = provider_parameters(record)?
        .into_iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            (key, value)
        })
        .collect();
    Ok(domain::ProviderSnapshot {
        profile_id: record.id.clone(),
        provider_name: record.name.clone(),
        dialect: match record.dialect.as_str() {
            "openai_chat_completions" => domain::ProviderDialect::OpenAiCompatible,
            "ollama_chat" => domain::ProviderDialect::Ollama,
            other => {
                return Err(AppError::internal(
                    "invalid_provider_dialect",
                    format!("Unknown persisted provider dialect: {other}"),
                ));
            }
        },
        base_url: record.base_url.clone(),
        model: record.default_model.clone(),
        parameters,
    })
}

fn provider_parameters(record: &ProviderProfileRecord) -> AppResult<Map<String, Value>> {
    let mut parameters =
        serde_json::from_str::<Map<String, Value>>(&record.parameters_json).map_err(json_error)?;
    parameters.remove(INTERNAL_DEFAULT_KEY);
    Ok(parameters)
}

fn decision_view(record: DecisionMarkRecord) -> AppResult<DecisionMarkView> {
    Ok(DecisionMarkView {
        id: record.id,
        workspace_id: record.workspace_id,
        run_id: record.run_id,
        status: match record.status.as_str() {
            "adopted" => DecisionStatusView::Accepted,
            "rejected" => DecisionStatusView::Rejected,
            "needs_validation" => DecisionStatusView::ToVerify,
            other => {
                return Err(AppError::internal(
                    "invalid_decision_status",
                    format!("Unknown persisted decision status: {other}"),
                ));
            }
        },
        reason: record.reason,
        created_at: timestamp_view(record.created_at),
    })
}

fn context_diff_item(item: &StoredContextItem) -> ContextDiffItemView {
    ContextDiffItemView {
        id: context_item_identity(item),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: message_role_view(&item.role).unwrap_or(MessageRoleView::User),
        source: item.source_kind.clone(),
        preview: item.content.chars().take(180).collect(),
    }
}

fn context_item_identity(item: &StoredContextItem) -> String {
    format!(
        "{}:{}:{}:{}",
        item.content_hash,
        item.source_kind,
        item.source_id.as_deref().unwrap_or("current"),
        item.position,
    )
}

fn exact_lineage_ids(
    current_run_id: Option<&str>,
    turns: &[TurnRecord],
    runs: &HashMap<String, ModelRunRecord>,
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

fn effective_route_run_id(
    requested_run_id: Option<&str>,
    runs: &HashMap<String, ModelRunRecord>,
    branch_pointers: &[BranchPointerRecord],
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

fn decision_status_record(status: DecisionStatusView) -> &'static str {
    match status {
        DecisionStatusView::Accepted => "adopted",
        DecisionStatusView::Rejected => "rejected",
        DecisionStatusView::ToVerify => "needs_validation",
    }
}

fn provider_dialect(value: &str) -> AppResult<ProviderDialect> {
    match value {
        "openai_chat_completions" => Ok(ProviderDialect::OpenAiChatCompletions),
        "ollama_chat" => Ok(ProviderDialect::OllamaChat),
        other => Err(AppError::internal(
            "invalid_provider_dialect",
            format!("Unknown persisted provider dialect: {other}"),
        )),
    }
}

fn provider_message_role(role: MessageRole) -> ProviderMessageRole {
    match role {
        MessageRole::System => ProviderMessageRole::System,
        MessageRole::User => ProviderMessageRole::User,
        MessageRole::Assistant => ProviderMessageRole::Assistant,
    }
}

fn provider_message_role_from_name(role: &str) -> AppResult<ProviderMessageRole> {
    match role {
        "system" => Ok(ProviderMessageRole::System),
        "user" => Ok(ProviderMessageRole::User),
        "assistant" => Ok(ProviderMessageRole::Assistant),
        other => Err(AppError::internal(
            "invalid_message_role",
            format!("Unknown persisted message role: {other}"),
        )),
    }
}

fn message_role_view(role: &str) -> AppResult<MessageRoleView> {
    Ok(match provider_message_role_from_name(role)? {
        ProviderMessageRole::System => MessageRoleView::System,
        ProviderMessageRole::User => MessageRoleView::User,
        ProviderMessageRole::Assistant => MessageRoleView::Assistant,
    })
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
        ContextSourceKind::CurrentPrompt => "current_prompt",
    }
}

fn inclusion_reason_name(reason: InclusionReason) -> &'static str {
    match reason {
        InclusionReason::SystemPolicy => "system_policy",
        InclusionReason::ExactAncestorPath => "exact_ancestor_path",
        InclusionReason::ExplicitPin => "explicit_pin",
        InclusionReason::CurrentPrompt => "current_prompt",
    }
}

fn source_label(source: &str) -> String {
    match source {
        "system" => "System prompt",
        "turn_prompt" => "Ancestor prompt",
        "model_run" => "Exact ancestor answer",
        "pinned" => "Pinned context",
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

fn run_status_view(status: RunStatusRecord) -> RunStatusView {
    match status {
        RunStatusRecord::Queued => RunStatusView::Pending,
        RunStatusRecord::Connecting => RunStatusView::Connecting,
        RunStatusRecord::Streaming => RunStatusView::Streaming,
        RunStatusRecord::Completed => RunStatusView::Completed,
        RunStatusRecord::Cancelled => RunStatusView::Cancelled,
        RunStatusRecord::Failed => RunStatusView::Failed,
        RunStatusRecord::Interrupted => RunStatusView::Interrupted,
    }
}

fn domain_run_status(status: RunStatusRecord) -> RunStatus {
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

fn parameter_f32(parameters: &Map<String, Value>, name: &str) -> Option<f32> {
    parameters
        .get(name)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn parameter_u32(parameters: &Map<String, Value>, name: &str) -> Option<u32> {
    parameters
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn parameter_strings(parameters: &Map<String, Value>, name: &str) -> Vec<String> {
    parameters
        .get(name)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
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

fn repository_error(error: RepositoryError) -> AppError {
    match error {
        RepositoryError::NotFound { entity, id } => {
            AppError::validation("not_found", format!("{entity} {id} was not found"))
        }
        RepositoryError::Conflict(message) => AppError::validation("conflict", message),
        RepositoryError::InvalidInput(message) => {
            AppError::validation("invalid_repository_input", message)
        }
        other => AppError::internal("repository_error", other.to_string()),
    }
}

fn domain_error(error: domain::DomainError) -> AppError {
    AppError::validation("domain_invariant", error.to_string())
}

fn json_error(error: serde_json::Error) -> AppError {
    AppError::internal("invalid_json", error.to_string())
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::internal("filesystem_error", error.to_string())
}

impl ApplicationBackend for DefaultApplicationBackend {
    fn list_workspaces(&self, include_archived: bool) -> AppFuture<'_, Vec<WorkspaceSummary>> {
        Box::pin(async move {
            self.repository
                .list_workspaces(include_archived)
                .await
                .map_err(repository_error)
                .map(|records| records.into_iter().map(workspace_view).collect())
        })
    }

    fn create_workspace(&self, input: CreateWorkspaceInput) -> AppFuture<'_, WorkspaceSummary> {
        Box::pin(async move {
            let now = now_millis();
            self.repository
                .create_workspace(&WorkspaceRecord {
                    id: Uuid::new_v4().to_string(),
                    title: input.name,
                    system_prompt: if input.goal.trim().is_empty() {
                        DEFAULT_SYSTEM_PROMPT.into()
                    } else {
                        input.goal
                    },
                    created_at: now,
                    updated_at: now,
                    archived_at: None,
                })
                .await
                .map(workspace_view)
                .map_err(repository_error)
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
                .map_err(repository_error)?;
            let archived_at = match input.archived {
                Some(true) => current.archived_at.or_else(|| Some(now_millis())),
                Some(false) => None,
                None => current.archived_at,
            };
            self.repository
                .update_workspace(
                    &input.id,
                    input.name.as_deref().unwrap_or(&current.title),
                    input.goal.as_deref().unwrap_or(&current.system_prompt),
                    archived_at,
                    now_millis(),
                )
                .await
                .map(workspace_view)
                .map_err(repository_error)
        })
    }

    fn inspect_context(&self, input: InspectContextInput) -> AppFuture<'_, ContextPreview> {
        Box::pin(async move { self.inspect_context_impl(input).await })
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
                .get_run_receipt(&run_id)
                .await
                .map_err(repository_error)
                .and_then(receipt_view)
        })
    }

    fn update_context_overrides(&self, input: UpdateContextOverridesInput) -> AppFuture<'_, ()> {
        Box::pin(async move {
            let key = Self::override_key(&input.workspace_id, input.parent_run_id.as_deref());
            let mut all = self.context_overrides.lock().map_err(|_| {
                AppError::internal("context_override_lock", "Context overrides are unavailable")
            })?;
            all.entry(key).or_default().insert(
                input.item_id,
                OverrideItem {
                    included: input.included,
                    pinned: input.pinned,
                },
            );
            Ok(())
        })
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
                .save_view_state(&ViewStateRecord {
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
                .map_err(repository_error)?;
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
                .map_err(repository_error)
                .and_then(|records| records.into_iter().map(provider_profile_view).collect())
        })
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
    async fn open_workspace_impl(&self, workspace_id: &str) -> AppResult<WorkspaceDetail> {
        let workspace = self
            .repository
            .get_workspace(workspace_id)
            .await
            .map_err(repository_error)?;
        let profiles = self
            .repository
            .list_provider_profiles()
            .await
            .map_err(repository_error)?
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect::<HashMap<_, _>>();
        let turns = self
            .repository
            .list_turns(workspace_id)
            .await
            .map_err(repository_error)?;
        let mut selected_run_ids = BTreeMap::new();
        let mut turn_views = Vec::with_capacity(turns.len());
        for turn in turns {
            let run_records = self
                .repository
                .list_runs_for_turn(&turn.id)
                .await
                .map_err(repository_error)?;
            let views = run_records
                .iter()
                .map(|run| {
                    run_view(
                        run,
                        profiles.get(run.provider_profile_id.as_deref().unwrap_or("")),
                    )
                })
                .collect::<AppResult<Vec<_>>>()?;
            if let Some(selected) = run_records
                .iter()
                .rev()
                .find(|run| run.status == RunStatusRecord::Completed)
                .or_else(|| run_records.last())
            {
                selected_run_ids.insert(turn.id.clone(), selected.id.clone());
            }
            turn_views.push(TurnView {
                id: turn.id,
                workspace_id: turn.workspace_id,
                parent_run_id: turn.parent_run_id,
                prompt: turn.prompt_markdown,
                title: (!turn.title.is_empty()).then_some(turn.title),
                created_at: timestamp_view(turn.created_at),
                runs: views,
            });
        }
        let adjacent_branches = self
            .repository
            .list_branch_pointers(workspace_id)
            .await
            .map_err(repository_error)?
            .into_iter()
            .map(|pointer| AdjacentBranchView {
                run_id: pointer.head_run_id,
                label: pointer.name,
            })
            .collect();
        let decision_marks = self
            .repository
            .list_decision_marks(workspace_id)
            .await
            .map_err(repository_error)?
            .into_iter()
            .map(decision_view)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(WorkspaceDetail {
            workspace: workspace_view(workspace),
            turns: turn_views,
            selected_run_ids,
            adjacent_branches,
            decision_marks,
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
            input.parent_run_id,
            input.prompt,
            input.provider_profile_id,
            input.preview_hash,
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
            .map_err(repository_error)?;
        let turn = self
            .repository
            .get_turn(exact_retry_turn_id(&original))
            .await
            .map_err(repository_error)?;
        self.prepare_and_launch(
            turn.workspace_id,
            None,
            Some(turn.id),
            turn.parent_run_id,
            turn.prompt_markdown,
            input.provider_profile_id,
            input.preview_hash,
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
        parent_run_id: Option<String>,
        prompt: String,
        provider_profile_id: String,
        preview_hash: String,
        credentials: Arc<dyn SessionCredentialLookup>,
        events: Arc<dyn RunEventSink>,
    ) -> AppResult<RunHandle> {
        let workspace = self
            .repository
            .get_workspace(&workspace_id)
            .await
            .map_err(repository_error)?;
        let profile = self
            .repository
            .get_provider_profile(&provider_profile_id)
            .await
            .map_err(repository_error)?;
        let graph = load_graph(&self.repository, &workspace_id).await?;
        let key = Self::override_key(&workspace_id, parent_run_id.as_deref());
        let override_items = self.overrides_for(&key)?;
        let compiled = self
            .compiler
            .compile(
                &graph,
                ContextCompileRequest {
                    workspace_id: workspace_id.clone(),
                    system_prompt: workspace.system_prompt,
                    parent_run_id: parent_run_id.clone(),
                    current_prompt: prompt.clone(),
                    overrides: Self::compiler_overrides(&override_items),
                    provider: Some(domain_provider_snapshot(&profile)?),
                },
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
        let manifest_id = Uuid::new_v4().to_string();
        let snapshot_id = Uuid::new_v4().to_string();
        let parameters = provider_parameters(&profile)?;
        let parameter_json = Value::Object(parameters.clone()).to_string();
        let content_blocks = content_blocks_for_manifest(&compiled.manifest.items, now);
        let prompt_hash = domain::sha256_hex(prompt.as_bytes());
        let prompt_block_id = content_block_id("user", &prompt_hash);
        let turn = new_turn_id.map(|id| TurnRecord {
            id,
            workspace_id: workspace_id.clone(),
            parent_run_id: parent_run_id.clone(),
            prompt_block_id,
            prompt_markdown: prompt.clone(),
            title: prompt.chars().take(80).collect(),
            created_at: now,
            deleted_at: None,
        });
        let provider_snapshot = json!({
            "profileId": profile.id,
            "providerName": profile.name,
            "dialect": profile.dialect,
            "baseUrl": profile.base_url,
            "model": profile.default_model,
            "parameters": Value::Object(parameters.clone())
        });
        let run = ModelRunRecord {
            id: run_id.clone(),
            turn_id: turn_id.clone(),
            workspace_id: workspace_id.clone(),
            provider_profile_id: Some(profile.id.clone()),
            model: profile.default_model.clone(),
            status: RunStatusRecord::Queued,
            output_markdown: String::new(),
            reasoning_markdown: String::new(),
            provider_snapshot_json: provider_snapshot.to_string(),
            usage_json: None,
            error_json: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            checkpointed_at: None,
        };
        let manifest = ContextManifestRecord {
            id: manifest_id.clone(),
            workspace_id: workspace_id.clone(),
            compiler_version: compiled.manifest.compiler_version.clone(),
            strategy: "ancestor_path_with_pins".into(),
            estimated_chars: compiled.estimated_chars as i64,
            canonical_hash: compiled.canonical_hash.clone(),
            warnings_json: serde_json::to_string(
                &compiled
                    .warnings
                    .iter()
                    .map(context_warning)
                    .collect::<Vec<_>>(),
            )
            .map_err(json_error)?,
            created_at: now,
        };
        let context_items = compiled
            .manifest
            .items
            .iter()
            .map(|item| RunContextItemRecord {
                manifest_id: manifest_id.clone(),
                workspace_id: workspace_id.clone(),
                position: item.position as i64,
                source_id: item.source_id.clone(),
                source_kind: context_source_name(item.source_kind).into(),
                role: message_role_name(item.role).into(),
                content_block_id: content_block_id(
                    message_role_name(item.role),
                    &item.content_hash,
                ),
                inclusion_reason: inclusion_reason_name(item.inclusion_reason).into(),
            })
            .collect();
        let request_json = json!({
            "messages": compiled.messages.iter().map(|message| json!({
                "role": message_role_name(message.role),
                "content": message.content
            })).collect::<Vec<_>>()
        })
        .to_string();
        let snapshot = ContextSnapshotRecord {
            id: snapshot_id,
            run_id: run_id.clone(),
            manifest_id,
            workspace_id: workspace_id.clone(),
            provider_profile_id: Some(profile.id.clone()),
            provider: profile.name.clone(),
            model: profile.default_model.clone(),
            base_url: profile.base_url.clone(),
            parameters_json: parameter_json,
            request_json,
            canonical_hash: compiled.canonical_hash,
            created_at: now,
        };
        let branch_pointer = turn.as_ref().map(|turn| BranchPointerRecord {
            id: Uuid::new_v4().to_string(),
            workspace_id: workspace_id.clone(),
            name: format!("Route {}", &turn.id[..8.min(turn.id.len())]),
            head_run_id: run_id.clone(),
            version: 0,
            created_at: now,
            updated_at: now,
        });
        self.repository
            .persist_run_start(&RunStartBundle {
                turn,
                run,
                content_blocks,
                manifest,
                context_items,
                snapshot,
                branch_pointer,
            })
            .await
            .map_err(repository_error)?;

        let credential = credentials
            .credential_for(&profile.id)?
            .map(|credential| {
                credential
                    .as_str()
                    .map(|value| SessionCredential::new(value.to_owned()))
            })
            .transpose()?;
        let invocation = ProviderInvocation {
            target: ProviderTarget {
                dialect: provider_dialect(&profile.dialect)?,
                base_url: profile.base_url,
            },
            credential,
            request: CanonicalRequest {
                run_id: run_id.clone(),
                model: profile.default_model,
                messages: compiled
                    .messages
                    .into_iter()
                    .map(|message| CanonicalMessage {
                        role: provider_message_role(message.role),
                        content: message.content,
                    })
                    .collect(),
                temperature: parameter_f32(&parameters, "temperature"),
                top_p: parameter_f32(&parameters, "top_p"),
                max_output_tokens: parameter_u32(&parameters, "max_output_tokens"),
                stop: parameter_strings(&parameters, "stop"),
            },
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
            return Err(repository_error(error));
        }
        spawn_run(
            self.repository.clone(),
            self.provider.clone(),
            self.run_registry.clone(),
            invocation,
            cancellation,
            events,
        );
        Ok(RunHandle { turn_id, run_id })
    }

    async fn route_projection_impl(
        &self,
        input: GetRouteProjectionInput,
    ) -> AppResult<RouteProjection> {
        let turns = self
            .repository
            .list_turns(&input.workspace_id)
            .await
            .map_err(repository_error)?;
        let decisions = self
            .repository
            .list_decision_marks(&input.workspace_id)
            .await
            .map_err(repository_error)?
            .into_iter()
            .map(|mark| (mark.run_id.clone(), mark))
            .collect::<HashMap<_, _>>();
        let mut all_runs = HashMap::new();
        let mut runs_by_turn = BTreeMap::new();
        for turn in &turns {
            let runs = self
                .repository
                .list_runs_for_turn(&turn.id)
                .await
                .map_err(repository_error)?;
            for run in &runs {
                all_runs.insert(run.id.clone(), run.clone());
            }
            runs_by_turn.insert(turn.id.clone(), runs);
        }
        let branch_pointers = self
            .repository
            .list_branch_pointers(&input.workspace_id)
            .await
            .map_err(repository_error)?;
        let effective_current_run_id =
            effective_route_run_id(input.current_run_id.as_deref(), &all_runs, &branch_pointers);
        let lineage = exact_lineage_ids(effective_current_run_id.as_deref(), &turns, &all_runs);
        let mut nodes = Vec::with_capacity(turns.len());
        let mut edges = Vec::new();
        for (index, turn) in turns.iter().enumerate() {
            let runs = runs_by_turn.get(&turn.id).map(Vec::as_slice).unwrap_or(&[]);
            let selected = effective_current_run_id
                .as_deref()
                .and_then(|current| runs.iter().find(|run| run.id == current))
                .or_else(|| runs.iter().find(|run| lineage.contains(&run.id)))
                .or_else(|| {
                    runs.iter()
                        .rev()
                        .find(|run| run.status == RunStatusRecord::Completed)
                })
                .or_else(|| runs.last());
            let state = self
                .repository
                .get_view_state(&input.workspace_id, &format!("route-node:{}", turn.id))
                .await
                .ok()
                .and_then(|record| serde_json::from_str::<Value>(&record.state_json).ok());
            let x = state
                .as_ref()
                .and_then(|value| value.get("x"))
                .and_then(Value::as_f64)
                .unwrap_or((index % 4) as f64 * 320.0);
            let y = state
                .as_ref()
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
                title: if turn.title.is_empty() {
                    turn.prompt_markdown.chars().take(48).collect()
                } else {
                    turn.title.clone()
                },
                summary: turn.prompt_markdown.chars().take(140).collect(),
                status: selected
                    .map(|run| run_status_view(run.status))
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
                        status: run_status_view(run.status),
                        can_branch: run.status == RunStatusRecord::Completed
                            || (!run.output_markdown.is_empty()
                                && matches!(
                                    run.status,
                                    RunStatusRecord::Cancelled
                                        | RunStatusRecord::Failed
                                        | RunStatusRecord::Interrupted
                                )),
                    })
                    .collect(),
            });
            if let Some(source_run_id) = &turn.parent_run_id {
                edges.push(RouteEdgeView {
                    id: format!("{}->{}", source_run_id, turn.id),
                    source_run_id: source_run_id.clone(),
                    target_turn_id: turn.id.clone(),
                    is_on_current_lineage: lineage.contains(source_run_id),
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
            .map_err(repository_error)?;
        let right = self
            .repository
            .get_run(&input.right_run_id)
            .await
            .map_err(repository_error)?;
        let left_receipt = self
            .repository
            .get_run_receipt(&left.id)
            .await
            .map_err(repository_error)?;
        let right_receipt = self
            .repository
            .get_run_receipt(&right.id)
            .await
            .map_err(repository_error)?;
        let left_ids = left_receipt
            .items
            .iter()
            .map(context_item_identity)
            .collect::<BTreeSet<_>>();
        let right_ids = right_receipt
            .items
            .iter()
            .map(context_item_identity)
            .collect::<BTreeSet<_>>();
        let only_left = left_receipt
            .items
            .iter()
            .filter(|item| !right_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        let only_right = right_receipt
            .items
            .iter()
            .filter(|item| !left_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        let shared = left_receipt
            .items
            .iter()
            .filter(|item| right_ids.contains(&context_item_identity(item)))
            .map(context_diff_item)
            .collect();
        Ok(CompareRunsResult {
            left: ComparableRunView {
                run_id: left.id,
                model: left.model,
                status: run_status_view(left.status),
            },
            right: ComparableRunView {
                run_id: right.id,
                model: right.model,
                status: run_status_view(right.status),
            },
            answer: AnswerComparison {
                left_markdown: left.output_markdown,
                right_markdown: right.output_markdown,
            },
            context_diff: ContextDiffView {
                only_left,
                only_right,
                shared,
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
            .save_decision_mark(&DecisionMarkRecord {
                id: existing
                    .as_ref()
                    .map(|mark| mark.id.clone())
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                workspace_id: input.workspace_id,
                run_id: input.run_id,
                status: decision_status_record(input.status).into(),
                reason: input.reason,
                created_at: existing.map(|mark| mark.created_at).unwrap_or(now),
                updated_at: now,
            })
            .await
            .map_err(repository_error)
            .and_then(decision_view)
    }

    async fn export_decision_packet_impl(
        &self,
        input: ExportDecisionPacketInput,
    ) -> AppResult<ExportResult> {
        let detail = self.open_workspace_impl(&input.workspace_id).await?;
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
                    .map_err(repository_error)?;
                let receipt = self
                    .repository
                    .get_run_receipt(&mark.run_id)
                    .await
                    .map_err(repository_error)?;
                markdown.push_str(&format!(
                    "### {} · {}\n\n**Rationale:** {}\n\n{}\n\n_Context receipt:_ `{}` · {} ordered items · {}\n\n",
                    run.model,
                    mark.run_id,
                    mark.reason,
                    run.output_markdown,
                    receipt.snapshot.canonical_hash,
                    receipt.items.len(),
                    receipt.snapshot.base_url,
                ));
            }
            if !found {
                markdown.push_str("_None._\n\n");
            }
        }
        std::fs::create_dir_all(&self.export_root).map_err(io_error)?;
        let destination = input.destination.map(PathBuf::from).unwrap_or_else(|| {
            self.export_root.join(format!(
                "decision-packet-{}-{}.md",
                input.workspace_id,
                now_millis()
            ))
        });
        std::fs::write(&destination, markdown.as_bytes()).map_err(io_error)?;
        Ok(ExportResult {
            path: destination.to_string_lossy().into_owned(),
            bytes_written: markdown.len() as u64,
        })
    }

    async fn save_provider_profile_impl(
        &self,
        input: SaveProviderProfileInput,
    ) -> AppResult<ProviderProfileView> {
        let now = now_millis();
        let id = input.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing = self.repository.get_provider_profile(&id).await.ok();
        let mut parameters = input.parameters.unwrap_or_default();
        parameters.insert(INTERNAL_DEFAULT_KEY.into(), Value::Bool(input.is_default));
        let record = self
            .repository
            .save_provider_profile(&ProviderProfileRecord {
                id,
                name: input.name,
                dialect: match input.dialect {
                    ProviderDialectView::OpenaiCompatible => "openai_chat_completions",
                    ProviderDialectView::Ollama => "ollama_chat",
                }
                .into(),
                base_url: input.base_url,
                default_model: input.model,
                parameters_json: Value::Object(parameters.into_iter().collect()).to_string(),
                created_at: existing.map(|profile| profile.created_at).unwrap_or(now),
                updated_at: now,
            })
            .await
            .map_err(repository_error)?;
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
            .map_err(repository_error)?;
        let mut endpoint = validate_base_url(&profile.base_url)
            .map_err(|error| AppError::validation(error.code(), error.to_string()))?;
        let base_path = endpoint.path().trim_end_matches('/');
        let path = if profile.dialect == "ollama_chat" {
            if base_path.ends_with("/api") {
                format!("{base_path}/tags")
            } else {
                format!("{base_path}/api/tags")
            }
        } else {
            format!("{base_path}/models")
        };
        endpoint.set_path(&path);
        let mut request = self.connection_client.get(endpoint);
        if let Some(credential) = credential {
            request = request.bearer_auth(credential.as_str()?);
        }
        let response = request.send().await.map_err(|error| AppError {
            code: "provider_unreachable".into(),
            message: error.to_string(),
            retryable: true,
            details: Value::Null,
        })?;
        let ok = response.status().is_success();
        Ok(ProviderConnectionResult {
            ok,
            message: if ok {
                format!("Connected to {}", profile.name)
            } else {
                format!("Provider returned HTTP {}", response.status())
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, turn_id: &str) -> ModelRunRecord {
        ModelRunRecord {
            id: id.into(),
            turn_id: turn_id.into(),
            workspace_id: "workspace-1".into(),
            provider_profile_id: Some("provider-1".into()),
            model: "model".into(),
            status: RunStatusRecord::Completed,
            output_markdown: "same answer shape".into(),
            reasoning_markdown: String::new(),
            provider_snapshot_json: "{}".into(),
            usage_json: None,
            error_json: None,
            created_at: 1,
            started_at: Some(2),
            finished_at: Some(3),
            checkpointed_at: Some(3),
        }
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
                source_kind: ContextSourceKind::TurnPrompt,
                role: MessageRole::User,
                content: "same text".into(),
                content_hash: hash.clone(),
                inclusion_reason: InclusionReason::ExactAncestorPath,
            },
            domain::RunContextItem {
                position: 1,
                source_id: Some("answer".into()),
                source_kind: ContextSourceKind::ModelRun,
                role: MessageRole::Assistant,
                content: "same text".into(),
                content_hash: hash.clone(),
                inclusion_reason: InclusionReason::ExactAncestorPath,
            },
        ];
        let blocks = content_blocks_for_manifest(&items, 1);

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
        let mut failed = run("run-failed", "turn-root");
        failed.status = RunStatusRecord::Failed;
        runs.insert(failed.id.clone(), failed);
        let pointers = vec![BranchPointerRecord {
            id: "branch-main".into(),
            workspace_id: "workspace-1".into(),
            name: "Main".into(),
            head_run_id: "run-failed".into(),
            version: 1,
            created_at: 1,
            updated_at: 2,
        }];

        assert_eq!(
            effective_route_run_id(None, &runs, &pointers).as_deref(),
            Some("run-failed")
        );
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
}
