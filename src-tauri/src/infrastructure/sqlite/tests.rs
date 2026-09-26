use super::*;
use crate::{
    domain::{
        ContentBlock, ContextManifest, ContextSnapshot, ContextSourceKind, ContextSourceRef,
        ContextSourceRefKind, InclusionReason, MessageRole, ModelRun, ProviderDialect,
        ProviderProfile, ProviderSnapshot, RunContextItem, RunDraft, RunFailure, RunStateSnapshot,
        RunStatus, Turn, Workspace,
    },
    ports::{
        CheckpointOutcome, PersistRunStart, RepositoryPort, RunCheckpoint as PortRunCheckpoint,
        RunFinish as PortRunFinish, RunPersistencePort, RunProviderProvenance,
    },
};
use sqlx::{ConnectOptions, Connection};
use std::collections::BTreeMap;

static TEST_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

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
            checkpoint_provenance_json: None,
            branch_summary_provenance_json: "[]".into(),
            created_at: 20,
        },
        context_items: vec![RunContextItemRecord {
            manifest_id: manifest_id.clone(),
            workspace_id: workspace_id.into(),
            position: 0,
            source_id: Some(turn_id.into()),
            source_ref_kind: "turn_prompt".into(),
            source_ref_id: Some(turn_id.into()),
            source_kind: "current_prompt".into(),
            role: "user".into(),
            content_block_id: prompt_block_id,
            inclusion_reason: "current_prompt".into(),
            mandatory: true,
        }],
        snapshot: ContextSnapshotRecord {
            id: format!("snapshot-{run_id}"),
            run_id: run_id.into(),
            manifest_id,
            workspace_id: workspace_id.into(),
            provider_profile_id: None,
            provider_id: Some("openai-compatible".into()),
            template_revision: Some(1),
            stream_protocol: Some("openai_sse".into()),
            auth_placement: Some("bearer_header".into()),
            auth_header_name: Some("Authorization".into()),
            additional_headers_json: "{}".into(),
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
        context_update: None,
    }
}

#[tokio::test]
async fn migration_creates_the_complete_strict_schema() {
    let repository = SqliteRepository::connect_in_memory()
        .await
        .expect("repository opens");

    let schema = repository.schema_info().await.expect("schema is readable");

    assert_eq!(schema.version, 7);
    assert_eq!(
        schema.strict_tables,
        vec![
            "agent_mission",
            "agent_operation",
            "branch_checkpoint_inheritance",
            "branch_pointer",
            "branch_revision",
            "content_block",
            "context_checkpoint",
            "context_draft",
            "context_maintenance_run",
            "context_manifest",
            "context_manifest_item",
            "context_override_item",
            "context_snapshot",
            "decision_mark",
            "model_run",
            "provider_profile",
            "turn",
            "view_state",
            "workspace",
            "workspace_context_cursor",
        ]
    );
}

fn legacy_fixture_bytes(schema_version: i64) -> &'static [u8] {
    match schema_version {
        1 => include_bytes!("../../../tests/fixtures/sqlite/legacy-v1.sqlite"),
        2 => include_bytes!("../../../tests/fixtures/sqlite/legacy-v2.sqlite"),
        3 => include_bytes!("../../../tests/fixtures/sqlite/legacy-v3.sqlite"),
        4 => include_bytes!("../../../tests/fixtures/sqlite/legacy-v4.sqlite"),
        _ => panic!("no frozen legacy fixture for schema v{schema_version}"),
    }
}

async fn assert_real_file_upgrade_from(schema_version: i64) {
    let directory = tempfile::tempdir().expect("temporary directory is created");
    let path = directory
        .path()
        .join(format!("thoughtsflow-v{schema_version}.sqlite"));
    std::fs::write(&path, legacy_fixture_bytes(schema_version))
        .expect("frozen legacy fixture is copied without mutation");

    let repository = SqliteRepository::connect(&path)
        .await
        .expect("production repository migrator upgrades the legacy file");
    let schema = repository.schema_info().await.expect("schema is readable");
    assert_eq!(schema.version, 7);
    assert_eq!(
        schema.strict_tables,
        vec![
            "agent_mission",
            "agent_operation",
            "branch_checkpoint_inheritance",
            "branch_pointer",
            "branch_revision",
            "content_block",
            "context_checkpoint",
            "context_draft",
            "context_maintenance_run",
            "context_manifest",
            "context_manifest_item",
            "context_override_item",
            "context_snapshot",
            "decision_mark",
            "model_run",
            "provider_profile",
            "turn",
            "view_state",
            "workspace",
            "workspace_context_cursor",
        ],
        "the real-file upgrade must preserve STRICT mode on every domain table"
    );
    let fallback = repository
        .get_context_cursor("workspace-upgrade")
        .await
        .expect("a migrated workspace exposes a deterministic cursor fallback");
    assert_eq!(fallback.active_run_id.as_deref(), Some("run-upgrade"));
    assert_eq!(
        fallback.branch_pointer_id.as_deref(),
        Some("branch-upgrade")
    );
    assert_eq!(fallback.version, 0);
    drop(repository);

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .foreign_keys(true);
    let mut connection = options.connect().await.expect("upgraded database opens");
    let applied = sqlx::query_as::<_, (i64, Vec<u8>)>(
        "SELECT version, checksum FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
    )
    .fetch_all(&mut connection)
    .await
    .expect("upgraded migration history is readable");
    assert_eq!(
        applied
            .iter()
            .map(|(version, _)| *version)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7]
    );
    let index_columns = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_index_info('idx_manifest_item_workspace_content') ORDER BY seqno",
    )
    .fetch_all(&mut connection)
    .await
    .unwrap();
    assert_eq!(index_columns, ["workspace_id", "content_block_id"]);
    for (version, checksum) in &applied {
        let migration = TEST_MIGRATOR
            .iter()
            .find(|migration| migration.version == *version)
            .expect("applied migration remains in the project migrator");
        assert_eq!(checksum.as_slice(), migration.checksum.as_ref());
    }

    let metadata = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT provider_id, template_revision, stream_protocol, auth_placement, auth_header_name \
         FROM context_snapshot WHERE id = 'snapshot-upgrade'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("legacy Receipt is readable after upgrade");
    assert_eq!(
        metadata,
        (None, None, None, None, None),
        "upgrade must not invent Provider Template semantics for a legacy Receipt"
    );
    let receipt_evidence = sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<String>,
            i64,
        ),
    >(
        "SELECT s.canonical_hash, m.canonical_hash, i.content_block_id, i.position, \
                i.source_ref_kind, i.source_ref_id, i.mandatory \
         FROM context_manifest m \
         JOIN context_snapshot s ON s.manifest_id = m.id \
         JOIN context_manifest_item i ON i.manifest_id = m.id \
         WHERE m.id = 'manifest-upgrade'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("legacy Receipt items remain readable after upgrade");
    assert_eq!(
        receipt_evidence,
        (
            "hash-upgrade".into(),
            "hash-upgrade".into(),
            "prompt-upgrade".into(),
            0,
            None,
            None,
            0
        ),
        "v5 preserves the old canonical hash and leaves legacy typed identity NULL/NULL"
    );
    let cursor_rows = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workspace_context_cursor")
        .fetch_one(&mut connection)
        .await
        .expect("cursor table is readable");
    assert_eq!(
        cursor_rows, 0,
        "reading a v4 fallback must not synthesize persisted history"
    );
    let baseline = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT revision, change_kind, head_run_id \
         FROM branch_revision WHERE branch_pointer_id = 'branch-upgrade'",
    )
    .fetch_one(&mut connection)
    .await
    .expect("legacy branch receives one explicit migration baseline");
    assert_eq!(
        baseline,
        (3, "migration_baseline".into(), "run-upgrade".into())
    );

    let immutable = sqlx::query(
        "UPDATE context_snapshot SET provider = 'rewritten' WHERE id = 'snapshot-upgrade'",
    )
    .execute(&mut connection)
    .await
    .expect_err("the immutable Receipt trigger must survive the real upgrade");
    assert!(
        immutable
            .to_string()
            .contains("context snapshots are immutable")
    );
    let immutable_manifest = sqlx::query(
        "UPDATE context_manifest \
         SET checkpoint_provenance_json = '{}' WHERE id = 'manifest-upgrade'",
    )
    .execute(&mut connection)
    .await
    .expect_err("v5 provenance columns must not create a Receipt rewrite seam");
    assert!(
        immutable_manifest
            .to_string()
            .contains("context manifests are immutable")
    );
    let immutable_item = sqlx::query(
        "UPDATE context_manifest_item \
         SET source_ref_kind = 'model_run', source_ref_id = 'run-upgrade' \
         WHERE manifest_id = 'manifest-upgrade'",
    )
    .execute(&mut connection)
    .await
    .expect_err("v5 typed identity columns must remain immutable for legacy Receipt items");
    assert!(
        immutable_item
            .to_string()
            .contains("context manifest items are immutable")
    );
    let foreign_key_violations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut connection)
            .await
            .expect("foreign-key integrity is inspectable");
    assert_eq!(
        foreign_key_violations, 0,
        "the full production migration must preserve every legacy foreign key"
    );

    connection.close().await.expect("upgraded database closes");
}

