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
                json!({"source":"terminal","terminal":{"tail":["hello from worker"],"limited":false},"cursor":"opaque:cursor"})
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

fn task_input() -> StartTaskInput {
    StartTaskInput {
        operation_id: OP.into(),
        mission_id: MISSION.into(),
        title: "Implement feature".into(),
        spec: "Keep `$HOME` and $(echo literal-only) literal; verify output".into(),
    }
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
    let input = task_input();
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
    let input = task_input();
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
    let input = task_input();
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
    assert!(service.start_task(task_input()).await.is_err());
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
    let mut task = task_input();
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
    assert!(service.start_task(task_input()).await.is_err());
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
            "orchestration worker-start" => service.start_task(task_input()).await.unwrap(),
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
        async move { service.start_task(task_input()).await }
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
            service.start_task(task_input()).await
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
            "orchestration worker-start" => service.start_task(task_input()).await.unwrap(),
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
            service.start_task(task_input()).await
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
