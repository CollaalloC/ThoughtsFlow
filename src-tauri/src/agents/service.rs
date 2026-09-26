use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex, Weak},
};

use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{
    orca::{CommandRunner, OrcaCli, RuntimeFailure},
    receipt::{self, ExpectedReceipt},
    store::AgentStore,
    *,
};
use crate::{
    application::{AppError, AppResult},
    infrastructure::sqlite::SqliteRepository,
};

pub struct AgentService {
    store: AgentStore,
    runner: Arc<dyn CommandRunner>,
    control: StdMutex<HashMap<String, Weak<Mutex<()>>>>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::validation("agent_control", message)
}

fn runtime_error(error: RuntimeFailure) -> AppError {
    invalid(error.message).with_details(error.receipt)
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn required(value: &Value, pointer: &str) -> AppResult<String> {
    text(value, pointer)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            invalid(format!(
                "Orca 回执缺少 {pointer}，请检查运行时；不要重复派发。"
            ))
        })
}

fn bounded(label: &str, value: &str, limit: usize) -> AppResult<()> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        return Err(invalid(format!(
            "{label}不能为空，且不能超过 {limit} 字节或包含空字符。"
        )));
    }
    Ok(())
}

fn uuid(label: &str, value: &str) -> AppResult<()> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid(format!("{label}必须是 UUID。")))
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn runtime_id(receipt: &Value) -> Option<String> {
    text(receipt, "/result/runtime/runtimeId")
        .or_else(|| text(receipt, "/_meta/runtimeId"))
        .filter(|id| id != "none")
}

impl AgentService {
    pub async fn new(repository: &SqliteRepository) -> AppResult<Self> {
        Self::with_runner(repository.agent_store(), Arc::new(OrcaCli::discover())).await
    }

    pub(crate) async fn with_runner(
        store: AgentStore,
        runner: Arc<dyn CommandRunner>,
    ) -> AppResult<Self> {
        store.recover().await?;
        Ok(Self {
            store,
            runner,
            control: StdMutex::new(HashMap::new()),
        })
    }

    fn mission_lock(&self, id: &str) -> AppResult<Arc<Mutex<()>>> {
        let mut locks = self
            .control
            .lock()
            .map_err(|_| invalid("Agent 任务锁不可用，请重新打开应用。"))?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(id.to_owned(), Arc::downgrade(&lock));
        Ok(lock)
    }

    async fn call(&self, values: &[&str]) -> AppResult<Value> {
        self.runner
            .execute(&args(values))
            .await
            .map_err(runtime_error)
    }

