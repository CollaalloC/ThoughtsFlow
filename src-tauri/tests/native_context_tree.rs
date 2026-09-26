//! Single-worker native command journey.
//!
//! This deliberately uses Tauri's built-in mock runtime rather than a browser
//! stand-in: every assertion crosses the generated IPC handler and the real
//! `AppState`, while SQLite is reopened from disk between app instances.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tauri::{
    WebviewWindow,
    ipc::{CallbackFn, InvokeBody},
    test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets},
    webview::InvokeRequest,
};
use thoughtsflow_lib::{
    application::{ApplicationBackend, DefaultApplicationBackend, GetContextTreeInput},
    builder_for_runtime,
    domain::{
        ContextCompileRequest, ContextCompiler, ContextOverrides, ContextPolicy, ConversationGraph,
        ModelRun, RunDraft, Turn,
    },
    infrastructure::{
        filesystem::LocalDecisionPacketWriter,
        provider::ReqwestProviderGateway,
        sqlite::{
            BranchPointerRecord, ContentBlockRecord, ContextDraftUpdateRecord,
            ContextManifestRecord, ContextSnapshotRecord, ModelRunRecord, RunContextItemRecord,
            RunFinish, RunStartBundle, RunStartContextUpdateRecord, RunStatusRecord,
            SqliteRepository, TurnRecord, WorkspaceRecord,
        },
    },
};

const WORKSPACE_ID: &str = "native-context-workspace";
const BRANCH_ID: &str = "native-context-main";

fn invoke(window: &WebviewWindow<MockRuntime>, command: &str, input: Value) -> Value {
    invoke_body(window, command, json!({ "input": input }))
}

fn invoke_body(window: &WebviewWindow<MockRuntime>, command: &str, body: Value) -> Value {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // Tauri serves its local origin as http://tauri.localhost on Windows.
            url: window.url().expect("mock window has a local app URL"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .unwrap_or_else(|error| panic!("{command} IPC failed: {error}"))
    .deserialize::<Value>()
    .unwrap()
}

fn backend(
    repository: Arc<SqliteRepository>,
    exports: std::path::PathBuf,
) -> Arc<DefaultApplicationBackend> {
    let provider = Arc::new(ReqwestProviderGateway::with_defaults().expect("provider client"));
    Arc::new(DefaultApplicationBackend::new(
        repository,
        provider.clone(),
        provider.clone(),
        provider,
        Arc::new(LocalDecisionPacketWriter::new(exports)),
    ))
}

fn app(backend: Arc<DefaultApplicationBackend>) -> tauri::App<MockRuntime> {
    builder_for_runtime(backend, mock_builder())
        .build(mock_context(noop_assets()))
        .expect("mock Tauri app opens")
}

fn window(app: &tauri::App<MockRuntime>) -> WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("mock native WebView opens")
}

fn workspace() -> WorkspaceRecord {
    WorkspaceRecord {
        id: WORKSPACE_ID.into(),
        title: "Native Context Tree".into(),
        goal: "Verify crash-safe Context navigation.".into(),
        system_prompt: "Keep Context receipts auditable.".into(),
        created_at: 1,
        updated_at: 1,
        archived_at: None,
    }
}

