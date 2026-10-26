use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    orca::{CommandRunner, RuntimeFailure, is_identity_environment, parse_response},
    store::AgentStore,
    *,
};
use crate::infrastructure::sqlite::SqliteRepository;

const MISSION: &str = "a6510d2d-61b1-4ced-aec9-c87715b19244";
const OP: &str = "b6510d2d-61b1-4ced-aec9-c87715b19244";
const SECOND_OP: &str = "c6510d2d-61b1-4ced-aec9-c87715b19244";

fn test_repository_path() -> String {
    std::env::temp_dir()
        .join("thoughtsflow-agent-test-repo")
        .to_string_lossy()
        .into_owned()
}

fn ok(result: Value) -> Value {
    json!({"ok":true,"result":result,"_meta":{"runtimeId":"runtime-1"}})
}
fn argument<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

#[derive(Default)]
struct MockOrca {
    calls: Mutex<Vec<Vec<String>>>,
    overrides: Mutex<HashMap<String, VecDeque<Result<Value, RuntimeFailure>>>>,
    runtime: Mutex<String>,
    generation: AtomicI64,
    workers: Mutex<Vec<Value>>,
    tasks: Mutex<Vec<Value>>,
    messages: Mutex<Vec<Value>>,
    terminals: Mutex<Vec<Value>>,
    pending_observed: AtomicBool,
    handoff_observed: AtomicBool,
    store: Option<AgentStore>,
}

impl MockOrca {
    fn new(store: AgentStore) -> Self {
        Self {
            runtime: Mutex::new("runtime-1".into()),
            generation: AtomicI64::new(1),
            store: Some(store),
            terminals: Mutex::new(vec![
                json!({"handle":"coordinator-2","tabId":"tab-1","leafId":"leaf-1","title":"renamed by user"}),
            ]),
            ..Self::default()
        }
    }
    fn override_next(&self, command: &str, response: Result<Value, RuntimeFailure>) {
        self.overrides
            .lock()
            .unwrap()
            .entry(command.to_owned())
            .or_default()
            .push_back(response);
    }
    fn count(&self, command: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|args| args.iter().take(2).cloned().collect::<Vec<_>>().join(" ") == command)
            .count()
    }
    fn count_all(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl CommandRunner for MockOrca {
    fn available(&self) -> bool {
        true
    }
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        self.calls.lock().unwrap().push(arguments.to_vec());
        let command = arguments
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if command == "orchestration worker-start" {
            let pending = self
                .store
                .as_ref()
                .unwrap()
                .operations(MISSION)
                .await
                .unwrap()
                .iter()
                .any(|operation| operation.kind == "start-task" && operation.status == "pending");
            self.pending_observed.store(pending, Ordering::SeqCst);
            let handoffs = self
                .store
                .as_ref()
                .unwrap()
                .handoffs(MISSION)
                .await
                .unwrap();
            self.handoff_observed
                .store(!handoffs.is_empty(), Ordering::SeqCst);
        }
        if let Some(result) = self
            .overrides
            .lock()
            .unwrap()
            .get_mut(&command)
            .and_then(VecDeque::pop_front)
        {
            return result;
        }
        let result = match command.as_str() {
            "status --json" => {
                json!({"runtime":{"reachable":true,"state":"ready","runtimeId":self.runtime.lock().unwrap().clone(),"capabilities":["orchestration.contract.v1"]},"graph":{"state":"ready"},"app":{"running":true}})
            }
            "repo list" => {
                json!({"repos":[{"id":"repo-1","name":"Test repo","path":test_repository_path()}]})
            }
            "terminal create" => {
                json!({"terminal":{"handle":"coordinator-1","tabId":"tab-1","leafId":"leaf-1"}})
            }
            "terminal list" => json!({"terminals":self.terminals.lock().unwrap().clone()}),
            "orchestration run-create" | "orchestration run-current" => {
                json!({"run":{"id":if argument(arguments,"--from") == Some("coordinator-b") {"run-2"} else {"run-1"},"consumer_generation":self.generation.load(Ordering::SeqCst),"coordinator_handle":argument(arguments,"--from")}})
            }
            "orchestration run-use" => {
                let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                json!({"run":{"id":"run-1","consumer_generation":generation,"coordinator_handle":argument(arguments,"--from")}})
            }
            "orchestration worker-start" => {
                json!({"dispatchId":"dispatch-1","taskId":"task-1","state":"ready","runId":argument(arguments,"--run")})
            }
            "orchestration worker-list" => {
                json!({"workers":self.workers.lock().unwrap().clone(),"page":{"hasMore":false}})
            }
            "orchestration task-list" => {
                json!({"runId":"run-1","tasks":self.tasks.lock().unwrap().clone()})
            }
            "orchestration check" => {
                assert!(arguments.iter().any(|arg| arg == "--peek"));
                assert!(
                    !arguments
                        .iter()
                        .any(|arg| arg == "--ack" || arg == "--unread")
                );
                json!({"messages":self.messages.lock().unwrap().clone()})
            }
            "orchestration reply" => {
                json!({"message":{"id":"reply-1","thread_id":argument(arguments,"--id"),"run_id":argument(arguments,"--run")}})
            }
            "orchestration worker-release" => {
                json!({"dispatchId":argument(arguments,"--dispatch"),"state":"released"})
            }
            "orchestration worker-read" => {
                json!({"dispatchId":argument(arguments,"--dispatch"),"source":"terminal","terminal":{"tail":["hello from worker"],"limited":false},"cursor":"opaque:cursor"})
            }
            _ => panic!("unexpected command {arguments:?}"),
        };
        Ok(ok(result))
    }
}

async fn setup() -> (SqliteRepository, AgentStore, Arc<MockOrca>, AgentService) {
    let repository = SqliteRepository::connect_in_memory().await.unwrap();
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("INSERT INTO workspace(id,title,goal,created_at,updated_at) VALUES ('workspace-1','Agents','goal',1,1)").execute(&mut *connection).await.unwrap();
    drop(connection);
    let store = repository.agent_store();
    let runner = Arc::new(MockOrca::new(store.clone()));
    let service = AgentService::with_runner(store.clone(), runner.clone())
        .await
        .unwrap();
    (repository, store, runner, service)
}

async fn ready(store: &AgentStore) {
    store
        .create_mission(&AgentMission {
            id: MISSION.into(),
            workspace_id: "workspace-1".into(),
            repository_id: "repo-1".into(),
            repository_path: test_repository_path(),
            objective: "Implement task".into(),
            run_id: Some("run-1".into()),
            coordinator_handle: Some("coordinator-1".into()),
            runtime_id: Some("runtime-1".into()),
            status: "ready".into(),
            error: None,
            created_at: "2026-09-22T00:00:00Z".into(),
        })
        .await
        .unwrap();
    store
        .save_binding(MISSION, Some("tab-1:leaf-1"), Some(1))
        .await
        .unwrap();
}

fn legacy_task_input() -> StartTaskInput {
    StartTaskInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
        title: "Implement feature".into(),
        spec: "Keep `$HOME` and $(echo literal-only) literal; verify output".into(),
        requested_model: None,
        handoff: None,
    }
}
async fn task_input(service: &AgentService) -> StartTaskInput {
    let input = legacy_task_input();
    let request = PreviewAgentHandoffInput {
        mission_id: input.mission_id,
        title: input.title,
        spec: input.spec,
        requested_model: None,
        context: AgentContextSelection {
            include_active_context: false,
            evidence_ids: vec![],
        },
    };
    let preview = service.preview_handoff(request.clone()).await.unwrap();
    confirmed_task(&request, &preview)
}
fn worker() -> Value {
    json!({"dispatchId":"dispatch-1","taskId":"task-1","runId":"run-1","agentTerminalHandle":"worker-1","workerState":"succeeded","dispatchStatus":"completed","terminalState":"reclaimable","resource":{"ownershipState":"owned","ownerDispatchId":"dispatch-1"},"projection":{"outcome":"succeeded","liveness":{"verdict":"live"},"attention":{"categories":[]}}})
}
fn task() -> Value {
    json!({"id":"task-1","run_id":"run-1","task_title":"Verify something","spec":"Full specification","status":"completed","dispatch_id":null})
}
fn question() -> Value {
    json!({"id":"question-1","run_id":"run-1","to_handle":"run:run-1","from_handle":"worker-1","type":"question","body":"Proceed?"})
}
fn ambiguous() -> RuntimeFailure {
    RuntimeFailure {
        message: "timeout after send".into(),
        receipt: Value::Null,
        ambiguous: true,
    }
}

#[tokio::test]
async fn creates_once_and_validates_repository_and_workspace_before_effects() {
    let (_repository, store, runner, service) = setup().await;
    let input = CreateMissionInput {
        id: MISSION.into(),
        workspace_id: "workspace-1".into(),
        repository_id: "repo-1".into(),
        objective: "Create feature".into(),
    };
    let mut bad = input.clone();
    bad.repository_id = "--path /etc".into();
    assert!(service.create_mission(bad).await.is_err());
    let mut bad = input.clone();
    bad.workspace_id = "missing".into();
    assert!(service.create_mission(bad).await.is_err());
    assert_eq!(runner.count("terminal create"), 0);
    let mission = service.create_mission(input.clone()).await.unwrap();
    assert_eq!(mission.status, "ready");
    assert_eq!(
        service
            .create_mission(input)
            .await
            .unwrap()
            .run_id
            .as_deref(),
        Some("run-1")
    );
    assert_eq!(runner.count("terminal create"), 1);
    assert_eq!(runner.count("orchestration run-create"), 1);
    assert_eq!(store.operations(MISSION).await.unwrap().len(), 2);
}

#[tokio::test]
async fn persists_before_dispatch_and_replays_without_process_or_shell() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let input = task_input(&service).await;
    let result = service.start_task(input.clone()).await.unwrap();
    assert_eq!(result.status, "succeeded");
    assert!(runner.pending_observed.load(Ordering::SeqCst));
    let calls = runner.count_all();
    assert_eq!(
        service.start_task(input.clone()).await.unwrap().receipt,
        result.receipt
    );
    assert_eq!(runner.count_all(), calls);
    let call = runner
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|args| args.get(1).is_some_and(|arg| arg == "worker-start"))
        .unwrap()
        .clone();
    assert_eq!(argument(&call, "--worktree"), Some("new-top-level"));
    assert_eq!(argument(&call, "--repo"), Some("id:repo-1"));
    assert_eq!(argument(&call, "--setup"), Some("skip"));
    assert_eq!(argument(&call, "--agent"), Some("omp"));
    assert_eq!(argument(&call, "--from"), Some("coordinator-1"));
    assert_eq!(argument(&call, "--run"), Some("run-1"));
    assert!(!call.contains(&"--model".into()));
    assert!(argument(&call, "--spec").unwrap().contains(&input.spec));
    let mut changed = input;
    changed.spec.push_str(" changed");
    assert!(service.start_task(changed).await.is_err());
    assert_eq!(runner.count("orchestration worker-start"), 1);
}

#[tokio::test]
async fn unknown_dispatch_is_never_retried_or_replaced_by_new_id() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    runner.override_next("orchestration worker-start", Err(ambiguous()));
    let input = task_input(&service).await;
    assert_eq!(
        service.start_task(input.clone()).await.unwrap().status,
        "unknown"
    );
    assert_eq!(
        service.start_task(input.clone()).await.unwrap().status,
        "unknown"
    );
    let mut another = input;
    another.operation_id = SECOND_OP.into();
    assert!(service.start_task(another).await.is_err());
    assert_eq!(runner.count("orchestration worker-start"), 1);
}

#[tokio::test]
async fn startup_recovers_pending_without_replaying_and_preserves_run_binding() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let input = task_input(&service).await;
    store
        .begin(
            OP,
            MISSION,
            "start-task",
            &serde_json::to_value(&input).unwrap(),
        )
        .await
        .unwrap();
    drop(service);
    let reopened = AgentService::with_runner(store.clone(), runner.clone())
        .await
        .unwrap();
    assert_eq!(reopened.start_task(input).await.unwrap().status, "unknown");
    assert_eq!(runner.count_all(), 0);
    assert_eq!(
        store
            .mission(MISSION)
            .await
            .unwrap()
            .unwrap()
            .run_id
            .as_deref(),
        Some("run-1")
    );
}