    async fn live_runtime(&self) -> AppResult<String> {
        let status = self.call(&["status", "--json"]).await?;
        if status
            .pointer("/result/runtime/reachable")
            .and_then(Value::as_bool)
            != Some(true)
            || status
                .pointer("/result/runtime/state")
                .and_then(Value::as_str)
                != Some("ready")
            || status
                .pointer("/result/graph/state")
                .and_then(Value::as_str)
                != Some("ready")
        {
            return Err(invalid(
                "Orca 运行时未就绪，请打开 Orca，待运行时连接后再操作。",
            ));
        }
        let capabilities = status
            .pointer("/result/runtime/capabilities")
            .and_then(Value::as_array);
        if !capabilities.is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("orchestration.contract.v1"))
        }) {
            return Err(invalid(
                "此 Orca 运行时不支持 orchestration.contract.v1，请更新 Orca 后重试。",
            ));
        }
        runtime_id(&status).ok_or_else(|| invalid("Orca 未返回运行时标识，请更新或重新打开 Orca。"))
    }

    async fn projects(&self) -> AppResult<Vec<AgentProject>> {
        let receipt = self.call(&["repo", "list", "--json"]).await?;
        let repos = receipt
            .pointer("/result/repos")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Orca 仓库清单格式不受支持。"))?;
        repos
            .iter()
            .map(|repo| {
                Ok(AgentProject {
                    id: required(repo, "/id")?,
                    name: text(repo, "/name")
                        .or_else(|| text(repo, "/displayName"))
                        .unwrap_or_else(|| "Repository".into()),
                    path: required(repo, "/path")?,
                })
            })
            .collect()
    }

    pub async fn environment(&self) -> AppResult<AgentEnvironment> {
        let (orca_version, omp_version) = self.runner.versions().await;
        let mut environment = AgentEnvironment {
            available: self.runner.available(),
            running: false,
            orca_version,
            omp_version,
            runtime_id: None,
            projects: vec![],
            message: None,
        };
        let result = async {
            environment.runtime_id = Some(self.live_runtime().await?);
            environment.running = true;
            environment.projects = self.projects().await?;
            Ok::<_, AppError>(())
        }
        .await;
        if let Err(error) = result {
            environment.message = Some(error.message);
        }
        Ok(environment)
    }

    pub async fn open_runtime(&self) -> AppResult<AgentEnvironment> {
        self.call(&["open", "--json"]).await?;
        self.environment().await
    }

    pub async fn list_missions(&self, workspace: &str) -> AppResult<Vec<AgentMission>> {
        bounded("工作区 ID", workspace, 200)?;
        self.store.missions(workspace).await
    }

    async fn mission(&self, id: &str) -> AppResult<AgentMission> {
        self.store
            .mission(id)
            .await?
            .ok_or_else(|| invalid("未找到此 Agent 任务。"))
    }

    pub async fn operations(&self, mission: &str) -> AppResult<Vec<AgentOperation>> {
        self.mission(mission).await?;
        self.store.operations(mission).await
    }

    async fn replay(
        &self,
        id: &str,
        mission: &str,
        kind: &str,
        request: &Value,
    ) -> AppResult<Option<AgentOperation>> {
        if let Some((mut operation, original)) = self.store.operation(id).await? {
            if operation.mission_id != mission || operation.kind != kind || &original != request {
                return Err(invalid("此操作 ID 已用于其他请求；不能修改请求后重放。"));
            }
            if operation.status == "pending" {
                operation.status = "unknown".into();
                operation.error = Some("尚无最终回执，请检查 Orca；不会自动重发。".into());
                self.store.save_operation(&operation).await?;
            }
            return Ok(Some(operation));
        }
        Ok(None)
    }

    async fn execute_operation(
        &self,
        mut operation: AgentOperation,
        arguments: Vec<String>,
        expected: ExpectedReceipt<'_>,
    ) -> AppResult<AgentOperation> {
        match self.runner.execute(&arguments).await {
            Ok(receipt) => {
                let state = receipt
                    .pointer("/result/state")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                operation.status = if state.contains("unknown") {
                    "unknown"
                } else if matches!(state, "failed" | "blocked" | "cancelled") {
                    "failed"
                } else {
                    "succeeded"
                }
                .into();
                if operation.kind == "start-task"
                    && !matches!(state, "ready" | "failed" | "blocked" | "cancelled")
                {
                    operation.status = "unknown".into();
                }
                if operation.status != "succeeded" {
                    operation.error = text(&receipt, "/result/lastError")
                        .or_else(|| text(&receipt, "/result/warning"))
                        .or_else(|| Some("Orca 未确认操作完成。已保留回执，不会自动重发。".into()));
                }
                if operation.status == "succeeded" {
                    if let Err(error) = receipt::validate(&receipt, expected) {
                        operation.status = "unknown".into();
                        operation.error = Some(error);
                    }
                }
                operation.receipt = receipt;
            }
            Err(error) => {
                operation.status = if error.ambiguous { "unknown" } else { "failed" }.into();
                operation.error = Some(error.message);
                operation.receipt = error.receipt;
            }
        }
        self.store.record_receipt(&operation).await?;
        Ok(operation)
    }

    async fn journal(
        &self,
        id: &str,
        mission: &str,
        kind: &str,
        request: Value,
        arguments: Vec<String>,
        expected: ExpectedReceipt<'_>,
    ) -> AppResult<AgentOperation> {
        if let Some(operation) = self.replay(id, mission, kind, &request).await? {
            return Ok(operation);
        }
        let operation = self.store.begin(id, mission, kind, &request).await?;
        let operation = self
            .execute_operation(operation, arguments, expected)
            .await?;
        self.store.finish_operation(&operation, None, None).await?;
        Ok(operation)
    }

    async fn mark_mission_error(
        &self,
        mission: &mut AgentMission,
        message: String,
    ) -> AppResult<()> {
        mission.status = "needs-attention".into();
        mission.error = Some(message);
        self.store.save_mission(mission).await
    }

    pub async fn create_mission(&self, input: CreateMissionInput) -> AppResult<AgentMission> {
        uuid("任务 ID", &input.id)?;
        bounded("目标", &input.objective, 32_000)?;
        bounded("仓库 ID", &input.repository_id, 200)?;
        let _guard = self.mission_lock(&input.id)?.lock_owned().await;
        if let Some(mission) = self.store.mission(&input.id).await? {
            if mission.workspace_id != input.workspace_id
                || mission.repository_id != input.repository_id
                || mission.objective != input.objective
            {
                return Err(invalid("此任务 ID 已绑定其他目标或工作区。"));
            }
            return Ok(mission);
        }
        let runtime = self.live_runtime().await?;
        let project = self
            .projects()
            .await?
            .into_iter()
            .find(|project| project.id == input.repository_id)
            .ok_or_else(|| invalid("所选仓库不在 Orca 当前仓库清单中，请刷新后重新选择。"))?;
        if !std::path::Path::new(&project.path).is_absolute() {
            return Err(invalid("Orca 仓库路径必须是绝对路径。"));
        }
        let mut mission = AgentMission {
            id: input.id,
            workspace_id: input.workspace_id,
            repository_id: project.id,
            repository_path: project.path,
            objective: input.objective,
            run_id: None,
            coordinator_handle: None,
            runtime_id: Some(runtime),
            status: "creating".into(),
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        // The workspace FK is checked before starting any process or creating any terminal.
        self.store.create_mission(&mission).await?;
        let result = self.initialize_mission(&mut mission).await;
        if let Err(error) = result {
            self.mark_mission_error(&mut mission, error.message).await?;
        }
        Ok(mission)
    }

    async fn create_coordinator(
        &self,
        mission: &mut AgentMission,
        operation_id: &str,
        kind: &str,
        runtime: &str,
    ) -> AppResult<AgentOperation> {
        let selector = format!("path:{}", mission.repository_path);
        let title = format!("ThoughsFlow {}", mission.id);
        let operation = self
            .store
            .begin(
                operation_id,
                &mission.id,
                kind,
                &json!({"step":"terminal", "repositoryId":mission.repository_id}),
            )
            .await?;
        let operation = self
            .execute_operation(
                operation,
                args(&[
                    "terminal",
                    "create",
                    "--worktree",
                    &selector,
                    "--title",
                    &title,
                    "--json",
                ]),
                ExpectedReceipt::Terminal,
            )
            .await?;
        let mut next = mission.clone();
        let pane_key = if operation.status == "succeeded" {
            next.coordinator_handle = text(&operation.receipt, "/result/terminal/handle");
            next.runtime_id = Some(runtime.to_owned());
            if kind == "reconnect" {
                next.status = "needs-attention".into();
                next.error = Some("已创建协调终端，等待确认 Run 绑定。".into());
            }
            terminal_key(&operation.receipt["result"]["terminal"])
        } else {
            next.status = "needs-attention".into();
            next.error = operation.error.clone();
            None
        };
        let binding = (operation.status == "succeeded").then_some((pane_key.as_deref(), None));
        self.store
            .finish_operation(&operation, Some(&next), binding)
            .await?;
        *mission = next;
        Ok(operation)
    }

    async fn finish_run_binding(
        &self,
        mission: &mut AgentMission,
        operation: &AgentOperation,
        handle: &str,
        runtime: &str,
        pane_key: Option<&str>,
    ) -> AppResult<()> {
        let mut next = mission.clone();
        let binding = if operation.status == "succeeded" {
            next.run_id = text(&operation.receipt, "/result/run/id");
            next.coordinator_handle = Some(handle.to_owned());
            next.runtime_id = Some(runtime.to_owned());
            next.status = "ready".into();
            next.error = None;
            Some((
                pane_key,
                operation
                    .receipt
                    .pointer("/result/run/consumer_generation")
                    .and_then(Value::as_i64),
            ))
        } else {
            next.status = "needs-attention".into();
            next.error = operation.error.clone();
            None
        };
        self.store
            .finish_operation(operation, Some(&next), binding)
            .await?;
        *mission = next;
        Ok(())
    }

    async fn initialize_mission(&self, mission: &mut AgentMission) -> AppResult<()> {
        let runtime = mission.runtime_id.clone().expect("runtime was verified");
        let terminal_operation_id = format!("{}:terminal", mission.id);
        let terminal = self
            .create_coordinator(mission, &terminal_operation_id, "create-mission", &runtime)
            .await?;
        if terminal.status != "succeeded" {
            return Ok(());
        }
        let handle = mission
            .coordinator_handle
            .clone()
            .expect("validated terminal handle");
        let operation = self
            .store
            .begin(
                &format!("{}:run", mission.id),
                &mission.id,
                "create-mission",
                &json!({"step":"run", "objective":mission.objective, "handle":handle}),
            )
            .await?;
        let operation = self
            .execute_operation(
                operation,
                args(&[
                    "orchestration",
                    "run-create",
                    "--from",
                    &handle,
                    "--objective",
                    &mission.objective,
                    "--json",
                ]),
                ExpectedReceipt::Run {
                    id: None,
                    coordinator: &handle,
                },
            )
            .await?;
        self.finish_run_binding(mission, &operation, &handle, &runtime, None)
            .await
    }

    async fn require_connection(&self, mission: &AgentMission) -> AppResult<()> {
        let runtime = self.live_runtime().await?;
        if mission.runtime_id.as_deref() != Some(&runtime) {
            return Err(invalid(
                "Orca 已重启，终端句柄已失效。请使用“重新连接”恢复此任务。",
            ));
        }
        let handle = mission
            .coordinator_handle
            .as_deref()
            .ok_or_else(|| invalid("任务缺少协调终端，请检查创建记录。"))?;
        let run = mission
            .run_id
            .as_deref()
            .ok_or_else(|| invalid("任务缺少 Orca Run，请检查创建记录。"))?;
        let current = self
            .call(&["orchestration", "run-current", "--from", handle, "--json"])
            .await?;
        let (_, generation) = self.store.binding(&mission.id).await?;
        if text(&current, "/result/run/id").as_deref() != Some(run)
            || generation.is_some_and(|expected| {
                current
                    .pointer("/result/run/consumer_generation")
                    .and_then(Value::as_i64)
                    != Some(expected)
            })
        {
            return Err(invalid("协调终端绑定已改变，请使用“重新连接”确认并恢复。"));
        }
        Ok(())
    }

    async fn workers(&self, mission: &AgentMission) -> AppResult<Vec<Value>> {
        let run = mission
            .run_id
            .as_deref()
            .ok_or_else(|| invalid("任务缺少 Orca Run。"))?;
        let mut workers = vec![];
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..20 {
            let mut arguments = args(&[
                "orchestration",
                "worker-list",
                "--run",
                run,
                "--limit",
                "100",
                "--json",
            ]);
            if let Some(cursor) = &cursor {
                arguments.extend(args(&["--cursor", cursor]));
            }
            let receipt = self
                .runner
                .execute(&arguments)
                .await
                .map_err(runtime_error)?;
            let page = receipt
                .pointer("/result/workers")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Orca worker 清单格式不受支持。"))?;
            for worker in page {
                if text(worker, "/runId").as_deref() != Some(run) {
                    return Err(invalid("Orca 返回了不属于当前 Run 的 worker，已停止操作。"));
                }
                workers.push(worker.clone());
            }
            if receipt
                .pointer("/result/page/hasMore")
                .and_then(Value::as_bool)
                != Some(true)
            {
                return Ok(workers);
            }
            let next = required(&receipt, "/result/page/nextCursor")?;
            if !seen.insert(next.clone()) {
                return Err(invalid("Orca worker 分页游标未前进。"));
            }
            cursor = Some(next);
        }
        Err(invalid(
            "此任务超过 2,000 个 worker，请在 Orca 检查，工作台暂停控制操作。",
        ))
    }

    async fn raw_tasks(&self, mission: &AgentMission) -> AppResult<Vec<Value>> {
        let run = mission
            .run_id
            .as_deref()
            .ok_or_else(|| invalid("任务缺少 Orca Run。"))?;
        let receipt = self
            .call(&["orchestration", "task-list", "--run", run, "--json"])
            .await?;
        let tasks = receipt
            .pointer("/result/tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Orca task 清单格式不受支持。"))?;
        if tasks
            .iter()
            .any(|task| text(task, "/run_id").as_deref().is_some_and(|id| id != run))
        {
            return Err(invalid("Orca 返回了不属于当前 Run 的 task。"));
        }
        Ok(tasks.clone())
    }

    async fn raw_messages(&self, mission: &AgentMission) -> AppResult<Vec<Value>> {
        let run = mission
            .run_id
            .as_deref()
            .ok_or_else(|| invalid("任务缺少 Orca Run。"))?;
        let handle = mission
            .coordinator_handle
            .as_deref()
            .ok_or_else(|| invalid("任务缺少协调终端。"))?;
        let receipt = self
            .call(&[
                "orchestration",
                "check",
                "--terminal",
                handle,
                "--run",
                run,
                "--peek",
                "--json",
            ])
            .await?;
        let messages = receipt
            .pointer("/result/messages")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Orca 邮箱格式不受支持。"))?;
        Ok(messages
            .iter()
            .filter(|message| {
                text(message, "/run_id").as_deref() == Some(run)
                    && matches!(text(message, "/to_handle"), Some(target) if target == handle || target == format!("run:{run}"))
            })
            .cloned()
            .collect())
    }

    pub async fn snapshot(&self, id: &str) -> AppResult<AgentSnapshot> {
        let _guard = self.mission_lock(id)?.lock_owned().await;
        let mut mission = self.mission(id).await?;
        let result = async {
            self.require_connection(&mission).await?;
            let workers = self.workers(&mission).await?;
            let tasks = self.raw_tasks(&mission).await?;
            let messages = self.raw_messages(&mission).await?;
            Ok::<_, AppError>((workers, tasks, messages))
        }
        .await;
        match result {
            Ok((workers, tasks, messages)) => {
                if mission.status == "needs-attention" {
                    mission.status = "ready".into();
                    mission.error = None;
                    self.store.save_mission(&mission).await?;
                }
                let mut projected_tasks = project_tasks(&tasks, &workers);
                let unresolved_releases = self.store.unresolved_releases(id).await?;
                for task in &mut projected_tasks {
                    if task
                        .dispatch_id
                        .as_ref()
                        .is_some_and(|id| unresolved_releases.contains(id))
                    {
                        task.can_release = false;
                    }
                }
                let replies = self.store.replied_message_ids(id).await?;
                let at_capacity = messages.len() >= 100;
                let unresolved_start = self.store.operations(id).await?.iter().any(|operation| {
                    matches!(
                        operation.kind.as_str(),
                        "create-mission" | "start-task" | "reconnect"
                    ) && matches!(operation.status.as_str(), "pending" | "unknown")
                });
                let projected_messages = messages
                    .iter()
                    .map(|message| project_message(message, &workers, &replies))
                    .collect();
                Ok(AgentSnapshot {
                    mission,
                    tasks: projected_tasks,
                    messages: projected_messages,
                    connected: true,
                    can_start_tasks: !at_capacity && !unresolved_start,
                    warning: if at_capacity {
                        Some("邮箱达到 100 条显示上限，请在 Orca 处理未读消息后继续。".into())
                    } else if unresolved_start {
                        Some("此任务有操作结果未确认，请在 Orca 核对后继续。".into())
                    } else {
                        None
                    },
                })
            }
            Err(error) => {
                self.mark_mission_error(&mut mission, error.message.clone())
                    .await?;
                Ok(AgentSnapshot {
                    mission,
                    tasks: vec![],
                    messages: vec![],
                    connected: false,
                    can_start_tasks: false,
                    warning: Some(error.message),
                })
            }
        }
    }

    pub async fn start_task(&self, input: StartTaskInput) -> AppResult<AgentOperation> {
        uuid("操作 ID", &input.operation_id)?;
        bounded("任务标题", &input.title, 200)?;
        bounded("任务说明", &input.spec, 64_000)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("serializable input");
        if let Some(operation) = self
            .replay(
                &input.operation_id,
                &input.mission_id,
                "start-task",
                &request,
            )
            .await?
        {
            return Ok(operation);
        }
        let mission = self.mission(&input.mission_id).await?;
        self.require_connection(&mission).await?;
        if self
            .store
            .operations(&mission.id)
            .await?
            .iter()
            .any(|operation| {
                matches!(
                    operation.kind.as_str(),
                    "create-mission" | "start-task" | "reconnect"
                ) && matches!(operation.status.as_str(), "pending" | "unknown")
            })
        {
            return Err(invalid(
                "此任务有派发结果未确认，请先检查 Orca 中的 worker；不会再次派发。",
            ));
        }
        if self.raw_messages(&mission).await?.len() >= 100 {
            return Err(invalid(
                "邮箱达到 100 条显示上限，请在 Orca 处理未读消息后继续。",
            ));
        }
        let project = self
            .projects()
            .await?
            .into_iter()
            .find(|project| project.id == mission.repository_id)
            .ok_or_else(|| invalid("任务绑定的仓库已不在 Orca 中，请检查仓库配置。"))?;
        if project.path != mission.repository_path {
            return Err(invalid("任务仓库路径发生变化，请建立新任务。"));
        }
        let spec = format!(
            "Mission objective:\n{}\n\nUser task specification (preserve scope and acceptance criteria):\n{}\n\nWork only in the assigned worktree for this task. Follow repository instructions. Verify the requested acceptance criteria and report results and remaining risks. Ask the coordinator when scope or authorization is unclear. Do not merge, push, publish, or modify unrelated work without explicit authorization in the task specification.",
            mission.objective, input.spec
        );
        let name = format!("tf-{}", input.operation_id.replace('-', ""));
        let repository = format!("id:{}", mission.repository_id);
        let arguments = args(&[
            "orchestration",
            "worker-start",
            "--spec",
            &spec,
            "--task-title",
            &input.title,
            "--worktree",
            "new-top-level",
            "--repo",
            &repository,
            "--name",
            &name,
            "--setup",
            "skip",
            "--agent",
            "omp",
            "--from",
            mission.coordinator_handle.as_deref().unwrap(),
            "--run",
            mission.run_id.as_deref().unwrap(),
            "--timeout-ms",
            "60000",
            "--json",
        ]);
        self.journal(
            &input.operation_id,
            &mission.id,
            "start-task",
            request,
            arguments,
            ExpectedReceipt::StartTask {
                run_id: mission.run_id.as_deref().unwrap(),
            },
        )
        .await
    }

    pub async fn reply(&self, input: ReplyInput) -> AppResult<AgentOperation> {
        uuid("操作 ID", &input.operation_id)?;
        bounded("回复", &input.body, 32_000)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("serializable input");
        if let Some(operation) = self
            .replay(&input.operation_id, &input.mission_id, "reply", &request)
            .await?
        {
            return Ok(operation);
        }
        let mission = self.mission(&input.mission_id).await?;
        self.require_connection(&mission).await?;
        let workers = self.workers(&mission).await?;
        let replies = self.store.replied_message_ids(&mission.id).await?;
        let messages = self.raw_messages(&mission).await?;
        let message = messages
            .iter()
            .find(|message| text(message, "/id").as_deref() == Some(&input.message_id))
            .ok_or_else(|| invalid("未找到属于此任务的待回复消息，请刷新。"))?;
        if !project_message(message, &workers, &replies).requires_reply {
            return Err(invalid(
                "此消息不是当前任务中可回复的问题，或已有回复操作。",
            ));
        }
        self.journal(
            &input.operation_id,
            &mission.id,
            "reply",
            request,
            args(&[
                "orchestration",
                "reply",
                "--id",
                &input.message_id,
                "--body",
                &input.body,
                "--from",
                mission.coordinator_handle.as_deref().unwrap(),
                "--run",
                mission.run_id.as_deref().unwrap(),
                "--json",
            ]),
            ExpectedReceipt::Reply {
                question_id: &input.message_id,
                thread_id: message["thread_id"].as_str(),
                run_id: mission.run_id.as_deref().unwrap(),
            },
        )
        .await
    }

    pub async fn release(&self, input: ReleaseWorkerInput) -> AppResult<AgentOperation> {
        uuid("操作 ID", &input.operation_id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("serializable input");
        if let Some(operation) = self
            .replay(&input.operation_id, &input.mission_id, "release", &request)
            .await?
        {
            return Ok(operation);
        }
        let mission = self.mission(&input.mission_id).await?;
        self.require_connection(&mission).await?;
        if self
            .store
            .unresolved_releases(&mission.id)
            .await?
            .contains(&input.dispatch_id)
        {
            return Err(invalid(
                "此 worker 有释放结果未确认，请在 Orca 检查；不会重复释放。",
            ));
        }
        let workers = self.workers(&mission).await?;
        let tasks = self.raw_tasks(&mission).await?;
        let projected = project_tasks(&tasks, &workers);
        if !projected
            .iter()
            .any(|task| task.dispatch_id.as_deref() == Some(&input.dispatch_id) && task.can_release)
        {
            return Err(invalid(
                "仅能释放此任务内已结算且可回收的 worker；空闲不代表完成。",
            ));
        }
        self.journal(
            &input.operation_id,
            &mission.id,
            "release",
            request,
            args(&[
                "orchestration",
                "worker-release",
                "--dispatch",
                &input.dispatch_id,
                "--json",
            ]),
            ExpectedReceipt::Release {
                dispatch_id: &input.dispatch_id,
            },
        )
        .await
    }

    pub async fn reconnect(&self, input: ReconnectInput) -> AppResult<AgentOperation> {
        uuid("操作 ID", &input.operation_id)?;
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let request = serde_json::to_value(&input).expect("serializable input");
        if let Some(operation) = self
            .replay(
                &input.operation_id,
                &input.mission_id,
                "reconnect",
                &request,
            )
            .await?
        {
            return Ok(operation);
        }
        let mut mission = self.mission(&input.mission_id).await?;
        if self
            .store
            .operations(&mission.id)
            .await?
            .iter()
            .any(|operation| {
                operation.kind == "reconnect"
                    && matches!(operation.status.as_str(), "pending" | "unknown")
            })
        {
            return Err(invalid(
                "上次重连结果尚未确认，请在 Orca 核对；不会创建重复协调终端。",
            ));
        }
        let run = mission
            .run_id
            .clone()
            .ok_or_else(|| invalid("创建尚未取得 Run 回执，请在 Orca 检查后建立新任务。"))?;
        let runtime = self.live_runtime().await?;
        let (pane_key, _) = self.store.binding(&mission.id).await?;
        let selector = format!("path:{}", mission.repository_path);
        let receipt = self
            .call(&["terminal", "list", "--worktree", &selector, "--json"])
            .await?;
        let terminals = receipt
            .pointer("/result/terminals")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Orca 终端清单格式不受支持。"))?;
        let matches: Vec<_> = terminals
            .iter()
            .filter(|terminal| {
                pane_key
                    .as_ref()
                    .is_some_and(|key| terminal_key(terminal).as_deref() == Some(key))
            })
            .collect();
        if matches.len() > 1 {
            return Err(invalid(
                "无法唯一确认此任务的专属协调终端，请在 Orca 检查；不会接管其他终端。",
            ));
        }
        let mut operation = self
            .store
            .begin(&input.operation_id, &mission.id, "reconnect", &request)
            .await?;
        let terminal = if let Some(terminal) = matches.first() {
            (*terminal).clone()
        } else {
            // Explicit reconnect authorizes a replacement pane; titles never establish ownership.
            let created = self
                .create_coordinator(
                    &mut mission,
                    &format!("{}:terminal", input.operation_id),
                    "reconnect",
                    &runtime,
                )
                .await?;
            if created.status != "succeeded" {
                operation.status = created.status;
                operation.error = created.error;
                operation.receipt = created.receipt;
                self.store.record_receipt(&operation).await?;
                self.store
                    .finish_operation(&operation, Some(&mission), None)
                    .await?;
                return Ok(operation);
            }
            created.receipt["result"]["terminal"].clone()
        };
        let handle = match required(&terminal, "/handle") {
            Ok(handle) => handle,
            Err(error) => {
                operation.status = "unknown".into();
                operation.error = Some(error.message);
                operation.receipt = receipt;
                self.store.record_receipt(&operation).await?;
                self.store.finish_operation(&operation, None, None).await?;
                return Ok(operation);
            }
        };
        let operation = self
            .execute_operation(
                operation,
                args(&[
                    "orchestration",
                    "run-use",
                    "--id",
                    &run,
                    "--from",
                    &handle,
                    "--json",
                ]),
                ExpectedReceipt::Run {
                    id: Some(&run),
                    coordinator: &handle,
                },
            )
            .await?;
        self.finish_run_binding(
            &mut mission,
            &operation,
            &handle,
            &runtime,
            terminal_key(&terminal).as_deref(),
        )
        .await?;
        Ok(operation)
    }

    pub async fn read_output(&self, input: ReadOutputInput) -> AppResult<AgentOutput> {
        if let Some(cursor) = &input.cursor {
            bounded("输出游标", cursor, 16_000)?;
        }
        let _guard = self.mission_lock(&input.mission_id)?.lock_owned().await;
        let mission = self.mission(&input.mission_id).await?;
        self.require_connection(&mission).await?;
        if !self
            .workers(&mission)
            .await?
            .iter()
            .any(|worker| text(worker, "/dispatchId").as_deref() == Some(&input.dispatch_id))
        {
            return Err(invalid("此 worker 不属于当前任务。"));
        }
        let mut arguments = args(&[
            "orchestration",
            "worker-read",
            "--dispatch",
            &input.dispatch_id,
            "--limit",
            "200",
            "--json",
        ]);
        if let Some(cursor) = &input.cursor {
            arguments.extend(args(&["--cursor", cursor]));
        }
        let receipt = self
            .runner
            .execute(&arguments)
            .await
            .map_err(runtime_error)?;
        let result = &receipt["result"];
        let cursor = text(result, "/cursor")
            .or_else(|| result["cursor"].as_u64().map(|number| number.to_string()));
        let has_more = result["hasMore"]
            .as_bool()
            .or_else(|| result["terminal"]["limited"].as_bool())
            .unwrap_or_else(|| cursor.is_some() && cursor != input.cursor);
        let text = if let Some(lines) = result.pointer("/terminal/tail").and_then(Value::as_array) {
            lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            result
                .pointer("/transcript/messages")
                .and_then(Value::as_array)
                .map(|messages| {
                    messages
                        .iter()
                        .map(format_transcript)
                        .collect::<Vec<_>>()
                        .join("\n\n")
                })
                .unwrap_or_default()
        };
        Ok(AgentOutput {
            text,
            cursor,
            has_more,
            source: text_value(result, "/source"),
            warning: output_warning(result),
        })
    }
}

