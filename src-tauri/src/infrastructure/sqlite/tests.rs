use super::*;
use crate::{
    domain::{
        ContentBlock, ContextManifest, ContextSnapshot, ContextSourceKind, InclusionReason,
        MessageRole, ModelRun, ProviderDialect, ProviderProfile, ProviderSnapshot, RunContextItem,
        RunDraft, RunFailure, RunStateSnapshot, RunStatus, Turn, Workspace,
    },
    ports::{
        CheckpointOutcome, PersistRunStart, RepositoryPort, RunCheckpoint as PortRunCheckpoint,
        RunFinish as PortRunFinish, RunPersistencePort,
    },
};
use std::collections::BTreeMap;

fn workspace(id: &str, title: &str) -> WorkspaceRecord {
    WorkspaceRecord {
        id: id.into(),
        title: title.into(),
        goal: "Choose the safest architecture.".into(),
        system_prompt: "You are a careful technical collaborator.".into(),
        created_at: 10,
        updated_at: 10,
        archived_at: None,
    }
}

fn root_bundle(workspace_id: &str, turn_id: &str, run_id: &str) -> RunStartBundle {
    let prompt_block_id = format!("block-{turn_id}");
    let manifest_id = format!("manifest-{run_id}");

    RunStartBundle {
        turn: Some(TurnRecord {
            id: turn_id.into(),
            workspace_id: workspace_id.into(),
            parent_run_id: None,
            prompt_block_id: prompt_block_id.clone(),
            prompt_markdown: "Compare the two architectures.".into(),
            title: "Architecture comparison".into(),
            created_at: 20,
            deleted_at: None,
        }),
        run: ModelRunRecord {
            id: run_id.into(),
            turn_id: turn_id.into(),
            workspace_id: workspace_id.into(),
            provider_profile_id: None,
            model: "test-model".into(),
            status: RunStatusRecord::Queued,
            output_markdown: String::new(),
            reasoning_markdown: String::new(),
            provider_snapshot_json: r#"{"dialect":"openai_chat_completions"}"#.into(),
            usage_json: None,
            error_json: None,
            created_at: 20,
            started_at: None,
            finished_at: None,
            checkpointed_at: None,
        },
        content_blocks: vec![ContentBlockRecord {
            id: prompt_block_id.clone(),
            role: "user".into(),
            content: "Compare the two architectures.".into(),
            content_hash: format!("hash-{turn_id}"),
            created_at: 20,
        }],
        manifest: ContextManifestRecord {
            id: manifest_id.clone(),
            workspace_id: workspace_id.into(),
            compiler_version: "1".into(),
            strategy: "ancestor_path_with_pins".into(),
            estimated_chars: 30,
            canonical_hash: format!("canonical-{run_id}"),
            warnings_json: "[]".into(),
            created_at: 20,
        },
        context_items: vec![RunContextItemRecord {
            manifest_id: manifest_id.clone(),
            workspace_id: workspace_id.into(),
            position: 0,
            source_id: Some(turn_id.into()),
            source_kind: "current_prompt".into(),
            role: "user".into(),
            content_block_id: prompt_block_id,
            inclusion_reason: "current_prompt".into(),
        }],
        snapshot: ContextSnapshotRecord {
            id: format!("snapshot-{run_id}"),
            run_id: run_id.into(),
            manifest_id,
            workspace_id: workspace_id.into(),
            provider_profile_id: None,
            provider: "OpenAI-compatible".into(),
            model: "test-model".into(),
            base_url: "https://example.invalid/v1".into(),
            parameters_json: "{}".into(),
            request_json:
                r#"{"messages":[{"role":"user","content":"Compare the two architectures."}]}"#
                    .into(),
            canonical_hash: format!("canonical-{run_id}"),
            created_at: 20,
        },
        branch_pointer: None,
    }
}

#[tokio::test]
async fn migration_creates_the_complete_strict_schema() {
    let repository = SqliteRepository::connect_in_memory()
        .await
        .expect("repository opens");

    let schema = repository.schema_info().await.expect("schema is readable");

    assert_eq!(schema.version, 2);
    assert_eq!(
        schema.strict_tables,
        vec![
            "branch_pointer",
            "content_block",
            "context_manifest",
            "context_manifest_item",
            "context_snapshot",
            "decision_mark",
            "model_run",
            "provider_profile",
            "turn",
            "view_state",
            "workspace",
        ]
    );
}