#[tokio::test]
async fn snapshots_keep_task_truth_liveness_and_mailbox_scope_separate() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let mut idle = worker();
    idle["projection"]["liveness"]["verdict"] = json!("idle");
    let mut unfinished = task();
    unfinished["status"] = json!("dispatched");
    unfinished["dispatch_id"] = json!("dispatch-1");
    *runner.workers.lock().unwrap() = vec![idle];
    *runner.tasks.lock().unwrap() = vec![unfinished];
    let mut other = question();
    other["id"] = json!("other-run");
    other["run_id"] = json!("run-2");
    let mut worker_inbox = question();
    worker_inbox["to_handle"] = json!("worker-2");
    *runner.messages.lock().unwrap() = vec![question(), other, worker_inbox];
    let snapshot = service.snapshot(MISSION).await.unwrap();
    assert!(snapshot.connected);
    assert_eq!(snapshot.tasks[0].status, "dispatched");
    assert_eq!(snapshot.tasks[0].liveness.as_deref(), Some("idle"));
    assert!(!snapshot.tasks[0].can_release);
    assert_eq!(snapshot.messages.len(), 1);
    assert!(snapshot.messages[0].requires_reply);
}

#[tokio::test]
async fn cross_mission_dispatch_and_unsettled_or_unowned_release_are_rejected() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.workers.lock().unwrap() = vec![worker()];
    *runner.tasks.lock().unwrap() = vec![task()];
    let release = ReleaseWorkerInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "foreign-dispatch".into(),
    };
    assert!(service.release(release.clone()).await.is_err());
    assert!(
        service
            .read_output(ReadOutputInput {
                mission_id: MISSION.into(),
                dispatch_id: "foreign-dispatch".into(),
                cursor: None
            })
            .await
            .is_err()
    );
    let mut release = release;
    release.dispatch_id = "dispatch-1".into();
    runner.workers.lock().unwrap()[0]["projection"]["outcome"] = json!("in_progress");
    assert!(service.release(release.clone()).await.is_err());
    runner.workers.lock().unwrap()[0] = worker();
    runner.workers.lock().unwrap()[0]["resource"]["ownershipState"] = json!("user_owned");
    assert!(service.release(release.clone()).await.is_err());
    runner.workers.lock().unwrap()[0] = worker();
    assert_eq!(service.release(release).await.unwrap().status, "succeeded");
    assert_eq!(runner.count("orchestration worker-release"), 1);
    assert_eq!(runner.count("orchestration worker-read"), 0);
}

#[tokio::test]
async fn unknown_replies_and_releases_block_new_ids_after_restart() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.workers.lock().unwrap() = vec![worker()];
    *runner.tasks.lock().unwrap() = vec![task()];
    *runner.messages.lock().unwrap() = vec![question()];
    runner.override_next("orchestration reply", Err(ambiguous()));
    let reply = ReplyInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
        message_id: "question-1".into(),
        body: "Proceed within scope".into(),
    };
    assert_eq!(
        service.reply(reply.clone()).await.unwrap().status,
        "unknown"
    );
    drop(service);
    let reopened = AgentService::with_runner(store.clone(), runner.clone())
        .await
        .unwrap();
    let mut second = reply;
    second.operation_id = SECOND_OP.into();
    assert!(reopened.reply(second).await.is_err());
    assert!(!reopened.snapshot(MISSION).await.unwrap().messages[0].requires_reply);
    runner.override_next("orchestration worker-release", Err(ambiguous()));
    let release = ReleaseWorkerInput {
        operation_id: SECOND_OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "dispatch-1".into(),
    };
    assert_eq!(
        reopened.release(release.clone()).await.unwrap().status,
        "unknown"
    );
    let mut second = release;
    second.operation_id = uuid::Uuid::new_v4().to_string();
    assert!(reopened.release(second).await.is_err());
    assert!(!reopened.snapshot(MISSION).await.unwrap().tasks[0].can_release);
    assert_eq!(runner.count("orchestration reply"), 1);
    assert_eq!(runner.count("orchestration worker-release"), 1);
}

#[tokio::test]
async fn stale_runtime_requires_explicit_reconnect_and_uses_replacement_handle_only() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.runtime.lock().unwrap() = "runtime-2".into();
    assert!(
        service
            .start_task(task_input(&service).await)
            .await
            .is_err()
    );
    assert!(!service.snapshot(MISSION).await.unwrap().connected);
    assert_eq!(runner.count("orchestration run-use"), 0);
    let input = ReconnectInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
    };
    assert_eq!(
        service.reconnect(input.clone()).await.unwrap().status,
        "succeeded"
    );
    assert_eq!(service.reconnect(input).await.unwrap().status, "succeeded");
    assert_eq!(runner.count("orchestration run-use"), 1);
    assert_eq!(runner.count("terminal create"), 0);
    assert_eq!(
        store
            .mission(MISSION)
            .await
            .unwrap()
            .unwrap()
            .coordinator_handle
            .as_deref(),
        Some("coordinator-2")
    );
    let mut task = task_input(&service).await;
    task.operation_id = SECOND_OP.into();
    service.start_task(task).await.unwrap();
    let calls = runner.calls.lock().unwrap();
    let call = calls
        .iter()
        .find(|args| args.get(1).is_some_and(|arg| arg == "worker-start"))
        .unwrap();
    assert_eq!(argument(call, "--from"), Some("coordinator-2"));
}

#[tokio::test]
async fn reconnect_never_uses_title_as_ownership() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.terminals.lock().unwrap() = vec![
        json!({"handle":"foreign-handle","tabId":"other-tab","leafId":"other-leaf","title":format!("ThoughtsFlow {MISSION}")}),
    ];
    service
        .reconnect(ReconnectInput {
            operation_id: OP.into(),
            mission_id: MISSION.into(),
        })
        .await
        .unwrap();
    assert_eq!(runner.count("terminal create"), 1);
    let calls = runner.calls.lock().unwrap();
    let call = calls
        .iter()
        .find(|args| args.get(1).is_some_and(|arg| arg == "run-use"))
        .unwrap();
    assert_eq!(argument(call, "--from"), Some("coordinator-1"));
}

#[tokio::test]
async fn paginated_workers_are_all_read_before_dispatch_membership_check() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    runner.override_next(
        "orchestration worker-list",
        Ok(ok(
            json!({"workers":[],"page":{"hasMore":true,"nextCursor":"page-2"}}),
        )),
    );
    runner.override_next(
        "orchestration worker-list",
        Ok(ok(json!({"workers":[worker()],"page":{"hasMore":false}}))),
    );
    let output = service
        .read_output(ReadOutputInput {
            mission_id: MISSION.into(),
            dispatch_id: "dispatch-1".into(),
            cursor: Some("opaque:previous".into()),
        })
        .await
        .unwrap();
    assert_eq!(output.text, "hello from worker");
    assert_eq!(output.cursor.as_deref(), Some("opaque:cursor"));
    let calls = runner.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|args| argument(args, "--cursor") == Some("page-2"))
    );
    assert!(
        calls
            .iter()
            .any(|args| argument(args, "--cursor") == Some("opaque:previous"))
    );
}

#[test]
fn parses_nonzero_json_errors_and_does_not_turn_unknown_into_failure() {
    let dropped=parse_response(false,br#"{"ok":false,"error":{"code":"runtime_timeout","message":"connection dropped","data":{"recovery":{"disposition":"outcome_unknown","requestId":"mutation-1"}}}}"#).unwrap_err();
    assert!(dropped.ambiguous);
    assert_eq!(
        dropped.receipt["error"]["data"]["recovery"]["requestId"],
        "mutation-1"
    );
    let error = parse_response(
        false,
        br#"{"ok":false,"error":{"code":"invalid_argument","message":"Bad repository"}}"#,
    )
    .unwrap_err();
    assert_eq!(error.message, "Bad repository");
    assert!(!error.ambiguous);
    assert_eq!(error.receipt["error"]["code"], "invalid_argument");
    let receipt = parse_response(
        false,
        br#"{"ok":true,"result":{"state":"outcome_unknown","dispatchId":"created"}}"#,
    )
    .unwrap();
    assert_eq!(receipt["result"]["dispatchId"], "created");
    assert!(
        parse_response(false, b"truncated output")
            .unwrap_err()
            .ambiguous
    );
    assert!(
        parse_response(
            false,
            br#"{"ok":false,"error":{"code":"mutation_outcome_unknown","message":"unknown"}}"#
        )
        .unwrap_err()
        .ambiguous
    );
}

#[test]
fn strips_inherited_terminal_identity_and_remote_routing() {
    for key in [
        "ORCA_TERMINAL_HANDLE",
        "ORCA_PANE_KEY",
        "ORCA_AGENT_LAUNCH_TOKEN",
        "ORCA_ORCHESTRATION_COMPATIBILITY_TOKEN",
        "ORCA_CLI_CWD",
        "ORCA_ENVIRONMENT",
        "ORCA_PAIRING_CODE",
        "ORCA_REMOTE_PAIRING",
    ] {
        assert!(is_identity_environment(key), "{key}");
    }
    assert!(!is_identity_environment("PATH"));
    assert!(!is_identity_environment("TF_AGENT_FIXTURE_STATE"));
}

#[tokio::test]
async fn mailbox_capacity_stops_dispatch_but_still_allows_reply() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.workers.lock().unwrap() = vec![worker()];
    *runner.messages.lock().unwrap() = (0..100)
        .map(|index| {
            let mut message = question();
            message["id"] = json!(format!("question-{index}"));
            message
        })
        .collect();
    let snapshot = service.snapshot(MISSION).await.unwrap();
    assert!(snapshot.connected);
    assert!(!snapshot.can_start_tasks);
    assert!(snapshot.warning.unwrap().contains("100"));
    assert!(
        service
            .start_task(task_input(&service).await)
            .await
            .is_err()
    );
    assert_eq!(runner.count("orchestration worker-start"), 0);
    assert_eq!(
        service
            .reply(ReplyInput {
                operation_id: SECOND_OP.into(),
                mission_id: MISSION.into(),
                message_id: "question-1".into(),
                body: "Continue".into()
            })
            .await
            .unwrap()
            .status,
        "succeeded"
    );
}

#[tokio::test]
async fn successful_refresh_clears_transient_error_without_rebinding() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    runner.override_next("status --json", Err(ambiguous()));
    assert!(!service.snapshot(MISSION).await.unwrap().connected);
    let snapshot = service.snapshot(MISSION).await.unwrap();
    assert!(snapshot.connected);
    assert_eq!(snapshot.mission.status, "ready");
    assert!(snapshot.mission.error.is_none());
    assert_eq!(runner.count("orchestration run-use"), 0);
}

#[tokio::test]
async fn unknown_reconnect_cannot_create_another_coordinator_with_new_id() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    runner.terminals.lock().unwrap().clear();
    runner.override_next("orchestration run-use", Err(ambiguous()));
    let result = service
        .reconnect(ReconnectInput {
            operation_id: OP.into(),
            mission_id: MISSION.into(),
        })
        .await
        .unwrap();
    assert_eq!(result.status, "unknown");
    assert!(
        service
            .reconnect(ReconnectInput {
                operation_id: SECOND_OP.into(),
                mission_id: MISSION.into()
            })
            .await
            .is_err()
    );
    assert_eq!(runner.count("terminal create"), 1);
    assert_eq!(runner.count("orchestration run-use"), 1);
}