fn text_value(value: &Value, pointer: &str) -> Option<String> {
    text(value, pointer)
}

fn output_warning(result: &Value) -> Option<String> {
    let warnings = result["warnings"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(3)
                .collect::<Vec<_>>()
                .join("；")
        })
        .unwrap_or_default();
    let clipped = result["clipping"]
        .as_array()
        .is_some_and(|values| !values.is_empty())
        || result["clipping"].as_object().is_some_and(|values| {
            values.values().any(|value| {
                value.as_bool() == Some(true) || value.as_u64().is_some_and(|number| number > 0)
            })
        })
        || result["terminal"]["limited"].as_bool() == Some(true)
        || result["contentComplete"].as_bool() == Some(false);
    if clipped || !warnings.is_empty() {
        let detail: String = warnings.chars().take(500).collect();
        Some(format!(
            "当前仅显示部分输出，可刷新或在 Orca 查看完整记录。{detail}"
        ))
    } else {
        None
    }
}

fn terminal_key(terminal: &Value) -> Option<String> {
    text(terminal, "/paneKey").or_else(|| {
        text(terminal, "/tabId")
            .map(|tab| format!("{tab}:{}", text(terminal, "/leafId").unwrap_or_default()))
    })
}

fn task_worker<'a>(task: &Value, workers: &'a [Value]) -> Option<&'a Value> {
    let task_id = task["id"].as_str()?;
    let dispatch_id = match task.get("dispatch_id") {
        Some(Value::String(dispatch)) => dispatch.as_str(),
        None | Some(Value::Null) => {
            // Orca task-list only joins active dispatches: settled Tasks return null.
            // Link one settled attempt only when both durable state and outcome agree.
            let outcome = match task["status"].as_str()? {
                "completed" => "succeeded",
                "failed" => "failed",
                _ => return None,
            };
            let mut attempts = workers
                .iter()
                .filter(|worker| worker["taskId"].as_str() == Some(task_id));
            let worker = attempts.next()?;
            if attempts.next().is_some()
                || worker["workerState"].as_str() != Some(outcome)
                || worker
                    .pointer("/projection/outcome")
                    .and_then(Value::as_str)
                    != Some(outcome)
            {
                return None;
            }
            worker["dispatchId"].as_str()?
        }
        _ => return None,
    };
    if dispatch_id.trim().is_empty() {
        return None;
    }
    let mut matching = workers
        .iter()
        .filter(|worker| worker["dispatchId"].as_str() == Some(dispatch_id));
    let worker = matching
        .next()
        .filter(|worker| worker["taskId"].as_str() == Some(task_id));
    if matching.next().is_none() {
        worker
    } else {
        None
    }
}

