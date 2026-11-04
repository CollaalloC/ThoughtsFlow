//! Application operations and immutable evidence, independent of any Agent harness.
use super::{
    herdr::{AgentRuntime, HerdrCore, RuntimeFailure},
    store::AgentStore,
    *,
};
use crate::{
    application::{AppError, AppResult},
    infrastructure::sqlite::SqliteRepository,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, Weak},
};
use tokio::sync::Mutex;

pub struct AgentService {
    store: AgentStore,
    runtime: Arc<dyn AgentRuntime>,
    control: StdMutex<HashMap<String, Weak<Mutex<()>>>>,
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::validation("agent_control", message)
}
fn runtime_error(e: RuntimeFailure) -> AppError {
    invalid(e.message).with_details(e.raw)
}
fn text(v: &Value, key: &str) -> Option<String> {
    v.pointer(key).and_then(Value::as_str).map(str::to_owned)
}
fn bounded(label: &str, value: &str, max: usize) -> AppResult<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(|c| c == '\0') {
        return Err(invalid(format!("{label}不能为空或超过 {max} 字节。")));
    }
    Ok(())
}
fn uuid(value: &str) -> AppResult<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid("操作标识必须为 UUID。"))
}
fn active(m: &AgentMission) -> AppResult<()> {
    if m.runtime_backend != "herdr-core" {
        return Err(invalid(
            "这是旧 Orca 协作，只能查看历史；请新建 herdr-core 协作。",
        ));
    }
    Ok(())
}
fn template(input: Option<&str>) -> AppResult<&str> {
    input
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && !s.starts_with('.')
                && s.chars().all(|c| !c.is_control() && c != '/' && c != '\\')
        })
        .ok_or_else(|| AppError::validation("agent_template", "请选择已有 herdr-core 模板。"))
}
fn status(t: &Value) -> String {
    if t["state"] == "finished" {
        return "finished_unverified".into();
    }
    match t.pointer("/status/state").and_then(Value::as_str) {
        Some("done") => "reported_done",
        Some("working") => "running",
        Some("blocked") => "blocked",
        Some("idle") => "idle",
        Some("error") => "reported_error",
        _ => "unverifiable",
    }
    .into()
}
fn same_terminal(t: &Value, e: &AgentExecution) -> bool {
    t["address"].as_str() == Some(&e.terminal_address)
        && t["created_ms"].as_u64() == Some(e.terminal_created_ms)
        && t["cwd"].as_str() == Some(&e.cwd)
        && t["template"].as_str() == Some(&e.template)
        && t.pointer("/labels/thoughtsflow.mission")
            .and_then(Value::as_str)
            == Some(&e.mission_id)
        && t.pointer("/labels/thoughtsflow.operation")
            .and_then(Value::as_str)
            == Some(&e.operation_id)
        && t["keep"].as_bool() == Some(false)
}
impl AgentService {
    pub async fn new(repository: &SqliteRepository) -> AppResult<Self> {
        Self::with_runtime(repository.agent_store(), Arc::new(HerdrCore::discover())).await
    }
    pub(crate) async fn with_runtime(
        store: AgentStore,
        runtime: Arc<dyn AgentRuntime>,
    ) -> AppResult<Self> {
        store.recover().await?;
        Ok(Self {
            store,
            runtime,
            control: StdMutex::new(HashMap::new()),
        })
    }
    fn mission_lock(&self, id: &str) -> AppResult<Arc<Mutex<()>>> {
        let mut locks = self.control.lock().map_err(|_| invalid("协作锁不可用"))?;
        locks.retain(|_, v| v.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(id.into(), Arc::downgrade(&lock));
        Ok(lock)
    }
    async fn mission(&self, id: &str) -> AppResult<AgentMission> {
        self.store
            .mission(id)
            .await?
            .ok_or_else(|| invalid("协作不存在。"))
    }
    async fn templates(&self, core: Option<&str>) -> AppResult<Vec<AgentTemplate>> {
        let r = self
            .runtime
            .call("templates.list", json!({}), core)
            .await
            .map_err(runtime_error)?;
        let list = r.result["templates"]
            .as_array()
            .ok_or_else(|| invalid("herdr-core 模板目录格式不受支持。"))?;
        Ok(list
            .iter()
            .filter(|t| t.get("error").is_none())
            .filter_map(|t| {
                Some(AgentTemplate {
                    name: t["name"].as_str()?.into(),
                    description: t["description"].as_str().unwrap_or("").into(),
                })
            })
            .take(200)
            .collect())
    }
    pub async fn environment(&self) -> AppResult<AgentEnvironment> {
        let mut env = AgentEnvironment {
            available: self.runtime.available(),
            running: false,
            runtime_version: self.runtime.version().await,
            templates: vec![],
            runtime_id: None,
            projects: vec![],
            message: Some("连接后使用 ThoughtsFlow 专用 core 会话；继承本机 Herdr 模板。".into()),
        };
        if !self.runtime.connected() {
            return Ok(env);
        }
        match self.runtime.call("hello", json!({}), None).await {
            Ok(h) => {
                env.runtime_version = text(&h.result, "/version");
                env.runtime_id = Some(h.core_id.clone());
                env.templates = self.templates(Some(&h.core_id)).await?;
                env.running = true;
                env.message = Some("模型、认证、工具和审批由所选原生模板管理。".into());
            }
            Err(e) => env.message = Some(e.message),
        }
        Ok(env)
    }
    pub async fn open_runtime(&self) -> AppResult<AgentEnvironment> {
        self.runtime.connect().await.map_err(runtime_error)?;
        self.environment().await
    }
    pub async fn list_missions(&self, id: &str) -> AppResult<Vec<AgentMission>> {
        self.store.missions(id).await
    }
    pub async fn create_mission(&self, input: CreateMissionInput) -> AppResult<AgentMission> {
        uuid(&input.id)?;
        bounded("协作目标", &input.objective, 32_000)?;
        let _guard = self.mission_lock(&input.id)?.lock_owned().await;
        if let Some(m) = self.store.mission(&input.id).await? {
            if m.workspace_id == input.workspace_id
                && m.repository_id == input.repository_id
                && m.objective == input.objective
            {
                return Ok(m);
            }
            return Err(invalid("协作 ID 已用于其他输入。"));
        }
        self.store
            .repository
            .get_workspace(&input.workspace_id)
            .await
            .map_err(|e| invalid(e.to_string()))?;
        let path = std::path::Path::new(&input.repository_id);
        if !path.is_absolute() || !path.is_dir() {
            return Err(invalid("请选择存在的绝对工作目录。"));
        }
        let cwd = std::fs::canonicalize(path)
            .map_err(|e| invalid(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        let hello = self
            .runtime
            .call("hello", json!({}), None)
            .await
            .map_err(runtime_error)?;
        let m = AgentMission {
            runtime_backend: "herdr-core".into(),
            id: input.id.clone(),
            workspace_id: input.workspace_id,
            repository_id: input.repository_id,
            repository_path: cwd,
            objective: input.objective,
            run_id: Some(input.id),
            coordinator_handle: None,
            runtime_id: Some(hello.core_id),
            status: "ready".into(),
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.store.create_mission(&m).await?;
        Ok(m)
    }
    async fn replay(
        &self,
        id: &str,
        mission: &str,
        kind: &str,
        request: &Value,
    ) -> AppResult<Option<AgentOperation>> {
        if let Some((mut op, original)) = self.store.operation(id).await? {
            if op.mission_id != mission || op.kind != kind || original != *request {
                return Err(invalid("此操作 ID 已用于不同请求。"));
            }
            if op.status == "pending" {
                op.status = "unknown".into();
                op.error = Some("未取得最终回执；不会自动重发。".into());
                self.store.save_operation(&op).await?;
            }
            return Ok(Some(op));
        }
        Ok(None)
    }
    async fn require_clear(&self, m: &AgentMission) -> AppResult<()> {
        active(m)?;
        if self
            .store
            .operations(&m.id)
            .await?
            .iter()
            .any(|o| matches!(o.status.as_str(), "pending" | "unknown"))
        {
            return Err(invalid(
                "有操作结果未确认，请核对 herdr-core；不会再次派发。",
            ));
        }
        Ok(())
    }
    pub async fn preview_handoff(
        &self,
        input: PreviewAgentHandoffInput,
    ) -> AppResult<AgentHandoffPreview> {
        bounded("任务标题", &input.title, 200)?;
        bounded("任务说明", &input.spec, 64_000)?;
        let m = self.mission(&input.mission_id).await?;
        active(&m)?;
        template(input.requested_template.as_deref())?;
        if input.requested_model.is_some() {
            return Err(AppError::validation(
                "agent_model_unsupported",
                "模型由 Herdr 模板配置，不能沿用旧 Orca 模型覆盖参数。",
            ));
        }
        Ok(self.store.prepare_handoff(&m, &input).await?.preview)
    }
    pub async fn start_task(&self, input: StartTaskInput) -> AppResult<AgentOperation> {
        uuid(&input.operation_id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("input");
        if let Some(op) = self
            .replay(
                &input.operation_id,
                &input.mission_id,
                "start-task",
                &request,
            )
            .await?
        {
            return Ok(op);
        }
        bounded("任务标题", &input.title, 200)?;
        bounded("任务说明", &input.spec, 64_000)?;
        let m = self.mission(&input.mission_id).await?;
        self.require_clear(&m).await?;
        let selected = template(input.requested_template.as_deref())?;
        if input.requested_model.is_some() {
            return Err(AppError::validation(
                "agent_model_unsupported",
                "模型沿用 Herdr 模板；尚未派发。",
            ));
        }
        let confirm = input
            .handoff
            .as_ref()
            .ok_or_else(|| AppError::validation("agent_handoff_required", "请先预览交接内容。"))?;
        if !self
            .templates(m.runtime_id.as_deref())
            .await?
            .iter()
            .any(|t| t.name == selected)
        {
            return Err(AppError::validation(
                "agent_template",
                "模板已移除或无效；尚未派发。",
            ));
        }
        let prepared = self
            .store
            .prepare_handoff(
                &m,
                &PreviewAgentHandoffInput {
                    mission_id: m.id.clone(),
                    title: input.title.clone(),
                    spec: input.spec.clone(),
                    requested_model: None,
                    requested_template: input.requested_template.clone(),
                    context: confirm.context.clone(),
                },
            )
            .await?;
        let p = &prepared.preview;
        if p.hash != confirm.preview_hash
            || p.cursor_version != confirm.expected_cursor_version
            || p.draft_version != confirm.expected_draft_version
            || p.branch_id != confirm.expected_branch_id
            || p.branch_version != confirm.expected_branch_version
        {
            return Err(super::context::stale());
        }
        let body = p.text.clone();
        let mut op = self
            .store
            .begin_handoff(&input.operation_id, &m, &request, prepared)
            .await?;
        let mut execution = None;
        match self.runtime.call("terminal.run",json!({"argv":[selected,body],"cwd":m.repository_path,"name":format!("tf-{}",&input.operation_id.replace('-',"")[..28]),"no_parent":true,"no_notify":true,"keep":false,"rm":false,"wait":false,"labels":{"thoughtsflow.mission":m.id,"thoughtsflow.operation":op.id}}),m.runtime_id.as_deref()).await {
            Ok(reply)=>{
                op.receipt=reply.raw;
                let t=&reply.result;
                let candidate=AgentExecution{operation_id:op.id.clone(),mission_id:m.id.clone(),core_id:reply.core_id,terminal_address:text(t,"/address").unwrap_or_default(),terminal_created_ms:t["created_ms"].as_u64().unwrap_or(0),template:selected.into(),cwd:m.repository_path.clone(),title:input.title,spec:input.spec};
                let valid=!candidate.terminal_address.is_empty() && candidate.terminal_created_ms>0 && same_terminal(t,&candidate) && m.runtime_id.as_deref()==Some(&candidate.core_id);
                if valid{execution=Some(candidate);}
                if valid && t.pointer("/first_message/sent").and_then(Value::as_bool)==Some(true){op.status="succeeded".into();}
                else{op.status="unknown".into();op.error=Some("终端可能已启动，但未确认任务送达或身份。请检查回执与原生终端；不会自动补发。".into());}
            },
            Err(e)=>{op.status=if e.ambiguous{"unknown"}else{"failed"}.into();op.error=Some(e.message);op.receipt=e.raw;}
        }
        self.store.record_receipt(&op).await?;
        self.store.finish_execution(&op, execution.as_ref()).await?;
        Ok(op)
    }
    async fn execution(&self, mission: &str, id: &str) -> AppResult<AgentExecution> {
        self.store
            .execution(mission, id)
            .await?
            .ok_or_else(|| invalid("此执行不属于当前协作。"))
    }
    async fn terminal(&self, e: &AgentExecution) -> AppResult<Value> {
        let r = self
            .runtime
            .call(
                "terminal.get",
                json!({"terminal":e.terminal_address}),
                Some(&e.core_id),
            )
            .await
            .map_err(runtime_error)?;
        if !same_terminal(&r.result, e) {
            return Err(invalid("终端身份或恢复状态改变；拒绝操作。"));
        }
        Ok(r.result)
    }
    pub async fn snapshot(&self, id: &str) -> AppResult<AgentSnapshot> {
        let m = self.mission(id).await?;
        let mut snapshot = AgentSnapshot {
            mission: m.clone(),
            tasks: vec![],
            messages: vec![],
            connected: false,
            can_start_tasks: false,
            warning: None,
        };
        if m.runtime_backend != "herdr-core" {
            snapshot.warning = Some(
                "旧 Orca 协作已归档；交接、回执和证据仍可离线查看。请新建协作使用 herdr-core。"
                    .into(),
            );
            return Ok(snapshot);
        }
        let executions = self.store.executions(id).await?;
        let ops = self.store.operations(id).await?;
        match self
            .runtime
            .call(
                "terminal.list",
                json!({"all":true,"labels":{"thoughtsflow.mission":id}}),
                m.runtime_id.as_deref(),
            )
            .await
        {
            Ok(r) => {
                let terminals = r.result["terminals"]
                    .as_array()
                    .ok_or_else(|| invalid("herdr-core 终端清单格式无效。"))?;
                snapshot.connected = true;
                snapshot.can_start_tasks = !ops
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "unknown" | "pending"));
                if !snapshot.can_start_tasks {
                    snapshot.warning = Some("有操作结果未确认，新增派发已暂停。".into());
                }
                for e in executions {
                    let candidates: Vec<_> =
                        terminals.iter().filter(|t| same_terminal(t, &e)).collect();
                    let terminal = (candidates.len() == 1).then(|| candidates[0]);
                    let released = ops.iter().any(|o| {
                        o.kind == "release"
                            && o.status == "succeeded"
                            && o.receipt
                                .pointer("/thoughtsflow/executionId")
                                .and_then(Value::as_str)
                                == Some(&e.operation_id)
                    });
                    let state = terminal.map(status).unwrap_or_else(|| {
                        if released { "released" } else { "unverifiable" }.into()
                    });
                    let safe = !ops.iter().any(|o| {
                        o.kind == "release" && matches!(o.status.as_str(), "pending" | "unknown")
                    });
                    snapshot.tasks.push(AgentTask {
                        id: e.operation_id.clone(),
                        title: e.title.clone(),
                        spec: e.spec.clone(),
                        status: state,
                        dispatch_id: Some(e.operation_id.clone()),
                        terminal_state: terminal.and_then(|t| text(t, "/state")),
                        liveness: terminal.map(|_| "observed".into()),
                        attention: terminal.and_then(|t| text(t, "/status/kind")),
                        can_release: safe && terminal.is_some_and(|t| t["state"] == "finished"),
                    });
                    if let Some(t) = terminal {
                        if t.pointer("/status/state").and_then(Value::as_str) == Some("idle") {
                            let turn = t["turn"].as_u64().unwrap_or(0);
                            let message_id = format!("{}:{turn}", e.operation_id);
                            let uncertain = ops.iter().any(|o| {
                                o.kind == "reply"
                                    && matches!(o.status.as_str(), "pending" | "unknown")
                            });
                            snapshot.messages.push(AgentMessage {
                                id: message_id,
                                r#type: "guidance".into(),
                                body:
                                    "Agent 报告空闲，可明确发送下一条指导；这不代表任务已通过验收。"
                                        .into(),
                                task_id: Some(e.operation_id.clone()),
                                dispatch_id: Some(e.operation_id),
                                requires_reply: !uncertain,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                snapshot.warning = Some(e.message);
            }
        }
        Ok(snapshot)
    }
    async fn control_operation(
        &self,
        id: &str,
        kind: &str,
        request: Value,
        method: &str,
        params: Value,
        e: &AgentExecution,
    ) -> AppResult<AgentOperation> {
        let mut op = self.store.begin(id, &e.mission_id, kind, &request).await?;
        match self.runtime.call(method, params, Some(&e.core_id)).await {
            Ok(r) => {
                op.receipt = json!({"thoughtsflow":{"executionId":e.operation_id,"coreId":e.core_id,"terminal":e.terminal_address},"rpc":r.raw});
                let valid = if kind == "release" {
                    r.result["removed"].as_str() == Some(&e.terminal_address)
                } else {
                    r.result["turn"].as_u64().is_some()
                };
                op.status = if valid { "succeeded" } else { "unknown" }.into();
                if !valid {
                    op.error = Some("回执缺少操作确认；不会自动重发。".into());
                }
            }
            Err(err) => {
                op.status = if err.ambiguous { "unknown" } else { "failed" }.into();
                op.receipt = err.raw;
                op.error = Some(err.message);
            }
        }
        self.store.record_receipt(&op).await?;
        self.store.finish_operation(&op).await?;
        Ok(op)
    }
    pub async fn reply(&self, input: ReplyInput) -> AppResult<AgentOperation> {
        uuid(&input.operation_id)?;
        bounded("指导", &input.body, 32_000)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("input");
        if let Some(op) = self
            .replay(&input.operation_id, &input.mission_id, "reply", &request)
            .await?
        {
            return Ok(op);
        }
        let m = self.mission(&input.mission_id).await?;
        self.require_clear(&m).await?;
        let (id, turn) = input
            .message_id
            .rsplit_once(':')
            .ok_or_else(|| invalid("指导目标格式无效"))?;
        let e = self.execution(&m.id, id).await?;
        let t = self.terminal(&e).await?;
        if t["state"] != "live"
            || t.pointer("/status/state").and_then(Value::as_str) != Some("idle")
            || t["turn"].as_u64().map(|x| x.to_string()).as_deref() != Some(turn)
        {
            return Err(invalid(
                "Agent 已非该空闲轮次，请刷新。审批在原生终端处理。",
            ));
        }
        self.control_operation(&input.operation_id,"reply",request,"terminal.send",json!({"terminal":e.terminal_address,"text":input.body,"if":"idle","force":false,"wait":false}),&e).await
    }
    pub async fn release(&self, input: ReleaseWorkerInput) -> AppResult<AgentOperation> {
        uuid(&input.operation_id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("input");
        if let Some(op) = self
            .replay(&input.operation_id, &input.mission_id, "release", &request)
            .await?
        {
            return Ok(op);
        }
        let m = self.mission(&input.mission_id).await?;
        self.require_clear(&m).await?;
        let e = self.execution(&m.id, &input.dispatch_id).await?;
        if self.terminal(&e).await?["state"] != "finished" {
            return Err(invalid("只能移除已退出的本工作台终端。"));
        }
        self.control_operation(
            &input.operation_id,
            "release",
            request,
            "terminal.rm",
            json!({"terminal":e.terminal_address}),
            &e,
        )
        .await
    }
    pub async fn reconnect(&self, input: ReconnectInput) -> AppResult<AgentOperation> {
        uuid(&input.operation_id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("input");
        if let Some(op) = self
            .replay(
                &input.operation_id,
                &input.mission_id,
                "reconnect",
                &request,
            )
            .await?
        {
            return Ok(op);
        }
        let m = self.mission(&input.mission_id).await?;
        active(&m)?;
        // Read-only reconciliation never replaces a core ID or settles an uncertain launch.
        let r = self
            .runtime
            .call("hello", json!({}), m.runtime_id.as_deref())
            .await
            .map_err(runtime_error)?;
        let mut op = self
            .store
            .begin(&input.operation_id, &m.id, "reconnect", &request)
            .await?;
        op.status = "succeeded".into();
        op.receipt = r.raw;
        self.store.record_receipt(&op).await?;
        self.store.finish_operation(&op).await?;
        Ok(op)
    }
    async fn read(
        &self,
        input: &ReadOutputInput,
    ) -> AppResult<(AgentOutput, Value, AgentExecution, String)> {
        if input.cursor.is_some() {
            return Err(invalid(
                "herdr-core 屏幕观察不提供分页游标，请重新读取当前屏幕。",
            ));
        }
        let m = self.mission(&input.mission_id).await?;
        active(&m)?;
        let e = self.execution(&m.id, &input.dispatch_id).await?;
        let before = self.terminal(&e).await?;
        // lines:0 is a passive screen observation. History reads can scroll a TUI.
        let read = self
            .runtime
            .call(
                "terminal.read",
                json!({"terminal":e.terminal_address,"lines":0}),
                Some(&e.core_id),
            )
            .await
            .map_err(runtime_error)?;
        let after = self.terminal(&e).await?;
        if read.result["how"].as_str() != Some("screen")
            || read.result["complete"].as_bool().is_none()
        {
            return Err(invalid("herdr-core 未确认当前屏幕观察格式，未保存证据。"));
        }
        let body = text(&read.result, "/text").ok_or_else(|| invalid("输出缺少文本"))?;
        bounded("输出观察", &body, 256_000)?;
        let warning = Some(if read.result["complete"].as_bool() == Some(true) {
            "当前屏幕已返回；它不代表完整会话或任务验收结果。".into()
        } else {
            "本次屏幕观察可能不完整，请在原生终端核对；不代表任务验收结果。".into()
        });
        let output = AgentOutput {
            text: body,
            cursor: None,
            has_more: false,
            source: Some("herdr-screen".into()),
            warning,
        };
        let raw = json!({"coreId":e.core_id,"terminal":e.terminal_address,"before":before,"read":read.raw,"after":after});
        Ok((output, raw, e, status(&after)))
    }
    pub async fn read_output(&self, input: ReadOutputInput) -> AppResult<AgentOutput> {
        Ok(self.read(&input).await?.0)
    }
    pub async fn capture_evidence(
        &self,
        input: CaptureAgentEvidenceInput,
    ) -> AppResult<AgentEvidence> {
        uuid(&input.id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        if let Some((existing, original)) = self.store.evidence(&input.id).await? {
            if original != input {
                return Err(invalid("证据 ID 已用于其他输入。"));
            }
            return Ok(existing);
        }
        let m = self.mission(&input.mission_id).await?;
        let (output, raw, e, observed) = self
            .read(&ReadOutputInput {
                mission_id: input.mission_id.clone(),
                dispatch_id: input.dispatch_id.clone(),
                cursor: input.cursor.clone(),
            })
            .await?;
        let evidence = AgentEvidence {
            id: input.id.clone(),
            workspace_id: m.workspace_id,
            mission_id: m.id.clone(),
            run_id: m.id,
            task_id: e.operation_id.clone(),
            dispatch_id: e.operation_id.clone(),
            handoff_operation_id: Some(e.operation_id),
            content_hash: crate::domain::sha256_hex(output.text.as_bytes()),
            text: output.text,
            source: output.source,
            request_cursor: None,
            cursor: None,
            has_more: false,
            warning: output.warning,
            observed_task_status: observed,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.store.save_evidence(&input, evidence, &raw).await
    }
    pub async fn operations(&self, id: &str) -> AppResult<Vec<AgentOperation>> {
        self.store.operations(id).await
    }
    pub async fn handoffs(&self, id: &str) -> AppResult<Vec<AgentHandoff>> {
        self.store.handoffs(id).await
    }
    pub async fn evidence(&self, id: &str) -> AppResult<Vec<AgentEvidence>> {
        self.store.evidence_list(id).await
    }
    pub async fn evidence_page(&self, i: AgentEvidencePageInput) -> AppResult<AgentEvidencePage> {
        self.store
            .evidence_page(&i.workspace_id, i.cursor.as_deref())
            .await
    }
    pub async fn handoff_page(&self, i: AgentHandoffPageInput) -> AppResult<AgentHandoffPage> {
        self.store
            .handoff_page(&i.mission_id, i.cursor.as_deref())
            .await
    }
    pub async fn get_evidence(&self, i: GetAgentEvidenceInput) -> AppResult<AgentEvidence> {
        self.store.get_evidence(&i.workspace_id, &i.id).await
    }
    pub async fn get_handoff(&self, i: GetAgentHandoffInput) -> AppResult<AgentHandoff> {
        self.store.get_handoff(&i.mission_id, &i.operation_id).await
    }
}