#[tokio::test]
async fn partial_output_is_explicit_and_cursor_is_preserved() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.workers.lock().unwrap() = vec![worker()];
    runner.override_next("orchestration worker-read",Ok(ok(json!({"source":"transcript","transcript":{"messages":[{"role":"assistant","blocks":[{"type":"text","text":"Partial result"}]}]},"cursor":"opaque==1","contentComplete":false,"clipping":["message_limit"],"warnings":["clipped output"]}))));
    let output = service
        .read_output(ReadOutputInput {
            mission_id: MISSION.into(),
            dispatch_id: "dispatch-1".into(),
            cursor: None,
        })
        .await
        .unwrap();
    assert!(output.text.contains("Partial result"));
    assert_eq!(output.cursor.as_deref(), Some("opaque==1"));
    assert!(output.warning.unwrap().contains("部分输出"));
}

#[tokio::test]
async fn task_projection_uses_exact_current_dispatch_and_never_guesses_from_order() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let old = worker();
    let mut current = worker();
    current["dispatchId"] = json!("dispatch-current");
    current["terminalState"] = json!("active");
    current["projection"]["outcome"] = json!("in_progress");
    let mut current_task = task();
    current_task["dispatch_id"] = json!("dispatch-current");
    *runner.tasks.lock().unwrap() = vec![current_task.clone()];
    for workers in [
        vec![old.clone(), current.clone()],
        vec![current.clone(), old.clone()],
    ] {
        *runner.workers.lock().unwrap() = workers;
        let snapshot = service.snapshot(MISSION).await.unwrap();
        assert_eq!(
            snapshot.tasks[0].dispatch_id.as_deref(),
            Some("dispatch-current")
        );
        assert!(!snapshot.tasks[0].can_release);
        assert!(
            service
                .release(ReleaseWorkerInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    dispatch_id: "dispatch-1".into()
                })
                .await
                .is_err()
        );
    }
    for missing in [Value::Null, json!("not-in-worker-list")] {
        current_task["dispatch_id"] = missing;
        *runner.tasks.lock().unwrap() = vec![current_task.clone()];
        let snapshot = service.snapshot(MISSION).await.unwrap();
        assert_eq!(snapshot.tasks[0].dispatch_id, None);
        assert!(!snapshot.tasks[0].can_release);
    }
    // A deliberate historical read still names an owned Dispatch explicitly.
    assert!(
        service
            .read_output(ReadOutputInput {
                mission_id: MISSION.into(),
                dispatch_id: "dispatch-1".into(),
                cursor: None
            })
            .await
            .unwrap()
            .text
            .contains("hello")
    );
    assert_eq!(runner.count("orchestration worker-release"), 0);
}

#[tokio::test]
async fn wrong_run_reconnect_stays_unknown_when_the_same_operation_is_replayed() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let receipt = ok(
        json!({"run":{"id":"wrong-run","consumer_generation":2,"coordinator_handle":"coordinator-2"}}),
    );
    runner.override_next("orchestration run-use", Ok(receipt.clone()));
    let input = ReconnectInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
    };
    let first = service.reconnect(input.clone()).await.unwrap();
    assert_eq!(first.status, "unknown");
    assert_eq!(first.receipt, receipt);
    let replay = service.reconnect(input).await.unwrap();
    assert_eq!(replay.status, "unknown");
    assert_eq!(runner.count("orchestration run-use"), 1);
    assert_eq!(
        store
            .mission(MISSION)
            .await
            .unwrap()
            .unwrap()
            .coordinator_handle
            .as_deref(),
        Some("coordinator-1")
    );
    assert_eq!(store.binding(MISSION).await.unwrap().1, Some(1));
}

#[tokio::test]
async fn successful_envelopes_without_action_identity_remain_unknown() {
    for command in [
        "orchestration worker-start",
        "orchestration reply",
        "orchestration worker-release",
    ] {
        let (_repository, store, runner, service) = setup().await;
        ready(&store).await;
        *runner.tasks.lock().unwrap() = vec![task()];
        *runner.workers.lock().unwrap() = vec![worker()];
        *runner.messages.lock().unwrap() = vec![question()];
        let receipt = if command.ends_with("worker-start") {
            ok(json!({"state":"ready"}))
        } else {
            ok(json!({}))
        };
        runner.override_next(command, Ok(receipt.clone()));
        let operation = match command {
            "orchestration worker-start" => service
                .start_task(task_input(&service).await)
                .await
                .unwrap(),
            "orchestration reply" => service
                .reply(ReplyInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    message_id: "question-1".into(),
                    body: "Proceed".into(),
                })
                .await
                .unwrap(),
            _ => service
                .release(ReleaseWorkerInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    dispatch_id: "dispatch-1".into(),
                })
                .await
                .unwrap(),
        };
        assert_eq!(operation.status, "unknown", "{command}");
        assert_eq!(operation.receipt, receipt);
        assert_eq!(
            store.operation(OP).await.unwrap().unwrap().0.status,
            "unknown"
        );
        assert_eq!(runner.count(command), 1);
    }
}

#[tokio::test]
async fn reconnect_binding_failure_rolls_back_success_and_keeps_raw_receipt() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("CREATE TRIGGER fail_binding BEFORE UPDATE OF coordinator_pane_key ON agent_mission BEGIN SELECT RAISE(ABORT, 'test binding failure'); END").execute(&mut *connection).await.unwrap();
    drop(connection);
    let input = ReconnectInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
    };
    assert!(service.reconnect(input.clone()).await.is_err());
    let stored = store.operation(OP).await.unwrap().unwrap().0;
    assert_eq!(stored.status, "pending");
    assert_eq!(stored.receipt["result"]["run"]["id"], "run-1");
    assert_eq!(
        store
            .mission(MISSION)
            .await
            .unwrap()
            .unwrap()
            .coordinator_handle
            .as_deref(),
        Some("coordinator-1")
    );
    assert_eq!(store.binding(MISSION).await.unwrap().1, Some(1));
    assert_eq!(service.reconnect(input).await.unwrap().status, "unknown");
    assert_eq!(runner.count("orchestration run-use"), 1);
}

#[tokio::test]
async fn create_stages_commit_binding_and_success_together_or_neither() {
    for fail_run in [false, true] {
        let (repository, store, runner, service) = setup().await;
        let mut connection = repository.acquire_test_connection().await.unwrap();
        let trigger = if fail_run {
            "CREATE TRIGGER fail_stage BEFORE UPDATE OF body_json ON agent_mission WHEN json_extract(NEW.body_json, '$.status') = 'ready' BEGIN SELECT RAISE(ABORT, 'test run failure'); END"
        } else {
            "CREATE TRIGGER fail_stage BEFORE UPDATE OF coordinator_pane_key ON agent_mission BEGIN SELECT RAISE(ABORT, 'test terminal failure'); END"
        };
        sqlx::query(trigger)
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        let input = CreateMissionInput {
            id: MISSION.into(),
            workspace_id: "workspace-1".into(),
            repository_id: "repo-1".into(),
            objective: "Create".into(),
        };
        let mission = service.create_mission(input.clone()).await.unwrap();
        assert_eq!(mission.status, "needs-attention");
        assert_eq!(mission.run_id, None);
        assert_eq!(mission.coordinator_handle.is_some(), fail_run);
        let stage = if fail_run { "run" } else { "terminal" };
        let operation = store
            .operation(&format!("{MISSION}:{stage}"))
            .await
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(operation.status, "pending");
        assert!(!operation.receipt.is_null());
        assert_eq!(store.binding(MISSION).await.unwrap().1, None);
        service.create_mission(input).await.unwrap();
        assert_eq!(runner.count("terminal create"), 1);
        assert_eq!(
            runner.count("orchestration run-create"),
            usize::from(fail_run)
        );
    }
}

struct PausedStartOrca {
    inner: Arc<MockOrca>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl CommandRunner for PausedStartOrca {
    fn available(&self) -> bool {
        true
    }
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        if arguments
            .get(1)
            .is_some_and(|argument| argument == "worker-start")
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.execute(arguments).await
    }
}

#[tokio::test]
async fn another_mission_can_snapshot_and_reply_while_start_is_pending_without_duplicate_dispatch()
{
    const OTHER_MISSION: &str = "d6510d2d-61b1-4ced-aec9-c87715b19244";
    let (_repository, store, runner, _) = setup().await;
    ready(&store).await;
    let mut other = store.mission(MISSION).await.unwrap().unwrap();
    other.id = OTHER_MISSION.into();
    other.run_id = Some("run-2".into());
    other.coordinator_handle = Some("coordinator-b".into());
    store.create_mission(&other).await.unwrap();
    store
        .save_binding(OTHER_MISSION, Some("tab-b:leaf-b"), Some(1))
        .await
        .unwrap();
    let mut other_worker = worker();
    other_worker["runId"] = json!("run-2");
    let mut other_task = task();
    other_task["run_id"] = json!("run-2");
    let mut other_question = question();
    other_question["run_id"] = json!("run-2");
    other_question["to_handle"] = json!("run:run-2");
    *runner.workers.lock().unwrap() = vec![other_worker];
    *runner.tasks.lock().unwrap() = vec![other_task];
    *runner.messages.lock().unwrap() = vec![other_question];
    let paused = Arc::new(PausedStartOrca {
        inner: runner.clone(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let service = Arc::new(
        AgentService::with_runner(store.clone(), paused.clone())
            .await
            .unwrap(),
    );
    let first = tokio::spawn({
        let service = service.clone();
        async move { service.start_task(task_input(&service).await).await }
    });
    paused.entered.notified().await;
    assert_eq!(
        store.operation(OP).await.unwrap().unwrap().0.status,
        "pending"
    );
    let duplicate_entered = Arc::new(tokio::sync::Notify::new());
    let duplicate = tokio::spawn({
        let service = service.clone();
        let entered = duplicate_entered.clone();
        async move {
            entered.notify_one();
            service.start_task(task_input(&service).await).await
        }
    });
    duplicate_entered.notified().await;
    // The timer is only a deadlock watchdog. Notify establishes the causal ordering.
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let snapshot = service.snapshot(OTHER_MISSION).await.unwrap();
        assert!(snapshot.connected);
        let reply = service
            .reply(ReplyInput {
                operation_id: SECOND_OP.into(),
                mission_id: OTHER_MISSION.into(),
                message_id: "question-1".into(),
                body: "Proceed".into(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, "succeeded");
    })
    .await
    .expect("unrelated mission control must not wait for the paused worker-start");
    assert!(!first.is_finished());
    assert!(!duplicate.is_finished());
    paused.release.notify_one();
    assert_eq!(first.await.unwrap().unwrap().status, "succeeded");
    assert_eq!(duplicate.await.unwrap().unwrap().status, "succeeded");
    assert_eq!(runner.count("orchestration worker-start"), 1);
}

#[tokio::test]
async fn conflicting_worker_rows_cannot_choose_a_release_target_by_array_order() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.tasks.lock().unwrap() = vec![task()];
    for conflicting_task in ["task-1", "another-task"] {
        let mut conflict = worker();
        conflict["taskId"] = json!(conflicting_task);
        conflict["projection"]["outcome"] = json!("in_progress");
        for workers in [
            vec![worker(), conflict.clone()],
            vec![conflict.clone(), worker()],
        ] {
            *runner.workers.lock().unwrap() = workers;
            let snapshot = service.snapshot(MISSION).await.unwrap();
            assert_eq!(snapshot.tasks[0].dispatch_id, None);
            assert!(!snapshot.tasks[0].can_release);
        }
    }
}

#[tokio::test]
async fn foreign_action_receipts_are_unknown_even_when_their_envelope_is_successful() {
    for command in [
        "orchestration worker-start",
        "orchestration reply",
        "orchestration worker-release",
    ] {
        let (_repository, store, runner, service) = setup().await;
        ready(&store).await;
        *runner.tasks.lock().unwrap() = vec![task()];
        *runner.workers.lock().unwrap() = vec![worker()];
        *runner.messages.lock().unwrap() = vec![question()];
        let receipt = match command {
            "orchestration worker-start" => ok(
                json!({"state":"ready","runId":"foreign-run","dispatchId":"dispatch-1","taskId":"task-1"}),
            ),
            "orchestration reply" => ok(
                json!({"message":{"id":"reply-1","run_id":"run-1","thread_id":"foreign-question"}}),
            ),
            _ => ok(json!({"state":"released","dispatchId":"foreign-dispatch"})),
        };
        runner.override_next(command, Ok(receipt.clone()));
        let operation = match command {
            "orchestration worker-start" => service
                .start_task(task_input(&service).await)
                .await
                .unwrap(),
            "orchestration reply" => service
                .reply(ReplyInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    message_id: "question-1".into(),
                    body: "Proceed".into(),
                })
                .await
                .unwrap(),
            _ => service
                .release(ReleaseWorkerInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    dispatch_id: "dispatch-1".into(),
                })
                .await
                .unwrap(),
        };
        assert_eq!(operation.status, "unknown", "{command}");
        assert_eq!(operation.receipt, receipt);
    }
}

