# Agent 协作验证记录

验证日期：2026-09-22。环境：本机 macOS、Orca 1.4.206、OMP 18.2.8；原生测试使用 WebKit 605.1.15。

此文件保留首次接入时的 fixture 验证记录。同日后续架构优化与真实单 Agent 验证分别见 [ARCHITECTURE_QA_RESULTS.md](ARCHITECTURE_QA_RESULTS.md) 和 [LIVE_AGENT_QA_RESULTS.md](LIVE_AGENT_QA_RESULTS.md)。

## 通过的验证

| 检查 | 结果 |
| --- | --- |
| 前端组件与既有回归 `npm run test:run` | 14 个文件，106 项通过 |
| Provider fixture | 9 项通过 |
| Orca CLI fixture | 1 项完整控制流程测试通过 |
| TypeScript / Vite 生产构建 | 通过 |
| WebView 测试 TypeScript | 通过 |
| Rust `cargo test --all-targets --all-features` | 236 项库测试与 3 项原生 IPC 集成测试通过 |
| Rust `cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| Rust fmt / git diff whitespace | 通过 |
| 独立协议与实现审查 | 已修复发现的问题，最终无待修复 P1/P2 |

## 真实 WKWebView 到 CLI 子进程

用 `node tests/webview/run-agent-journey.mjs` 启动隔离应用数据目录和模拟 Orca 可执行文件，约 42 秒通过完整旅程：创建 ThoughsFlow 工作区、选择代码项目、创建协作目标、先后派发两个独立任务并同时显示、分别回复问题、读取任务输出、分别释放执行器，最后重载 WebView 并确认协作和释放状态恢复。表单、React、DesktopBridge、Tauri 命令、真实 SQLite 迁移和进程 argv 链路都参与运行。

截图见 [原生 Agent 工作面](../output/agent-workspace-native.png)。模拟输出明确标注 `no model was called`；本测试不读写业务项目、不联系模型、不创建真实 worktree。

测试驱动打印了 `tauri-driver not found`、窗口 IPC 预探测和结束时 mock-store 清理警告；配置实际使用 embedded driver，WebKit 会话与全部旅程断言均成功。这些警告不作为产品功能成功或失败的判断依据。

## 实际 Orca 控制链路

本机实际执行并验证了：运行时启动与 capability 检测、已有项目清单、专用空闲协调终端创建、从后台显式 `--from` 创建 Run、指定 Run 的 Task/worker 列表和只读邮箱。测试协调终端随后关闭，回执确认 `ptyKilled: true`，没有启动任何模型 worker。

## 边界

- 此次初始 fixture 验证没有真实 OMP 模型任务完成证据；后续用户授权的真实单任务结果另见上述记录。
- 自动任务拆解、依赖调度、代码合并、结果采纳到 Decision Packet 不在本次范围。
- 运行中任务停止和工具审批仍使用 Orca/OMP 原生界面。
- 只读邮箱达 100 条时暂停新增任务并提示在 Orca 处理；不静默消费消息。
- 未验证 Windows/Linux 实机运行。
