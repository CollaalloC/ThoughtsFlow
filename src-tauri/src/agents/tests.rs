use super::{
    herdr::{AgentRuntime, RuntimeFailure, RuntimeReply},
    store::AgentStore,
    *,
};
use crate::infrastructure::sqlite::SqliteRepository;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
const MISSION: &str = "a6510d2d-61b1-4ced-aec9-c87715b19244";
const OP: &str = "b6510d2d-61b1-4ced-aec9-c87715b19244";
const SECOND: &str = "c6510d2d-61b1-4ced-aec9-c87715b19244";
#[derive(Default)]
struct FakeCore {
    calls: Mutex<Vec<(String, Value)>>,
    terminal: Mutex<Value>,
    overrides: Mutex<HashMap<String, VecDeque<Result<Value, RuntimeFailure>>>>,
    core: Mutex<String>,
    store: Option<AgentStore>,
}
impl FakeCore {
    fn count(&self, m: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.0 == m)
            .count()
    }
    fn next(&self, m: &str, r: Result<Value, RuntimeFailure>) {
        self.overrides
            .lock()
            .unwrap()
            .entry(m.into())
            .or_default()
            .push_back(r);
    }
}
#[async_trait]
impl AgentRuntime for FakeCore {
    fn available(&self) -> bool {
        true
    }
    fn connected(&self) -> bool {
        true
    }
    async fn version(&self) -> Option<String> {
        Some("0.1.17-fixture".into())
    }
    async fn connect(&self) -> Result<RuntimeReply, RuntimeFailure> {
        self.call("hello", json!({}), None).await
    }
    async fn call(
        &self,
        method: &str,
        params: Value,
        expected: Option<&str>,
    ) -> Result<RuntimeReply, RuntimeFailure> {
        let core = self.core.lock().unwrap().clone();
        if expected.is_some_and(|x| x != core) {
            return Err(RuntimeFailure {
                message: "core changed".into(),
                raw: Value::Null,
                ambiguous: false,
            });
        }
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        let injected = self
            .overrides
            .lock()
            .unwrap()
            .get_mut(method)
            .and_then(VecDeque::pop_front);
        let result = if let Some(r) = injected {
            r?
        } else {
            match method {
                "hello" => json!({"generation":1,"version":"0.1.17","core":{"id":core}}),
                "templates.list" => {
                    json!({"templates":[{"name":"reviewer","description":"existing harness"},{"name":"broken","error":"invalid"}]})
                }
                "terminal.run" => {
                    let op = params
                        .pointer("/labels/thoughtsflow.operation")
                        .unwrap()
                        .as_str()
                        .unwrap();
                    assert_eq!(
                        self.store
                            .as_ref()
                            .unwrap()
                            .operation(op)
                            .await
                            .unwrap()
                            .unwrap()
                            .0
                            .status,
                        "pending"
                    );
                    let handoff = self
                        .store
                        .as_ref()
                        .unwrap()
                        .get_handoff(MISSION, op)
                        .await
                        .unwrap();
                    assert_eq!(params["argv"][1], handoff.preview.text);
                    assert_eq!(params["no_parent"], true);
                    assert_eq!(params["keep"], false);
                    assert!(params.get("line").is_none());
                    let t = json!({"address":"t1","created_ms":123,"cwd":params["cwd"],"template":params["argv"][0],"labels":params["labels"],"keep":false,"state":"live","status":{"state":"idle"},"turn":0,"first_message":{"sent":true}});
                    *self.terminal.lock().unwrap() = t.clone();
                    t
                }
                "terminal.get" => self.terminal.lock().unwrap().clone(),
                "terminal.list" => {
                    let t = self.terminal.lock().unwrap().clone();
                    json!({"terminals":if t.is_null(){vec![]}else{vec![t]}})
                }
                "terminal.read" => {
                    assert_eq!(params["lines"], 0);
                    json!({"text":"immutable screen observation","complete":false,"how":"screen","why":"working"})
                }
                "terminal.send" => {
                    assert_eq!(params["force"], false);
                    assert_eq!(params["if"], "idle");
                    json!({"turn":0,"status":"idle","started":false})
                }
                "terminal.rm" => json!({"removed":params["terminal"]}),
                _ => panic!("unexpected {method}"),
            }
        };
        Ok(RuntimeReply {
            core_id: core,
            result: result.clone(),
            raw: json!({"jsonrpc":"2.0","id":2,"result":result}),
        })
    }
}
async fn setup() -> (SqliteRepository, AgentStore, Arc<FakeCore>, AgentService) {
    let repo = SqliteRepository::connect_in_memory().await.unwrap();
    let mut c = repo.acquire_test_connection().await.unwrap();
    sqlx::query("INSERT INTO workspace(id,title,goal,created_at,updated_at) VALUES ('workspace-1','Agents','goal',1,1)").execute(&mut *c).await.unwrap();
    drop(c);
    let store = repo.agent_store();
    let runtime = Arc::new(FakeCore {
        core: Mutex::new("core-1".into()),
        store: Some(store.clone()),
        ..Default::default()
    });
    let service = AgentService::with_runtime(store.clone(), runtime.clone())
        .await
        .unwrap();
    service
        .create_mission(CreateMissionInput {
            id: MISSION.into(),
            workspace_id: "workspace-1".into(),
            repository_id: std::env::temp_dir().to_string_lossy().into_owned(),
            objective: "Review scope".into(),
        })
        .await
        .unwrap();
    (repo, store, runtime, service)
}
fn preview_input() -> PreviewAgentHandoffInput {
    PreviewAgentHandoffInput {
        mission_id: MISSION.into(),
        title: "Review".into(),
        spec: "Preserve $(echo literal) and $HOME; do not execute shell text".into(),
        requested_model: None,
        requested_template: Some("reviewer".into()),
        context: AgentContextSelection {
            include_active_context: false,
            evidence_ids: vec![],
        },
    }
}
async fn task(s: &AgentService) -> StartTaskInput {
    let i = preview_input();
    let p = s.preview_handoff(i.clone()).await.unwrap();
    StartTaskInput {
        operation_id: OP.into(),
        mission_id: i.mission_id,
        title: i.title,
        spec: i.spec,
        requested_model: None,
        requested_template: i.requested_template,
        handoff: Some(AgentHandoffConfirmation {
            context: i.context,
            preview_hash: p.hash,
            expected_cursor_version: p.cursor_version,
            expected_draft_version: p.draft_version,
            expected_branch_id: p.branch_id,
            expected_branch_version: p.branch_version,
        }),
    }
}
#[tokio::test]
async fn native_launch_persists_handoff_and_exactly_replays_offline() {
    let (_, store, r, s) = setup().await;
    let i = task(&s).await;
    assert_eq!(s.start_task(i.clone()).await.unwrap().status, "succeeded");
    let calls = r.calls.lock().unwrap().len();
    *r.core.lock().unwrap() = "offline".into();
    assert_eq!(s.start_task(i).await.unwrap().status, "succeeded");
    assert_eq!(r.calls.lock().unwrap().len(), calls);
    assert_eq!(r.count("terminal.run"), 1);
    assert_eq!(store.executions(MISSION).await.unwrap().len(), 1);
}
#[tokio::test]
async fn ambiguous_launch_never_retries_and_keeps_receipt() {
    let (_, store, r, s) = setup().await;
    let i = task(&s).await;
    let raw = json!({"error":{"message":"timeout","data":{"kind":"unavailable"}}});
    r.next(
        "terminal.run",
        Err(RuntimeFailure {
            message: "uncertain".into(),
            raw: raw.clone(),
            ambiguous: true,
        }),
    );
    assert_eq!(s.start_task(i.clone()).await.unwrap().status, "unknown");
    assert_eq!(store.operation(OP).await.unwrap().unwrap().0.receipt, raw);
    assert_eq!(s.start_task(i.clone()).await.unwrap().status, "unknown");
    let mut next = i;
    next.operation_id = SECOND.into();
    assert!(s.start_task(next).await.is_err());
    assert_eq!(r.count("terminal.run"), 1);
}
#[tokio::test]
async fn delivery_and_identity_need_positive_proof() {
    for (key, value) in [
        ("/first_message", json!({"sent":false})),
        ("/first_message", Value::Null),
        ("/labels", json!({})),
        ("/template", json!("other")),
        ("/keep", json!(true)),
    ] {
        let (_, store, r, s) = setup().await;
        let i = task(&s).await;
        let m = store.mission(MISSION).await.unwrap().unwrap();
        let mut t = json!({"address":"t1","created_ms":123,"cwd":m.repository_path,"template":"reviewer","labels":{"thoughtsflow.mission":MISSION,"thoughtsflow.operation":OP},"keep":false,"state":"live","first_message":{"sent":true}});
        *t.pointer_mut(key).unwrap() = value;
        r.next("terminal.run", Ok(t.clone()));
        let op = s.start_task(i).await.unwrap();
        assert_eq!(op.status, "unknown", "{key}");
        assert_eq!(op.receipt["result"], t);
    }
}
#[tokio::test]
async fn preview_template_context_changes_and_old_model_override_refuse_before_launch() {
    let (_, _, r, s) = setup().await;
    let mut i = task(&s).await;
    i.requested_template = Some("different".into());
    assert!(s.start_task(i).await.is_err());
    let mut i = task(&s).await;
    i.spec.push_str(" changed");
    assert_eq!(
        s.start_task(i).await.unwrap_err().code,
        "agent_handoff_stale"
    );
    let mut i = task(&s).await;
    i.requested_model = Some("@smol".into());
    assert_eq!(
        s.start_task(i).await.unwrap_err().code,
        "agent_model_unsupported"
    );
    assert_eq!(r.count("terminal.run"), 0);
}
#[tokio::test]
async fn evidence_is_scoped_passive_immutable_and_reusable_offline() {
    let (_, store, r, s) = setup().await;
    s.start_task(task(&s).await).await.unwrap();
    let input = CaptureAgentEvidenceInput {
        id: SECOND.into(),
        mission_id: MISSION.into(),
        dispatch_id: OP.into(),
        cursor: None,
    };
    let evidence = s.capture_evidence(input.clone()).await.unwrap();
    assert_eq!(evidence.source.as_deref(), Some("herdr-screen"));
    assert!(!evidence.has_more);
    assert!(evidence.warning.is_some());
    assert_eq!(
        evidence.content_hash,
        crate::domain::sha256_hex(evidence.text.as_bytes())
    );
    let mut selected = preview_input();
    selected.context.evidence_ids.push(SECOND.into());
    assert!(
        s.preview_handoff(selected)
            .await
            .unwrap()
            .text
            .contains(&evidence.text)
    );
    let calls = r.calls.lock().unwrap().len();
    *r.core.lock().unwrap() = "offline".into();
    assert_eq!(
        s.capture_evidence(input).await.unwrap().content_hash,
        evidence.content_hash
    );
    assert_eq!(r.calls.lock().unwrap().len(), calls);
    assert!(store.get_evidence("other", SECOND).await.is_err());
    assert_eq!(
        store
            .evidence_page("workspace-1", None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
}
#[tokio::test]
async fn changed_core_terminal_or_scope_blocks_reads_and_controls() {
    let (_, _, r, s) = setup().await;
    s.start_task(task(&s).await).await.unwrap();
    r.terminal.lock().unwrap()["created_ms"] = json!(999);
    assert!(
        s.read_output(ReadOutputInput {
            mission_id: MISSION.into(),
            dispatch_id: OP.into(),
            cursor: None
        })
        .await
        .is_err()
    );
    assert_eq!(r.count("terminal.read"), 0);
    *r.core.lock().unwrap() = "other-core".into();
    assert!(!s.snapshot(MISSION).await.unwrap().connected);
    assert!(
        s.release(ReleaseWorkerInput {
            operation_id: SECOND.into(),
            mission_id: MISSION.into(),
            dispatch_id: OP.into()
        })
        .await
        .is_err()
    );
    assert_eq!(r.count("terminal.rm"), 0);
}
#[tokio::test]
async fn reported_done_and_exit_do_not_imply_task_success_and_only_finished_can_remove() {
    let (_, _, r, s) = setup().await;
    s.start_task(task(&s).await).await.unwrap();
    r.terminal.lock().unwrap()["status"] = json!({"state":"done"});
    let snap = s.snapshot(MISSION).await.unwrap();
    assert_eq!(snap.tasks[0].status, "reported_done");
    assert!(!snap.tasks[0].can_release);
    let input = ReleaseWorkerInput {
        operation_id: SECOND.into(),
        mission_id: MISSION.into(),
        dispatch_id: OP.into(),
    };
    assert!(s.release(input.clone()).await.is_err());
    r.terminal.lock().unwrap()["state"] = json!("finished");
    let snap = s.snapshot(MISSION).await.unwrap();
    assert_eq!(snap.tasks[0].status, "finished_unverified");
    assert!(snap.tasks[0].can_release);
    assert_eq!(s.release(input.clone()).await.unwrap().status, "succeeded");
    assert_eq!(s.release(input).await.unwrap().status, "succeeded");
    assert_eq!(r.count("terminal.rm"), 1);
}
#[tokio::test]
async fn blocked_approvals_and_changed_turn_are_never_sent_forcefully() {
    let (_, _, r, s) = setup().await;
    s.start_task(task(&s).await).await.unwrap();
    let input = ReplyInput {
        operation_id: SECOND.into(),
        mission_id: MISSION.into(),
        message_id: format!("{OP}:0"),
        body: "guidance".into(),
    };
    r.terminal.lock().unwrap()["status"] = json!({"state":"blocked","kind":"permission"});
    assert!(s.reply(input.clone()).await.is_err());
    assert_eq!(r.count("terminal.send"), 0);
    r.terminal.lock().unwrap()["status"] = json!({"state":"idle"});
    assert_eq!(s.reply(input.clone()).await.unwrap().status, "succeeded");
    assert_eq!(s.reply(input).await.unwrap().status, "succeeded");
    assert_eq!(r.count("terminal.send"), 1);
}
#[tokio::test]
async fn legacy_mission_remains_read_only_and_legacy_payload_has_no_new_fields() {
    let (repo, _, r, s) = setup().await;
    let mut c = repo.acquire_test_connection().await.unwrap();
    sqlx::query(
        "UPDATE agent_mission SET body_json=json_remove(body_json,'$.runtimeBackend') WHERE id=?",
    )
    .bind(MISSION)
    .execute(&mut *c)
    .await
    .unwrap();
    let before: String = sqlx::query_scalar("SELECT body_json FROM agent_mission WHERE id=?")
        .bind(MISSION)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    drop(c);
    let calls = r.calls.lock().unwrap().len();
    let snap = s.snapshot(MISSION).await.unwrap();
    assert!(!snap.can_start_tasks);
    assert!(!snap.connected);
    assert_eq!(snap.mission.runtime_backend, "orca");
    assert!(s.preview_handoff(preview_input()).await.is_err());
    assert_eq!(r.calls.lock().unwrap().len(), calls);
    let mut c = repo.acquire_test_connection().await.unwrap();
    let after: String = sqlx::query_scalar("SELECT body_json FROM agent_mission WHERE id=?")
        .bind(MISSION)
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(before, after);
    let legacy: StartTaskInput = serde_json::from_value(
        json!({"operationId":OP,"missionId":MISSION,"title":"old","spec":"old"}),
    )
    .unwrap();
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("requestedTemplate")
            .is_none()
    );
}
#[tokio::test]
async fn transaction_failure_prevents_launch_and_startup_recovers_pending() {
    let (repo, store, r, s) = setup().await;
    let input = task(&s).await;
    let mut c = repo.acquire_test_connection().await.unwrap();
    sqlx::query("CREATE TRIGGER reject_handoff BEFORE INSERT ON agent_handoff BEGIN SELECT RAISE(ABORT,'fixture'); END").execute(&mut *c).await.unwrap();
    drop(c);
    assert!(s.start_task(input).await.is_err());
    assert_eq!(r.count("terminal.run"), 0);
    assert!(store.operation(OP).await.unwrap().is_none());
    let pending = store
        .begin(OP, MISSION, "start-task", &json!({}))
        .await
        .unwrap();
    assert_eq!(pending.status, "pending");
    let _ = AgentService::with_runtime(store.clone(), r.clone())
        .await
        .unwrap();
    assert_eq!(
        store.operation(OP).await.unwrap().unwrap().0.status,
        "unknown"
    );
    assert_eq!(r.count("terminal.run"), 0);
}

#[tokio::test]
async fn binding_commit_failure_preserves_raw_receipt_without_redispatch() {
    let (repo, store, r, s) = setup().await;
    let input = task(&s).await;
    let mut c = repo.acquire_test_connection().await.unwrap();
    sqlx::query("CREATE TRIGGER reject_binding BEFORE INSERT ON agent_execution BEGIN SELECT RAISE(ABORT,'injected'); END").execute(&mut *c).await.unwrap();
    drop(c);
    assert!(s.start_task(input.clone()).await.is_err());
    let op = store.operation(OP).await.unwrap().unwrap().0;
    assert_eq!(op.status, "pending");
    assert_eq!(op.receipt["result"]["first_message"]["sent"], true);
    assert!(store.executions(MISSION).await.unwrap().is_empty());
    let restored = AgentService::with_runtime(store, r.clone()).await.unwrap();
    assert_eq!(restored.start_task(input).await.unwrap().status, "unknown");
    assert_eq!(r.count("terminal.run"), 1);
}