#[tokio::test]
async fn run_receipts_need_the_expected_coordinator_and_a_generation_before_committing() {
    for run in [
        json!({"id":"run-1","coordinator_handle":"coordinator-1"}),
        json!({"id":"run-1","coordinator_handle":"foreign-handle","consumer_generation":1}),
    ] {
        let (_repository, store, runner, service) = setup().await;
        runner.override_next("orchestration run-create", Ok(ok(json!({"run":run}))));
        let mission = service
            .create_mission(CreateMissionInput {
                id: MISSION.into(),
                workspace_id: "workspace-1".into(),
                repository_id: "repo-1".into(),
                objective: "Create".into(),
            })
            .await
            .unwrap();
        assert_eq!(mission.status, "needs-attention");
        assert_eq!(mission.run_id, None);
        assert_eq!(store.binding(MISSION).await.unwrap().1, None);
        assert_eq!(
            store
                .operation(&format!("{MISSION}:run"))
                .await
                .unwrap()
                .unwrap()
                .0
                .status,
            "unknown"
        );
    }
}

#[tokio::test]
async fn replies_accept_existing_message_threads_and_validate_tracked_question_identity() {
    for tracked in [false, true] {
        let (_repository, store, runner, service) = setup().await;
        ready(&store).await;
        *runner.workers.lock().unwrap() = vec![worker()];
        let mut question = question();
        question["thread_id"] = json!("existing-thread");
        *runner.messages.lock().unwrap() = vec![question];
        let result = if tracked {
            json!({"message":{"id":"reply-1","run_id":"run-1","thread_id":"question-1"},"question":{"message_id":"question-1","answer_message_id":"reply-1","run_id":"run-1","status":"answered"}})
        } else {
            json!({"message":{"id":"reply-1","run_id":"run-1","thread_id":"existing-thread"}})
        };
        runner.override_next("orchestration reply", Ok(ok(result)));
        let input = ReplyInput {
            operation_id: OP.into(),
            mission_id: MISSION.into(),
            message_id: "question-1".into(),
            body: "Proceed".into(),
        };
        assert_eq!(
            service.reply(input.clone()).await.unwrap().status,
            "succeeded"
        );
        assert_eq!(service.reply(input).await.unwrap().status, "succeeded");
        assert_eq!(runner.count("orchestration reply"), 1);
    }
}

#[tokio::test]
async fn settled_task_without_active_dispatch_links_its_only_consistent_settled_worker() {
    for (task_status, worker_outcome) in [("completed", "succeeded"), ("failed", "failed")] {
        let (_repository, store, runner, service) = setup().await;
        ready(&store).await;
        let mut settled_task = task();
        // Real Orca task-list joins only pending/dispatched dispatch_contexts.
        settled_task["dispatch_id"] = Value::Null;
        settled_task["status"] = json!(task_status);
        let mut settled_worker = worker();
        settled_worker["workerState"] = json!(worker_outcome);
        settled_worker["dispatchStatus"] = json!(task_status);
        settled_worker["projection"]["outcome"] = json!(worker_outcome);
        *runner.tasks.lock().unwrap() = vec![settled_task];
        *runner.workers.lock().unwrap() = vec![settled_worker];
        let snapshot = service.snapshot(MISSION).await.unwrap();
        assert_eq!(snapshot.tasks[0].dispatch_id.as_deref(), Some("dispatch-1"));
        assert!(snapshot.tasks[0].can_release);
        assert!(
            service
                .read_output(ReadOutputInput {
                    mission_id: MISSION.into(),
                    dispatch_id: "dispatch-1".into(),
                    cursor: None
                })
                .await
                .unwrap()
                .text
                .contains("hello")
        );
        assert_eq!(
            service
                .release(ReleaseWorkerInput {
                    operation_id: OP.into(),
                    mission_id: MISSION.into(),
                    dispatch_id: "dispatch-1".into()
                })
                .await
                .unwrap()
                .status,
            "succeeded"
        );
    }
}

#[tokio::test]
async fn missing_active_dispatch_never_guesses_an_unsettled_or_ambiguous_worker() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    let mut no_dispatch_task = task();
    no_dispatch_task["dispatch_id"] = Value::Null;
    *runner.tasks.lock().unwrap() = vec![no_dispatch_task.clone()];
    let mut unsettled = worker();
    unsettled["workerState"] = json!("ready");
    let mut inconsistent = worker();
    inconsistent["projection"]["outcome"] = json!("failed");
    let mut missing_identity = worker();
    missing_identity["dispatchId"] = Value::Null;
    let mut other_attempt = worker();
    other_attempt["dispatchId"] = json!("dispatch-2");
    let mut identity_conflict = worker();
    identity_conflict["taskId"] = json!("other-task");
    for workers in [
        vec![unsettled],
        vec![inconsistent],
        vec![missing_identity],
        vec![worker(), other_attempt.clone()],
        vec![other_attempt, worker()],
        vec![worker(), identity_conflict],
    ] {
        *runner.workers.lock().unwrap() = workers;
        let snapshot = service.snapshot(MISSION).await.unwrap();
        assert_eq!(snapshot.tasks[0].dispatch_id, None);
        assert!(!snapshot.tasks[0].can_release);
    }
    no_dispatch_task["status"] = json!("dispatched");
    *runner.tasks.lock().unwrap() = vec![no_dispatch_task];
    *runner.workers.lock().unwrap() = vec![worker()];
    let snapshot = service.snapshot(MISSION).await.unwrap();
    assert_eq!(snapshot.tasks[0].dispatch_id, None);
    assert!(!snapshot.tasks[0].can_release);
}

struct DelayedSnapshotOrca {
    inner: Arc<MockOrca>,
    delay: std::time::Duration,
}

#[async_trait]
impl CommandRunner for DelayedSnapshotOrca {
    fn available(&self) -> bool {
        true
    }
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        tokio::time::sleep(self.delay).await;
        self.inner.execute(arguments).await
    }
}

/// Run the same harness before/after a scheduling change. The delay is an artificial
/// CLI latency model, not a measurement of Orca, a model provider, or another OS.
#[tokio::test]
#[ignore = "manual controlled CLI-delay benchmark; no live Orca process"]
async fn benchmark_snapshot_cli_delay_model() {
    const ROUNDS: usize = 9;
    const WARMUPS: usize = 2;
    const DELAY_MS: u64 = 25;
    let (_repository, store, runner, _) = setup().await;
    ready(&store).await;
    *runner.tasks.lock().unwrap() = vec![task()];
    *runner.workers.lock().unwrap() = vec![worker()];
    *runner.messages.lock().unwrap() = vec![question()];
    let service = AgentService::with_runner(
        store,
        Arc::new(DelayedSnapshotOrca {
            inner: runner.clone(),
            delay: std::time::Duration::from_millis(DELAY_MS),
        }),
    )
    .await
    .unwrap();
    for _ in 0..WARMUPS {
        assert!(service.snapshot(MISSION).await.unwrap().connected);
    }
    runner.calls.lock().unwrap().clear();
    let mut elapsed_ms = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let started = std::time::Instant::now();
        let snapshot = service.snapshot(MISSION).await.unwrap();
        elapsed_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(snapshot.connected);
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot.tasks[0].can_release);
        assert!(snapshot.messages[0].requires_reply);
    }
    assert_eq!(runner.count_all(), ROUNDS * 5);
    let counts: std::collections::BTreeMap<_, _> = [
        "status --json",
        "orchestration run-current",
        "orchestration worker-list",
        "orchestration task-list",
        "orchestration check",
    ]
    .into_iter()
    .map(|command| {
        let count = runner.count(command);
        assert_eq!(count, ROUNDS);
        (command, count)
    })
    .collect();
    let mut ordered = elapsed_ms.clone();
    ordered.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({
            "benchmark":"snapshot-cli-delay-model", "label":std::env::var("THOUGHSFLOW_BENCH_LABEL").unwrap_or_else(|_|"current".into()),
            "platform":format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH),
            "rounds":ROUNDS,"warmups":WARMUPS,"delayPerCliMs":DELAY_MS,"samplesMs":elapsed_ms,
            "medianMs":ordered[ROUNDS/2],"cliCalls":runner.count_all(),"callsPerSnapshot":5,"commandCounts":counts,
        })
    );
}

fn is_snapshot_read(arguments: &[String]) -> bool {
    arguments
        .first()
        .is_some_and(|command| command == "orchestration")
        && arguments.get(1).is_some_and(|command| {
            matches!(command.as_str(), "worker-list" | "task-list" | "check")
        })
}

struct GatedSnapshotOrca {
    inner: Arc<MockOrca>,
    started: tokio::sync::Barrier,
    release: tokio::sync::Barrier,
    reads: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl CommandRunner for GatedSnapshotOrca {
    fn available(&self) -> bool {
        true
    }
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        if is_snapshot_read(arguments) && self.reads.fetch_add(1, Ordering::SeqCst) < 3 {
            assert_eq!(self.inner.count("status --json"), 1);
            assert_eq!(self.inner.count("orchestration run-current"), 1);
            self.started.wait().await;
            self.release.wait().await;
        }
        self.inner.execute(arguments).await
    }
}

#[tokio::test]
async fn snapshot_starts_all_three_reads_after_connection_checks_and_keeps_mission_control_locked()
{
    let (_repository, store, runner, _) = setup().await;
    ready(&store).await;
    let gate = Arc::new(GatedSnapshotOrca {
        inner: runner.clone(),
        started: tokio::sync::Barrier::new(4),
        release: tokio::sync::Barrier::new(4),
        reads: std::sync::atomic::AtomicUsize::new(0),
    });
    let service = Arc::new(
        AgentService::with_runner(store.clone(), gate.clone())
            .await
            .unwrap(),
    );
    let snapshot = tokio::spawn({
        let service = service.clone();
        async move { service.snapshot(MISSION).await }
    });
    // A watchdog detects deadlocks; barriers establish concurrency without timing comparisons.
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.started.wait())
        .await
        .expect("all three independent reads must begin before any is released");
    assert!(!snapshot.is_finished());
    assert_eq!(runner.count_all(), 2);
    let writer_entered = Arc::new(tokio::sync::Notify::new());
    let writer = tokio::spawn({
        let service = service.clone();
        let entered = writer_entered.clone();
        async move {
            entered.notify_one();
            service.start_task(task_input(&service).await).await
        }
    });
    writer_entered.notified().await;
    assert!(!writer.is_finished());
    assert!(store.operation(OP).await.unwrap().is_none());
    assert_eq!(runner.count("orchestration worker-start"), 0);
    gate.release.wait().await;
    assert!(snapshot.await.unwrap().unwrap().connected);
    assert_eq!(writer.await.unwrap().unwrap().status, "succeeded");
    assert_eq!(runner.count("orchestration worker-start"), 1);
}