#[tokio::test]
async fn real_file_v1_database_upgrades_through_the_production_migrator() {
    assert_real_file_upgrade_from(1).await;
}

#[tokio::test]
async fn real_file_v2_database_upgrades_through_the_production_migrator() {
    assert_real_file_upgrade_from(2).await;
}

#[tokio::test]
async fn real_file_v3_database_upgrades_through_the_production_migrator() {
    assert_real_file_upgrade_from(3).await;
}

#[tokio::test]
async fn real_file_v4_database_upgrades_through_the_production_migrator() {
    assert_real_file_upgrade_from(4).await;
}

#[tokio::test]
async fn provider_dialect_migration_preserves_rows_foreign_keys_and_strictness() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0001_core.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0002_workspace_goal.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0003_provider_template.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        "INSERT INTO provider_profile \
         (id, provider_id, name, dialect, base_url, default_model, created_at, updated_at) \
         VALUES ('provider-old', 'openai-compatible', 'Old provider', \
                 'openai_chat_completions', 'https://example.com/v1', 'old-model', 1, 1); \
         INSERT INTO workspace \
         (id, title, goal, system_prompt, created_at, updated_at) \
         VALUES ('workspace-old', 'Old workspace', '', '', 1, 1); \
         INSERT INTO content_block \
         (id, role, content, content_hash, created_at) \
         VALUES ('prompt-old', 'user', 'hello', 'prompt-hash-old', 1); \
         INSERT INTO turn \
         (id, workspace_id, parent_run_id, prompt_block_id, title, created_at) \
         VALUES ('turn-old', 'workspace-old', NULL, 'prompt-old', '', 1); \
         INSERT INTO model_run \
         (id, turn_id, workspace_id, provider_profile_id, model, status, output_markdown, \
          reasoning_markdown, provider_snapshot_json, created_at, finished_at) \
         VALUES ('run-old', 'turn-old', 'workspace-old', 'provider-old', 'old-model', \
                 'completed', 'answer', '', '{}', 1, 2);",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../migrations/0004_provider_dialects.sql"
    ))
    .execute(&pool)
    .await
    .expect("dialect migration expands the parent table atomically");

    let old = sqlx::query_as::<_, (String, String, String)>(
        "SELECT provider_id, protocol_dialect, default_model FROM provider_profile WHERE id = 'provider-old'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        old,
        (
            "openai-compatible".into(),
            "openai_chat_completions".into(),
            "old-model".into()
        )
    );
    let strict: i64 =
        sqlx::query_scalar("SELECT strict FROM pragma_table_list WHERE name = 'provider_profile'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(strict, 1);
    let foreign_key_errors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(foreign_key_errors, 0);

    sqlx::query(
        "INSERT INTO provider_profile \
         (id, provider_id, name, dialect, protocol_dialect, base_url, default_model, created_at, updated_at) \
         VALUES ('provider-anthropic', 'anthropic', 'Anthropic', 'openai_chat_completions', \
                 'anthropic_messages', 'https://api.anthropic.com', 'claude', 3, 3), \
                ('provider-google', 'google', 'Google', 'openai_chat_completions', \
                 'google_generative_ai', 'https://generativelanguage.googleapis.com/v1beta', \
                 'gemini', 3, 3)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let invalid = sqlx::query(
        "INSERT INTO provider_profile \
         (id, provider_id, name, dialect, protocol_dialect, base_url, default_model, created_at, updated_at) \
         VALUES ('provider-invalid', 'invalid', 'Invalid', 'openai_chat_completions', \
                 'query_injected', 'https://example.com', 'model', 3, 3)",
    )
    .execute(&pool)
    .await
    .expect_err("the widened CHECK must still fail closed for unknown dialects");
    assert!(invalid.to_string().contains("CHECK constraint failed"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_profile")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3, "the rejected statement must roll back completely");
    assert!(
        sqlx::query("DELETE FROM provider_profile WHERE id = 'provider-old'")
            .execute(&pool)
            .await
            .is_err(),
        "the existing model_run foreign key must survive the table rebuild"
    );
}

#[tokio::test]
async fn repository_round_trips_every_provider_dialect_through_the_strict_schema() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    let profiles = [
        (
            "profile-openai",
            "openai",
            ProviderDialect::OpenAiCompatible,
        ),
        ("profile-ollama", "ollama", ProviderDialect::Ollama),
        ("profile-anthropic", "anthropic", ProviderDialect::Anthropic),
        (
            "profile-google",
            "google",
            ProviderDialect::GoogleGenerativeAi,
        ),
    ];

    for (index, (id, provider_id, dialect)) in profiles.into_iter().enumerate() {
        let profile = ProviderProfile {
            id: id.into(),
            provider_id: provider_id.into(),
            name: format!("Provider {index}"),
            dialect,
            base_url: "https://provider.example.com/v1".into(),
            model: "model".into(),
            parameters: BTreeMap::new(),
            created_at: 1,
            updated_at: 1,
        };
        RepositoryPort::save_provider_profile(&repository, profile.clone())
            .await
            .unwrap();
        assert_eq!(
            RepositoryPort::get_provider_profile(&repository, id)
                .await
                .unwrap(),
            profile
        );
    }
}

#[tokio::test]
async fn provider_template_migration_assigns_stable_ids_to_existing_profiles() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0001_core.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO provider_profile \
         (id, name, dialect, base_url, default_model, created_at, updated_at) \
         VALUES ('openai-old', 'OpenAI old', 'openai_chat_completions', 'https://example.com/v1', 'gpt', 1, 1), \
                ('ollama-old', 'Ollama old', 'ollama_chat', 'http://127.0.0.1:11434', 'qwen3', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        "INSERT INTO workspace \
         (id, title, system_prompt, created_at, updated_at) \
         VALUES ('workspace-old', 'Old workspace', '', 1, 1); \
         INSERT INTO content_block \
         (id, role, content, content_hash, created_at) \
         VALUES ('prompt-old', 'user', 'hello', 'prompt-hash-old', 1); \
         INSERT INTO turn \
         (id, workspace_id, parent_run_id, prompt_block_id, title, created_at) \
         VALUES ('turn-old', 'workspace-old', NULL, 'prompt-old', '', 1); \
         INSERT INTO model_run \
         (id, turn_id, workspace_id, provider_profile_id, model, status, output_markdown, \
          reasoning_markdown, provider_snapshot_json, created_at, finished_at) \
         VALUES ('run-old', 'turn-old', 'workspace-old', 'openai-old', 'qwen3', 'completed', \
                 'answer', '', '{\"dialect\":\"ollama_chat\"}', 1, 2); \
         INSERT INTO context_manifest \
         (id, workspace_id, compiler_version, strategy, estimated_chars, canonical_hash, \
          warnings_json, created_at) \
         VALUES ('manifest-old', 'workspace-old', '1', 'ancestor_path_with_pins', 5, \
                 'hash-old', '[]', 1); \
         INSERT INTO context_snapshot \
         (id, run_id, manifest_id, workspace_id, provider_profile_id, provider, model, base_url, \
          parameters_json, request_json, canonical_hash, created_at) \
         VALUES ('snapshot-old', 'run-old', 'manifest-old', 'workspace-old', 'openai-old', \
                 'Ollama old', 'qwen3', 'http://127.0.0.1:11434', '{}', '{}', 'hash-old', 1);",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../migrations/0003_provider_template.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, provider_id FROM provider_profile ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("ollama-old".into(), "ollama".into()),
            ("openai-old".into(), "openai-compatible".into()),
        ]
    );
    let snapshot = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT provider_id, template_revision, stream_protocol, auth_placement, auth_header_name \
         FROM context_snapshot WHERE id = 'snapshot-old'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        snapshot,
        (None, None, None, None, None),
        "legacy Receipts must not be retroactively assigned current template semantics",
    );
    let immutable =
        sqlx::query("UPDATE context_snapshot SET provider = 'rewritten' WHERE id = 'snapshot-old'")
            .execute(&pool)
            .await
            .expect_err("migration must restore the immutable snapshot trigger");
    assert!(
        immutable
            .to_string()
            .contains("context snapshots are immutable")
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
            provider_id: "ollama".into(),
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
async fn workspace_run_provenance_is_loaded_from_snapshots_in_one_projection() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab A"))
        .await
        .unwrap();
    repository
        .create_workspace(&workspace("workspace-b", "Decision lab B"))
        .await
        .unwrap();

    let mut first = root_bundle("workspace-a", "turn-a", "run-a");
    first.run.model = "immutable-model-a".into();
    first.snapshot.provider = "Original provider A".into();
    first.snapshot.base_url = "https://original-a.example/v1".into();
    first.snapshot.model = "immutable-model-a".into();
    repository.persist_run_start(&first).await.unwrap();

    let mut retry = root_bundle("workspace-a", "turn-a", "run-b");
    retry.turn = None;
    retry.run.model = "immutable-model-b".into();
    retry.snapshot.provider = "Original provider B".into();
    retry.snapshot.base_url = "https://original-b.example/v1".into();
    retry.snapshot.model = "immutable-model-b".into();
    repository.persist_run_start(&retry).await.unwrap();

    let mut other_workspace = root_bundle("workspace-b", "turn-c", "run-c");
    other_workspace.snapshot.provider = "Other workspace provider".into();
    repository
        .persist_run_start(&other_workspace)
        .await
        .unwrap();

    assert_eq!(
        SqliteRepository::list_run_provider_provenance(&repository, "workspace-a")
            .await
            .unwrap(),
        vec![
            RunProviderProvenanceRecord {
                run_id: "run-a".into(),
                provider_name: "Original provider A".into(),
                base_url: "https://original-a.example/v1".into(),
                model: "immutable-model-a".into(),
            },
            RunProviderProvenanceRecord {
                run_id: "run-b".into(),
                provider_name: "Original provider B".into(),
                base_url: "https://original-b.example/v1".into(),
                model: "immutable-model-b".into(),
            },
        ]
    );
    assert_eq!(
        RepositoryPort::list_run_provider_provenance(&repository, "workspace-a")
            .await
            .unwrap(),
        vec![
            RunProviderProvenance {
                run_id: "run-a".into(),
                provider_name: "Original provider A".into(),
                base_url: "https://original-a.example/v1".into(),
                model: "immutable-model-a".into(),
            },
            RunProviderProvenance {
                run_id: "run-b".into(),
                provider_name: "Original provider B".into(),
                base_url: "https://original-b.example/v1".into(),
                model: "immutable-model-b".into(),
            },
        ]
    );
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
async fn new_receipt_items_require_a_paired_nonempty_typed_source_identity() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();
    let mut bundle = root_bundle("workspace-a", "turn-a", "run-a");
    bundle.context_items[0].source_ref_kind = "model_run".into();
    bundle.context_items[0].source_ref_id = None;

    let error = repository
        .persist_run_start(&bundle)
        .await
        .expect_err("new Receipt items cannot persist a half-typed source identity");
    assert!(matches!(error, RepositoryError::InvalidInput(_)));
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
async fn sqlite_rejects_half_typed_receipt_items_even_when_repository_validation_is_bypassed() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "Decision lab"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();

    let mut connection = repository.acquire_test_connection().await.unwrap();
    let error = sqlx::query(
        "INSERT INTO context_manifest_item \
         (manifest_id, workspace_id, position, source_id, source_ref_kind, source_ref_id, \
          source_kind, role, content_block_id, inclusion_reason, mandatory) \
         VALUES ('manifest-run-a', 'workspace-a', 1, 'run-a', 'model_run', NULL, \
                 'ancestor_answer', 'assistant', 'block-turn-a', 'ancestor_path', 0)",
    )
    .execute(&mut *connection)
    .await
    .expect_err("the v5 trigger must reject a half-typed source identity");
    assert!(
        error
            .to_string()
            .contains("typed context source identity must be paired and nonempty")
    );
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
            source_ref_kind: "workspace_system".into(),
            source_ref_id: Some("workspace-a".into()),
            source_kind: "system".into(),
            role: "system".into(),
            content_block_id: "block-system-a".into(),
            inclusion_reason: "system_policy".into(),
            mandatory: true,
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
async fn workspace_content_lookup_preserves_ownership_deduplication_and_order() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    for id in ["workspace-a", "workspace-b"] {
        repository
            .create_workspace(&workspace(id, id))
            .await
            .unwrap();
    }
    let mut first = root_bundle("workspace-a", "turn-a", "run-a");
    let receipt_only = ContentBlockRecord {
        id: "block-receipt-only".into(),
        role: "assistant".into(),
        content: "Shared immutable evidence".into(),
        content_hash: "shared-evidence-hash".into(),
        created_at: 20,
    };
    first.content_blocks.push(receipt_only.clone());
    first.content_blocks.push(ContentBlockRecord {
        id: "unreferenced".into(),
        role: "user".into(),
        content: "No workspace owns this block".into(),
        content_hash: "unreferenced-hash".into(),
        created_at: 1,
    });
    let mut item = first.context_items[0].clone();
    item.position = 1;
    item.content_block_id = receipt_only.id.clone();
    item.role = receipt_only.role.clone();
    first.context_items.push(item);
    repository.persist_run_start(&first).await.unwrap();
    let receipt = repository.get_run_receipt("run-a").await.unwrap();

    let mut second = root_bundle("workspace-b", "turn-b", "run-b");
    let mut shared = second.context_items[0].clone();
    shared.position = 1;
    shared.content_block_id = receipt_only.id.clone();
    shared.role = receipt_only.role.clone();
    second.context_items.push(shared);
    repository.persist_run_start(&second).await.unwrap();

    let draft_only = ContentBlockRecord {
        id: "block-draft-only".into(),
        role: "manual".into(),
        content: "Pinned for the next send".into(),
        content_hash: "draft-only-hash".into(),
        created_at: 19,
    };
    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: None,
            expected_version: 0,
            content_blocks: vec![draft_only.clone()],
            items: [&draft_only, &first.content_blocks[0]]
                .iter()
                .enumerate()
                .map(|(position, block)| ContextOverrideItemRecord {
                    workspace_id: "workspace-a".into(),
                    position: position as i64,
                    operation: "pin".into(),
                    source_kind: "content_block".into(),
                    source_id: Some(block.id.clone()),
                    content_block_id: Some(block.id.clone()),
                    content_hash: Some(block.content_hash.clone()),
                    created_at: 21,
                })
                .collect(),
            updated_at: 21,
        })
        .await
        .unwrap();

    // A prompt can exist without a Receipt, including a soft-deleted Turn.
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO content_block VALUES \
             ('turn-only', 'user', 'Historical prompt', 'turn-only-hash', 21); \
         INSERT INTO turn (id, workspace_id, prompt_block_id, created_at, deleted_at) \
             VALUES ('deleted-turn', 'workspace-a', 'turn-only', 21, 22);",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    drop(connection);

    let expected_ids = [
        "block-draft-only",
        "block-receipt-only",
        "block-turn-a",
        "turn-only",
    ];
    let blocks = repository.list_content_blocks("workspace-a").await.unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>(),
        expected_ids
    );
    assert_eq!(
        repository
            .load_workspace_context_records("workspace-a")
            .await
            .unwrap()
            .content_blocks,
        blocks
    );
    assert_eq!(
        repository.list_content_blocks("workspace-b").await.unwrap(),
        vec![receipt_only, second.content_blocks[0].clone()]
    );
    assert!(
        repository
            .list_content_blocks("missing-workspace")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(repository.get_run_receipt("run-a").await.unwrap(), receipt);
}