fn project_tasks(tasks: &[Value], workers: &[Value]) -> Vec<AgentTask> {
    tasks
        .iter()
        .map(|task| {
            let id = text(task, "/id").unwrap_or_default();
            let worker = task_worker(task, workers);
            let status = text(task, "/status").unwrap_or_else(|| "unknown".into());
            let terminal_state = worker.and_then(|worker| text(worker, "/terminalState"));
            let can_release = matches!(status.as_str(), "completed" | "failed")
                && terminal_state.as_deref() == Some("reclaimable")
                && worker.is_some_and(|worker| {
                    matches!(
                        worker
                            .pointer("/projection/outcome")
                            .and_then(Value::as_str),
                        Some("succeeded" | "failed")
                    ) && worker
                        .pointer("/resource/ownershipState")
                        .and_then(Value::as_str)
                        == Some("owned")
                        && worker
                            .pointer("/resource/ownerDispatchId")
                            .and_then(Value::as_str)
                            == worker["dispatchId"].as_str()
                });
            AgentTask {
                id,
                title: text(task, "/display_name")
                    .or_else(|| text(task, "/task_title"))
                    .unwrap_or_else(|| "Agent task".into()),
                spec: text(task, "/spec").unwrap_or_default(),
                status,
                dispatch_id: worker.and_then(|worker| text(worker, "/dispatchId")),
                terminal_state,
                liveness: worker.and_then(|worker| text(worker, "/projection/liveness/verdict")),
                attention: worker
                    .and_then(|worker| {
                        worker
                            .pointer("/projection/attention/categories")
                            .and_then(Value::as_array)
                    })
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|text| !text.is_empty()),
                can_release,
            }
        })
        .collect()
}