#[tokio::test]
async fn invalid_connection_never_starts_parallel_snapshot_reads() {
    for changed_runtime in [true, false] {
        let (_repository, store, runner, service) = setup().await;
        ready(&store).await;
        if changed_runtime {
            *runner.runtime.lock().unwrap() = "runtime-restarted".into();
        } else {
            runner.generation.store(2, Ordering::SeqCst);
        }
        let snapshot = service.snapshot(MISSION).await.unwrap();
        assert!(!snapshot.connected);
        assert!(!snapshot.can_start_tasks);
        for command in [
            "orchestration worker-list",
            "orchestration task-list",
            "orchestration check",
            "orchestration run-use",
        ] {
            assert_eq!(runner.count(command), 0, "{command}");
        }
    }
}

struct UnfinishedSnapshotRead<'a> {
    cancelled: &'a std::sync::atomic::AtomicUsize,
    completed: bool,
}

impl Drop for UnfinishedSnapshotRead<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct FailingSnapshotOrca {
    inner: Arc<MockOrca>,
    started: tokio::sync::Barrier,
    fail: tokio::sync::Notify,
    failed_command: &'static str,
    cancelled: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl CommandRunner for FailingSnapshotOrca {
    fn available(&self) -> bool {
        true
    }
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        if !is_snapshot_read(arguments) {
            return self.inner.execute(arguments).await;
        }
        let mut read = UnfinishedSnapshotRead {
            cancelled: &self.cancelled,
            completed: false,
        };
        self.started.wait().await;
        if arguments[1] == self.failed_command {
            self.fail.notified().await;
            read.completed = true;
            Err(ambiguous())
        } else {
            std::future::pending().await
        }
    }
}

#[tokio::test]
async fn failed_parallel_snapshot_read_cancels_other_reads_and_never_publishes_partial_data() {
    for failed_command in ["worker-list", "task-list", "check"] {
        let (_repository, store, runner, _) = setup().await;
        ready(&store).await;
        let gate = Arc::new(FailingSnapshotOrca {
            inner: runner.clone(),
            started: tokio::sync::Barrier::new(4),
            fail: tokio::sync::Notify::new(),
            failed_command,
            cancelled: std::sync::atomic::AtomicUsize::new(0),
        });
        let service = Arc::new(
            AgentService::with_runner(store.clone(), gate.clone())
                .await
                .unwrap(),
        );
        let snapshot = tokio::spawn({
            let service = service.clone();
            async move { service.snapshot(MISSION).await }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), gate.started.wait())
            .await
            .expect("all reads reached the barrier");
        gate.fail.notify_one();
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(2), snapshot)
            .await
            .expect("failed read cancels unfinished reads")
            .unwrap()
            .unwrap();
        assert_eq!(gate.cancelled.load(Ordering::SeqCst), 2);
        assert!(!snapshot.connected);
        assert!(!snapshot.can_start_tasks);
        assert!(snapshot.tasks.is_empty());
        assert!(snapshot.messages.is_empty());
        assert_eq!(
            store.mission(MISSION).await.unwrap().unwrap().status,
            "needs-attention"
        );
        let recovered = AgentService::with_runner(store.clone(), runner)
            .await
            .unwrap()
            .snapshot(MISSION)
            .await
            .unwrap();
        assert!(recovered.connected);
        assert_eq!(recovered.mission.status, "ready");
    }
}

#[tokio::test]
async fn later_worker_page_failure_discards_the_whole_parallel_snapshot() {
    let (_repository, store, runner, service) = setup().await;
    ready(&store).await;
    *runner.tasks.lock().unwrap() = vec![task()];
    *runner.messages.lock().unwrap() = vec![question()];
    runner.override_next(
        "orchestration worker-list",
        Ok(ok(
            json!({"workers":[worker()],"page":{"hasMore":true,"nextCursor":"page-2"}}),
        )),
    );
    let mut foreign = worker();
    foreign["runId"] = json!("foreign-run");
    runner.override_next(
        "orchestration worker-list",
        Ok(ok(json!({"workers":[foreign],"page":{"hasMore":false}}))),
    );
    let snapshot = service.snapshot(MISSION).await.unwrap();
    assert!(!snapshot.connected);
    assert!(!snapshot.can_start_tasks);
    assert!(snapshot.tasks.is_empty());
    assert!(snapshot.messages.is_empty());
    assert_eq!(runner.count("orchestration worker-list"), 2);
    assert_eq!(runner.count("orchestration worker-start"), 0);
}

fn preview_input(include_active_context: bool) -> PreviewAgentHandoffInput {
    PreviewAgentHandoffInput {
        mission_id: MISSION.into(),
        title: "Review selected context".into(),
        spec: "Explain the evidence and uncertainties.".into(),
        requested_model: None,
        context: AgentContextSelection {
            include_active_context,
            evidence_ids: vec![],
        },
    }
}

fn confirmed_task(
    input: &PreviewAgentHandoffInput,
    preview: &AgentHandoffPreview,
) -> StartTaskInput {
    StartTaskInput {
        operation_id: OP.into(),
        mission_id: input.mission_id.clone(),
        title: input.title.clone(),
        spec: input.spec.clone(),
        requested_model: input.requested_model.clone(),
        handoff: Some(AgentHandoffConfirmation {
            context: input.context.clone(),
            preview_hash: preview.hash.clone(),
            expected_cursor_version: preview.cursor_version,
            expected_draft_version: preview.draft_version,
            expected_branch_id: preview.branch_id.clone(),
            expected_branch_version: preview.branch_version,
        }),
    }
}

async fn graph_fixture(repository: &SqliteRepository) {
    let mut connection = repository.acquire_test_connection().await.unwrap();
    for (id, prompt, answer) in [
        ("selected", "selected-question", "selected-answer"),
        ("sibling", "sibling-question", "sibling-answer"),
    ] {
        sqlx::query("INSERT INTO content_block(id,role,content,content_hash,created_at) VALUES (?,'user',?,?,1)")
            .bind(format!("block-{id}")).bind(prompt).bind(crate::domain::sha256_hex(prompt.as_bytes()))
            .execute(&mut *connection).await.unwrap();
        sqlx::query("INSERT INTO turn(id,workspace_id,prompt_block_id,created_at) VALUES (?,'workspace-1',?,1)")
            .bind(format!("turn-{id}")).bind(format!("block-{id}")).execute(&mut *connection).await.unwrap();
        sqlx::query("INSERT INTO model_run(id,turn_id,workspace_id,model,status,output_markdown,provider_snapshot_json,created_at,finished_at) VALUES (?,?,'workspace-1','historical-model','completed',?,'{}',1,2)")
            .bind(format!("model-{id}")).bind(format!("turn-{id}")).bind(answer).execute(&mut *connection).await.unwrap();
    }
    sqlx::query("INSERT INTO branch_pointer(id,workspace_id,name,head_run_id,created_at,updated_at) VALUES ('branch-selected','workspace-1','Selected','model-selected',1,2)")
        .execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO workspace_context_cursor(workspace_id,active_run_id,branch_pointer_id,version,updated_at) VALUES ('workspace-1','model-selected','branch-selected',1,2)")
        .execute(&mut *connection).await.unwrap();
}

#[tokio::test]
async fn handoff_is_provider_free_persisted_before_dispatch_and_replayed_offline() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    let input = preview_input(false);
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    assert_eq!(runner.count_all(), 0, "preview never runs CLI or a model");
    let mut connection = repository.acquire_test_connection().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM provider_profile")
            .fetch_one(&mut *connection)
            .await
            .unwrap(),
        0
    );
    drop(connection);
    assert!(preview.text.contains("Implement task"));
    assert!(preview.text.contains(&input.title));
    assert!(preview.sources.is_empty());
    let task = confirmed_task(&input, &preview);
    assert_eq!(
        service.start_task(task.clone()).await.unwrap().status,
        "succeeded"
    );
    assert!(runner.pending_observed.load(Ordering::SeqCst));
    assert!(runner.handoff_observed.load(Ordering::SeqCst));
    let handoffs = service.handoffs(MISSION).await.unwrap();
    assert_eq!(handoffs.len(), 1);
    assert_eq!(handoffs[0].preview.hash, preview.hash);
    {
        let calls = runner.calls.lock().unwrap();
        let start = calls
            .iter()
            .find(|args| args.get(1).map(String::as_str) == Some("worker-start"))
            .unwrap();
        assert_eq!(argument(start, "--spec"), Some(preview.text.as_str()));
    }
    let before = runner.count_all();
    *runner.runtime.lock().unwrap() = "offline-runtime".into();
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query(
        "UPDATE workspace SET system_prompt='changed after dispatch' WHERE id='workspace-1'",
    )
    .execute(&mut *connection)
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE agent_handoff SET body_json='{}'")
            .execute(&mut *connection)
            .await
            .is_err()
    );
    drop(connection);
    assert_eq!(service.start_task(task).await.unwrap().status, "succeeded");
    assert_eq!(
        runner.count_all(),
        before,
        "replay must not compile or reconnect"
    );
    assert_eq!(
        service.handoffs(MISSION).await.unwrap()[0].preview.text,
        preview.text
    );
}

#[tokio::test]
async fn handoff_uses_only_effective_context_and_rejects_stale_preview_before_dispatch() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    graph_fixture(&repository).await;
    let input = preview_input(true);
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    assert!(preview.text.contains("selected-answer"));
    assert!(!preview.text.contains("sibling-answer"));
    assert_eq!(preview.source_run_ids, vec!["model-selected"]);
    assert_eq!(preview.branch_version, Some(0));
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("INSERT INTO context_draft(workspace_id,parent_run_id,version,updated_at) VALUES ('workspace-1','model-selected',1,3)").execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO context_override_item(workspace_id,position,operation,source_kind,source_id,created_at) VALUES ('workspace-1',0,'exclude','model_run','model-selected',3)")
        .execute(&mut *connection).await.unwrap();
    drop(connection);
    let error = service
        .start_task(confirmed_task(&input, &preview))
        .await
        .unwrap_err();
    assert_eq!(error.code, "agent_handoff_stale");
    assert_eq!(runner.count("orchestration worker-start"), 0);
    assert!(store.operations(MISSION).await.unwrap().is_empty());
    let excluded = service.preview_handoff(input.clone()).await.unwrap();
    assert!(!excluded.text.contains("selected-answer"));
    assert!(excluded.source_run_ids.is_empty());
    assert!(excluded.text.contains("selected-question"));
    let mut changed_title = input.clone();
    changed_title.title = "Different title".into();
    assert_ne!(
        service.preview_handoff(changed_title).await.unwrap().hash,
        excluded.hash
    );
}