async fn workspace_content_read_steps(repository: &SqliteRepository) -> usize {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let mut connection = repository.acquire_test_connection().await.unwrap();
    connection
        .lock_handle()
        .await
        .unwrap()
        .set_progress_handler(100, move || {
            observed.fetch_add(100, Ordering::Relaxed);
            true
        });
    drop(connection);
    let blocks = repository.list_content_blocks("workspace-a").await.unwrap();
    assert_eq!(blocks.len(), 20);
    let mut connection = repository.acquire_test_connection().await.unwrap();
    connection
        .lock_handle()
        .await
        .unwrap()
        .remove_progress_handler();
    count.load(Ordering::Relaxed)
}

#[tokio::test]
async fn workspace_content_read_work_does_not_scale_with_unrelated_history() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    for id in ["workspace-a", "workspace-b"] {
        repository
            .create_workspace(&workspace(id, id))
            .await
            .unwrap();
    }
    for index in 0..20 {
        repository
            .persist_run_start(&root_bundle(
                "workspace-a",
                &format!("turn-{index}"),
                &format!("run-{index}"),
            ))
            .await
            .unwrap();
    }
    // Warm the statement before comparing SQLite VM work, independent of CPU speed.
    workspace_content_read_steps(&repository).await;
    let baseline = workspace_content_read_steps(&repository).await;
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::raw_sql(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000) \
         INSERT INTO content_block \
         SELECT 'noise-' || i, 'user', 'Other workspace ' || i, 'noise-hash-' || i, i FROM n; \
         INSERT INTO turn (id, workspace_id, prompt_block_id, created_at) \
         SELECT 'turn-' || id, 'workspace-b', id, created_at FROM content_block WHERE id LIKE 'noise-%'; \
         INSERT INTO context_manifest \
             (id, workspace_id, compiler_version, strategy, estimated_chars, canonical_hash, created_at) \
         SELECT 'manifest-' || id, 'workspace-b', '4', 'test', 1, 'canonical-' || id, created_at \
         FROM content_block WHERE id LIKE 'noise-%'; \
         INSERT INTO context_manifest_item \
             (manifest_id, workspace_id, position, source_kind, role, content_block_id, inclusion_reason) \
         SELECT 'manifest-' || id, 'workspace-b', 0, 'current_prompt', 'user', id, 'current_prompt' \
         FROM content_block WHERE id LIKE 'noise-%';",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    drop(connection);
    let with_unrelated_history = workspace_content_read_steps(&repository).await;
    assert!(
        with_unrelated_history <= baseline * 2 + 1000,
        "loading the same 20 blocks must stay workspace-scoped: {baseline} -> {with_unrelated_history} SQLite VM steps"
    );
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

