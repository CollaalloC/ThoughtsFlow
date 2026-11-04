# Agent 工作台架构

当前协作基座为 **herdr-core**，不再调用 Orca。设计决定见 [ADR 0004](adr/0004-herdr-core-workbench.md)，协议与验证见 [2026-11-04 核查](upstream/2026-11-04.md)。

| 模块 | 拥有的内容 |
| --- | --- |
| `domain` / Context Compiler | 对话树、上下文选择、不可变 Context Receipt |
| `agents/context.rs` | 显式选定上下文和输出的交接快照及 hash |
| `agents/service.rs` | Mission、操作意图、首条消息确认、准确执行身份与 unknown 处理 |
| `agents/store.rs` | SQLite 事务、操作日志、不可变交接/证据、有界历史摘要 |
| `agents/herdr.rs` | generation 1 JSON-RPC、core 身份、超时和回执大小限制 |
| `features/agents` | 原生模板选择、进展与屏幕观察、人工证据复用 |
| herdr-core / 原生 Agent | 持久终端、模板、状态与消息；模型、工具、Skills、认证和审批 |

先安装 herdr-core，并在它的原生配置目录中准备模板。ThoughtsFlow 不创建或改写模型配置。点击“连接 herdr-core”使用 `thoughtsflow` 专用会话；随后选择已有本地目录、新建协作、选择模板、预览并提交任务。

`THOUGHTSFLOW_HERDR_BIN` 可指定原生 CLI 的绝对路径；默认在 PATH、用户 `.local/bin` 和常见系统目录中寻找 `herdr-core`（Windows 为 `.exe`）。开发 fixture 可显式指定 `.mjs` 与 `THOUGHSFLOW_NODE_BIN`；不支持 `.cmd/.bat`。`THOUGHTSFLOW_HERDR_SESSION` 可显式指定专用会话名，默认 `thoughtsflow`。应用不会自动停止 core。

原生介入示例（Unix）：`HC_SESSION=thoughtsflow herdr-core attach t1`。终端地址必须来自本次回执，不能套用示例的 `t1`。Windows 在同一终端先设置 `$env:HC_SESSION="thoughtsflow"`，再使用实际地址。审批在原生界面处理，工作台不提供强制确认。

提交成功只证明准确终端接纳了首条任务；Agent 自报完成和终端退出均需要审阅。屏幕读取始终 `lines: 0`，不会为了查看历史而向 TUI 滚动。只有已退出、身份准确且属于本工作台的终端可释放，输出需要释放前保存。

旧 Orca 协作只读，现存终端不会被迁移、停止或重启。可把旧证据显式选择进新的 Herdr 任务，旧原始记录与 hash 不变。新任务在选定目录运行，不自动创建 worktree；共享目录不是权限沙箱。进展区最多投影最近 200 次执行，完整交接和证据按每页 50 条浏览。

常用验证：`npm run check`、Rust 全量测试、`npm run test:webview:agents`（固定 fixture）。`TF_HERDR_LIVE=1 npm run test:herdr:live` 使用隔离状态的已安装 core 和无模型测试程序；它不证明真实模型或 Agent 配置可用。

真实 Agent 验证需显式设置 `TF_AGENT_LIVE=1`、`TF_AGENT_LIVE_AGENT_BIN`（OMP 原生可执行文件绝对路径）和可选 `THOUGHTSFLOW_HERDR_BIN`，再运行 `npm run test:webview:agents:live`。它会使用原有模型认证与配置，创建隔离 core、临时模板、两个工作目录与独立会话目录，每个 Agent 设置 300 秒 / 2 USD 的原生运行上限；不覆盖模型、Provider 或审批参数。完整日志只保存在本机临时证据目录。当前 macOS 两阶段通过记录见 [真实 Agent QA](upstream/2026-11-04-real-agent.md)。