#[tokio::test]
async fn handoff_transaction_rechecks_cursor_draft_branch_and_system_before_any_intent() {
    for update in [
        "UPDATE workspace_context_cursor SET version=version+1, updated_at=3 WHERE workspace_id='workspace-1'",
        "INSERT INTO context_draft(workspace_id,parent_run_id,version,updated_at) VALUES ('workspace-1','model-selected',1,3)",
        "UPDATE branch_pointer SET name='renamed',version=version+1,updated_at=3 WHERE id='branch-selected'",
        "UPDATE workspace SET system_prompt='new instruction' WHERE id='workspace-1'",
    ] {
        let (repository, store, _, service) = setup().await;
        ready(&store).await;
        graph_fixture(&repository).await;
        let mission = store.mission(MISSION).await.unwrap().unwrap();
        let prepared = store
            .prepare_handoff(&mission, &preview_input(true))
            .await
            .unwrap();
        let mut connection = repository.acquire_test_connection().await.unwrap();
        sqlx::query(update).execute(&mut *connection).await.unwrap();
        drop(connection);
        let error = store
            .begin_handoff(OP, &mission, &json!({}), prepared)
            .await
            .unwrap_err();
        assert_eq!(error.code, "agent_handoff_stale", "{update}");
        assert!(service.handoffs(MISSION).await.unwrap().is_empty());
        assert!(store.operations(MISSION).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn handoff_insert_failure_rolls_back_pending_operation_and_unknown_is_never_replayed() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    let input = preview_input(false);
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("CREATE TRIGGER reject_test_handoff BEFORE INSERT ON agent_handoff BEGIN SELECT RAISE(ABORT,'test failed insert'); END").execute(&mut *connection).await.unwrap();
    drop(connection);
    assert!(
        service
            .start_task(confirmed_task(&input, &preview))
            .await
            .is_err()
    );
    assert_eq!(runner.count("orchestration worker-start"), 0);
    assert!(store.operations(MISSION).await.unwrap().is_empty());
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("DROP TRIGGER reject_test_handoff")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    runner.override_next("orchestration worker-start", Err(ambiguous()));
    let task = confirmed_task(&input, &preview);
    assert_eq!(
        service.start_task(task.clone()).await.unwrap().status,
        "unknown"
    );
    assert_eq!(
        service.start_task(task.clone()).await.unwrap().status,
        "unknown"
    );
    let mut replacement = task;
    replacement.operation_id = SECOND_OP.into();
    assert!(service.start_task(replacement).await.is_err());
    assert_eq!(runner.count("orchestration worker-start"), 1);
    assert_eq!(service.handoffs(MISSION).await.unwrap().len(), 1);
}

#[tokio::test]
async fn evidence_capture_is_exact_immutable_idempotent_and_reusable_only_in_its_workspace() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    runner.workers.lock().unwrap().push(worker());
    runner.tasks.lock().unwrap().push(task());
    let preview_input = preview_input(false);
    let preview = service
        .preview_handoff(preview_input.clone())
        .await
        .unwrap();
    service
        .start_task(confirmed_task(&preview_input, &preview))
        .await
        .unwrap();
    runner.override_next("orchestration worker-read", Ok(ok(json!({
        "dispatchId":"dispatch-1", "projection":null, "source":"terminal", "terminal":{"tail":["Captured answer"],"limited":true},"cursor":"next-page","hasMore":false,"warnings":["tail only"]
    }))));
    let input = CaptureAgentEvidenceInput {
        id: SECOND_OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "dispatch-1".into(),
        cursor: Some("previous-page".into()),
    };
    let evidence = service.capture_evidence(input.clone()).await.unwrap();
    assert_eq!(evidence.request_cursor.as_deref(), Some("previous-page"));
    assert_eq!(evidence.cursor.as_deref(), Some("next-page"));
    assert!(!evidence.has_more);
    assert!(evidence.warning.is_some());
    assert_eq!(evidence.handoff_operation_id.as_deref(), Some(OP));
    assert_eq!(
        evidence.content_hash,
        crate::domain::sha256_hex(evidence.text.as_bytes())
    );
    let before = runner.count_all();
    *runner.runtime.lock().unwrap() = "offline".into();
    assert_eq!(
        service.capture_evidence(input.clone()).await.unwrap().text,
        "Captured answer"
    );
    assert_eq!(service.evidence("workspace-1").await.unwrap().len(), 1);
    assert_eq!(runner.count_all(), before);
    let mut changed = input.clone();
    changed.cursor = None;
    assert!(service.capture_evidence(changed).await.is_err());
    let mut next = preview_input.clone();
    next.context.evidence_ids.push(evidence.id.clone());
    let next_preview = service.preview_handoff(next.clone()).await.unwrap();
    assert!(next_preview.text.contains("one page; may be partial"));
    assert!(next_preview.text.contains("Captured answer"));
    assert!(next_preview.text.contains("dispatch-1"));
    assert_ne!(next_preview.hash, preview.hash);
    let mut connection = repository.acquire_test_connection().await.unwrap();
    assert!(
        sqlx::query("UPDATE agent_evidence SET body_json='{}'")
            .execute(&mut *connection)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM agent_evidence")
            .execute(&mut *connection)
            .await
            .is_err()
    );
    let raw: String = sqlx::query_scalar("SELECT raw_receipt_json FROM agent_evidence WHERE id=?")
        .bind(&evidence.id)
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert!(raw.contains("tail only"));
    sqlx::query("INSERT INTO workspace(id,title,goal,created_at,updated_at) VALUES ('workspace-2','Other','other',1,1)").execute(&mut *connection).await.unwrap();
    drop(connection);
    let mut other = store.mission(MISSION).await.unwrap().unwrap();
    other.id = "d6510d2d-61b1-4ced-aec9-c87715b19244".into();
    other.workspace_id = "workspace-2".into();
    store.create_mission(&other).await.unwrap();
    next.mission_id = other.id;
    assert!(
        service
            .preview_handoff(next)
            .await
            .unwrap_err()
            .message
            .contains("其他工作区")
    );
    assert_eq!(runner.count_all(), before);
}

#[tokio::test]
async fn evidence_never_saves_output_without_exact_dispatch_and_optional_projection_identity() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    runner.workers.lock().unwrap().push(worker());
    runner.tasks.lock().unwrap().push(task());
    let input = CaptureAgentEvidenceInput {
        id: SECOND_OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "dispatch-1".into(),
        cursor: None,
    };
    for identity in [
        json!({}),
        json!({"dispatchId":null}),
        json!({"dispatchId":12}),
        json!({"dispatchId":"foreign"}),
        json!({"dispatchId":"dispatch-1","projection":{"dispatchId":"other","taskId":"task-1","runId":"run-1"}}),
        json!({"dispatchId":"dispatch-1","projection":{"dispatchId":"dispatch-1","taskId":"other","runId":"run-1"}}),
        json!({"dispatchId":"dispatch-1","projection":{"dispatchId":"dispatch-1","taskId":"task-1","runId":"other"}}),
        json!({"dispatchId":"dispatch-1","projection":{}}),
    ] {
        let mut result = identity;
        result["terminal"] = json!({"tail":["must not be archived"],"limited":false});
        runner.override_next("orchestration worker-read", Ok(ok(result)));
        assert!(service.capture_evidence(input.clone()).await.is_err());
        assert!(service.evidence("workspace-1").await.unwrap().is_empty());
    }
    runner.override_next("orchestration worker-read", Ok(ok(json!({"dispatchId":"dispatch-1","projection":{"dispatchId":"dispatch-1","taskId":"task-1","runId":"run-1"},"terminal":{"tail":["verified observation"],"limited":false}}))));
    assert_eq!(
        service.capture_evidence(input).await.unwrap().text,
        "verified observation"
    );
}

#[tokio::test]
async fn evidence_rejects_runtime_identity_and_source_payload_conflicts() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    runner.workers.lock().unwrap().push(worker());
    runner.tasks.lock().unwrap().push(task());
    let input = CaptureAgentEvidenceInput {
        id: SECOND_OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "dispatch-1".into(),
        cursor: None,
    };
    for runtime in [
        None,
        Some(Value::Null),
        Some(json!(12)),
        Some(json!("foreign-runtime")),
    ] {
        let mut response = ok(
            json!({"dispatchId":"dispatch-1", "source":"terminal", "terminal":{"tail":["wrong runtime"]}}),
        );
        match runtime {
            Some(value) => response["_meta"]["runtimeId"] = value,
            None => {
                response.as_object_mut().unwrap().remove("_meta");
            }
        }
        runner.override_next("orchestration worker-read", Ok(response));
        assert!(service.capture_evidence(input.clone()).await.is_err());
        assert!(service.evidence("workspace-1").await.unwrap().is_empty());
    }
    for payload in [
        json!({"source":"transcript","terminal":{"tail":["mislabeled"]}}),
        json!({"source":"terminal","transcript":{"messages":[]}}),
        json!({"source":"unknown","terminal":{"tail":["unknown source"]}}),
        json!({"source":9,"terminal":{"tail":["invalid source"]}}),
    ] {
        let mut result = payload;
        result["dispatchId"] = json!("dispatch-1");
        runner.override_next("orchestration worker-read", Ok(ok(result)));
        assert!(service.capture_evidence(input.clone()).await.is_err());
        assert!(service.evidence("workspace-1").await.unwrap().is_empty());
    }
    runner.override_next("orchestration worker-read", Ok(ok(json!({
        "dispatchId":"dispatch-1", "source":"transcript",
        "terminal":{"tail":["do not mislabel this"]},
        "transcript":{"messages":[{"role":"assistant","blocks":[{"type":"text","text":"the declared transcript"}]}]},
    }))));
    let observation = service.capture_evidence(input).await.unwrap();
    assert!(observation.text.contains("the declared transcript"));
    assert!(!observation.text.contains("do not mislabel this"));
    assert_eq!(observation.source.as_deref(), Some("transcript"));
}

#[tokio::test]
async fn new_dispatch_requires_handoff_but_legacy_operations_still_replay_without_effects() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let input = legacy_task_input();
    assert_eq!(
        service.start_task(input.clone()).await.unwrap_err().code,
        "agent_handoff_required"
    );
    assert_eq!(runner.count_all(), 0);
    assert!(store.operations(MISSION).await.unwrap().is_empty());
    let mut legacy = store
        .begin(
            OP,
            MISSION,
            "start-task",
            &serde_json::to_value(&input).unwrap(),
        )
        .await
        .unwrap();
    legacy.status = "succeeded".into();
    legacy.receipt = ok(
        json!({"dispatchId":"historical-dispatch","taskId":"historical-task","runId":"run-1","state":"ready"}),
    );
    store.finish_operation(&legacy, None, None).await.unwrap();
    assert_eq!(
        service.start_task(input).await.unwrap().receipt,
        legacy.receipt
    );
    assert_eq!(runner.count_all(), 0);
    assert!(
        service.handoffs(MISSION).await.unwrap().is_empty(),
        "legacy replay must not invent a handoff"
    );
}

#[tokio::test]
async fn pinned_sibling_is_an_explicit_handoff_source_while_excluded_path_answer_is_not() {
    let (repository, store, _, service) = setup().await;
    ready(&store).await;
    graph_fixture(&repository).await;
    let hash = crate::domain::sha256_hex(b"sibling-answer");
    let block_id = format!("block-assistant-{hash}");
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("INSERT INTO content_block(id,role,content,content_hash,created_at) VALUES (?,'assistant','sibling-answer',?,3)").bind(&block_id).bind(&hash).execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO context_draft(workspace_id,parent_run_id,version,updated_at) VALUES ('workspace-1','model-selected',1,3)").execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO context_override_item(workspace_id,position,operation,source_kind,source_id,created_at) VALUES ('workspace-1',0,'exclude','model_run','model-selected',3)").execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO context_override_item(workspace_id,position,operation,source_kind,source_id,content_block_id,content_hash,created_at) VALUES ('workspace-1',1,'pin','model_run','model-sibling',?,?,3)").bind(&block_id).bind(&hash).execute(&mut *connection).await.unwrap();
    drop(connection);
    let preview = service.preview_handoff(preview_input(true)).await.unwrap();
    assert!(preview.text.contains("sibling-answer"));
    assert!(!preview.text.contains("selected-answer"));
    assert_eq!(preview.source_run_ids, vec!["model-sibling"]);
    assert!(
        preview
            .sources
            .iter()
            .any(|source| source.content_hash == hash)
    );
}