#[tokio::test]
async fn context_cursor_uses_read_only_v4_fallback_then_requires_versioned_cas() {
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

    let fallback = repository.get_context_cursor("workspace-a").await.unwrap();
    assert_eq!(fallback.active_run_id.as_deref(), Some("run-a"));
    assert_eq!(fallback.branch_pointer_id.as_deref(), Some("branch-main"));
    assert_eq!(
        fallback.version, 0,
        "a legacy fallback is not invented history"
    );
    assert_eq!(
        repository
            .get_context_cursor("workspace-a")
            .await
            .unwrap()
            .version,
        0,
        "a read must not persist the fallback"
    );

    let stored = repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 0,
            updated_at: 30,
        })
        .await
        .unwrap();
    assert_eq!(stored.version, 1);

    let stale = repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: None,
            branch_pointer_id: None,
            expected_version: 0,
            updated_at: 31,
        })
        .await
        .expect_err("a stale cursor writer must not overwrite version 1");
    assert!(matches!(
        stale,
        RepositoryError::VersionConflict {
            resource: "context_cursor",
            expected: 0,
            actual: 1,
            ..
        }
    ));
    assert_eq!(
        repository.get_context_cursor("workspace-a").await.unwrap(),
        stored
    );
}

#[tokio::test]
async fn cursor_cas_returns_its_own_outcome_when_the_next_writer_is_already_waiting() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-b", "run-b"))
        .await
        .unwrap();
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: None,
            expected_version: 0,
            updated_at: 30,
        })
        .await
        .unwrap();

    let held_connection = repository.acquire_test_connection().await.unwrap();
    let first_repository = repository.clone();
    let mut first = tokio::spawn(async move {
        first_repository
            .set_context_cursor(&ContextCursorUpdateRecord {
                workspace_id: "workspace-a".into(),
                active_run_id: Some("run-b".into()),
                branch_pointer_id: None,
                expected_version: 1,
                updated_at: 31,
            })
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut first)
            .await
            .is_err(),
        "the first writer must be waiting for the held SQLite connection"
    );
    let second_repository = repository.clone();
    let mut second = tokio::spawn(async move {
        second_repository
            .set_context_cursor(&ContextCursorUpdateRecord {
                workspace_id: "workspace-a".into(),
                active_run_id: Some("run-a".into()),
                branch_pointer_id: None,
                expected_version: 2,
                updated_at: 32,
            })
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut second)
            .await
            .is_err(),
        "the second writer must be queued behind the first writer"
    );

    drop(held_connection);
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();

    assert_eq!(first.version, 2);
    assert_eq!(first.active_run_id.as_deref(), Some("run-b"));
    assert_eq!(second.version, 3);
    assert_eq!(second.active_run_id.as_deref(), Some("run-a"));
}

#[tokio::test]
async fn active_context_and_draft_rebase_commit_or_roll_back_together() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-b", "run-b"))
        .await
        .unwrap();
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: None,
            expected_version: 0,
            updated_at: 30,
        })
        .await
        .unwrap();
    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 0,
            content_blocks: Vec::new(),
            items: vec![ContextOverrideItemRecord {
                workspace_id: "workspace-a".into(),
                position: 0,
                operation: "pin".into(),
                source_kind: "content_block".into(),
                source_id: Some("block-turn-a".into()),
                content_block_id: Some("block-turn-a".into()),
                content_hash: Some("hash-turn-a".into()),
                created_at: 31,
            }],
            updated_at: 31,
        })
        .await
        .unwrap();

    let rebased = repository
        .set_context_cursor_and_rebase_draft(
            &ContextCursorUpdateRecord {
                workspace_id: "workspace-a".into(),
                active_run_id: Some("run-b".into()),
                branch_pointer_id: None,
                expected_version: 1,
                updated_at: 32,
            },
            1,
        )
        .await
        .unwrap();
    assert_eq!(rebased.version, 2);
    assert_eq!(rebased.active_run_id.as_deref(), Some("run-b"));
    let draft = repository.get_context_draft("workspace-a").await.unwrap();
    assert_eq!(draft.version, 2);
    assert_eq!(draft.parent_run_id.as_deref(), Some("run-b"));
    assert!(draft.items.is_empty());
    assert_eq!(draft.consumed_by_run_id, None);

    let stale_draft = repository
        .set_context_cursor_and_rebase_draft(
            &ContextCursorUpdateRecord {
                workspace_id: "workspace-a".into(),
                active_run_id: Some("run-a".into()),
                branch_pointer_id: None,
                expected_version: 2,
                updated_at: 33,
            },
            1,
        )
        .await
        .expect_err("a stale draft CAS rolls back the preceding cursor CAS");
    assert!(matches!(
        stale_draft,
        RepositoryError::VersionConflict {
            resource: "context_draft",
            expected: 1,
            actual: 2,
            ..
        }
    ));
    assert_eq!(
        repository.get_context_cursor("workspace-a").await.unwrap(),
        rebased
    );

    let stale_cursor = repository
        .set_context_cursor_and_rebase_draft(
            &ContextCursorUpdateRecord {
                workspace_id: "workspace-a".into(),
                active_run_id: Some("run-a".into()),
                branch_pointer_id: None,
                expected_version: 1,
                updated_at: 34,
            },
            2,
        )
        .await
        .expect_err("a stale cursor CAS must leave the draft untouched");
    assert!(matches!(
        stale_cursor,
        RepositoryError::VersionConflict {
            resource: "context_cursor",
            expected: 1,
            actual: 2,
            ..
        }
    ));
    assert_eq!(
        repository.get_context_draft("workspace-a").await.unwrap(),
        draft
    );
}