fn project_message(message: &Value, workers: &[Value], replies: &HashSet<String>) -> AgentMessage {
    let id = text(message, "/id").unwrap_or_default();
    let payload = message["payload"]
        .as_str()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .unwrap_or(Value::Null);
    let sender = text(message, "/from_handle");
    let worker = workers.iter().find(|worker| {
        sender.as_ref().is_some_and(|sender| {
            text(worker, "/agentTerminalHandle").as_deref() == Some(sender)
                || text(worker, "/dispatchId")
                    .is_some_and(|dispatch| sender == &format!("dispatch:{dispatch}"))
        })
    });
    let already_replied = replies.contains(&id);
    AgentMessage {
        id,
        r#type: text(message, "/type").unwrap_or_else(|| "message".into()),
        body: text(message, "/body").unwrap_or_default(),
        task_id: worker
            .and_then(|worker| text(worker, "/taskId"))
            .or_else(|| text(&payload, "/taskId")),
        dispatch_id: worker
            .and_then(|worker| text(worker, "/dispatchId"))
            .or_else(|| text(&payload, "/dispatchId")),
        requires_reply: message["type"].as_str() == Some("question")
            && worker.is_some()
            && !already_replied,
    }
}

fn format_transcript(message: &Value) -> String {
    let mut parts = vec![format!(
        "[{}]",
        message["role"].as_str().unwrap_or("message")
    )];
    if let Some(blocks) = message["blocks"].as_array() {
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => parts.push(text(block, "/text").unwrap_or_default()),
                Some("tool-call") => parts.push(format!(
                    "{} {}",
                    block["name"].as_str().unwrap_or("tool"),
                    block["input"]
                )),
                Some("tool-result") => parts.push(
                    block["output"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| block["output"].to_string()),
                ),
                _ => {}
            }
        }
    }
    parts.join("\n")
}
