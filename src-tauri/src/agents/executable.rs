//! Resolve trusted local runtimes without evaluating a shell command.
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub(super) fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Orca,
    Omp,
    Node,
}

impl Tool {
    fn variable(self) -> &'static str {
        match self {
            Self::Orca => "THOUGHSFLOW_ORCA_BIN",
            Self::Omp => "THOUGHSFLOW_OMP_BIN",
            Self::Node => "THOUGHSFLOW_NODE_BIN",
        }
    }

    fn filename(self, platform: Platform) -> &'static str {
        match (self, platform) {
            (Self::Orca, Platform::Windows) => "orca.exe",
            (Self::Orca, Platform::Linux) => "orca-ide",
            (Self::Orca, Platform::MacOs) => "orca",
            (Self::Omp, Platform::Windows) => "omp.exe",
            (Self::Omp, _) => "omp",
            (Self::Node, Platform::Windows) => "node.exe",
            (Self::Node, _) => "node",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LaunchPlan {
    pub program: PathBuf,
    pub prefix_arguments: Vec<OsString>,
}

impl LaunchPlan {
    pub(super) fn validate_arguments(
        &self,
        platform: Platform,
        arguments: &[String],
    ) -> Result<(), String> {
        if arguments.iter().any(|argument| argument.contains('\0')) {
            return Err("运行时参数不能包含空字符。".into());
        }
        if platform == Platform::Windows {
            // An upper bound on Rust's Windows argv quoting, including spaces,
            // quotes, doubled backslashes, the program name and the final NUL.
            // This is a size check only; Command still owns argument encoding.
            let length = std::iter::once(self.program.as_os_str())
                .chain(self.prefix_arguments.iter().map(OsString::as_os_str))
                .chain(arguments.iter().map(OsStr::new))
                .fold(1_usize, |length, argument| {
                    length.saturating_add(windows_argument_bound(argument))
                });
            if length >= 32_767 {
                return Err(
                    "任务正文超过 Windows 进程参数长度限制，请缩短后重试。命令尚未启动。".into(),
                );
            }
        }
        Ok(())
    }
}

fn windows_argument_bound(argument: &OsStr) -> usize {
    #[cfg(windows)]
    let units = {
        use std::os::windows::ffi::OsStrExt;
        argument.encode_wide()
    };
    #[cfg(not(windows))]
    let text = argument.to_string_lossy();
    #[cfg(not(windows))]
    let units = text.encode_utf16();
    units.fold(3_usize, |length, unit| {
        length.saturating_add(if unit == 0x22 || unit == 0x5c { 2 } else { 1 })
    })
}

struct SearchPaths {
    platform: Platform,
    path: Vec<PathBuf>,
    home: Option<PathBuf>,
}

impl SearchPaths {
    fn from_environment() -> Self {
        let platform = Platform::current();
        Self {
            platform,
            path: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
            home: std::env::var_os(if platform == Platform::Windows {
                "USERPROFILE"
            } else {
                "HOME"
            })
            .map(PathBuf::from),
        }
    }

    fn directories(&self) -> Vec<PathBuf> {
        let mut directories = self.path.clone();
        // Windows installers permit custom directories. The desktop Orca.exe
        // is not its CLI: use the registered resources/bin PATH entry or an
        // explicit native CLI path rather than guessing installation roots.
        if self.platform != Platform::Windows {
            if let Some(home) = &self.home {
                directories.push(home.join(".local/bin"));
            }
            if self.platform == Platform::MacOs {
                directories.push("/opt/homebrew/bin".into());
            }
            directories.push("/usr/local/bin".into());
            directories.push("/usr/bin".into());
        }
        directories
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileStatus {
    Missing,
    File,
    Executable,
}

fn file_status(path: &Path) -> FileStatus {
    let Ok(metadata) = path.metadata() else {
        return FileStatus::Missing;
    };
    if !metadata.is_file() {
        return FileStatus::Missing;
    }
    if matches!(extension(path).as_str(), "js" | "mjs") {
        return FileStatus::File;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return FileStatus::File;
        }
    }
    FileStatus::Executable
}

pub(super) fn discover(tool: Tool) -> Result<LaunchPlan, String> {
    let explicit = std::env::var_os(tool.variable()).map(PathBuf::from);
    let node = std::env::var_os(Tool::Node.variable()).map(PathBuf::from);
    resolve(
        tool,
        explicit.as_deref(),
        node.as_deref(),
        &SearchPaths::from_environment(),
        &file_status,
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}

fn resolve(
    tool: Tool,
    explicit: Option<&Path>,
    node: Option<&Path>,
    paths: &SearchPaths,
    probe: &impl Fn(&Path) -> FileStatus,
) -> Result<LaunchPlan, String> {
    if let Some(path) = explicit {
        if !path.is_absolute() {
            return Err(format!("{} 必须是可执行文件的绝对路径。", tool.variable()));
        }
        if matches!(extension(path).as_str(), "cmd" | "bat") {
            return Err(format!(
                "{} 不支持 .cmd/.bat；请使用原生 CLI .exe 或显式 .js/.mjs 文件，避免重新解释任务正文。",
                tool.variable()
            ));
        }
        if tool != Tool::Node && matches!(extension(path).as_str(), "js" | "mjs") {
            if probe(path) == FileStatus::Missing {
                return Err(format!("{} 指定的脚本不存在。", tool.variable()));
            }
            let mut launch = resolve(Tool::Node, node, None, paths, probe)?;
            launch.prefix_arguments.push(path.as_os_str().to_owned());
            return Ok(launch);
        }
        if is_native(path, paths.platform, probe) {
            return Ok(LaunchPlan {
                program: path.into(),
                prefix_arguments: vec![],
            });
        }
        return Err(format!(
            "{} 指定的文件不存在、没有执行权限或不是此平台支持的原生可执行文件。",
            tool.variable()
        ));
    }
    paths
        .directories()
        .into_iter()
        .map(|directory| directory.join(tool.filename(paths.platform)))
        .find(|path| is_native(path, paths.platform, probe))
        .map(|program| LaunchPlan {
            program,
            prefix_arguments: vec![],
        })
        .ok_or_else(|| {
            format!(
                "未找到 {}，请安装运行时或设置 {} 为可执行文件的绝对路径。",
                tool.filename(paths.platform),
                tool.variable()
            )
        })
}

fn is_native(path: &Path, platform: Platform, probe: &impl Fn(&Path) -> FileStatus) -> bool {
    path.is_absolute()
        && !matches!(extension(path).as_str(), "cmd" | "bat" | "js" | "mjs")
        && (platform != Platform::Windows || extension(path) == "exe")
        && probe(path) == FileStatus::Executable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(platform: Platform) -> SearchPaths {
        let root = std::env::temp_dir().join("ThoughsFlow 平台 fixtures");
        SearchPaths {
            platform,
            path: vec![root.join("first"), root.join("second")],
            home: Some(root.join("home")),
        }
    }

    #[test]
    fn all_platforms_prefer_explicit_then_path_then_platform_fallbacks() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let paths = paths(platform);
            let filename = Tool::Orca.filename(platform);
            let explicit = paths.path[1].join(filename);
            let all_executable = |_: &Path| FileStatus::Executable;
            assert_eq!(
                resolve(Tool::Orca, Some(&explicit), None, &paths, &all_executable)
                    .unwrap()
                    .program,
                explicit
            );
            assert_eq!(
                resolve(Tool::Orca, None, None, &paths, &all_executable)
                    .unwrap()
                    .program,
                paths.path[0].join(filename)
            );
            if platform == Platform::Windows {
                assert!(resolve(Tool::Orca, None, None, &paths, &|_| FileStatus::Missing).is_err());
                continue;
            }
            let fallback = paths
                .home
                .as_ref()
                .unwrap()
                .join(".local/bin")
                .join(filename);
            let only_fallback = |candidate: &Path| {
                if candidate == fallback {
                    FileStatus::Executable
                } else {
                    FileStatus::Missing
                }
            };
            assert_eq!(
                resolve(Tool::Orca, None, None, &paths, &only_fallback)
                    .unwrap()
                    .program,
                fallback
            );
        }
    }

    #[test]
    fn invalid_explicit_path_never_falls_back_and_relative_path_entries_are_ignored() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let mut paths = paths(platform);
            let invalid = paths.path[0].join(Tool::Orca.filename(platform));
            for status in [FileStatus::Missing, FileStatus::File] {
                let probe = |candidate: &Path| {
                    if candidate == invalid {
                        status
                    } else {
                        FileStatus::Executable
                    }
                };
                assert!(resolve(Tool::Orca, Some(&invalid), None, &paths, &probe).is_err());
            }
            assert!(
                resolve(
                    Tool::Orca,
                    Some(Path::new("relative")),
                    None,
                    &paths,
                    &|_| FileStatus::Executable
                )
                .is_err()
            );
            paths.path.insert(0, "relative".into());
            assert_eq!(
                resolve(Tool::Orca, None, None, &paths, &|_| FileStatus::Executable)
                    .unwrap()
                    .program,
                paths.path[1].join(Tool::Orca.filename(platform))
            );
        }
    }

    #[test]
    fn windows_batch_files_are_rejected_without_searching_or_launching_a_shell() {
        let paths = paths(Platform::Windows);
        for suffix in ["cmd", "CMD", "bat", "BAT"] {
            let script = paths.path[0].join(format!("orca.{suffix}"));
            let result = resolve(Tool::Orca, Some(&script), None, &paths, &|_| {
                panic!("batch files must be rejected before probing")
            });
            assert!(result.unwrap_err().contains(".cmd/.bat"));
        }
    }

    #[test]
    fn explicit_scripts_use_fixed_node_and_preserve_the_script_as_one_argument() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let paths = paths(platform);
            let script = paths.path[0].join("脚本 with spaces.mjs");
            let node = paths.path[1].join(Tool::Node.filename(platform));
            let probe = |candidate: &Path| {
                if candidate == script {
                    FileStatus::File
                } else if candidate == node {
                    FileStatus::Executable
                } else {
                    FileStatus::Missing
                }
            };
            assert_eq!(
                resolve(Tool::Orca, Some(&script), Some(&node), &paths, &probe).unwrap(),
                LaunchPlan {
                    program: node.clone(),
                    prefix_arguments: vec![script.as_os_str().to_owned()]
                }
            );
            assert!(
                resolve(
                    Tool::Omp,
                    Some(&script),
                    Some(&paths.path[0].join("missing")),
                    &paths,
                    &probe
                )
                .is_err()
            );
        }
    }

    #[test]
    fn windows_length_guard_counts_utf16_and_quoting_before_process_start() {
        let launch = LaunchPlan {
            program: PathBuf::from("C:/程序 文件/orca.exe"),
            prefix_arguments: vec!["a\\\"b".into()],
        };
        assert!(
            launch
                .validate_arguments(
                    Platform::Windows,
                    &["中文正文\n$HOME & echo literal".into()]
                )
                .is_ok()
        );
        assert!(
            launch
                .validate_arguments(Platform::Windows, &["a".repeat(32_767)])
                .is_err()
        );
        assert!(
            launch
                .validate_arguments(Platform::Windows, &["😀".repeat(17_000)])
                .is_err()
        );
        assert!(
            launch
                .validate_arguments(Platform::Windows, &["\\".repeat(17_000)])
                .is_err()
        );
        assert!(
            launch
                .validate_arguments(Platform::Linux, &["a".repeat(64_000)])
                .is_ok()
        );
        assert!(
            launch
                .validate_arguments(Platform::MacOs, &["invalid\0value".into()])
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_files_need_execute_permission_but_explicit_node_scripts_do_not() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("orca");
        std::fs::write(&program, "fixture").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(file_status(&program) == FileStatus::File);
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(file_status(&program) == FileStatus::Executable);
    }
}
