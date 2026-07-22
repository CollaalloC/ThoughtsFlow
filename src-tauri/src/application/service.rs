use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
        self, BranchPointer, ContextCompileRequest, ContextCompiler, ContextPolicy,
        ContextSourceKind, ContextWarning, DecisionMark, DecisionStatus, InclusionReason,
        MessageRole, ModelRun, ProviderProfile, RunDraft, RunFailure, RunStatus, Turn, ViewState,
        Workspace,
    },
    ports::provider::{
        CanonicalMessage, CanonicalRequest, MessageRole as ProviderMessageRole,
        ProviderConnectionTester, ProviderDialect, ProviderGateway, ProviderInvocation,
        ProviderTarget, RunEvent, SessionCredential, Usage,
    },
    ports::{
        CheckpointOutcome, DecisionPacketWriter, PersistRunStart, RepositoryPort,
        RepositoryPortError, RunCheckpoint, RunFinish, RunPersistencePort,
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
    repository: Arc<dyn RepositoryPort>,
    run_persistence: Arc<dyn RunPersistencePort>,
    provider: Arc<dyn ProviderGateway>,
    connection_tester: Arc<dyn ProviderConnectionTester>,
    compiler: ContextCompiler,
    run_registry: Arc<Mutex<HashMap<String, CancellationToken>>>,
    context_overrides: Arc<Mutex<HashMap<OverrideKey, HashMap<String, OverrideItem>>>>,
    decision_packet_writer: Arc<dyn DecisionPacketWriter>,
}

impl DefaultApplicationBackend {
    pub fn new<R>(
        repository: Arc<R>,
        provider: Arc<dyn ProviderGateway>,
        connection_tester: Arc<dyn ProviderConnectionTester>,
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
            compiler: ContextCompiler::new(ContextPolicy {
                compiler_version: domain::CONTEXT_COMPILER_VERSION.into(),
                max_chars: DEFAULT_MAX_CONTEXT_CHARS,
            }),
            run_registry: Arc::new(Mutex::new(HashMap::new())),
            context_overrides: Arc::new(Mutex::new(HashMap::new())),
            decision_packet_writer,
        }
    }

    pub async fn initialize(&self) -> AppResult<()> {
        self.repository
            .recover_interrupted_runs(now_millis())
            .await
            .map_err(repository_port_error)?;
        self.ensure_default_provider().await
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
            .map_err(repository_port_error)?;
        let profile = self
            .repository
            .get_provider_profile(&input.provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let graph = self
            .repository
            .load_conversation_graph(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
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
            model: profile.model,
            base_url: profile.base_url,
            items,
        })
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
            let id = content_block_id(message_role_name(item.role), &item.content_hash);
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

fn run_view(record: &ModelRun, profile: Option<&ProviderProfile>) -> AppResult<RunView> {
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
        provider_name: profile
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "Unknown provider".into()),
        model: record.model.clone(),
        base_url: profile
            .map(|profile| profile.base_url.clone())
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
        provider_name: receipt.provider.provider_name,
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
        name: record.name,
        dialect: match record.dialect {
            domain::ProviderDialect::OpenAiCompatible => ProviderDialectView::OpenaiCompatible,
            domain::ProviderDialect::Ollama => ProviderDialectView::Ollama,
        },
        base_url: record.base_url,
        model: record.model,
        is_default,
        parameters: (!parameters.is_empty()).then_some(parameters),
    })
}

fn domain_provider_snapshot(record: &ProviderProfile) -> AppResult<domain::ProviderSnapshot> {
    let parameters = record
        .parameters
        .iter()
        .filter(|(key, _)| key.as_str() != INTERNAL_DEFAULT_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(domain::ProviderSnapshot {
        profile_id: record.id.clone(),
        provider_name: record.name.clone(),
        dialect: record.dialect,
        base_url: record.base_url.clone(),
        model: record.model.clone(),
        parameters,
    })
}

fn parse_parameter_value(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.into()))
}