#[tokio::test]
async fn workspace_context_projection_uses_one_snapshot_when_a_writer_is_waiting() {
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
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 0,
            updated_at: 30,
        })
        .await
        .unwrap();

    let mut child = root_bundle("workspace-a", "turn-b", "run-b");
    child.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    child.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-b".into(),
        version: 1,
        created_at: 20,
        updated_at: 31,
    });
    child.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 1,
        expected_draft_version: 0,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
        result_branch_pointer_id: Some("branch-main".into()),
        updated_at: 31,
    });

    let held_connection = repository.acquire_test_connection().await.unwrap();
    let reader_repository = repository.clone();
    let mut reader = tokio::spawn(async move {
        reader_repository
            .load_workspace_context_records("workspace-a")
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut reader)
            .await
            .is_err(),
        "the reader must be waiting for the held SQLite connection"
    );
    let writer_repository = repository.clone();
    let mut writer = tokio::spawn(async move { writer_repository.persist_run_start(&child).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut writer)
            .await
            .is_err(),
        "the writer must be queued behind the reader"
    );

    drop(held_connection);
    let snapshot = reader.await.unwrap().unwrap();
    writer.await.unwrap().unwrap();

    assert_eq!(snapshot.cursor.version, 1);
    assert_eq!(snapshot.cursor.active_run_id.as_deref(), Some("run-a"));
    assert!(
        snapshot.runs.iter().all(|run| run.id != "run-b"),
        "one projection cannot mix the old cursor with the next committed Run"
    );
    assert_eq!(snapshot.branch_pointers[0].head_run_id, "run-a");
    assert_eq!(snapshot.branch_pointers[0].version, 0);
    assert_eq!(snapshot.draft.version, 0);
    assert_eq!(snapshot.draft.consumed_by_run_id, None);

    let current = repository
        .load_workspace_context_records("workspace-a")
        .await
        .unwrap();
    assert_eq!(current.cursor.version, 2);
    assert_eq!(current.cursor.active_run_id.as_deref(), Some("run-b"));
    assert!(current.runs.iter().any(|run| run.id == "run-b"));
}

#[tokio::test]
async fn workspace_context_projection_counts_eight_logical_selects() {
    let repository = SqliteRepository::connect_in_memory_with_context_read_probe()
        .await
        .unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();

    SqliteRepository::reset_context_read_count();
    let records = repository
        .load_workspace_context_records("workspace-a")
        .await
        .unwrap();

    assert_eq!(records.runs.len(), 1);
    assert_eq!(
        SqliteRepository::context_read_count(),
        8,
        "the probe counts the eight logical SELECT helpers, not pool acquisitions"
    );
}

#[tokio::test]
async fn context_draft_round_trips_typed_pins_and_rejects_mandatory_exclusions() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    repository
        .create_workspace(&workspace("workspace-a", "A"))
        .await
        .unwrap();
    repository
        .persist_run_start(&root_bundle("workspace-a", "turn-a", "run-a"))
        .await
        .unwrap();

    let stored = repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 0,
            content_blocks: Vec::new(),
            items: vec![ContextOverrideItemRecord {
                workspace_id: "workspace-a".into(),
                position: 0,
                operation: "pin".into(),
                source_kind: "content_block".into(),
                source_id: Some("block-turn-a".into()),
                content_block_id: Some("block-turn-a".into()),
                content_hash: Some("hash-turn-a".into()),
                created_at: 30,
            }],
            updated_at: 30,
        })
        .await
        .unwrap();
    assert_eq!(stored.version, 1);
    assert_eq!(stored.items[0].source_kind, "content_block");
    assert_eq!(
        stored.items[0].content_block_id.as_deref(),
        Some("block-turn-a")
    );
    assert_eq!(stored.items[0].content_hash.as_deref(), Some("hash-turn-a"));

    for mandatory_kind in ["workspace_system", "current_prompt"] {
        let error = repository
            .update_context_draft(&ContextDraftUpdateRecord {
                workspace_id: "workspace-a".into(),
                parent_run_id: Some("run-a".into()),
                expected_version: 1,
                content_blocks: Vec::new(),
                items: vec![ContextOverrideItemRecord {
                    workspace_id: "workspace-a".into(),
                    position: 0,
                    operation: "exclude".into(),
                    source_kind: mandatory_kind.into(),
                    source_id: None,
                    content_block_id: None,
                    content_hash: None,
                    created_at: 31,
                }],
                updated_at: 31,
            })
            .await
            .expect_err("mandatory context cannot be persisted as excluded");
        assert!(error.to_string().contains("CHECK constraint failed"));
        assert_eq!(
            repository.get_context_draft("workspace-a").await.unwrap(),
            stored,
            "the failed update must roll back the draft version and typed pin"
        );
    }

    let fresh_content = "fresh completed leaf answer";
    let fresh_hash = crate::domain::sha256_hex(fresh_content.as_bytes());
    let fresh_block_id = format!("block-assistant-{fresh_hash}");
    let fresh_block = ContentBlockRecord {
        id: fresh_block_id.clone(),
        role: "assistant".into(),
        content: fresh_content.into(),
        content_hash: fresh_hash.clone(),
        created_at: 32,
    };
    let fresh_pin = ContextOverrideItemRecord {
        workspace_id: "workspace-a".into(),
        position: 0,
        operation: "pin".into(),
        source_kind: "model_run".into(),
        source_id: Some("run-a".into()),
        content_block_id: Some(fresh_block_id.clone()),
        content_hash: Some(fresh_hash),
        created_at: 32,
    };
    let stale = repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 0,
            content_blocks: vec![fresh_block.clone()],
            items: vec![fresh_pin.clone()],
            updated_at: 32,
        })
        .await
        .expect_err("a stale draft CAS must roll back its newly materialized Content Block");
    assert!(matches!(
        stale,
        RepositoryError::VersionConflict {
            resource: "context_draft",
            expected: 0,
            actual: 1,
            ..
        }
    ));
    assert!(
        repository
            .list_content_blocks("workspace-a")
            .await
            .unwrap()
            .iter()
            .all(|block| block.id != fresh_block_id),
        "a failed CAS cannot leak an otherwise valid fresh-leaf Content Block"
    );

    let fresh_draft = repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 1,
            content_blocks: vec![fresh_block],
            items: vec![fresh_pin],
            updated_at: 33,
        })
        .await
        .expect("the block and typed pin commit in one draft transaction");
    assert_eq!(fresh_draft.version, 2);
    assert_eq!(
        fresh_draft.items[0].content_block_id.as_deref(),
        Some(fresh_block_id.as_str())
    );
    assert!(
        repository
            .list_content_blocks("workspace-a")
            .await
            .unwrap()
            .iter()
            .any(|block| block.id == fresh_block_id)
    );
}