fn run_bundle(
    turn_id: &str,
    run_id: &str,
    parent_run_id: Option<&str>,
    prompt: &str,
    created_at: i64,
    pointer: Option<BranchPointerRecord>,
    context_update: Option<RunStartContextUpdateRecord>,
) -> RunStartBundle {
    let block_id = format!("block-{turn_id}");
    let manifest_id = format!("manifest-{run_id}");
    RunStartBundle {
        turn: Some(TurnRecord {
            id: turn_id.into(),
            workspace_id: WORKSPACE_ID.into(),
            parent_run_id: parent_run_id.map(Into::into),
            prompt_block_id: block_id.clone(),
            prompt_markdown: prompt.into(),
            title: prompt.into(),
            created_at,
            deleted_at: None,
        }),
        run: ModelRunRecord {
            id: run_id.into(),
            turn_id: turn_id.into(),
            workspace_id: WORKSPACE_ID.into(),
            provider_profile_id: Some("provider-local-ollama".into()),
            model: "qwen3".into(),
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
            id: block_id.clone(),
            role: "user".into(),
            content: prompt.into(),
            content_hash: thoughtsflow_lib::domain::sha256_hex(prompt.as_bytes()),
            created_at,
        }],
        manifest: ContextManifestRecord {
            id: manifest_id.clone(),
            workspace_id: WORKSPACE_ID.into(),
            compiler_version: "4".into(),
            strategy: "ancestor_path_with_manual_checkpoint".into(),
            estimated_chars: prompt.len() as i64,
            canonical_hash: format!("canonical-{run_id}"),
            warnings_json: "[]".into(),
            checkpoint_provenance_json: None,
            branch_summary_provenance_json: "[]".into(),
            created_at,
        },
        context_items: vec![RunContextItemRecord {
            manifest_id: manifest_id.clone(),
            workspace_id: WORKSPACE_ID.into(),
            position: 0,
            source_id: Some(turn_id.into()),
            source_ref_kind: "turn_prompt".into(),
            source_ref_id: Some(turn_id.into()),
            source_kind: "current_prompt".into(),
            role: "user".into(),
            content_block_id: block_id,
            inclusion_reason: "current_prompt".into(),
            mandatory: true,
        }],
        snapshot: ContextSnapshotRecord {
            id: format!("snapshot-{run_id}"),
            run_id: run_id.into(),
            manifest_id,
            workspace_id: WORKSPACE_ID.into(),
            provider_profile_id: Some("provider-local-ollama".into()),
            provider_id: Some("ollama".into()),
            template_revision: Some(1),
            stream_protocol: Some("ollama_ndjson".into()),
            auth_placement: Some("none".into()),
            auth_header_name: None,
            additional_headers_json: "{}".into(),
            provider: "Local Ollama".into(),
            model: "qwen3".into(),
            base_url: "http://127.0.0.1:11434".into(),
            parameters_json: "{}".into(),
            request_json: json!({ "messages": [{ "role": "user", "content": prompt }] })
                .to_string(),
            canonical_hash: format!("canonical-{run_id}"),
            created_at,
        },
        branch_pointer: pointer,
        context_update,
    }
}

