# OMP / Orca 上游兼容跟踪

最后核查：2026-09-26。机器可读基线见 [baseline.json](baseline.json)。已设置本任务的每周一 10:00 跟踪；只有重要变化、兼容性问题、已完成集成或需要决定的事项才通知。

| 上游 | 本机已安装 | 核查时最新稳定版 | 官方来源 |
| --- | --- | --- | --- |
| OMP | 18.3.2（开始核查时为 18.2.10） | 18.3.2，2026-09-26 | [Release](https://github.com/can1357/oh-my-pi/releases/tag/v18.3.2) |
| Orca | 1.4.207 | 1.4.212，2026-09-25 | [Release](https://github.com/stablyai/orca/releases/tag/v1.4.212) |

此轮没有执行运行时升级命令，也没有读取或改写模型凭据。开始核查时 OMP 为 18.2.10，收尾 CLI 复核时已是 18.3.2；这里保留两次观察，避免使用过期安装状态。上次真实 OMP 单任务验证使用 18.2.8 / Orca 1.4.206，不能替代新版实测。

## 已吸收的能力

- OMP Auth Gateway：通过现有模型请求接口接入 `provider/model` 目录和多厂商路由，保持正常 Model Run 与 Agent Mission 分离。需要已配置 Auth Broker，不直接继承本机 `agent.db`。
- 动态模型目录：读取声明的显示名与上下文大小，过滤已知不适用的模型类型，未知字段维持未知；前端按输入筛选且最多渲染 100 个候选。
- Orca 跨平台启动约定：Windows 使用原生 `resources/bin/orca.exe`，Linux 使用 `orca-ide`，macOS 支持 PATH 和用户级安装目录；不经过 `.cmd/.bat` 重新解释任务正文。
- 能力验证继续以运行时 `orchestration.contract.v1` 为准；离线和连接失败不等于某能力已证实不支持。读取执行器输出继续使用 Dispatch 与结构化 `worker-read`，不要求执行器一定有 PTY。

## 有价值但尚未直接接入的更新

| 更新 | 价值 | 当前取舍 |
| --- | --- | --- |
| OMP 18.3.1 `prompt_result`、`session_settled` / `isSettled` | 区分提示词完成与整个会话后台工作结束 | 只有另建 Direct OMP adapter 时才使用；目前生命周期由 Orca 管理，不争用其 RPC 连接 |
| OMP `open_session`、`set_event_filter`、`--no-ui` | 宿主会话恢复与降低事件传输量 | 记录在下一适配阶段，当前软件没有 Direct OMP adapter |
| OMP 18.3.2 Windows TEMP、原始输出链接和 compaction 修复 | 提升上游运行可靠性 | 作为升级候选，先验证版本契约和实际回归，再升级兼容基线 |
| Orca 原生聊天 OMP 模型目录 / 模型切换 | 真实模型选择能力 | 不等同于 supervised worker 的模型覆盖；1.4.212 的 worker-start 仍不支持 OMP `--model` / `--effort`，界面不显示无效选择器 |
| Orca WSL relay、Linux 孤儿进程释放、结构化任务状态修复 | 跨平台与恢复可靠性 | 对应平台 runner 执行后才提升验证状态 |

来源：[OMP RPC](https://github.com/can1357/oh-my-pi/blob/v18.3.2/docs/rpc.md)、[OMP Gateway](https://github.com/can1357/oh-my-pi/blob/v18.3.2/packages/coding-agent/src/cli/auth-gateway-cli.ts)、[Orca worker 参数](https://github.com/stablyai/orca/blob/v1.4.212/src/cli/specs/orchestration-worker-specs.ts)、[Orca OMP 原生聊天目录](https://github.com/stablyai/orca/blob/v1.4.212/src/shared/agent-session-option-catalog-omp.ts)。

## 每次跟踪的执行顺序

1. 读取本基线和当前 Git 状态，保留用户未提交的工作。
2. 核对官方稳定 release、固定版本源码和本机 CLI；记录来源和核查时间。
3. 把变化归为可直接复用、需能力门控、需独立适配或暂不相关；不依据版本号猜测全部能力。
4. 对可复用变化先补契约测试，再实现最小改动；性能变化保留相同条件下的前后证据。
5. 跑本地回归，按功能提交，并更新基线；不自动推送、发布、修改凭据或安装上游应用。
6. Windows/Linux 的状态只由对应 CI / 实机记录提升。模型费用和账号权限不由离线目录或 fixture 测试推断。