#[tokio::test]
async fn historical_run_start_forks_without_moving_the_old_head_and_each_stale_guard_rolls_back() {
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
    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 0,
            content_blocks: Vec::new(),
            items: vec![ContextOverrideItemRecord {
                workspace_id: "workspace-a".into(),
                position: 0,
                operation: "pin".into(),
                source_kind: "content_block".into(),
                source_id: Some("block-turn-a".into()),
                content_block_id: Some("block-turn-a".into()),
                content_hash: Some("hash-turn-a".into()),
                created_at: 30,
            }],
            updated_at: 30,
        })
        .await
        .unwrap();

    let mut child = root_bundle("workspace-a", "turn-b", "run-b");
    child.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    child.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-b".into(),
        version: 1,
        created_at: 20,
        updated_at: 31,
    });
    child.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 0,
        expected_draft_version: 1,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
        result_branch_pointer_id: Some("branch-main".into()),
        updated_at: 31,
    });

    let outcome = repository.persist_run_start(&child).await.unwrap();
    assert_eq!(outcome.cursor.active_run_id.as_deref(), Some("run-b"));
    assert_eq!(
        outcome.cursor.branch_pointer_id.as_deref(),
        Some("branch-main")
    );
    assert_eq!(outcome.cursor.version, 1);
    assert_eq!(outcome.draft_version, 2);
    assert_eq!(outcome.branch_pointer.unwrap().version, 1);
    let consumed = repository.get_context_draft("workspace-a").await.unwrap();
    assert_eq!(consumed.version, 2);
    assert_eq!(consumed.consumed_by_run_id.as_deref(), Some("run-b"));
    assert!(consumed.items.is_empty());
    assert_eq!(
        repository
            .list_branch_revisions("workspace-a")
            .await
            .unwrap()
            .iter()
            .map(|revision| revision.revision)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );

    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-a".into()),
            expected_version: 2,
            content_blocks: Vec::new(),
            items: Vec::new(),
            updated_at: 32,
        })
        .await
        .unwrap();

    let mut historical_fork = root_bundle("workspace-a", "turn-c", "run-c");
    historical_fork.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    historical_fork.branch_pointer = Some(BranchPointerRecord {
        id: "branch-fork".into(),
        workspace_id: "workspace-a".into(),
        name: "Fork from run A".into(),
        head_run_id: "run-c".into(),
        version: 0,
        created_at: 32,
        updated_at: 32,
    });
    historical_fork.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 1,
        expected_draft_version: 3,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(1),
        result_branch_pointer_id: Some("branch-fork".into()),
        updated_at: 32,
    });
    repository
        .persist_run_start(&historical_fork)
        .await
        .unwrap();
    let old_branch = repository.get_branch_pointer("branch-main").await.unwrap();
    assert_eq!(old_branch.head_run_id, "run-b");
    assert_eq!(old_branch.version, 1);
    let fork_branch = repository.get_branch_pointer("branch-fork").await.unwrap();
    assert_eq!(fork_branch.head_run_id, "run-c");
    assert_eq!(fork_branch.version, 0);
    let fork_cursor = repository.get_context_cursor("workspace-a").await.unwrap();
    assert_eq!(fork_cursor.active_run_id.as_deref(), Some("run-c"));
    assert_eq!(
        fork_cursor.branch_pointer_id.as_deref(),
        Some("branch-fork")
    );
    assert_eq!(fork_cursor.version, 2);
    assert_eq!(
        repository
            .get_context_draft("workspace-a")
            .await
            .unwrap()
            .version,
        4
    );

    for (suffix, cursor_version, draft_version, branch_version, resource) in [
        ("stale-cursor", 1, 4, 0, "context_cursor"),
        ("stale-draft", 2, 3, 0, "context_draft"),
        ("stale-branch", 2, 4, 1, "branch_pointer"),
    ] {
        let turn_id = format!("turn-{suffix}");
        let run_id = format!("run-{suffix}");
        let branch_id = format!("branch-{suffix}");
        let mut stale = root_bundle("workspace-a", &turn_id, &run_id);
        stale.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
        stale.branch_pointer = Some(BranchPointerRecord {
            id: branch_id.clone(),
            workspace_id: "workspace-a".into(),
            name: format!("Failed {suffix}"),
            head_run_id: run_id.clone(),
            version: 0,
            created_at: 40,
            updated_at: 40,
        });
        stale.context_update = Some(RunStartContextUpdateRecord {
            expected_cursor_version: cursor_version,
            expected_draft_version: draft_version,
            expected_branch_pointer_id: Some("branch-fork".into()),
            expected_branch_version: Some(branch_version),
            result_branch_pointer_id: Some(branch_id.clone()),
            updated_at: 40,
        });
        let error = repository
            .persist_run_start(&stale)
            .await
            .expect_err("each stale context guard must abort the whole Run start");
        assert!(
            matches!(
                error,
                RepositoryError::VersionConflict {
                    resource: actual,
                    ..
                } if actual == resource
            ),
            "unexpected guard error for {suffix}: {error:?}"
        );
        assert!(matches!(
            repository.get_turn(&turn_id).await,
            Err(RepositoryError::NotFound { .. })
        ));
        assert!(matches!(
            repository.get_run(&run_id).await,
            Err(RepositoryError::NotFound { .. })
        ));
        assert!(matches!(
            repository.get_run_receipt(&run_id).await,
            Err(RepositoryError::NotFound { .. })
        ));
        assert!(matches!(
            repository.get_branch_pointer(&branch_id).await,
            Err(RepositoryError::NotFound { .. })
        ));
    }

    assert_eq!(
        repository
            .get_context_cursor("workspace-a")
            .await
            .unwrap()
            .active_run_id
            .as_deref(),
        Some("run-c")
    );
    assert_eq!(
        repository.get_branch_pointer("branch-main").await.unwrap(),
        old_branch
    );
    assert_eq!(
        repository.get_branch_pointer("branch-fork").await.unwrap(),
        fork_branch
    );
}

#[tokio::test]
async fn run_start_repeatedly_consumes_the_empty_next_send_draft_without_manual_updates() {
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

    let mut first = root_bundle("workspace-a", "turn-b", "run-b");
    first.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    first.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-b".into(),
        version: 1,
        created_at: 20,
        updated_at: 30,
    });
    first.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 0,
        expected_draft_version: 0,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
        result_branch_pointer_id: Some("branch-main".into()),
        updated_at: 30,
    });
    let first_outcome = repository.persist_run_start(&first).await.unwrap();
    assert_eq!(first_outcome.draft_version, 1);
    finish_test_run(&repository, "run-b").await;

    let mut second = root_bundle("workspace-a", "turn-c", "run-c");
    second.turn.as_mut().unwrap().parent_run_id = Some("run-b".into());
    second.branch_pointer = Some(BranchPointerRecord {
        id: "branch-main".into(),
        workspace_id: "workspace-a".into(),
        name: "Main".into(),
        head_run_id: "run-c".into(),
        version: 2,
        created_at: 20,
        updated_at: 40,
    });
    second.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 1,
        expected_draft_version: 1,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(1),
        result_branch_pointer_id: Some("branch-main".into()),
        updated_at: 40,
    });
    let second_outcome = repository.persist_run_start(&second).await.unwrap();
    assert_eq!(second_outcome.draft_version, 2);
    assert_eq!(
        second_outcome.cursor.active_run_id.as_deref(),
        Some("run-c")
    );
    assert_eq!(
        repository
            .get_context_draft("workspace-a")
            .await
            .unwrap()
            .consumed_by_run_id
            .as_deref(),
        Some("run-c")
    );
}

#[tokio::test]
async fn maintenance_conflict_is_audited_without_activating_a_checkpoint() {
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
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 0,
            updated_at: 21,
        })
        .await
        .unwrap();
    let running = ContextMaintenanceRunRecord {
        id: "maintenance-a".into(),
        workspace_id: "workspace-a".into(),
        kind: "compaction".into(),
        status: "running".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "source-hash-a".into(),
        provider_snapshot_json: None,
        request_json: r#"{"instruction":"summarize"}"#.into(),
        summary_block_id: None,
        summary: None,
        error_json: None,
        created_at: 30,
        started_at: Some(30),
        finished_at: None,
    };
    let stale_guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-a".into(),
        expected_cursor_version: 1,
        expected_draft_version: None,
        branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
    };
    repository
        .start_context_maintenance(&running, &stale_guard)
        .await
        .unwrap();

    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 1,
            updated_at: 31,
        })
        .await
        .unwrap();

    let summary = ContentBlockRecord {
        id: "block-system-summary-a".into(),
        role: "system".into(),
        content: "Summary A".into(),
        content_hash: "summary-hash-a".into(),
        created_at: 32,
    };
    let checkpoint = ContextCheckpointRecord {
        id: "checkpoint-a".into(),
        workspace_id: "workspace-a".into(),
        maintenance_run_id: "maintenance-a".into(),
        kind: "compaction".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        summary_block_id: summary.id.clone(),
        summary: summary.content.clone(),
        summary_content_hash: summary.content_hash.clone(),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "source-hash-a".into(),
        provider_snapshot_json: None,
        created_at: 32,
    };
    let mut completed = running.clone();
    completed.status = "completed".into();
    completed.summary_block_id = Some(summary.id.clone());
    completed.summary = Some(summary.content.clone());
    completed.finished_at = Some(32);
    let context_update = FinishContextMaintenanceUpdateRecord {
        active_run_id: Some("run-a".into()),
        branch_pointer_id: Some("branch-main".into()),
        updated_at: 32,
    };

    let conflict = repository
        .finish_context_maintenance(
            &completed,
            Some(&checkpoint),
            Some(&summary),
            &stale_guard,
            Some(&context_update),
        )
        .await
        .expect_err("cursor movement during summarization must prevent activation");
    assert!(matches!(
        conflict,
        RepositoryError::VersionConflict {
            resource: "context_cursor",
            expected: 1,
            actual: 2,
            ..
        }
    ));
    let audited = repository
        .get_context_maintenance_run("maintenance-a")
        .await
        .unwrap();
    assert_eq!(audited.status, "conflicted");
    assert!(
        audited
            .error_json
            .unwrap()
            .contains("context_cursor_conflict")
    );
    assert!(
        repository
            .list_context_checkpoints("workspace-a")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repository.get_run_receipt("run-a").await.is_ok(),
        "the original immutable Receipt remains available"
    );
}

