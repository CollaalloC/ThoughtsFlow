//! Generation-1 JSON-RPC over `herdr-core connect`; no shell or Agent protocol emulation.
use super::executable::{self, LaunchPlan, Tool};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug)]
pub(crate) struct RuntimeFailure {
    pub message: String,
    pub raw: Value,
    pub ambiguous: bool,
}
impl RuntimeFailure {
    fn new(message: impl Into<String>, ambiguous: bool) -> Self {
        Self {
            message: message.into(),
            raw: Value::Null,
            ambiguous,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct RuntimeReply {
    pub core_id: String,
    pub result: Value,
    pub raw: Value,
}
#[async_trait]
pub(crate) trait AgentRuntime: Send + Sync {
    fn available(&self) -> bool;
    fn connected(&self) -> bool;
    async fn version(&self) -> Option<String>;
    async fn connect(&self) -> Result<RuntimeReply, RuntimeFailure>;
    async fn call(
        &self,
        method: &str,
        params: Value,
        core: Option<&str>,
    ) -> Result<RuntimeReply, RuntimeFailure>;
}
pub(crate) struct HerdrCore {
    launch: Result<LaunchPlan, String>,
    enabled: AtomicBool,
}
impl HerdrCore {
    pub fn discover() -> Self {
        Self {
            launch: executable::discover(Tool::Herdr),
            enabled: AtomicBool::new(false),
        }
    }
    fn command(&self) -> Result<Command, RuntimeFailure> {
        let plan = self
            .launch
            .as_ref()
            .map_err(|e| RuntimeFailure::new(e, false))?;
        plan.validate_arguments(executable::Platform::current(), &["connect".into()])
            .map_err(|e| RuntimeFailure::new(e, false))?;
        let mut cmd = Command::new(&plan.program);
        cmd.args(&plan.prefix_arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // A dedicated core retains the user's native templates/config, but never adopts
        // the calling agent's parent, socket or process ownership.
        for key in [
            "HC_TERMINAL_ID",
            "HC_SCOPE",
            "HC_SOCKET",
            "HC_SESSION",
            "ORCA_TERMINAL_HANDLE",
            "ORCA_PANE_KEY",
            "ORCA_AGENT_LAUNCH_TOKEN",
            "ORCA_CLI_CWD",
            "ORCA_ENVIRONMENT",
            "ORCA_PAIRING_CODE",
            "ORCA_REMOTE_PAIRING",
            "ORCA_USER_DATA_PATH",
            "ORCA_WORKSPACE_ID",
            "ORCA_WORKTREE_ID",
            "ORCA_STRUCTURED_SESSION",
            "ORCA_CLI_COMMAND",
            "ORCA_DEV_REPO_ROOT",
            "ORCA_DEV_CLI_INVOCATION",
            "ORCA_AGENT_SESSION_ID",
            "ORCA_AGENT_SESSION_SPAWN_TOKEN",
        ] {
            cmd.env_remove(key);
        }
        cmd.env(
            "HC_SESSION",
            std::env::var("THOUGHTSFLOW_HERDR_SESSION").unwrap_or_else(|_| "thoughtsflow".into()),
        );
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        Ok(cmd)
    }
    async fn exchange(
        &self,
        method: &str,
        params: Value,
        expected: Option<&str>,
    ) -> Result<RuntimeReply, RuntimeFailure> {
        let mut child = self
            .command()?
            .arg("connect")
            .spawn()
            .map_err(|e| RuntimeFailure::new(format!("无法连接 herdr-core：{e}"), false))?;
        let mut stdin = child.stdin.take().expect("piped");
        let mut reader = BufReader::new(child.stdout.take().expect("piped"));
        let stderr = child.stderr.take().expect("piped");
        let drain = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stderr.take(LIMIT as u64), &mut tokio::io::sink()).await;
        });
        let mut sent = false;
        let result = tokio::time::timeout(Duration::from_secs(75), async {
            stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\",\"params\":{\"generation\":1}}\n").await.map_err(|e| RuntimeFailure::new(e.to_string(), false))?;
            let hello = read_reply(&mut reader, 1).await?;
            let h = &hello["result"];
            let core_id = h.pointer("/core/id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| RuntimeFailure::new("herdr-core 缺少稳定 core 身份。", false))?.to_owned();
            if !supported_version(h["version"].as_str().unwrap_or(""))
                || h["generation"].as_u64() != Some(1) || expected.is_some_and(|e| e != core_id) { return Err(RuntimeFailure::new("herdr-core 协议或 core 身份已改变；未派发。", false)); }
            if method == "hello" { return Ok(RuntimeReply { core_id, result: h.clone(), raw: hello }); }
            if !h["methods"].as_array().is_some_and(|xs| xs.iter().any(|x| x.as_str() == Some(method))) { return Err(RuntimeFailure::new(format!("herdr-core 未声明 {method}；未派发。"), false)); }
            let request = json!({"jsonrpc":"2.0","id":2,"method":method,"params":params});
            sent = true; // Even a partial pipe write is not proof of non-execution.
            stdin.write_all(format!("{request}\n").as_bytes()).await.map_err(|e| RuntimeFailure::new(e.to_string(), true))?;
            let raw = read_reply(&mut reader, 2).await.map_err(|mut e| {
                e.ambiguous = !matches!(e.raw.pointer("/error/data/kind").and_then(Value::as_str), Some("blocked" | "status_mismatch" | "invalid_params")); e
            })?;
            Ok(RuntimeReply { core_id, result: raw["result"].clone(), raw })
        }).await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        drain.abort();
        result.unwrap_or_else(|_| {
            Err(RuntimeFailure::new(
                "herdr-core 回执超时；不会自动重发，请核对已有终端。",
                sent,
            ))
        })
    }
}
fn supported_version(value: &str) -> bool {
    let parts: Vec<_> = value
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .map(str::parse::<u64>)
        .collect();
    matches!(parts.as_slice(), [Ok(major), Ok(minor), Ok(patch)] if (*major, *minor, *patch) >= (0, 1, 17))
}

async fn read_reply(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    id: u64,
) -> Result<Value, RuntimeFailure> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader
            .fill_buf()
            .await
            .map_err(|e| RuntimeFailure::new(e.to_string(), true))?;
        if chunk.is_empty() {
            return Err(RuntimeFailure::new("herdr-core 连接在回执前关闭。", true));
        }
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(chunk.len());
        if bytes.len() + n > LIMIT {
            return Err(RuntimeFailure::new(
                "herdr-core 回执超过 4 MiB 上限。",
                true,
            ));
        }
        bytes.extend_from_slice(&chunk[..n]);
        let done = chunk[n - 1] == b'\n';
        reader.consume(n);
        if done {
            break;
        }
    }
    let raw: Value = serde_json::from_slice(&bytes)
        .map_err(|_| RuntimeFailure::new("herdr-core 回执不是有效 JSON。", true))?;
    if raw["jsonrpc"] != "2.0" || raw["id"].as_u64() != Some(id) {
        return Err(RuntimeFailure {
            message: "herdr-core 回执身份不匹配。".into(),
            raw,
            ambiguous: true,
        });
    }
    if raw.get("error").is_some() {
        return Err(RuntimeFailure {
            message: raw
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("herdr-core 拒绝请求")
                .into(),
            raw,
            ambiguous: false,
        });
    }
    if !raw["result"].is_object() {
        return Err(RuntimeFailure {
            message: "herdr-core 回执缺少结果。".into(),
            raw,
            ambiguous: true,
        });
    }
    Ok(raw)
}
#[async_trait]
impl AgentRuntime for HerdrCore {
    fn available(&self) -> bool {
        self.launch.is_ok()
    }
    fn connected(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    async fn version(&self) -> Option<String> {
        let mut cmd = self.command().ok()?;
        let output = tokio::time::timeout(Duration::from_secs(5), cmd.arg("--version").output())
            .await
            .ok()?
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
    async fn connect(&self) -> Result<RuntimeReply, RuntimeFailure> {
        let reply = self.exchange("hello", json!({}), None).await?;
        self.enabled.store(true, Ordering::SeqCst);
        Ok(reply)
    }
    async fn call(
        &self,
        method: &str,
        params: Value,
        core: Option<&str>,
    ) -> Result<RuntimeReply, RuntimeFailure> {
        if !self.connected() {
            return Err(RuntimeFailure::new("请先连接 herdr-core 专用会话。", false));
        }
        self.exchange(method, params, core).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn subprocess_fixture_exercises_the_real_json_rpc_transport() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/herdr-core.mjs");
        // Wrapper sets only this fixture's temporary state; never runs the installed core.
        let wrapper = dir.path().join("transport.mjs");
        std::fs::write(
            &wrapper,
            format!(
                "process.env.TF_AGENT_FIXTURE_STATE={}; await import({});",
                serde_json::to_string(&state.to_string_lossy()).unwrap(),
                serde_json::to_string(
                    &url::Url::from_file_path(script.canonicalize().unwrap())
                        .unwrap()
                        .to_string()
                )
                .unwrap()
            ),
        )
        .unwrap();
        let mut plan = executable::discover(Tool::Node).unwrap();
        plan.prefix_arguments.push(wrapper.into_os_string());
        let core = HerdrCore {
            launch: Ok(plan),
            enabled: AtomicBool::new(false),
        };
        assert!(core.call("terminal.list", json!({}), None).await.is_err());
        assert_eq!(core.connect().await.unwrap().core_id, "fixture-core");
        assert!(
            core.call("terminal.run", json!({}), Some("foreign-core"))
                .await
                .is_err()
        );
        let r=core.call("terminal.run",json!({"argv":["reviewer","literal $(echo no)"],"cwd":"/fixture","no_parent":true,"keep":false,"labels":{}}),Some("fixture-core")).await.unwrap();
        assert_eq!(r.result["first_message"]["sent"], true);
        let r = core
            .call(
                "terminal.read",
                json!({"terminal":r.result["address"],"lines":0}),
                Some("fixture-core"),
            )
            .await
            .unwrap();
        assert_eq!(
            r.result["text"],
            "Herdr fixture output; no model was called"
        );
        let data = std::fs::read_to_string(state.with_extension("json.calls.jsonl")).unwrap();
        assert!(!data.contains("foreign-core"));
    }
}