#[tokio::test]
async fn workspace_goal_migration_repairs_v1_goal_as_system_prompt_rows() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0001_core.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO workspace \
         (id, title, system_prompt, created_at, updated_at, archived_at) \
         VALUES (?, ?, ?, ?, ?, ?), (?, ?, ?, ?, ?, ?)",
    )
    .bind("workspace-goal")
    .bind("Goal")
    .bind("Choose the safest architecture")
    .bind(1_i64)
    .bind(1_i64)
    .bind(None::<i64>)
    .bind("workspace-placeholder")
    .bind("Placeholder")
    .bind("尚未设置工作区目标")
    .bind(1_i64)
    .bind(1_i64)
    .bind(None::<i64>)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(include_str!("../../../migrations/0002_workspace_goal.sql"))
        .execute(&pool)
        .await
        .unwrap();

    let goal = sqlx::query_as::<_, (String, String)>(
        "SELECT goal, system_prompt FROM workspace WHERE id = 'workspace-goal'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let placeholder = sqlx::query_as::<_, (String, String)>(
        "SELECT goal, system_prompt FROM workspace WHERE id = 'workspace-placeholder'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(goal.0, "Choose the safest architecture");
    assert_eq!(placeholder.0, "");
    assert_eq!(goal.1, placeholder.1);
    assert!(
        goal.1
            .starts_with("You are a careful technical reasoning partner.")
    );
}

#[tokio::test]
async fn workspace_round_trip_and_archive_are_observable_through_the_repository() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();

    repository
        .update_workspace(
            "workspace-a",
            "Renamed lab",
            "Choose the lowest-risk migration path",
            "Updated system prompt",
            Some(40),
            40,
        )
        .await
        .unwrap();

    let stored = repository.get_workspace("workspace-a").await.unwrap();
    assert_eq!(stored.title, "Renamed lab");
    assert_eq!(stored.goal, "Choose the lowest-risk migration path");
    assert_eq!(stored.system_prompt, "Updated system prompt");
    assert_eq!(stored.archived_at, Some(40));
    assert!(repository.list_workspaces(false).await.unwrap().is_empty());
    assert_eq!(
        repository.list_workspaces(true).await.unwrap(),
        vec![stored]
    );
}

#[tokio::test]
async fn repository_port_reads_legacy_unquoted_provider_parameters() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .save_provider_profile(&ProviderProfileRecord {
            id: "provider-legacy".into(),
            name: "Legacy profile".into(),
            dialect: "ollama_chat".into(),
            base_url: "http://127.0.0.1:11434".into(),
            default_model: "qwen3".into(),
            parameters_json: r#"{"temperature":0.2,"_thoughsflowIsDefault":true}"#.into(),
            created_at: 10,
            updated_at: 10,
        })
        .await
        .unwrap();

    let profile = RepositoryPort::get_provider_profile(&repository, "provider-legacy")
        .await
        .unwrap();
    assert_eq!(profile.parameters["temperature"], "0.2");
    assert_eq!(profile.parameters["_thoughsflowIsDefault"], "true");
}

#[tokio::test]
async fn run_start_bundle_is_atomic_when_a_context_item_is_invalid() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();
    let mut bundle = root_bundle("workspace-a", "turn-a", "run-a");
    bundle.context_items[0].content_block_id = "missing-block".into();

    assert!(repository.persist_run_start(&bundle).await.is_err());
    assert!(matches!(
        repository.get_turn("turn-a").await,
        Err(RepositoryError::NotFound { .. })
    ));
    assert!(matches!(
        repository.get_run("run-a").await,
        Err(RepositoryError::NotFound { .. })
    ));
}

