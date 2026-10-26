# OMP / Orca 上游兼容跟踪

最后核查：2026-10-26。机器可读基线见 [baseline.json](baseline.json)，本轮契约与回归证据见 [2026-10-26 核查记录](2026-10-26.md)。已设置本任务的每周一 10:00 跟踪；只有重要变化、兼容性问题、已完成集成或需要决定的事项才通知。

| 上游 | 本机已安装 | 核查时最新稳定版 | 官方来源 |
| --- | --- | --- | --- |
| OMP | 18.12.5，安装元数据核实 | 18.12.6，2026-10-21 | [Release](https://github.com/can1357/oh-my-pi/releases/tag/v18.12.6) |
| Orca | 1.4.237 | 1.4.241，2026-10-26 | [Release](https://github.com/stablyai/orca/releases/tag/v1.4.241) |

本轮没有升级运行时、修改模型凭据或调用真实模型，也没有执行 OMP CLI 或模型目录探测。Orca CLI 可用，运行时为 `not_running`，因此未进行在线能力握手。上次真实 OMP 单任务验证使用 18.2.8 / Orca 1.4.206，不能替代新版实测。

## 当前兼容矩阵

| 路径或能力 | 已核实契约与当前实现 | 验证边界 |
| --- | --- | --- |
| Orca supervised OMP worker | 保留 `orchestration.contract.v1`、contractVersion 1，任务默认继承 OMP harness | 当前版本未进行真实任务执行 |
| 本次任务模型选择 | 已接入可选 `--model`，选择器冻结在交接中；不支持 `--effort` | 通用 launch-preferences 能力只是必要条件；还需严格核验 requested/effective 回执，不证明实际使用模型 |
| 原生模型目录 | 尚未接入；用户可填写已有 `provider/model` 或角色选择器 | `omp models` 不是已核实的只读接口，不自动运行 |
| 身份与进程归属隔离 | 清除继承的 `ORCA_AGENT_SESSION_ID` 与 `ORCA_AGENT_SESSION_SPAWN_TOKEN` | spawn token 是子进程归属信息，不是 `--from` 调用者身份 |
| worker 输出与证据 | 按 Dispatch 读取 terminal/transcript；按来源判断 `limited`，只对推进的游标显示下一页 | 保存的是一页不可变观察，不推断任务成功，不修改历史证据 |
| Direct OMP RPC | 尚无直接适配器；完整会话接入须先明确所有权、审批和恢复 | 不与 Orca 同时控制同一会话 |
| OMP HTTP Gateway | 保留已有兼容入口，需单独配置 Auth Broker | 不自动继承本机 Agent 凭据，不作为 Agent harness；stdio Gateway 尚未接入 |

模型请求默认省略新增字段，旧序列化和交接 hash 不变，数据库 schema 仍为 10。明确指定模型后，工作台不会在不支持、回执不完整或连接失败时删掉参数改用默认模型；结果不明确就保留 unknown，禁止自动重派。模型目录暂缓的副作用依据与固定源码链接见[本轮记录](2026-10-26.md)。

## 已吸收的基础能力

- Orca 跨平台启动约定：Windows 使用原生 `resources/bin/orca.exe`，Linux 使用 `orca-ide`，macOS 支持 PATH 和用户级安装目录；不经过 `.cmd/.bat` 重新解释任务正文。
- 基础编排能力以运行时 `orchestration.contract.v1` 为准；离线和连接失败不等于某能力已证实不支持。读取执行器输出继续使用 Dispatch 与结构化 `worker-read`，不要求执行器一定有 PTY。
- 兼容 Provider 入口保留 HTTP Gateway 的 `provider/model` 目录和多厂商路由。目录按声明的显示名、上下文大小和适用模型类型投影，前端最多渲染 100 个候选；这不是 Agent 原生目录接入。

## 后续跟踪重点

依据 [Agent 工作台架构决定](../adr/0003-agent-workbench-context-handoffs.md)，优先复用原生 harness、模型与配置继承、会话分叉和恢复、结构化输出及 Orca 编排能力，不建设第二套 Provider、认证、工具或 Skills 生态。

| 更新方向 | 价值与当前取舍 |
| --- | --- |
| OMP 原生模型目录与会话模型控制 | 需使用明确允许的会话接口或可验证的只读探测；完整模型对象先白名单投影，避免把连接配置暴露给界面 |
| OMP `prompt_result`、`session_settled` / `isSettled` | 后续 Direct RPC 必须区分请求接纳、提示词结果与后台工作结算；当前生命周期继续由 Orca 管理 |
| OMP `open_session`、`fork`、`set_event_filter`、`--no-ui` | 会话恢复、分叉与有界事件传输；先解决会话归属及未知结果恢复，不自动重发 prompt |
| Orca 原生会话与模型目录 | 原生聊天路径与 supervised worker 分开核实，不能将某一路径的支持推断为另一条路径已支持 |
| Orca WSL relay、Linux 生命周期与恢复 | 保持平台适配；对应 runner 或实机执行后才提高运行验证等级 |

较早的 Gateway、会话和工作台方向核查保留在 [2026-10-06](2026-10-06.md)；更早的性能与兼容基线见 [2026-09-28](2026-09-28.md)。历史记录中的版本结论只适用于当时核查的版本。

## 验证边界

本轮本地通过前端/fixture 回归、Rust 全量测试、原生 MockRuntime IPC、固定 CLI 延迟基准及 macOS 原生 WebView Agent journey，涵盖默认继承、可选选择器、准确 argv、预览与离线历史。所有模型选择测试使用固定 fixture，没有启动真实 OMP worker。

[历史 Desktop CI 三平台运行](https://github.com/CollaalloC/ThoughtsFlow/actions/runs/36253970625)只证明提交 `3e24dafa09ece6a84ffef74957604c4f0c5aa280` 的核心检查与原生 MockRuntime IPC。本轮只静态检查三平台工作流，未触发远程 runner。Windows/Linux 的真实 Orca + OMP 执行仍未验证；本地 fixture、GUI 或 CI 配置检查均不提升这一状态。

## 每次跟踪的执行顺序

1. 读取本基线、`CONTEXT.md`、最新架构决定和当前 Git 状态，保留用户未提交的工作。
2. 核对官方稳定 release、固定版本源码和本机 CLI 或安装元数据；记录来源和核查时间，避免探测命令的隐含写入。
3. 把变化归为可直接复用、需能力门控、需独立适配或暂不相关；不依据版本号或通用能力名猜测全部支持范围。
4. 对可复用变化补契约测试并实现最小改动；性能变化保留相同条件下的前后证据。
5. 跑本地回归，按功能提交，并更新基线；不自动推送、发布、修改凭据或安装上游应用。
6. Windows/Linux 的状态只由对应 CI / 实机记录提升。真实 Agent 验证、模型费用和账号权限不由目录、fixture 或构建结果推断。
