use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const MAX_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeFailure {
    pub message: String,
    pub receipt: Value,
    pub ambiguous: bool,
}

impl RuntimeFailure {
    fn transport(message: impl Into<String>, ambiguous: bool) -> Self {
        Self {
            message: message.into(),
            receipt: Value::Null,
            ambiguous,
        }
    }
}

#[async_trait]
pub(crate) trait CommandRunner: Send + Sync {
    fn available(&self) -> bool;
    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure>;
    async fn versions(&self) -> (Option<String>, Option<String>) {
        (None, None)
    }
}

pub(crate) struct OrcaCli {
    executable: Option<PathBuf>,
}

impl OrcaCli {
    pub fn discover() -> Self {
        let executable = match std::env::var_os("THOUGHSFLOW_ORCA_BIN") {
            Some(path) => {
                let path = PathBuf::from(path);
                (path.is_absolute() && path.is_file()).then_some(path)
            }
            None => find_executable(if cfg!(target_os = "linux") {
                "orca-ide"
            } else {
                "orca"
            }),
        };
        Self { executable }
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let defaults = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    defaults
        .into_iter()
        .chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ))
        .map(|directory| directory.join(name))
        .find(|path| path.is_absolute() && path.is_file())
}

pub(crate) fn is_identity_environment(key: &str) -> bool {
    matches!(
        key,
        "ORCA_TERMINAL_HANDLE"
            | "ORCA_PANE_KEY"
            | "ORCA_AGENT_LAUNCH_TOKEN"
            | "ORCA_CLI_CWD"
            | "ORCA_ENVIRONMENT"
            | "ORCA_PAIRING_CODE"
            | "ORCA_REMOTE_PAIRING"
    ) || key.starts_with("ORCA_ORCHESTRATION_COMPATIBILITY_")
}

async fn bounded_read(mut stream: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let size = stream.read(&mut chunk).await?;
        if size == 0 {
            return Ok(output);
        }
        if output.len() + size > MAX_OUTPUT {
            return Err(std::io::Error::other("Orca 输出超过 4 MiB 安全上限"));
        }
        output.extend_from_slice(&chunk[..size]);
    }
}

async fn run_process(
    executable: &Path,
    arguments: &[String],
    seconds: u64,
) -> Result<(bool, Vec<u8>), RuntimeFailure> {
    // Shell syntax is never evaluated; every user value is one argv element.
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, _) in std::env::vars_os() {
        if is_identity_environment(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
    let mut child = command.spawn().map_err(|error| {
        RuntimeFailure::transport(format!("无法启动 Orca 命令：{error}"), false)
    })?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let output = tokio::time::timeout(Duration::from_secs(seconds), async {
        let (status, stdout, _) =
            tokio::try_join!(child.wait(), bounded_read(stdout), bounded_read(stderr))?;
        Ok::<_, std::io::Error>((status.success(), stdout))
    })
    .await;
    match output {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(RuntimeFailure::transport(
                format!("读取 Orca 回执失败：{error}。请检查 Orca，不要重复派发。"),
                true,
            ))
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(RuntimeFailure::transport(
                "等待 Orca 回执超时。操作可能已执行，请检查 Orca；不会自动重发。",
                true,
            ))
        }
    }
}

pub(crate) fn parse_response(success: bool, stdout: &[u8]) -> Result<Value, RuntimeFailure> {
    let receipt: Value = serde_json::from_slice(stdout).map_err(|_| {
        RuntimeFailure::transport(
            "Orca 未返回有效 JSON 回执。请检查运行时版本；不会自动重发。",
            true,
        )
    })?;
    if receipt["ok"].as_bool() == Some(false) {
        let ambiguous = receipt
            .pointer("/error/data/outcomeUnknown")
            .and_then(Value::as_bool)
            == Some(true)
            || receipt
                .pointer("/error/data/recovery/disposition")
                .and_then(Value::as_str)
                == Some("outcome_unknown")
            || receipt
                .pointer("/result/state")
                .and_then(Value::as_str)
                .is_some_and(|state| state.contains("unknown"))
            || receipt
                .pointer("/error/code")
                .and_then(Value::as_str)
                .is_some_and(|code| {
                    code.contains("unknown")
                        || matches!(
                            code,
                            "runtime_timeout" | "runtime_unavailable" | "invalid_runtime_response"
                        )
                });
        return Err(RuntimeFailure {
            message: receipt
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Orca 拒绝了操作")
                .to_owned(),
            receipt,
            ambiguous,
        });
    }
    if receipt["ok"].as_bool() != Some(true) || !receipt["result"].is_object() {
        return Err(RuntimeFailure {
            message: "Orca 回执结构不受支持，请检查运行时版本。".into(),
            receipt,
            ambiguous: true,
        });
    }
    // Some Orca operations deliberately exit 1 with an ok:true, outcome_unknown receipt.
    // Preserve that receipt so the service can classify the actual operation state.
    if !success && receipt["result"]["state"].as_str().is_none() {
        return Err(RuntimeFailure {
            message: "Orca 命令异常退出；已保留回执，请检查 Orca。".into(),
            receipt,
            ambiguous: true,
        });
    }
    Ok(receipt)
}

#[async_trait]
impl CommandRunner for OrcaCli {
    fn available(&self) -> bool {
        self.executable.is_some()
    }

    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        let executable = self.executable.as_ref().ok_or_else(|| RuntimeFailure::transport(
            "未找到 Orca CLI，请安装 Orca，或设置 THOUGHSFLOW_ORCA_BIN 为可执行文件的绝对路径。", false))?;
        let long = arguments.first().is_some_and(|command| command == "open")
            || arguments.get(1).is_some_and(|command| {
                matches!(
                    command.as_str(),
                    "create"
                        | "worker-start"
                        | "worker-release"
                        | "run-create"
                        | "run-use"
                        | "reply"
                )
            });
        let (success, stdout) =
            run_process(executable, arguments, if long { 75 } else { 10 }).await?;
        parse_response(success, &stdout)
    }

    async fn versions(&self) -> (Option<String>, Option<String>) {
        let orca = version(self.executable.as_deref()).await;
        let omp_path = match std::env::var_os("THOUGHSFLOW_OMP_BIN") {
            Some(path) => {
                let path = PathBuf::from(path);
                (path.is_absolute() && path.is_file()).then_some(path)
            }
            None => find_executable("omp"),
        };
        let omp = version(omp_path.as_deref()).await;
        (orca, omp)
    }
}

async fn version(executable: Option<&Path>) -> Option<String> {
    let executable = executable?;
    run_process(executable, &["--version".into()], 5)
        .await
        .ok()
        .filter(|(success, _)| *success)
        .and_then(|(_, bytes)| String::from_utf8(bytes).ok())
        .and_then(|text| {
            text.lines()
                .next()
                .map(|line| line.chars().take(120).collect())
        })
}
