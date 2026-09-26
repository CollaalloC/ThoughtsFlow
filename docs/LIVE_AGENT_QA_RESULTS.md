# 真实 OMP Agent 验证

2026-09-22，用户明确授权接入真实 Agent 后执行。本次通过真实 macOS WKWebView 的 ThoughtsFlow 工作面发起，经 Rust IPC 与本机 Orca 运行时启动 OMP，未使用模拟 CLI。

**结论：单 Agent 短任务的端到端流程通过，耗时 1 分 46.2 秒。**

## 实际环境与任务

- Orca 1.4.206；OMP 18.2.8。
- 运行中的 Orca worker-list 报告模型为 `openai-codex/gpt-6-astra`，沿用用户已有配置。
- Task 为 `task_6eb5c47aee69`，Dispatch 为 `ctx_4defa370e000`，Orchestration Run 为 `run_fa760e504ba6`。
- 在独立 worktree 内只创建 `.thoughsflow-smoke-proof.json`，写入本次随机标记、`sum=42` 和工具实际观察到的工作目录，再用 Python 标准库加载并断言。

## 核验链路

1. ThoughtsFlow 界面创建本地工作区与 Mission，并收到真实启动回执。
2. 真实 OMP transcript 包含 `write` 工具创建 177 字节 JSON 文件的记录，以及 Python `eval` 中的读取和断言。
3. Orca 接受了准确对应 Task / Dispatch 的 `worker_done`，结果为 `succeeded`；任务状态为 `completed`。
4. ThoughtsFlow 界面读取真实输出，并释放已结算执行器。随后 Orca 返回 `terminalState=released`、`liveness=exited`，输出存档状态为 `captured`。
5. 主代理和独立复核代理分别直接读取磁盘文件，用 Python 再次验证 nonce、`17+25==42` 与实际 cwd。受 Git 跟踪的项目文件没有改动，状态仅列出指定的新增验证文件。
6. 最终回执已确认，测试专用协调终端关闭回执为 `ptyKilled: true`。保留验证文件所在的 worktree，便于检查。

产物副本：[real-agent-proof-20260922.json](../output/real-agent-proof-20260922.json)。机器可读摘要：[real-agent-verification-20260922.json](../output/real-agent-verification-20260922.json)。文件 SHA-256 为 `292c62b751b2a03585b7fa285aa62a93102d78f949e3a95f05a317f030d45223`。

真实原文件位于 `/Users/collaalloc/orca/workspaces/ThoughtsFlow/tf-8a0f84e7823d482c8d1e8119dc3072e1/.thoughsflow-smoke-proof.json`。测试应用数据库、操作回执和截图保留在 `/var/folders/c3/wx18vm150r570n3088tz94bc0000gn/T/thoughsflow-real-agent-ZMfEMR/`。

## 实测发现与修复

真实 Orca 的 task-list 仅连接活动 Dispatch，任务结束后 `dispatch_id` 为空或不再返回。原模拟数据始终保留这个字段，无法暴露已完成任务丢失输出和释放入口的问题。

已按真实语义修复：有活动 ID 时精确关联；已结算任务只有在唯一 worker 与任务结果一致时才关联；活动任务缺 ID、多个尝试和身份冲突仍不猜测。模拟 CLI 与 Rust 回归同步对齐，Agent 测试共 28 项通过，完整 Rust 回归为 248 项库测试和 3 项原生 IPC 测试。

首次 live 测试在 Agent 工作面懒加载完成前选择控件，停在 UI 等待，尚未创建 Mission 或派发 Agent。改为等待工作面出现后再操作；随后只有一个真实 OMP Task 被启动，没有盲目重派。

## 结论边界

- 此次只证明一个短任务的真实模型、工具、回传与释放流程；真实多 Agent 并发、依赖图和长时间重启恢复仍需独立验收。
- OMP 的 Python 文件断言已输出 PASS；同一次调用随后读取指南的临时输出时发生 `FileNotFoundError`，模型改用正常 artifact 读取后继续。最终文件由两个独立检查再次验证通过，不能将这次运行表述为每个工具调用都无错误。
- 归档文本包含截断提示，不是完整的无损 transcript；模型名称依据释放前的运行时观察，释放后该字段不再可用。
- 任务结果进入 Decision Packet 仍是后续产品能力。本报告是测试证据，不会把 OMP 输出伪造成 ThoughtsFlow 的 Provider Receipt。