#[tokio::test]
async fn checkpoint_publication_without_cursor_or_branch_change_invalidates_atomic_handoff() {
    use crate::infrastructure::sqlite::{
        ContentBlockRecord, ContextCheckpointRecord, ContextMaintenanceRunRecord,
        MaintenanceContextGuardRecord,
    };
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    graph_fixture(&repository).await;
    let mission = store.mission(MISSION).await.unwrap().unwrap();
    let input = preview_input(true);
    let prepared = store.prepare_handoff(&mission, &input).await.unwrap();
    let original_preview = prepared.preview.clone();
    let summary_hash = crate::domain::sha256_hex(b"A manual checkpoint");
    let summary = ContentBlockRecord {
        id: format!("block-system-{summary_hash}"),
        role: "system".into(),
        content: "A manual checkpoint".into(),
        content_hash: summary_hash,
        created_at: 4,
    };
    let running = ContextMaintenanceRunRecord {
        id: "manual-checkpoint".into(),
        workspace_id: "workspace-1".into(),
        kind: "compaction".into(),
        status: "running".into(),
        branch_pointer_id: Some("branch-selected".into()),
        branch_revision: Some(0),
        anchor_run_id: "model-selected".into(),
        first_kept_run_id: Some("model-selected".into()),
        source_run_ids_json: "[\"model-selected\"]".into(),
        source_hash: "source-checkpoint".into(),
        provider_snapshot_json: None,
        request_json: "{}".into(),
        summary_block_id: None,
        summary: None,
        error_json: None,
        created_at: 3,
        started_at: Some(3),
        finished_at: None,
    };
    let guard = MaintenanceContextGuardRecord {
        workspace_id: "workspace-1".into(),
        expected_cursor_version: 1,
        expected_draft_version: None,
        branch_pointer_id: Some("branch-selected".into()),
        expected_branch_version: Some(0),
    };
    repository
        .start_context_maintenance(&running, &guard)
        .await
        .unwrap();
    let checkpoint = ContextCheckpointRecord {
        id: "manual-checkpoint".into(),
        workspace_id: running.workspace_id.clone(),
        maintenance_run_id: running.id.clone(),
        kind: running.kind.clone(),
        branch_pointer_id: running.branch_pointer_id.clone(),
        branch_revision: running.branch_revision,
        anchor_run_id: running.anchor_run_id.clone(),
        first_kept_run_id: running.first_kept_run_id.clone(),
        summary_block_id: summary.id.clone(),
        summary: summary.content.clone(),
        summary_content_hash: summary.content_hash.clone(),
        source_run_ids_json: running.source_run_ids_json.clone(),
        source_hash: running.source_hash.clone(),
        provider_snapshot_json: None,
        created_at: 4,
    };
    let completed = ContextMaintenanceRunRecord {
        status: "completed".into(),
        summary_block_id: Some(summary.id.clone()),
        summary: Some(summary.content.clone()),
        finished_at: Some(4),
        ..running
    };
    repository
        .finish_context_maintenance(&completed, Some(&checkpoint), Some(&summary), &guard, None)
        .await
        .unwrap();
    let fresh = service.preview_handoff(input).await.unwrap();
    assert_eq!(fresh.cursor_version, original_preview.cursor_version);
    assert_eq!(fresh.draft_version, original_preview.draft_version);
    assert_eq!(fresh.branch_version, original_preview.branch_version);
    assert_ne!(fresh.hash, original_preview.hash);
    assert_eq!(
        store
            .begin_handoff(OP, &mission, &json!({}), prepared)
            .await
            .unwrap_err()
            .code,
        "agent_handoff_stale"
    );
    assert!(service.handoffs(MISSION).await.unwrap().is_empty());
    assert!(store.operations(MISSION).await.unwrap().is_empty());
    assert_eq!(runner.count_all(), 0);
}

#[tokio::test]
async fn evidence_rejects_ambiguous_worker_or_task_rows_without_reading_output() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let input = CaptureAgentEvidenceInput {
        id: SECOND_OP.into(),
        mission_id: MISSION.into(),
        dispatch_id: "dispatch-1".into(),
        cursor: None,
    };
    runner.workers.lock().unwrap().extend([worker(), worker()]);
    runner.tasks.lock().unwrap().push(task());
    assert!(service.capture_evidence(input.clone()).await.is_err());
    runner.workers.lock().unwrap().pop();
    runner.tasks.lock().unwrap().push(task());
    assert!(service.capture_evidence(input).await.is_err());
    assert_eq!(runner.count("orchestration worker-read"), 0);
    assert!(service.evidence("workspace-1").await.unwrap().is_empty());
}