#[tokio::test]
async fn same_millisecond_checkpoints_keep_commit_order_and_inheritance_time() {
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
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 0,
            updated_at: 21,
        })
        .await
        .unwrap();
    let guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-a".into(),
        expected_cursor_version: 1,
        expected_draft_version: Some(0),
        branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
    };

    let first = checkpoint_attempt("z-first", 31);
    repository
        .start_context_maintenance(&first.0, &guard)
        .await
        .unwrap();
    repository
        .finish_context_maintenance(&first.1, Some(&first.2), Some(&first.3), &guard, None)
        .await
        .unwrap();

    let second = checkpoint_attempt("a-second", 31);
    repository
        .start_context_maintenance(&second.0, &guard)
        .await
        .unwrap();
    repository
        .finish_context_maintenance(&second.1, Some(&second.2), Some(&second.3), &guard, None)
        .await
        .unwrap();
    repository
        .finish_context_maintenance(&second.1, Some(&second.2), Some(&second.3), &guard, None)
        .await
        .expect("replaying a normalized checkpoint remains idempotent");

    let checkpoints = repository
        .list_context_checkpoints("workspace-a")
        .await
        .unwrap();
    assert_eq!(
        checkpoints
            .iter()
            .map(|checkpoint| (checkpoint.id.as_str(), checkpoint.created_at))
            .collect::<Vec<_>>(),
        vec![("checkpoint-z-first", 31), ("checkpoint-a-second", 32)],
        "persistent time, not a random checkpoint id, breaks same-millisecond ties"
    );
    let inheritance = repository
        .list_branch_checkpoint_inheritance("workspace-a")
        .await
        .unwrap();
    for checkpoint in &checkpoints {
        assert_eq!(
            inheritance
                .iter()
                .find(|evidence| evidence.checkpoint_id == checkpoint.id)
                .map(|evidence| evidence.inherited_at),
            Some(checkpoint.created_at),
            "inheritance must record the checkpoint's effective persisted time"
        );
    }
}

#[tokio::test]
async fn completed_maintenance_atomically_activates_one_immutable_checkpoint_and_cursor() {
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
    repository
        .set_context_cursor(&ContextCursorUpdateRecord {
            workspace_id: "workspace-a".into(),
            active_run_id: Some("run-a".into()),
            branch_pointer_id: Some("branch-main".into()),
            expected_version: 0,
            updated_at: 21,
        })
        .await
        .unwrap();
    repository
        .create_branch_pointer(&BranchPointerRecord {
            id: "branch-before-checkpoint".into(),
            workspace_id: "workspace-a".into(),
            name: "Before checkpoint".into(),
            head_run_id: "run-a".into(),
            version: 0,
            created_at: 22,
            updated_at: 22,
        })
        .await
        .unwrap();
    let guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-a".into(),
        expected_cursor_version: 1,
        expected_draft_version: Some(0),
        branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
    };
    let running = ContextMaintenanceRunRecord {
        id: "maintenance-success".into(),
        workspace_id: "workspace-a".into(),
        kind: "compaction".into(),
        status: "running".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "source-hash-success".into(),
        provider_snapshot_json: None,
        request_json: r#"{"instruction":"summarize"}"#.into(),
        summary_block_id: None,
        summary: None,
        error_json: None,
        created_at: 30,
        started_at: Some(30),
        finished_at: None,
    };
    repository
        .start_context_maintenance(&running, &guard)
        .await
        .unwrap();

    let summary = ContentBlockRecord {
        id: "block-system-summary-success".into(),
        role: "system".into(),
        content: "Auditable summary".into(),
        content_hash: "summary-hash-success".into(),
        created_at: 31,
    };
    let checkpoint = ContextCheckpointRecord {
        id: "checkpoint-success".into(),
        workspace_id: "workspace-a".into(),
        maintenance_run_id: "maintenance-success".into(),
        kind: "compaction".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        summary_block_id: summary.id.clone(),
        summary: summary.content.clone(),
        summary_content_hash: summary.content_hash.clone(),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "source-hash-success".into(),
        provider_snapshot_json: None,
        created_at: 31,
    };
    let mut completed = running.clone();
    completed.status = "completed".into();
    completed.summary_block_id = Some(summary.id.clone());
    completed.summary = Some(summary.content.clone());
    completed.finished_at = Some(31);
    let update = FinishContextMaintenanceUpdateRecord {
        active_run_id: Some("run-a".into()),
        branch_pointer_id: Some("branch-main".into()),
        updated_at: 31,
    };
    let stored = repository
        .finish_context_maintenance(
            &completed,
            Some(&checkpoint),
            Some(&summary),
            &guard,
            Some(&update),
        )
        .await
        .unwrap();
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.summary.as_deref(), Some("Auditable summary"));
    assert!(
        repository
            .list_content_blocks("workspace-a")
            .await
            .unwrap()
            .contains(&summary)
    );
    assert!(
        repository
            .load_workspace_context_records("workspace-a")
            .await
            .unwrap()
            .content_blocks
            .contains(&summary)
    );
    assert_eq!(
        repository
            .get_context_cursor("workspace-a")
            .await
            .unwrap()
            .version,
        2
    );
    assert_eq!(
        repository
            .list_context_checkpoints("workspace-a")
            .await
            .unwrap(),
        vec![checkpoint.clone()]
    );
    assert_eq!(
        repository
            .list_branch_checkpoint_inheritance("workspace-a")
            .await
            .unwrap(),
        vec![BranchCheckpointInheritanceRecord {
            workspace_id: "workspace-a".into(),
            branch_pointer_id: "branch-main".into(),
            checkpoint_id: "checkpoint-success".into(),
            inherited_at: 31,
        }],
        "a branch created before the checkpoint must not inherit it retroactively"
    );

    let replayed = repository
        .finish_context_maintenance(
            &completed,
            Some(&checkpoint),
            Some(&summary),
            &guard,
            Some(&update),
        )
        .await
        .expect("a retry after commit-before-response returns the completed operation");
    assert_eq!(replayed, stored);
    assert_eq!(
        repository
            .get_context_cursor("workspace-a")
            .await
            .unwrap()
            .version,
        2,
        "an idempotent replay must not move the cursor twice"
    );
    assert_eq!(
        repository
            .list_context_checkpoints("workspace-a")
            .await
            .unwrap(),
        vec![checkpoint.clone()]
    );
    let mut mismatched_checkpoint = checkpoint.clone();
    mismatched_checkpoint.id = "checkpoint-reused-operation".into();
    assert!(
        repository
            .finish_context_maintenance(
                &completed,
                Some(&mismatched_checkpoint),
                Some(&summary),
                &guard,
                Some(&update),
            )
            .await
            .is_err(),
        "the same operation id cannot be reused with different checkpoint evidence"
    );
    assert_eq!(
        repository
            .list_context_checkpoints("workspace-a")
            .await
            .unwrap(),
        vec![checkpoint]
    );

    let mut after = root_bundle("workspace-a", "turn-after", "run-after");
    after.turn.as_mut().unwrap().parent_run_id = Some("run-a".into());
    after.branch_pointer = Some(BranchPointerRecord {
        id: "branch-after-checkpoint".into(),
        workspace_id: "workspace-a".into(),
        name: "After checkpoint".into(),
        head_run_id: "run-after".into(),
        version: 0,
        created_at: 40,
        updated_at: 40,
    });
    after.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 2,
        expected_draft_version: 0,
        expected_branch_pointer_id: Some("branch-main".into()),
        expected_branch_version: Some(0),
        result_branch_pointer_id: Some("branch-after-checkpoint".into()),
        updated_at: 40,
    });
    repository.persist_run_start(&after).await.unwrap();
    finish_test_run(&repository, "run-after").await;

    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: "workspace-a".into(),
            parent_run_id: Some("run-after".into()),
            expected_version: 1,
            content_blocks: Vec::new(),
            items: Vec::new(),
            updated_at: 41,
        })
        .await
        .unwrap();
    let mut transitive = root_bundle("workspace-a", "turn-transitive", "run-transitive");
    transitive.turn.as_mut().unwrap().parent_run_id = Some("run-after".into());
    transitive.branch_pointer = Some(BranchPointerRecord {
        id: "branch-transitive".into(),
        workspace_id: "workspace-a".into(),
        name: "Transitive fork".into(),
        head_run_id: "run-transitive".into(),
        version: 0,
        created_at: 42,
        updated_at: 42,
    });
    transitive.context_update = Some(RunStartContextUpdateRecord {
        expected_cursor_version: 3,
        expected_draft_version: 2,
        expected_branch_pointer_id: Some("branch-after-checkpoint".into()),
        expected_branch_version: Some(0),
        result_branch_pointer_id: Some("branch-transitive".into()),
        updated_at: 42,
    });
    repository.persist_run_start(&transitive).await.unwrap();

    let inheritance = repository
        .list_branch_checkpoint_inheritance("workspace-a")
        .await
        .unwrap();
    assert_eq!(
        inheritance
            .iter()
            .map(|evidence| (
                evidence.branch_pointer_id.as_str(),
                evidence.checkpoint_id.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("branch-after-checkpoint", "checkpoint-success"),
            ("branch-main", "checkpoint-success"),
            ("branch-transitive", "checkpoint-success"),
        ],
        "a later fork copies explicit visibility and a child fork inherits it transitively"
    );
}