#[tokio::test]
async fn later_run_reuses_an_immutable_content_block_without_rewriting_first_seen_time() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();

    let mut retry = root_bundle("workspace-a", "turn-a", "run-b");
    retry.turn = None;
    retry.content_blocks[0].created_at = 99;
    repository.persist_run_start(&retry).await.unwrap();

    let blocks = repository.list_content_blocks("workspace-a").await.unwrap();
    let reused = blocks
        .iter()
        .find(|block| block.id == "block-turn-a")
        .expect("ancestor prompt block remains available");
    assert_eq!(reused.created_at, 20);
    assert_eq!(repository.get_run("run-b").await.unwrap().turn_id, "turn-a");
}

#[tokio::test]
async fn equal_content_hashes_are_distinct_when_message_roles_differ() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();
    let mut bundle = root_bundle("workspace-a", "turn-a", "run-a");
    let shared_hash = bundle.content_blocks[0].content_hash.clone();
    let shared_content = bundle.content_blocks[0].content.clone();
    bundle.content_blocks.push(ContentBlockRecord {
        id: "block-system-a".into(),
        role: "system".into(),
        content: shared_content,
        content_hash: shared_hash,
        created_at: 20,
    });
    bundle.context_items[0].position = 1;
    bundle.context_items.insert(
        0,
        RunContextItemRecord {
            manifest_id: bundle.manifest.id.clone(),
            workspace_id: "workspace-a".into(),
            position: 0,
            source_id: Some("workspace-a:system".into()),
            source_kind: "system".into(),
            role: "system".into(),
            content_block_id: "block-system-a".into(),
            inclusion_reason: "system_policy".into(),
        },
    );

    repository.persist_run_start(&bundle).await.unwrap();
    let blocks = repository.list_content_blocks("workspace-a").await.unwrap();
    assert!(blocks.iter().any(|block| block.id == "block-turn-a"));
    assert!(blocks.iter().any(|block| block.id == "block-system-a"));
}

#[tokio::test]
async fn child_turn_cannot_reference_a_run_from_another_workspace() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .create_workspace(&workspace("workspace-b", "B"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    finish_test_run(&repository, "run-a").await;

    let mut child = root_bundle("workspace-b", "turn-b", "run-b");
    child.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());

    assert!(repository.persist_run_start(&child).await.is_err());
    assert!(matches!(
        repository.get_turn("turn-b").await,
        Err(RepositoryError::NotFound { .. })
    ));
}

#[tokio::test]
async fn child_turn_cannot_reference_an_unfinished_run() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    let mut child = root_bundle("workspace-a", "turn-b", "run-b");
    child.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());

    assert!(repository.persist_run_start(&child).await.is_err());
    assert!(matches!(
        repository.get_turn("turn-b").await,
        Err(RepositoryError::NotFound { .. })
    ));
}

#[tokio::test]
async fn run_start_atomically_advances_an_existing_branch_pointer() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    let mut root = root_bundle("workspace-a", "turn-a", "run-a");
    root.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-a".into(),
        version: 0,
        created_at: 20,
        updated_at: 20,
    });
    repository.persist_run_start(&root).await.unwrap();
    finish_test_run(&repository, "run-a").await;

    let mut child = root_bundle("workspace-a", "turn-b", "run-b");
    child.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    child.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-b".into(),
        version: 1,
        created_at: 20,
        updated_at: 30,
    });

    repository.persist_run_start(&child).await.unwrap();
    let pointer = repository.get_branch_pointer("branch-main").await.unwrap();
    assert_eq!(pointer.head_run_id, "run-b");
    assert_eq!(pointer.version, 1);
}