async fn insert_history_fixture(repository: &SqliteRepository, index: usize) {
    let operation = format!("history-operation-{index}");
    let evidence = format!("history-evidence-{index}");
    let mut handoff = json!({
        "operationId": operation, "workspaceId":"workspace-1", "missionId":MISSION,
        "createdAt":"2026-10-06T01:00:00Z", "preview":{
            "hash":format!("handoff-hash-{index}"), "title":format!("Task {index}"), "text":"historical handoff body ".repeat(1024),
            "compilerVersion":"agent-handoff/1; context/4", "sourceRunIds":["model-selected"], "evidenceIds":["parent-evidence"], "sources":[],
            "parentRunId":null, "branchId":null, "cursorVersion":0, "draftVersion":0, "branchVersion":null
        }
    });
    // Old handoffs legitimately predate the additive, human-readable title.
    if index == 0 {
        handoff["preview"].as_object_mut().unwrap().remove("title");
    }
    let record = json!({
        "id":evidence,"workspaceId":"workspace-1","missionId":MISSION,"runId":"run-1","taskId":format!("task-{index}"),
        "dispatchId":format!("dispatch-{index}"),"handoffOperationId":operation,"text":"historical output body ".repeat(2048),
        "contentHash":format!("evidence-hash-{index}"),"source":"terminal","requestCursor":null,"cursor":"opaque-output-cursor",
        "hasMore":true,"warning":"one page only","observedTaskStatus":"completed","createdAt":"2026-10-06T01:00:00Z"
    });
    let mut connection = repository.acquire_test_connection().await.unwrap();
    sqlx::query("INSERT INTO agent_operation(id,mission_id,kind,status,request_json,receipt_json,created_at) VALUES (?,?,'start-task','succeeded','{}',?,'2026-10-06T01:00:00Z')")
        .bind(&operation).bind(MISSION).bind(ok(json!({"dispatchId":format!("dispatch-{index}"),"taskId":format!("task-{index}"),"runId":"run-1"})).to_string())
        .execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO agent_handoff(operation_id,workspace_id,mission_id,body_json) VALUES (?,'workspace-1',?,?)")
        .bind(&operation).bind(MISSION).bind(handoff.to_string()).execute(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO agent_evidence(id,workspace_id,mission_id,request_json,body_json,raw_receipt_json) VALUES (?,'workspace-1',?,'{}',?,'{}')")
        .bind(&evidence).bind(MISSION).bind(record.to_string()).execute(&mut *connection).await.unwrap();
}

#[tokio::test]
async fn bounded_history_pages_project_summaries_and_keep_keyset_position_after_insertion() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    for index in 0..51 {
        insert_history_fixture(&repository, index).await;
    }
    let evidence_page = service
        .evidence_page(AgentEvidencePageInput {
            workspace_id: "workspace-1".into(),
            cursor: None,
        })
        .await
        .unwrap();
    let handoff_page = service
        .handoff_page(AgentHandoffPageInput {
            mission_id: MISSION.into(),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(evidence_page.items.len(), 50);
    assert_eq!(handoff_page.items.len(), 50);
    assert_eq!(evidence_page.items[0].id, "history-evidence-50");
    assert_eq!(evidence_page.items[0].title.as_deref(), Some("Task 50"));
    assert!(evidence_page.items[0].has_more);
    assert_eq!(handoff_page.items[0].title, "Task 50");
    for page in [
        serde_json::to_value(&evidence_page).unwrap(),
        serde_json::to_value(&handoff_page).unwrap(),
    ] {
        assert!(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item.get("text").is_none()
                    && item.get("preview").is_none()
                    && item.get("sources").is_none())
        );
        assert!(!page.to_string().contains("historical output body"));
        assert!(!page.to_string().contains("historical handoff body"));
        assert!(page.to_string().len() < 40_000);
    }
    insert_history_fixture(&repository, 51).await;
    let evidence_tail = service
        .evidence_page(AgentEvidencePageInput {
            workspace_id: "workspace-1".into(),
            cursor: evidence_page.next_cursor.clone(),
        })
        .await
        .unwrap();
    let handoff_tail = service
        .handoff_page(AgentHandoffPageInput {
            mission_id: MISSION.into(),
            cursor: handoff_page.next_cursor.clone(),
        })
        .await
        .unwrap();
    assert_eq!(
        evidence_tail
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["history-evidence-0"]
    );
    assert_eq!(
        handoff_tail
            .items
            .iter()
            .map(|item| item.operation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["history-operation-0"]
    );
    assert!(evidence_tail.next_cursor.is_none());
    assert!(handoff_tail.next_cursor.is_none());
    assert!(evidence_tail.items[0].title.is_none());
    assert!(handoff_tail.items[0].title.is_empty());
    let restarted = service
        .evidence_page(AgentEvidencePageInput {
            workspace_id: "workspace-1".into(),
            cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(restarted.items[0].id, "history-evidence-51");
    assert_eq!(runner.count_all(), 0, "history is local-only");
}

#[tokio::test]
async fn history_cursor_kind_scope_and_detail_ownership_are_checked_without_changing_records() {
    let (repository, store, runner, service) = setup().await;
    ready(&store).await;
    for index in 0..51 {
        insert_history_fixture(&repository, index).await;
    }
    let mut connection = repository.acquire_test_connection().await.unwrap();
    let original: (String,String) = sqlx::query_as("SELECT e.body_json,h.body_json FROM agent_evidence e JOIN agent_handoff h ON h.operation_id=json_extract(e.body_json,'$.handoffOperationId') WHERE e.id='history-evidence-0'")
        .fetch_one(&mut *connection).await.unwrap();
    sqlx::query("INSERT INTO workspace(id,title,goal,created_at,updated_at) VALUES ('workspace-2','Other','other',1,1)").execute(&mut *connection).await.unwrap();
    drop(connection);
    let mut mission = store.mission(MISSION).await.unwrap().unwrap();
    mission.id = SECOND_OP.into();
    mission.workspace_id = "workspace-2".into();
    store.create_mission(&mission).await.unwrap();
    let cursor = service
        .evidence_page(AgentEvidencePageInput {
            workspace_id: "workspace-1".into(),
            cursor: None,
        })
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    for bad in [
        "not-a-cursor".into(),
        format!("{cursor}corrupted"),
        "h1.-1.invalid".into(),
        "x".repeat(129),
    ] {
        assert_eq!(
            service
                .evidence_page(AgentEvidencePageInput {
                    workspace_id: "workspace-1".into(),
                    cursor: Some(bad)
                })
                .await
                .unwrap_err()
                .code,
            "agent_history_cursor"
        );
    }
    for bad in [
        cursor.clone(),
        "not-a-cursor".into(),
        format!("{cursor}corrupted"),
        "".into(),
    ] {
        assert!(
            service
                .evidence_page(AgentEvidencePageInput {
                    workspace_id: "workspace-2".into(),
                    cursor: Some(bad)
                })
                .await
                .is_err()
        );
    }
    assert!(
        service
            .handoff_page(AgentHandoffPageInput {
                mission_id: MISSION.into(),
                cursor: Some(cursor)
            })
            .await
            .is_err()
    );
    let cursor = service
        .handoff_page(AgentHandoffPageInput {
            mission_id: MISSION.into(),
            cursor: None,
        })
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    assert!(
        service
            .handoff_page(AgentHandoffPageInput {
                mission_id: SECOND_OP.into(),
                cursor: Some(cursor)
            })
            .await
            .is_err()
    );
    assert!(
        service
            .get_evidence(GetAgentEvidenceInput {
                workspace_id: "workspace-2".into(),
                id: "history-evidence-0".into()
            })
            .await
            .is_err()
    );
    assert!(
        service
            .get_handoff(GetAgentHandoffInput {
                mission_id: SECOND_OP.into(),
                operation_id: "history-operation-0".into()
            })
            .await
            .is_err()
    );
    let evidence = service
        .get_evidence(GetAgentEvidenceInput {
            workspace_id: "workspace-1".into(),
            id: "history-evidence-0".into(),
        })
        .await
        .unwrap();
    let handoff = service
        .get_handoff(GetAgentHandoffInput {
            mission_id: MISSION.into(),
            operation_id: "history-operation-0".into(),
        })
        .await
        .unwrap();
    assert_eq!(evidence.text, "historical output body ".repeat(2048));
    assert_eq!(
        handoff.preview.text,
        "historical handoff body ".repeat(1024)
    );
    assert_eq!(evidence.content_hash, "evidence-hash-0");
    assert_eq!(handoff.preview.hash, "handoff-hash-0");
    let mut connection = repository.acquire_test_connection().await.unwrap();
    let after: (String,String) = sqlx::query_as("SELECT e.body_json,h.body_json FROM agent_evidence e JOIN agent_handoff h ON h.operation_id=json_extract(e.body_json,'$.handoffOperationId') WHERE e.id='history-evidence-0'")
        .fetch_one(&mut *connection).await.unwrap();
    assert_eq!(after, original);
    assert_eq!(runner.count_all(), 0);
}

fn model_preview_input(model: &str) -> PreviewAgentHandoffInput {
    let mut input = serde_json::to_value(preview_input(false)).unwrap();
    input["requestedModel"] = json!(model);
    serde_json::from_value(input).unwrap()
}

fn model_start_input(
    input: &PreviewAgentHandoffInput,
    preview: &AgentHandoffPreview,
    model: &str,
) -> StartTaskInput {
    let mut task = serde_json::to_value(confirmed_task(input, preview)).unwrap();
    task["requestedModel"] = json!(model);
    serde_json::from_value(task).unwrap()
}

fn model_runtime(runtime: &str) -> Value {
    ok(
        json!({"runtime":{"reachable":true,"state":"ready","runtimeId":runtime,"capabilities":["orchestration.contract.v1","orchestration.worker-launch-preferences.v1"]},"graph":{"state":"ready"}}),
    )
}

fn model_start_receipt(model: &str) -> Value {
    ok(
        json!({"dispatchId":"dispatch-1","taskId":"task-1","runId":"run-1","state":"ready","launch":{"requested":{"agent":"omp","model":model,"effort":null},"effective":{"agent":"omp","model":model,"effort":null}}}),
    )
}

#[tokio::test]
async fn model_selection_is_omitted_by_default_and_frozen_in_handoff_hash() {
    let (_, store, _, service) = setup().await;
    ready(&store).await;
    let legacy = legacy_task_input();
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        json!({"operationId":OP,"missionId":MISSION,"title":legacy.title,"spec":legacy.spec})
    );
    let input = preview_input(false);
    let default_preview = service.preview_handoff(input.clone()).await.unwrap();
    assert!(
        serde_json::to_value(&input)
            .unwrap()
            .get("requestedModel")
            .is_none()
    );
    assert!(
        serde_json::to_value(&default_preview)
            .unwrap()
            .get("requestedModel")
            .is_none()
    );
    assert_eq!(
        default_preview.hash,
        "13571531395596f5594cb7e2a5b151b075e9de2648f52555c19a3d4132b51f25"
    );
    let first = service
        .preview_handoff(model_preview_input("provider/model-one"))
        .await
        .unwrap();
    let second = service
        .preview_handoff(model_preview_input("@reviewer"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap()["requestedModel"],
        "provider/model-one"
    );
    assert_ne!(default_preview.hash, first.hash);
    assert_ne!(first.hash, second.hash);
    assert_eq!(default_preview.text, first.text);
    let mut old_preview = serde_json::to_value(default_preview).unwrap();
    old_preview["hash"] = json!("historic-hash");
    let restored: AgentHandoffPreview = serde_json::from_value(old_preview.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), old_preview);
}

#[tokio::test]
async fn model_selection_validation_is_bounded_and_runs_before_runtime_access() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let valid = service.preview_handoff(preview_input(false)).await.unwrap();
    for model in [
        "".to_owned(),
        " ".into(),
        " model".into(),
        "model ".into(),
        "\u{feff}provider/model".into(),
        "provider/model\u{feff}".into(),
        "model\n".into(),
        "a\u{0085}b".into(),
        "x".repeat(513),
        "😀".repeat(257),
    ] {
        let input = model_preview_input(&model);
        assert_eq!(
            service
                .preview_handoff(input.clone())
                .await
                .unwrap_err()
                .code,
            "agent_model"
        );
        let task = model_start_input(&input, &valid, &model);
        assert_eq!(
            service.start_task(task).await.unwrap_err().code,
            "agent_model"
        );
    }
    for model in ["provider/model", "@reviewer", &"😀".repeat(256)] {
        service
            .preview_handoff(model_preview_input(model))
            .await
            .unwrap();
    }
    assert_eq!(runner.count_all(), 0);
    assert!(store.operations(MISSION).await.unwrap().is_empty());
}

#[tokio::test]
async fn model_selection_requires_capability_and_same_mission_runtime_before_dispatch() {
    for status in [None, Some(model_runtime("another-runtime"))] {
        let (_, store, runner, service) = setup().await;
        ready(&store).await;
        let input = model_preview_input("provider/model");
        let preview = service.preview_handoff(input.clone()).await.unwrap();
        if let Some(status) = status {
            runner.override_next("status --json", Ok(status));
        }
        assert!(
            service
                .start_task(model_start_input(&input, &preview, "provider/model"))
                .await
                .is_err()
        );
        assert_eq!(runner.count("status --json"), 1);
        assert_eq!(runner.count("orchestration worker-start"), 0);
        assert!(store.operations(MISSION).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn model_selection_is_single_native_argument_and_replays_offline() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let model = "provider/model;$(literal)";
    let input = model_preview_input(model);
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    let task = model_start_input(&input, &preview, model);
    runner.override_next("status --json", Ok(model_runtime("runtime-1")));
    let receipt = model_start_receipt(model);
    runner.override_next("orchestration worker-start", Ok(receipt.clone()));
    let result = service.start_task(task.clone()).await.unwrap();
    assert_eq!(result.status, "succeeded");
    assert_eq!(result.receipt, receipt);
    let calls = runner.calls.lock().unwrap().clone();
    let launch = calls
        .iter()
        .find(|args| args.get(1).map(String::as_str) == Some("worker-start"))
        .unwrap();
    assert_eq!(
        launch
            .iter()
            .filter(|arg| arg.as_str() == "--model")
            .count(),
        1
    );
    assert_eq!(argument(launch, "--model"), Some(model));
    assert!(!launch.iter().any(|arg| arg == "--effort"));
    assert_eq!(runner.count("status --json"), 1);
    assert!(runner.handoff_observed.load(Ordering::SeqCst));
    assert_eq!(
        serde_json::to_value(store.handoffs(MISSION).await.unwrap()).unwrap()[0]["preview"]["requestedModel"],
        model
    );
    let before = runner.count_all();
    runner.override_next(
        "status --json",
        Err(RuntimeFailure {
            message: "offline".into(),
            ambiguous: false,
            receipt: json!({}),
        }),
    );
    assert_eq!(service.start_task(task).await.unwrap().receipt, receipt);
    assert_eq!(runner.count_all(), before);
}

#[tokio::test]
async fn changed_model_invalidates_confirmation_without_dispatch() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let input = model_preview_input("provider/first");
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    runner.override_next("status --json", Ok(model_runtime("runtime-1")));
    let error = service
        .start_task(model_start_input(&input, &preview, "provider/second"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "agent_handoff_stale");
    assert_eq!(runner.count("orchestration worker-start"), 0);
    assert!(store.operations(MISSION).await.unwrap().is_empty());
}

#[tokio::test]
async fn model_selection_refusal_is_recorded_without_fallback() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    let input = model_preview_input("provider/model");
    let preview = service.preview_handoff(input.clone()).await.unwrap();
    let task = model_start_input(&input, &preview, "provider/model");
    runner.override_next("status --json", Ok(model_runtime("runtime-1")));
    let refusal = json!({"ok":false,"error":{"code":"invalid_argument","message":"This agent does not support --model"}});
    runner.override_next(
        "orchestration worker-start",
        Err(RuntimeFailure {
            message: "This agent does not support --model".into(),
            ambiguous: false,
            receipt: refusal.clone(),
        }),
    );
    let operation = service.start_task(task.clone()).await.unwrap();
    assert_eq!(operation.status, "failed");
    assert_eq!(operation.receipt, refusal);
    assert_eq!(service.start_task(task).await.unwrap().receipt, refusal);
    assert_eq!(runner.count("orchestration worker-start"), 1);
}

#[tokio::test]
async fn missing_or_mismatched_model_launch_receipts_are_unknown_and_block_new_ids() {
    let model = "provider/model";
    let mut variants = vec![];
    let mut missing = model_start_receipt(model);
    missing["result"].as_object_mut().unwrap().remove("launch");
    variants.push(missing);
    for (pointer, value) in [
        ("/result/launch/effective", Value::Null),
        ("/result/launch/requested/agent", json!("codex")),
        ("/result/launch/effective/agent", json!("codex")),
        ("/result/launch/requested/model", json!("other/model")),
        ("/result/launch/effective/model", json!("other/model")),
        ("/result/launch/requested/effort", json!("high")),
        ("/result/launch/effective/effort", json!("high")),
        ("/result/runId", json!("foreign-run")),
        ("/result/taskId", Value::Null),
        ("/result/dispatchId", Value::Null),
    ] {
        let mut receipt = model_start_receipt(model);
        *receipt.pointer_mut(pointer).unwrap() = value;
        variants.push(receipt);
    }
    for selection in ["requested", "effective"] {
        let mut missing_effort = model_start_receipt(model);
        missing_effort["result"]["launch"][selection]
            .as_object_mut()
            .unwrap()
            .remove("effort");
        variants.push(missing_effort);
    }
    for receipt in variants {
        let (_, store, runner, service) = setup().await;
        ready(&store).await;
        let input = model_preview_input(model);
        let preview = service.preview_handoff(input.clone()).await.unwrap();
        let task = model_start_input(&input, &preview, model);
        runner.override_next("status --json", Ok(model_runtime("runtime-1")));
        runner.override_next("orchestration worker-start", Ok(receipt.clone()));
        let operation = service.start_task(task.clone()).await.unwrap();
        assert_eq!(operation.status, "unknown", "{receipt}");
        assert_eq!(operation.receipt, receipt);
        let before = runner.count_all();
        assert_eq!(
            service.start_task(task.clone()).await.unwrap().status,
            "unknown"
        );
        assert_eq!(runner.count_all(), before);
        let mut replacement = task;
        replacement.operation_id = SECOND_OP.into();
        runner.override_next("status --json", Ok(model_runtime("runtime-1")));
        assert!(service.start_task(replacement).await.is_err());
        assert_eq!(runner.count("orchestration worker-start"), 1);
    }
}

#[tokio::test]
async fn transcript_pagination_uses_selected_source_limit_and_cursor_progress() {
    let (_, store, runner, service) = setup().await;
    ready(&store).await;
    runner.workers.lock().unwrap().push(worker());
    for (limited, cursor, request_cursor, top_has_more, expected_more) in [
        (Some(true), Some("next"), None, None, true),
        (Some(false), Some("next"), None, None, false),
        (Some(true), Some("same"), Some("same"), None, false),
        (Some(true), None, None, None, false),
        (Some(true), Some(""), None, None, false),
        (Some(true), Some("next"), None, Some(false), false),
        (Some(false), Some("next"), None, Some(true), true),
        (None, Some("next"), None, None, true),
        (None, Some("same"), Some("same"), Some(true), false),
    ] {
        let mut result = json!({"dispatchId":"dispatch-1","source":"transcript","transcript":{"messages":[{"role":"assistant","blocks":[{"type":"text","text":"Output page"}]}]},"terminal":{"tail":["ignore"],"limited":true},"cursor":cursor});
        if let Some(limited) = limited {
            result["transcript"]["limited"] = json!(limited);
        }
        if let Some(has_more) = top_has_more {
            result["hasMore"] = json!(has_more);
        }
        runner.override_next("orchestration worker-read", Ok(ok(result)));
        let output = service
            .read_output(ReadOutputInput {
                mission_id: MISSION.into(),
                dispatch_id: "dispatch-1".into(),
                cursor: request_cursor.map(str::to_owned),
            })
            .await
            .unwrap();
        assert_eq!(
            output.has_more, expected_more,
            "limited={limited:?}, cursor={cursor:?}, request={request_cursor:?}, top={top_has_more:?}"
        );
        assert_eq!(output.warning.is_some(), limited == Some(true));
        assert!(output.text.contains("Output page"));
        assert!(!output.text.contains("ignore"));
    }
}