async fn finish(repository: &SqliteRepository, run_id: &str, output: &str, at: i64) {
    repository.mark_run_connecting(run_id, at).await.unwrap();
    repository.mark_run_streaming(run_id, at + 1).await.unwrap();
    repository
        .finish_run(
            run_id,
            &RunFinish {
                status: RunStatusRecord::Completed,
                output_markdown: output.into(),
                reasoning_markdown: String::new(),
                usage_json: None,
                error_json: None,
                finished_at: at + 2,
            },
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn native_ipc_legacy_default_profile_survives_upgrade_and_sends_preserved_settings() {
    for (name, bytes) in [
        (
            "legacy-v7",
            &include_bytes!("fixtures/sqlite/legacy-v7.sqlite")[..],
        ),
        (
            "development-v5",
            &include_bytes!("fixtures/sqlite/development-v5.sqlite")[..],
        ),
    ] {
        assert_upgraded_default_profile_sends_preserved_settings(name, bytes).await;
    }
}

async fn assert_upgraded_default_profile_sends_preserved_settings(name: &str, bytes: &[u8]) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join(format!("{name}-default.sqlite3"));
    std::fs::write(&database, bytes).unwrap();

    // A local HTTP server records the real Reqwest request; it never reaches a
    // model vendor. Every socket operation has a bound even if the test fails.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "provider request did not arrive");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept provider request: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let (header_end, content_length) = loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0, "request ended before its body");
            request.extend_from_slice(&chunk[..count]);
            assert!(
                request.len() < 64 * 1024,
                "unexpectedly large fixture request"
            );
            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .expect("request Content-Length");
                break (header_end + 4, content_length);
            }
        };
        assert!(content_length < 64 * 1024);
        while request.len() < header_end + content_length {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0, "request body was truncated");
            request.extend_from_slice(&chunk[..count]);
        }
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"升级后的请求成功\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        String::from_utf8(request).unwrap()
    });

    // Redirect the mutable endpoint and create an empty workspace while the
    // database is still on its old schema. Historical receipts deliberately use
    // synthetic hashes for migration tests; leave those evidence rows intact.
    // The default marker, model and sampling values come from the frozen fixture.
    let old_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&database))
        .await
        .unwrap();
    let old_parameters: String = sqlx::query_scalar(
        "SELECT parameters_json FROM provider_profile WHERE id = 'provider-upgrade'",
    )
    .fetch_one(&old_pool)
    .await
    .unwrap();
    assert!(old_parameters.contains("_thoughsflowIsDefault"));
    sqlx::query("UPDATE provider_profile SET base_url = ? WHERE id = 'provider-upgrade'")
        .bind(&base_url)
        .execute(&old_pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO workspace (id, title, goal, system_prompt, created_at, updated_at) \
         VALUES ('legacy-send-workspace', 'Existing workspace', 'Preserve settings', \
                 'Use the selected model settings.', 1, 1)",
    )
    .execute(&old_pool)
    .await
    .unwrap();
    old_pool.close().await;

    // Use the production migration and startup paths before crossing the
    // public IPC surface that the desktop frontend uses to select a default.
    let repository = Arc::new(
        SqliteRepository::connect(&database)
            .await
            .unwrap_or_else(|error| panic!("{name} upgrade failed: {error}")),
    );
    let backend = backend(repository.clone(), directory.path().join("exports"));
    backend.initialize().await.unwrap();
    let app = app(backend);
    let window = window(&app);
    let profiles = invoke_body(&window, "list_provider_profiles", json!({}));
    let defaults: Vec<_> = profiles["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|profile| profile["isDefault"] == true)
        .collect();
    assert_eq!(defaults.len(), 1);
    let selected = defaults[0];
    assert_eq!(selected["id"], "provider-upgrade");
    assert_eq!(selected["model"], "legacy-model");
    assert_eq!(selected["parameters"], json!({ "temperature": 0.25 }));
    let stored = repository
        .get_provider_profile("provider-upgrade")
        .await
        .unwrap();
    let parameters: Value = serde_json::from_str(&stored.parameters_json).unwrap();
    assert_eq!(parameters["_thoughtsflowIsDefault"], "true");
    assert!(parameters.get("_thoughsflowIsDefault").is_none());

    invoke(
        &window,
        "set_session_credential",
        json!({
            "providerProfileId": selected["id"],
            "credentialLabel": "Local migration test",
            "credential": "local-fixture-only",
        }),
    );
    let workspace = invoke_body(
        &window,
        "open_workspace",
        json!({ "id": "legacy-send-workspace" }),
    );
    assert_eq!(workspace["data"]["workspace"]["name"], "Existing workspace");
    let cursor = &workspace["data"]["contextCursor"];
    let tree = invoke(
        &window,
        "get_context_tree",
        json!({ "workspaceId": "legacy-send-workspace" }),
    );
    let branch_version = tree["data"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["id"] == cursor["branchId"])
        .map(|branch| branch["version"].clone());
    let preview = invoke(
        &window,
        "inspect_context",
        json!({
            "workspaceId": "legacy-send-workspace",
            "parentRunId": cursor["activeRunId"],
            "prompt": "验证数据库迁移后仍能发送",
            "providerProfileId": selected["id"],
            "branchId": cursor["branchId"],
        }),
    );
    assert_eq!(preview["data"]["blocked"], false);
    let started = invoke_body(
        &window,
        "create_turn_and_start_run",
        json!({
            "input": {
                "workspaceId": "legacy-send-workspace",
                "parentRunId": cursor["activeRunId"],
                "prompt": "验证数据库迁移后仍能发送",
                "providerProfileId": selected["id"],
                "previewHash": preview["data"]["hash"],
                "branchId": cursor["branchId"],
                "expectedCursorVersion": cursor["version"],
                "expectedBranchVersion": branch_version,
                "expectedDraftVersion": preview["data"]["draftVersion"],
            },
            "onEvent": "__CHANNEL__:1",
        }),
    );
    let run_id = started["data"]["runId"].as_str().unwrap();
    let run = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = repository.get_run(run_id).await.unwrap();
            if run.finished_at.is_some() {
                break run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("migrated profile request completes");
    assert_eq!(run.status, RunStatusRecord::Completed, "{run:?}");
    assert_eq!(run.output_markdown, "升级后的请求成功");

    let request = server.join().expect("local provider server succeeds");
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    let wire: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(wire["model"], "legacy-model");
    assert_eq!(wire["temperature"], 0.25);
    assert!(!request.contains("_thoughsflowIsDefault"));
    assert!(!request.contains("_thoughtsflowIsDefault"));
    let receipt = invoke_body(&window, "get_run_snapshot", json!({ "runId": run_id }));
    assert_eq!(receipt["data"]["model"], "legacy-model");
    assert_eq!(
        receipt["data"]["parameters"],
        json!({ "temperature": 0.25 })
    );
    let raw_receipt = repository.get_run_receipt(run_id).await.unwrap();
    for value in [
        &raw_receipt.snapshot.parameters_json,
        &raw_receipt.snapshot.request_json,
    ] {
        assert!(!value.contains("_thoughsflowIsDefault"));
        assert!(!value.contains("_thoughtsflowIsDefault"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn native_ipc_context_tree_survives_reopen_checkpoint_and_restart_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("native-context.sqlite3");
    let exports = directory.path().join("exports");

    let repository = Arc::new(SqliteRepository::connect(&database).await.unwrap());
    repository.create_workspace(&workspace()).await.unwrap();
    let initial_backend = backend(repository.clone(), exports.clone());
    initial_backend.initialize().await.unwrap();

    repository
        .persist_run_start(&run_bundle(
            "turn-root",
            "run-root",
            None,
            "确认不可变基线",
            10,
            Some(BranchPointerRecord {
                id: BRANCH_ID.into(),
                workspace_id: WORKSPACE_ID.into(),
                name: "主路线".into(),
                head_run_id: "run-root".into(),
                version: 0,
                created_at: 10,
                updated_at: 10,
            }),
            Some(RunStartContextUpdateRecord {
                expected_cursor_version: 0,
                expected_draft_version: 0,
                expected_branch_pointer_id: None,
                expected_branch_version: None,
                result_branch_pointer_id: Some(BRANCH_ID.into()),
                updated_at: 10,
            }),
        ))
        .await
        .unwrap();
    finish(&repository, "run-root", "基线回答", 11).await;
    // A consumed “next send” draft is explicitly reopened for the next leaf;
    // this mirrors the preview/update command that the desktop composer makes.
    repository
        .update_context_draft(&ContextDraftUpdateRecord {
            workspace_id: WORKSPACE_ID.into(),
            parent_run_id: Some("run-root".into()),
            expected_version: 1,
            content_blocks: vec![],
            items: vec![],
            updated_at: 19,
        })
        .await
        .unwrap();

    repository
        .persist_run_start(&run_bundle(
            "turn-leaf",
            "run-leaf",
            Some("run-root"),
            "从精确叶继续",
            20,
            Some(BranchPointerRecord {
                id: BRANCH_ID.into(),
                workspace_id: WORKSPACE_ID.into(),
                name: "主路线".into(),
                head_run_id: "run-leaf".into(),
                version: 1,
                created_at: 10,
                updated_at: 20,
            }),
            Some(RunStartContextUpdateRecord {
                expected_cursor_version: 1,
                expected_draft_version: 2,
                expected_branch_pointer_id: Some(BRANCH_ID.into()),
                expected_branch_version: Some(0),
                result_branch_pointer_id: Some(BRANCH_ID.into()),
                updated_at: 20,
            }),
        ))
        .await
        .unwrap();
    finish(&repository, "run-leaf", "叶回答", 21).await;

    let first_app = app(initial_backend.clone());
    let first_window = window(&first_app);
    let cut_to_root = invoke(
        &first_window,
        "set_active_context",
        json!({
            "workspaceId": WORKSPACE_ID,
            "runId": "run-root",
            "branchId": BRANCH_ID,
            "expectedCursorVersion": 2,
            "expectedDraftVersion": 3,
        }),
    );
    assert_eq!(cut_to_root["data"]["activeRunId"], "run-root");
    assert_eq!(cut_to_root["data"]["version"], 3);

    drop(first_window);
    drop(first_app);
    drop(initial_backend);
    drop(repository);

    let reopened_repository = Arc::new(SqliteRepository::connect(&database).await.unwrap());
    let reopened_backend = backend(reopened_repository.clone(), exports.clone());
    reopened_backend.initialize().await.unwrap();
    let reopened_app = app(reopened_backend.clone());
    let reopened_window = window(&reopened_app);

    let after_reopen = invoke(
        &reopened_window,
        "get_context_tree",
        json!({ "workspaceId": WORKSPACE_ID }),
    );
    assert_eq!(after_reopen["data"]["cursor"]["activeRunId"], "run-root");
    assert_eq!(after_reopen["data"]["cursor"]["version"], 3);

    let restore_leaf = invoke(
        &reopened_window,
        "set_active_context",
        json!({
            "workspaceId": WORKSPACE_ID,
            "runId": "run-leaf",
            "branchId": BRANCH_ID,
            "expectedCursorVersion": 3,
            "expectedDraftVersion": 4,
        }),
    );
    assert_eq!(restore_leaf["data"]["activeRunId"], "run-leaf");
    assert_eq!(restore_leaf["data"]["version"], 4);

    let checkpoint = invoke(
        &reopened_window,
        "create_context_checkpoint",
        json!({
            "clientOperationId": "0f6f8d8b-9065-4bb6-91d3-9bfec751b2d4",
            "workspaceId": WORKSPACE_ID,
            "branchId": BRANCH_ID,
            "kind": "compaction",
            "sourceRunIds": ["run-root"],
            "firstKeptRunId": "run-leaf",
            "summary": "已确认基线；保留精确叶。",
            "expectedCursorVersion": 4,
            "expectedBranchVersion": 1,
        }),
    );
    assert_eq!(checkpoint["data"]["sourceRunIds"], json!(["run-root"]));
    let source_hash = checkpoint["data"]["sourceHash"]
        .as_str()
        .expect("checkpoint source hash")
        .to_owned();
    assert_eq!(source_hash.len(), 64);
    assert!(
        source_hash
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );

    reopened_repository
        .persist_run_start(&run_bundle(
            "turn-restart",
            "run-restart",
            Some("run-leaf"),
            "模拟进程重启",
            30,
            None,
            None,
        ))
        .await
        .unwrap();
    reopened_repository
        .mark_run_connecting("run-restart", 31)
        .await
        .unwrap();
    reopened_repository
        .mark_run_streaming("run-restart", 32)
        .await
        .unwrap();
    reopened_repository
        .checkpoint_run(
            "run-restart",
            &thoughtsflow_lib::infrastructure::sqlite::RunCheckpoint {
                output_markdown: "崩溃前的部分输出".into(),
                reasoning_markdown: String::new(),
                usage_json: None,
                checkpointed_at: 33,
            },
        )
        .await
        .unwrap();

    drop(reopened_window);
    drop(reopened_app);
    drop(reopened_backend);
    drop(reopened_repository);

    let recovered_repository = Arc::new(SqliteRepository::connect(&database).await.unwrap());
    let recovered_backend = backend(recovered_repository, exports);
    recovered_backend.initialize().await.unwrap();
    let recovered_app = app(recovered_backend);
    let recovered_window = window(&recovered_app);
    let recovered = invoke(
        &recovered_window,
        "get_context_tree",
        json!({ "workspaceId": WORKSPACE_ID }),
    );

    assert_eq!(recovered["data"]["cursor"]["activeRunId"], "run-leaf");
    assert!(
        recovered["data"]["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["sourceRunIds"] == json!(["run-root"])
                && item["sourceHash"] == source_hash)
    );
    assert!(
        recovered["data"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["runId"] == "run-restart" && node["status"] == "interrupted")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn projects_one_thousand_turns_and_two_thousand_runs_in_under_half_a_second() {
    let directory = tempfile::tempdir().unwrap();
    let repository = Arc::new(
        SqliteRepository::connect_in_memory_with_context_read_probe()
            .await
            .unwrap(),
    );
    repository.create_workspace(&workspace()).await.unwrap();
    let backend = backend(repository.clone(), directory.path().join("exports"));
    backend.initialize().await.unwrap();

    for index in 0..1_000 {
        let turn_id = format!("turn-{index:04}");
        let primary_run_id = format!("run-{index:04}-a");
        let parent_run_id = (index > 0).then(|| format!("run-{:04}-a", index - 1));
        let prompt = format!("性能路径问题 {index}");
        let created_at = 100 + i64::from(index) * 10;

        repository
            .persist_run_start(&run_bundle(
                &turn_id,
                &primary_run_id,
                parent_run_id.as_deref(),
                &prompt,
                created_at,
                None,
                None,
            ))
            .await
            .unwrap();
        finish(
            &repository,
            &primary_run_id,
            "可继续的性能回答",
            created_at + 1,
        )
        .await;

        let retry_run_id = format!("run-{index:04}-b");
        let mut retry = run_bundle(
            &turn_id,
            &retry_run_id,
            None,
            &prompt,
            created_at + 5,
            None,
            None,
        );
        retry.turn = None;
        retry.content_blocks.clear();
        repository.persist_run_start(&retry).await.unwrap();
    }

    SqliteRepository::reset_context_read_count();
    let started_at = Instant::now();
    let projection = backend
        .get_context_tree(GetContextTreeInput {
            workspace_id: WORKSPACE_ID.into(),
        })
        .await
        .unwrap();
    let elapsed = started_at.elapsed();
    let read_count = SqliteRepository::context_read_count();

    assert_eq!(projection.nodes.len(), 2_000);
    assert_eq!(projection.edges.len(), 2_000);
    eprintln!("Context Tree projection benchmark: {elapsed:?}, reads: {read_count}");
    assert!(
        read_count <= 8,
        "Context Tree projection performed {read_count} SQLite reads; budget is 8"
    );
    assert!(
        elapsed.as_millis() <= 500,
        "Context Tree projection took {elapsed:?}; budget is 500ms for 1,000 Turns / 2,000 Runs"
    );
}

#[test]
fn rebuilds_a_depth_one_thousand_context_path_in_under_one_hundred_milliseconds() {
    let mut turns = Vec::with_capacity(1_000);
    let mut runs = Vec::with_capacity(1_000);
    let mut parent_run_id: Option<String> = None;
    for index in 0..1_000 {
        let turn_id = format!("compiler-turn-{index:04}");
        let run_id = format!("compiler-run-{index:04}");
        let turn = match parent_run_id.as_deref() {
            Some(parent) => Turn::branch(
                &turn_id,
                WORKSPACE_ID,
                parent,
                format!("问题 {index}"),
                index,
            ),
            None => Turn::root(&turn_id, WORKSPACE_ID, format!("问题 {index}"), index),
        };
        let mut run = ModelRun::queued(RunDraft {
            id: run_id.clone(),
            turn_id: turn_id.clone(),
            provider_profile_id: None,
            model: "fixture-model".into(),
            created_at: index,
        });
        run.connect(index).unwrap();
        run.begin_streaming(index).unwrap();
        run.checkpoint("简短回答", "", index).unwrap();
        run.complete(None, index).unwrap();
        turns.push(turn);
        runs.push(run);
        parent_run_id = Some(run_id);
    }
    let graph = ConversationGraph::try_new(turns, runs, Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy {
        compiler_version: "4".into(),
        max_chars: 1_000_000,
    });
    let started_at = Instant::now();
    let preview = compiler
        .inspect(
            &graph,
            ContextCompileRequest {
                workspace_id: WORKSPACE_ID.into(),
                system_prompt: "系统策略".into(),
                parent_run_id,
                current_prompt: "下一轮问题".into(),
                overrides: ContextOverrides::default(),
                provider: None,
            },
        )
        .unwrap();
    let elapsed = started_at.elapsed();

    assert_eq!(preview.raw_items.len(), 2_002);
    eprintln!("Depth-1,000 Context rebuild benchmark: {elapsed:?}");
    assert!(
        elapsed.as_millis() <= 100,
        "Depth-1,000 Context rebuild took {elapsed:?}; budget is 100ms"
    );
}