async fn finish_test_run(repository: &SqliteRepository, run_id: &str) {
    repository.mark_run_connecting(run_id, 21).await.unwrap();
    repository.mark_run_streaming(run_id, 22).await.unwrap();
    repository
        .finish_run(
            run_id,
            &RunFinish {
                status: RunStatusRecord::Completed,
                output_markdown: "A usable answer".into(),
                reasoning_markdown: String::new(),
                usage_json: None,
                error_json: None,
                finished_at: 23,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn checkpoint_is_idempotent_and_startup_recovery_preserves_partial_output() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    repository.mark_run_connecting("run-a", 30).await.unwrap();
    repository.mark_run_streaming("run-a", 31).await.unwrap();

    let checkpoint = RunCheckpoint {
        output_markdown: "partial answer".into(),
        reasoning_markdown: "partial reasoning".into(),
        usage_json: Some(r#"{"completion_tokens":2}"#.into()),
        checkpointed_at: 35,
    };
    assert_eq!(
        repository
            .checkpoint_run("run-a", &checkpoint)
            .await
            .unwrap(),
        CheckpointWriteOutcome::Saved
    );
    assert_eq!(
        repository
            .checkpoint_run("run-a", &checkpoint)
            .await
            .unwrap(),
        CheckpointWriteOutcome::Saved
    );

    assert_eq!(repository.recover_interrupted_runs(40).await.unwrap(), 1);
    let recovered = repository.get_run("run-a").await.unwrap();
    assert_eq!(recovered.status, RunStatusRecord::Interrupted);
    assert_eq!(recovered.output_markdown, "partial answer");
    assert_eq!(recovered.reasoning_markdown, "partial reasoning");
    assert_eq!(recovered.finished_at, Some(40));
}
#[tokio::test]
async fn terminal_run_output_cannot_be_checkpointed_or_replaced() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    repository.mark_run_connecting("run-a", 30).await.unwrap();
    repository.mark_run_streaming("run-a", 31).await.unwrap();
    repository
        .finish_run(
            "run-a",
            &RunFinish {
                status: RunStatusRecord::Completed,
                output_markdown: "final answer".into(),
                reasoning_markdown: String::new(),
                usage_json: Some(r#"{"completion_tokens":2}"#.into()),
                error_json: None,
                finished_at: 50,
            },
        )
        .await
        .unwrap();

    let result = repository
        .checkpoint_run(
            "run-a",
            &RunCheckpoint {
                output_markdown: "replacement".into(),
                reasoning_markdown: String::new(),
                usage_json: None,
                checkpointed_at: 60,
            },
        )
        .await;

    assert_eq!(
        result.unwrap(),
        CheckpointWriteOutcome::SkippedTerminal(RunStatusRecord::Completed)
    );
    assert_eq!(
        repository.get_run("run-a").await.unwrap().output_markdown,
        "final answer"
    );
}

#[tokio::test]
async fn run_persistence_port_skips_a_late_checkpoint_after_cancellation() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    repository.mark_run_connecting("run-a", 30).await.unwrap();
    RunPersistencePort::mark_run_streaming(&repository, "run-a", 31)
        .await
        .unwrap();
    RunPersistencePort::finish_run(
        &repository,
        "run-a",
        PortRunFinish {
            status: RunStatus::Cancelled,
            output_markdown: "partial answer".into(),
            reasoning_markdown: String::new(),
            usage: None,
            error: None,
            finished_at: 40,
        },
    )
    .await
    .unwrap();

    let outcome = RunPersistencePort::checkpoint_run(
        &repository,
        "run-a",
        PortRunCheckpoint {
            output_markdown: "stale replacement".into(),
            reasoning_markdown: String::new(),
            usage: None,
            checkpointed_at: 41,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        outcome,
        CheckpointOutcome::SkippedTerminal(RunStatus::Cancelled)
    );
    let stored = repository.get_run("run-a").await.unwrap();
    assert_eq!(stored.status, RunStatusRecord::Cancelled);
    assert_eq!(stored.output_markdown, "partial answer");
}

#[tokio::test]
async fn repository_port_round_trips_domain_run_snapshot_and_graph() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    let workspace = Workspace::new(
        "workspace-a",
        "Decision lab",
        "Choose a database strategy.",
        "Be exact.",
        10,
    );
    RepositoryPort::save_workspace(&repository, workspace.clone())
        .await
        .unwrap();
    assert_eq!(
        RepositoryPort::get_workspace(&repository, "workspace-a")
            .await
            .unwrap(),
        workspace
    );
    let updated_workspace = Workspace {
        title: "Decision lab renamed".into(),
        goal: "Choose a database strategy with rollback evidence.".into(),
        updated_at: 12,
        ..workspace.clone()
    };
    assert_eq!(
        RepositoryPort::save_workspace(&repository, updated_workspace.clone())
            .await
            .unwrap(),
        updated_workspace
    );
    assert_eq!(
        RepositoryPort::get_workspace(&repository, "workspace-a")
            .await
            .unwrap(),
        updated_workspace
    );

    let profile = ProviderProfile {
        id: "provider-a".into(),
        name: "Local model".into(),
        dialect: ProviderDialect::Ollama,
        base_url: "http://127.0.0.1:11434".into(),
        model: "qwen3".into(),
        parameters: BTreeMap::from([("temperature".into(), "0.2".into())]),
        created_at: 11,
        updated_at: 11,
    };
    RepositoryPort::save_provider_profile(&repository, profile.clone())
        .await
        .unwrap();

    let turn = Turn::root("turn-a", "workspace-a", "Compare the options.", 20);
    let run = ModelRun::queued(RunDraft {
        id: "run-a".into(),
        turn_id: turn.id.clone(),
        provider_profile_id: Some(profile.id.clone()),
        model: profile.model.clone(),
        created_at: 20,
    });
    let prompt_block = ContentBlock {
        id: "prompt-block-a".into(),
        workspace_id: workspace.id.clone(),
        role: MessageRole::User,
        content: turn.prompt_markdown.clone(),
        content_hash: "prompt-hash-a".into(),
        created_at: 20,
    };
    let manifest = ContextManifest {
        compiler_version: "1".into(),
        items: vec![RunContextItem {
            position: 0,
            source_id: Some(turn.id.clone()),
            source_kind: ContextSourceKind::CurrentPrompt,
            role: MessageRole::User,
            content: turn.prompt_markdown.clone(),
            content_hash: prompt_block.content_hash.clone(),
            inclusion_reason: InclusionReason::CurrentPrompt,
        }],
        estimated_chars: 20,
        canonical_hash: "canonical-a".into(),
    };
    let snapshot = ContextSnapshot {
        id: "snapshot-a".into(),
        run_id: run.id.clone(),
        manifest: manifest.clone(),
        provider: ProviderSnapshot {
            profile_id: profile.id.clone(),
            provider_name: profile.name.clone(),
            dialect: profile.dialect,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            parameters: profile.parameters.clone(),
        },
        created_at: 20,
    };

    RepositoryPort::persist_run_start(
        &repository,
        PersistRunStart {
            turn: Some(turn.clone()),
            run,
            snapshot: snapshot.clone(),
            content_blocks: vec![prompt_block],
            branch_pointer: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        RepositoryPort::get_run_snapshot(&repository, "run-a")
            .await
            .unwrap(),
        snapshot
    );
    RepositoryPort::mark_run_connecting(&repository, "run-a", 21)
        .await
        .unwrap();
    RunPersistencePort::mark_run_streaming(&repository, "run-a", 22)
        .await
        .unwrap();
    let failed = ModelRun::rehydrate(
        RunDraft {
            id: "run-a".into(),
            turn_id: "turn-a".into(),
            provider_profile_id: Some("provider-a".into()),
            model: "qwen3".into(),
            created_at: 20,
        },
        RunStateSnapshot {
            status: RunStatus::Failed,
            output_markdown: "partial".into(),
            reasoning_markdown: String::new(),
            error: Some(RunFailure {
                code: "rate_limit".into(),
                message: "Too many requests".into(),
                retryable: true,
                status: Some(429),
            }),
            usage: None,
            started_at: Some(21),
            checkpointed_at: Some(22),
            finished_at: Some(23),
        },
    )
    .unwrap();
    RunPersistencePort::finish_run(
        &repository,
        "run-a",
        PortRunFinish {
            status: RunStatus::Failed,
            output_markdown: "partial".into(),
            reasoning_markdown: String::new(),
            usage: None,
            error: failed.failure().cloned(),
            finished_at: 23,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        RepositoryPort::get_run(&repository, "run-a")
            .await
            .unwrap()
            .failure(),
        failed.failure()
    );
    let graph = RepositoryPort::load_conversation_graph(&repository, "workspace-a")
        .await
        .unwrap();
    assert_eq!(graph.turn("turn-a"), Some(&turn));
    assert_eq!(graph.run("run-a").unwrap().model, "qwen3");
}