#[tokio::test]
async fn maintenance_operation_ids_are_idempotent_workspace_scoped_and_recovered_after_restart() {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    for (workspace_id, turn_id, run_id) in [
        ("workspace-a", "turn-a", "run-a"),
        ("workspace-b", "turn-b", "run-b"),
    ] {
        repository
            .create_workspace(&workspace(workspace_id, workspace_id))
            .await
            .unwrap();
        repository
            .persist_run_start(&root_bundle(workspace_id, turn_id, run_id))
            .await
            .unwrap();
    }
    let guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-a".into(),
        expected_cursor_version: 0,
        expected_draft_version: Some(0),
        branch_pointer_id: None,
        expected_branch_version: None,
    };
    let running = ContextMaintenanceRunRecord {
        id: "client-operation-a".into(),
        workspace_id: "workspace-a".into(),
        kind: "compaction".into(),
        status: "running".into(),
        branch_pointer_id: None,
        branch_revision: None,
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "operation-source-hash".into(),
        provider_snapshot_json: None,
        request_json: r#"{"instruction":"summarize"}"#.into(),
        summary_block_id: None,
        summary: None,
        error_json: None,
        created_at: 30,
        started_at: Some(30),
        finished_at: None,
    };
    let first_repository = repository.clone();
    let first_run = running.clone();
    let first_guard = guard.clone();
    let second_repository = repository.clone();
    let second_run = running.clone();
    let second_guard = guard.clone();
    let (first, concurrent_replay) = tokio::join!(
        async move {
            first_repository
                .start_context_maintenance(&first_run, &first_guard)
                .await
                .unwrap()
        },
        async move {
            second_repository
                .start_context_maintenance(&second_run, &second_guard)
                .await
                .unwrap()
        },
    );
    assert_eq!(
        usize::from(first.1) + usize::from(concurrent_replay.1),
        1,
        "two concurrent callers with one operation id authorize Provider work exactly once"
    );
    assert_eq!(first.0, concurrent_replay.0);
    let first = if first.1 { first } else { concurrent_replay };
    let mut replay_request = running.clone();
    replay_request.created_at += 1;
    replay_request.started_at = replay_request.started_at.map(|value| value + 1);
    let replay = repository
        .start_context_maintenance(&replay_request, &guard)
        .await
        .expect("same operation and immutable request returns the existing attempt");
    assert!(first.1, "the first caller owns Provider work");
    assert!(
        !replay.1,
        "an idempotent replay must not repeat Provider work"
    );
    assert_eq!(replay.0, first.0);

    let mut cross_workspace = running.clone();
    cross_workspace.workspace_id = "workspace-b".into();
    cross_workspace.anchor_run_id = "run-b".into();
    cross_workspace.first_kept_run_id = Some("run-b".into());
    cross_workspace.source_run_ids_json = r#"["run-b"]"#.into();
    let cross_workspace_guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-b".into(),
        expected_cursor_version: 0,
        expected_draft_version: Some(0),
        branch_pointer_id: None,
        expected_branch_version: None,
    };
    assert!(matches!(
        repository
            .start_context_maintenance(&cross_workspace, &cross_workspace_guard)
            .await,
        Err(RepositoryError::Conflict(_))
    ));

    assert_eq!(
        repository
            .recover_interrupted_context_maintenance(50)
            .await
            .unwrap(),
        1
    );
    let recovered = repository
        .get_context_maintenance_run("client-operation-a")
        .await
        .unwrap();
    assert_eq!(recovered.status, "failed");
    assert_eq!(recovered.finished_at, Some(50));
    assert!(
        recovered
            .error_json
            .as_deref()
            .unwrap()
            .contains("application_restarted")
    );
    assert_eq!(
        repository
            .get_context_checkpoint_for_maintenance("client-operation-a")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repository
            .recover_interrupted_context_maintenance(60)
            .await
            .unwrap(),
        0,
        "recovery is idempotent"
    );
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

fn checkpoint_attempt(
    suffix: &str,
    requested_created_at: i64,
) -> (
    ContextMaintenanceRunRecord,
    ContextMaintenanceRunRecord,
    ContextCheckpointRecord,
    ContentBlockRecord,
) {
    let maintenance_id = format!("maintenance-{suffix}");
    let summary = ContentBlockRecord {
        id: format!("block-system-summary-{suffix}"),
        role: "system".into(),
        content: format!("Summary {suffix}"),
        content_hash: format!("summary-hash-{suffix}"),
        created_at: requested_created_at,
    };
    let running = ContextMaintenanceRunRecord {
        id: maintenance_id.clone(),
        workspace_id: "workspace-a".into(),
        kind: "compaction".into(),
        status: "running".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "same-source-hash".into(),
        provider_snapshot_json: None,
        request_json: r#"{"instruction":"summarize"}"#.into(),
        summary_block_id: None,
        summary: None,
        error_json: None,
        created_at: 30,
        started_at: Some(30),
        finished_at: None,
    };
    let checkpoint = ContextCheckpointRecord {
        id: format!("checkpoint-{suffix}"),
        workspace_id: "workspace-a".into(),
        maintenance_run_id: maintenance_id,
        kind: "compaction".into(),
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(0),
        anchor_run_id: "run-a".into(),
        first_kept_run_id: Some("run-a".into()),
        summary_block_id: summary.id.clone(),
        summary: summary.content.clone(),
        summary_content_hash: summary.content_hash.clone(),
        source_run_ids_json: r#"["run-a"]"#.into(),
        source_hash: "same-source-hash".into(),
        provider_snapshot_json: None,
        created_at: requested_created_at,
    };
    let mut completed = running.clone();
    completed.status = "completed".into();
    completed.summary_block_id = Some(summary.id.clone());
    completed.summary = Some(summary.content.clone());
    completed.finished_at = Some(requested_created_at);
    (running, completed, checkpoint, summary)
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
        provider_id: "ollama".into(),
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
    assert_eq!(
        RepositoryPort::get_provider_profile(&repository, &profile.id)
            .await
            .unwrap(),
        profile
    );

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
        content_hash: crate::domain::sha256_hex(turn.prompt_markdown.as_bytes()),
        created_at: 20,
    };
    let manifest = ContextManifest {
        compiler_version: "1".into(),
        items: vec![RunContextItem {
            position: 0,
            source_id: Some(turn.id.clone()),
            source_ref: ContextSourceRef::new(ContextSourceRefKind::TurnPrompt, turn.id.clone()),
            source_kind: ContextSourceKind::CurrentPrompt,
            role: MessageRole::User,
            content: turn.prompt_markdown.clone(),
            content_block_id: prompt_block.id.clone(),
            content_hash: prompt_block.content_hash.clone(),
            inclusion_reason: InclusionReason::CurrentPrompt,
            mandatory: true,
        }],
        estimated_chars: 20,
        canonical_hash: "canonical-a".into(),
        warnings: vec![crate::domain::ContextWarning::DuplicatePinnedSource(
            "content-block:prompt-block-a".into(),
        )],
        checkpoint_provenance: None,
        branch_summary_provenance: Vec::new(),
    };
    let snapshot = ContextSnapshot {
        id: "snapshot-a".into(),
        run_id: run.id.clone(),
        manifest: manifest.clone(),
        provider: ProviderSnapshot {
            profile_id: profile.id.clone(),
            provider_id: Some(profile.provider_id.clone()),
            template_revision: Some(1),
            provider_name: profile.name.clone(),
            dialect: profile.dialect,
            stream_protocol: Some(crate::domain::StreamProtocol::OllamaNdjson),
            auth_placement: Some(crate::domain::AuthPlacement::None),
            auth_header_name: None,
            additional_headers: BTreeMap::new(),
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
            context_update: None,
        },
    )
    .await
    .unwrap();

    let stored_receipt = repository.get_run_receipt("run-a").await.unwrap();
    let stored_request: serde_json::Value =
        serde_json::from_str(&stored_receipt.snapshot.request_json).unwrap();
    assert_eq!(
        stored_request["parameters"]["temperature"],
        serde_json::json!(0.2),
        "the frozen canonical request must preserve the actual numeric parameter type"
    );
    let stored_run = repository.get_run("run-a").await.unwrap();
    let stored_provider: serde_json::Value =
        serde_json::from_str(&stored_run.provider_snapshot_json).unwrap();
    assert_eq!(
        stored_provider["parameters"]["temperature"],
        serde_json::json!(0.2)
    );

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
