use std::{process::Stdio, time::Duration};

use async_trait::async_trait;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

use super::executable::{self, LaunchPlan, Platform, Tool};

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
    executable: Result<LaunchPlan, String>,
}

impl OrcaCli {
    pub fn discover() -> Self {
        let executable = executable::discover(Tool::Orca);
        Self { executable }
    }
}

pub(crate) fn is_identity_environment(key: &str) -> bool {
    is_identity_environment_for_platform(key, Platform::current())
}

fn is_identity_environment_for_platform(key: &str, platform: Platform) -> bool {
    let normalized;
    let key = if platform == Platform::Windows {
        normalized = key.to_ascii_uppercase();
        normalized.as_str()
    } else {
        key
    };
    matches!(
        key,
        "ORCA_TERMINAL_HANDLE"
            | "ORCA_PANE_KEY"
            | "ORCA_AGENT_LAUNCH_TOKEN"
            | "ORCA_CLI_CWD"
            | "ORCA_ENVIRONMENT"
            | "ORCA_PAIRING_CODE"
            | "ORCA_REMOTE_PAIRING"
            | "ORCA_USER_DATA_PATH"
            | "ORCA_WORKSPACE_ID"
            | "ORCA_WORKTREE_ID"
            | "ORCA_STRUCTURED_SESSION"
            | "ORCA_CLI_COMMAND"
            | "ORCA_DEV_REPO_ROOT"
            | "ORCA_DEV_CLI_INVOCATION"
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
    executable: &LaunchPlan,
    arguments: &[String],
    seconds: u64,
) -> Result<(bool, Vec<u8>), RuntimeFailure> {
    executable
        .validate_arguments(Platform::current(), arguments)
        .map_err(|message| RuntimeFailure::transport(message, false))?;
    // Shell syntax is never evaluated; every user value is one argv element.
    let mut command = Command::new(&executable.program);
    command
        .args(&executable.prefix_arguments)
        .args(arguments)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
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
        self.executable.is_ok()
    }

    async fn execute(&self, arguments: &[String]) -> Result<Value, RuntimeFailure> {
        let executable = self
            .executable
            .as_ref()
            .map_err(|message| RuntimeFailure::transport(message.clone(), false))?;
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
        let omp = executable::discover(Tool::Omp);
        tokio::join!(
            version(self.executable.as_ref().ok()),
            version(omp.as_ref().ok())
        )
    }
}

async fn version(executable: Option<&LaunchPlan>) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_identity_environment_names_are_case_insensitive() {
        for key in [
            "orca_terminal_handle",
            "Orca_Pane_Key",
            "orca_pairing_code",
            "orca_orchestration_compatibility_v1",
        ] {
            assert!(is_identity_environment_for_platform(key, Platform::Windows));
            assert!(!is_identity_environment_for_platform(key, Platform::Linux));
        }
        assert!(!is_identity_environment_for_platform(
            "Path",
            Platform::Windows
        ));
        assert!(!is_identity_environment_for_platform(
            "THOUGHSFLOW_ORCA_BIN",
            Platform::Windows
        ));
    }

    #[test]
    fn inherited_runtime_and_workspace_overrides_are_removed_on_every_platform() {
        for key in [
            "ORCA_USER_DATA_PATH",
            "ORCA_WORKSPACE_ID",
            "ORCA_WORKTREE_ID",
            "ORCA_STRUCTURED_SESSION",
            "ORCA_CLI_COMMAND",
            "ORCA_DEV_REPO_ROOT",
            "ORCA_DEV_CLI_INVOCATION",
        ] {
            for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
                assert!(is_identity_environment_for_platform(key, platform));
            }
            assert!(is_identity_environment_for_platform(
                &key.to_ascii_lowercase(),
                Platform::Windows
            ));
        }
        for key in [
            "PATH",
            "APPDATA",
            "XDG_CONFIG_HOME",
            "HOME",
            "THOUGHSFLOW_NODE_BIN",
        ] {
            assert!(!is_identity_environment_for_platform(
                key,
                Platform::Windows
            ));
        }
    }

    #[tokio::test]
    async fn explicit_node_launch_preserves_arguments_without_evaluating_shell_syntax() {
        let mut launch =
            executable::discover(Tool::Node).expect("Node is required by the workspace toolchain");
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("参数 fixture.mjs");
        std::fs::write(
            &script,
            "process.stdout.write(JSON.stringify(process.argv.slice(2)))",
        )
        .unwrap();
        launch.prefix_arguments.push(script.into_os_string());
        let arguments = vec![
            "a \"quoted\" path\\".into(),
            "中文\n$HOME $(echo injected) & echo literal".into(),
            "".into(),
        ];
        let (success, output) = run_process(&launch, &arguments, 5).await.unwrap();
        assert!(success);
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&output).unwrap(),
            arguments
        );
    }

    #[tokio::test]
    async fn invalid_arguments_fail_unambiguously_before_launch() {
        let launch = LaunchPlan {
            program: std::env::temp_dir().join("must-not-be-spawned"),
            prefix_arguments: vec![],
        };
        let error = run_process(&launch, &["invalid\0value".into()], 5)
            .await
            .unwrap_err();
        assert!(!error.ambiguous);
        assert!(error.message.contains("空字符"));
    }
}