fn provider_parameters(record: &ProviderProfile) -> AppResult<Map<String, Value>> {
    Ok(record
        .parameters
        .iter()
        .filter(|(key, _)| key.as_str() != INTERNAL_DEFAULT_KEY)
        .map(|(key, value)| (key.clone(), parse_parameter_value(value)))
        .collect())
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

fn context_diff_item(item: &domain::RunContextItem) -> ContextDiffItemView {
    ContextDiffItemView {
        id: context_item_identity(item),
        ordinal: u32::try_from(item.position).unwrap_or(u32::MAX),
        role: domain_message_role_view(item.role),
        source: context_source_name(item.source_kind).into(),
        preview: item.content.chars().take(180).collect(),
    }
}

fn context_item_identity(item: &domain::RunContextItem) -> String {
    format!(
        "{}:{}:{}:{}",
        item.content_hash,
        context_source_name(item.source_kind),
        item.source_id.as_deref().unwrap_or("current"),
        item.position,
    )
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
    }
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

fn repository_port_error(error: RepositoryPortError) -> AppError {
    match error {
        RepositoryPortError::NotFound { entity, id } => {
            AppError::validation("not_found", format!("{entity} {id} was not found"))
        }
        RepositoryPortError::Conflict(message) => AppError::validation("conflict", message),
        RepositoryPortError::InvalidData(message) => {
            AppError::validation("invalid_repository_data", message)
        }
        RepositoryPortError::Unavailable(message) => {
            AppError::internal("repository_error", message)
        }
    }
}

fn domain_error(error: domain::DomainError) -> AppError {
    AppError::validation("domain_invariant", error.to_string())
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
            .map_err(repository_port_error)?;
        let profiles = self
            .repository
            .list_provider_profiles()
            .await
            .map_err(repository_port_error)?
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect::<HashMap<_, _>>();
        let turns = self
            .repository
            .list_turns(workspace_id)
            .await
            .map_err(repository_port_error)?;
        let mut selected_run_ids = BTreeMap::new();
        let mut turn_views = Vec::with_capacity(turns.len());
        for turn in turns {
            let run_records = self
                .repository
                .list_runs_for_turn(&turn.id)
                .await
                .map_err(repository_port_error)?;
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
                .find(|run| run.status() == RunStatus::Completed)
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
        let adjacent_branches = self
            .repository
            .list_branch_pointers(workspace_id)
            .await
            .map_err(repository_port_error)?
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
            .map_err(repository_port_error)?;
        let profile = self
            .repository
            .get_provider_profile(&provider_profile_id)
            .await
            .map_err(repository_port_error)?;
        let graph = self
            .repository
            .load_conversation_graph(&workspace_id)
            .await
            .map_err(repository_port_error)?;
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
        let snapshot_id = Uuid::new_v4().to_string();
        let parameters = provider_parameters(&profile)?;
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
            provider: domain_provider_snapshot(&profile)?,
            created_at: now,
        };
        let branch_pointer = turn.as_ref().map(|turn| BranchPointer {
            id: Uuid::new_v4().to_string(),
            workspace_id: workspace_id.clone(),
            name: format!("Route {}", &turn.id[..8.min(turn.id.len())]),
            head_run_id: run_id.clone(),
            version: 0,
            updated_at: now,
        });
        self.repository
            .persist_run_start(PersistRunStart {
                turn,
                run,
                content_blocks,
                snapshot,
                branch_pointer,
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
        let invocation = ProviderInvocation {
            target: ProviderTarget {
                dialect: provider_dialect(profile.dialect),
                base_url: profile.base_url,
            },
            credential,
            request: CanonicalRequest {
                run_id: run_id.clone(),
                model: profile.model,
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
        let mut runs_by_turn = BTreeMap::new();
        for turn in &turns {
            let runs = self
                .repository
                .list_runs_for_turn(&turn.id)
                .await
                .map_err(repository_port_error)?;
            for run in &runs {
                all_runs.insert(run.id.clone(), run.clone());
            }
            runs_by_turn.insert(turn.id.clone(), runs);
        }
        let branch_pointers = self
            .repository
            .list_branch_pointers(&input.workspace_id)
            .await
            .map_err(repository_port_error)?;
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
                        .find(|run| run.status() == RunStatus::Completed)
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
        let mut parameters = input.parameters.unwrap_or_default();
        parameters.insert(INTERNAL_DEFAULT_KEY.into(), Value::Bool(input.is_default));
        let record = self
            .repository
            .save_provider_profile(ProviderProfile {
                id,
                name: input.name,
                dialect: match input.dialect {
                    ProviderDialectView::OpenaiCompatible => {
                        domain::ProviderDialect::OpenAiCompatible
                    }
                    ProviderDialectView::Ollama => domain::ProviderDialect::Ollama,
                },
                base_url: input.base_url,
                model: input.model,
                parameters: parameters
                    .into_iter()
                    .map(|(key, value)| (key, value.to_string()))
                    .collect(),
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
        let response = self
            .connection_tester
            .test(
                ProviderTarget {
                    dialect: provider_dialect(profile.dialect),
                    base_url: profile.base_url,
                },
                credential,
            )
            .await
            .map_err(|error| AppError {
                code: error.code().into(),
                message: error.to_string(),
                retryable: error.retryable(),
                details: error
                    .status()
                    .map(|status| json!({ "status": status }))
                    .unwrap_or(Value::Null),
            })?;
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
