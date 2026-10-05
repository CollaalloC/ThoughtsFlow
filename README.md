# ThoughtsFlow

**管理上下文，让现有 Agent 在可追溯的工作流中协作。**

ThoughtsFlow 是一个本地优先的 Agent 工作台。它负责图形化选择上下文、向 Agent 交接任务、保存输出证据并组织后续工作；模型连接、工具、Skills、MCP 和审批沿用现有 Agent harness。当前执行路径复用本机 Orca 与 OMP，无需在 ThoughtsFlow 再配置 Provider。

[下载测试版](https://github.com/CollaalloC/ThoughtsFlow/releases/tag/v0.1.0-beta.2) · [工作台架构](docs/adr/0003-agent-workbench-context-handoffs.md) · [参与贡献](CONTRIBUTING.md) · [English overview](#english-overview)

> 最新已发布安装包仍为 `v0.1.0-beta.2`。以下 Agent 主入口与上下文交接描述当前源码的未发布改动；旧安装包不包含它们。安装包、平台和已知限制以发布页为准。

## 能做什么

- **复用已有 Agent。** 直接创建工作区并连接 Orca，任务沿用 OMP 已有模型、认证与工具配置。
- **显式交接上下文。** 在图上选择准确的历史路径，为任务选择当前上下文或已保存的 Agent 证据，预览后再派发。内容或版本变化后需要重新预览。
- **保存可追溯的输出。** 保存指定 Dispatch 的一页输出观察及 hash，离线查看并用于后续任务。部分输出和任务成功分别标明。
- **组织人工工作流。** 查看交接与证据的来源关系，人工决定后续工作；未知派发结果不会自动重试。
- **保留历史推演能力。** 兼容直连对话中的准确回答分支、固定/排除、摘要检查点、Context Receipt、路线比较和决策文档导出。

## 安装与首次使用

从 [Releases](https://github.com/CollaalloC/ThoughtsFlow/releases) 下载与你的操作系统和 CPU 架构匹配的安装包，并核对发布页提供的校验和。只有实际列出的文件才是该版本已发布的安装包；支持构建某个平台不等于已完成该平台的使用验证。

| 安装包 | 使用方式 |
| --- | --- |
| macOS `.dmg` | 打开磁盘映像，将 ThoughtsFlow 拖入 Applications 后启动 |
| Windows `.exe` | 运行安装向导，完成后从开始菜单启动 |
| Linux `.deb` | 在兼容的 Debian/Ubuntu 系统上使用系统包管理器安装 |

早期 macOS/Windows 包未经过正式分发签名，macOS 包也未公证，系统可能提示无法验证开发者。确认下载来源和校验和后，按系统提供的单应用确认流程处理；不要为安装本项目全局关闭系统安全保护。安装包范围、签名状态和构建方式见[发布说明](docs/RELEASING.md)。

1. 安装并配置 [Orca](https://github.com/stablyai/orca) 和 [OMP](https://github.com/can1357/oh-my-pi)，确认 OMP 原生执行可用，在 Orca 登记代码项目。
2. 打开 ThoughtsFlow 的 **Agent 工作台**，创建或选择本地工作区，再选择项目和协作目标。
3. 填写任务范围与验收要求，按需选择当前图谱上下文和已有输出证据，查看交接预览后启动。
4. 查看任务与执行状态，在需要时回复协作问题。保存输出观察，审阅后选择它作为下一任务的输入。

主路径无需在 ThoughtsFlow 输入 API Key、Base URL 或 Provider。当前 Agent 模型选择沿用 OMP 配置；Orca worker 未支持 OMP 模型覆盖时，工作台不会显示无效选择器。ThoughtsFlow 不附带模型，也不提供调用额度。

## 兼容入口：直连模型

已有直连对话与 Provider 设置继续保留，用于原有工作区和明确需要的独立模型请求；它们不承担 Agent harness。此路径可独立于 Orca/OMP 运行，需要配置相应 Provider。

| 连接方式 | 当前状态 |
| --- | --- |
| OpenAI / 通用 OpenAI-compatible、OpenRouter | Chat Completions 流式请求 |
| Anthropic、Google Gemini、Ollama | 各自的原生流式协议 |
| DeepSeek、xAI、Mistral、Groq、Together、Moonshot、SiliconFlow | 预置兼容端点，复用 Chat Completions 协议 |
| Qwen 北京 / 新加坡、Z.AI | 预置兼容端点，当前手动填写模型 ID |
| OMP Gateway | 可选连接，需另行配置 OMP Auth Broker 与网关 token |
| Azure OpenAI | 仅有模板，当前不可运行 |

共 **18 个模板**，其中包含地区变体与尚不可运行的 Azure 模板。模板并不代表每家供应商的所有模型、订阅、地区和参数都经过真实账号验证。OAuth、Bedrock、Vertex ADC 目前没有原生接入。

当前接入的 OMP **HTTP** Gateway 不会自动继承本机 OMP 登录或 `models.yml`，也不代表完整 Agent 执行。通过网关请求时，Receipt 记录 ThoughtsFlow 发给网关的内容，不代表最终供应商请求。上游新增的 stdio Gateway 尚未接入。配置条件见[模型连接指南](docs/MODEL_CONNECTIONS.md)。

## Orca + OMP 的职责

先分别安装并配置 [Orca](https://github.com/stablyai/orca) 和 [oh-my-pi / OMP](https://github.com/can1357/oh-my-pi)，在 Orca 中登记代码项目，并为 OMP 配好模型与认证。

任务由 Orca 在独立工作区中启动 OMP；ThoughtsFlow 查看进度、读取输出并回复协作问题。任务结果由 Orca 的准确 Task/Dispatch 关联确认，执行器退出不等于成功。

- Agent 使用 OMP 自己的配置。普通对话历史和已有证据只有经显式选择、预览后才随任务交接。
- 交接 hash 覆盖工作台发给 harness 的文本；不声称包含 Agent 内部提示、工具结果或所有模型请求。已保存输出是一页观察，分页末尾也不能单独证明完整或成功。
- 任务由人工拆分和审查；当前不会自动合并代码或把结果自动采纳到 Decision Packet。
- 工具审批及停止正在运行的任务继续在 Orca/OMP 原生界面处理。
- Git worktree 提供代码隔离，**不提供操作系统权限沙箱**。
- 连接中断或操作结果不明时，不会自动重复派发任务。

CLI 路径、运行时版本与恢复语义见 [Agent 架构](docs/AGENT_ARCHITECTURE.md)、[跨平台说明](docs/CROSS_PLATFORM.md)和[上游兼容跟踪](docs/upstream/README.md)。Orca 与 OMP 是分别安装的外部运行时，本项目安装包不捆绑它们。

## 数据与隐私

工作区、回答、交接快照、输出证据、兼容设置和操作回执保存在本机 SQLite；导出文件也保存在本机。数据库及导出文件**没有透明加密**，请按需要使用操作系统磁盘加密和备份。

主 Agent 路径的认证由原生 Agent 管理。仅在使用兼容 Provider 入口时，API Key 提交后存于当前 Rust 进程内存，不写入数据库、Receipt 或导出文件；退出应用后需要重新输入。多个命名凭据由用户显式切换，不会自动轮换。

“本地优先”不表示模型请求始终留在本机。使用远程模型时，选定的上下文会发往配置的端点；发送前可以检查目标 Host 和内容。模型服务与外部 Agent 运行时各自适用其隐私、计费和权限规则。

异常退出后，未结束的模型请求会标为 `interrupted`，保留已经写入的部分输出，不自动重试。安全边界和漏洞披露方式见 [SECURITY.md](SECURITY.md)。

项目曾使用 ThoughsFlow 拼写。为继续读取早期版本的数据，应用标识 `io.thoughsflow.desktop`、数据库名 `thoughsflow.sqlite3` 和既有 `THOUGHSFLOW_*` 环境变量保持兼容。新版本会在事务中迁移默认模型配置标记，保留原值；历史 Receipt、原始回执和 hash 不变。数据库文件仍在原路径，详见[命名兼容说明](docs/NAME_COMPATIBILITY.md)。

## 从源码运行

需要 **Node.js 24**、**Rust 1.97.1** 和当前系统的 [Tauri 2 前置依赖](https://v2.tauri.app/start/prerequisites/)。Rust 版本由 `rust-toolchain.toml` 固定。Windows 需要相应的 C++ 构建工具与 WebView2，Linux 需要 WebKitGTK 等系统库，macOS 需要 Xcode Command Line Tools；详细安装步骤以 Tauri 文档为准。

```bash
git clone https://github.com/CollaalloC/ThoughtsFlow.git
cd ThoughtsFlow
npm ci
npm run tauri:dev
```

构建当前系统的安装包：

```bash
npm run tauri -- build
```

安装包输出到 Cargo target 目录下的 `release/bundle/`，默认位于 `src-tauri/target/release/bundle/`。单独运行 `npm run dev` 只启动前端资源服务；完整应用需要 Tauri 的 Rust 后端。

主要检查：

```bash
npm run check
npm run test:webview:typecheck
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
```

三平台 CI 与独立的原生 WebView fixture 测试已配置。真实模型测试必须显式启用，可能产生模型费用或运行外部 Agent；普通测试不需要模型密钥。测试分层和执行方式见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 架构与边界

```text
React / TypeScript UI
        │ typed DesktopBridge
        ▼
Tauri commands → Rust application services → domain + ports
        ├─ SQLite：工作区、精确回答树、交接与不可变证据
        ├─ ContextCompiler：图谱路径与显式选择
        ├─ Provider adapters：兼容直连请求与目录
        ├─ Filesystem adapter：决策文档导出
        └─ Orca adapter → 本机 Orca → OMP workers
```

上下文图谱通过显式交接连接 Agent 执行。界面不直接访问数据库或执行任意 shell 命令；旧回执不会按当前配置重新计算，输出证据不会伪造为历史 Model Run。

当前还没有通用 Agent 原生聊天界面、云同步、多人协作、附件/RAG、完整全文搜索、可恢复的工作区导入包或自动任务依赖调度。图上的证据关联用于人工编排。完整会话与模型目录接入以 OMP Direct RPC 等已验证接口为后续路线，Agent 工具能力继续来自原生 harness。

进一步阅读：[产品构想](PRODUCT_BLUEPRINT.md) · [构想与实现对照](docs/VISION_ALIGNMENT.md) · [架构路线](docs/PLATFORM_MODEL_ROADMAP.md) · [最近的性能测量](docs/PERFORMANCE_REFRESH_STORAGE_20260926.md) · [变更记录](CHANGELOG.md)

## 致谢与许可证

ThoughtsFlow 的设计深入学习了 **[oh-my-pi](https://github.com/can1357/oh-my-pi)** 的 Agent 执行、模型接入、会话树与上下文管理，以及 **[Orca](https://github.com/stablyai/orca)** 的多 Agent 编排和工作区协作方式。感谢这些项目及其贡献者公开设计与实现。

ThoughtsFlow 是独立项目，与上述项目没有官方隶属或背书关系。代码许可见 [LICENSE](LICENSE)；上游许可证、参考来源和第三方声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。外部 Orca、OMP 运行时仍分别遵循各自的许可证与使用条款。

## English overview

ThoughtsFlow is a local-first Agent workbench for graphical context management and workflow orchestration. The primary workflow reuses separately installed Orca and OMP, including the Agent's existing model configuration, tools, Skills and approvals. Preview exact context handoffs, save immutable output observations, and explicitly reuse evidence in later tasks without setting up another Provider.

Built with Tauri 2, React, TypeScript, Rust and SQLite. Existing direct-model conversations and four streaming protocols remain compatibility features. Historical receipts remain unchanged; a handoff records the input delivered to an Agent harness, not all internal model requests. Saved evidence is a bounded output observation, not proof of success. Native general-purpose Agent chat and automatic DAG scheduling are not implemented. Local databases are not encrypted.

This is an early beta. Download available platform packages from [Releases](https://github.com/CollaalloC/ThoughtsFlow/releases), or build with Node.js 24, Rust 1.97.1 (pinned in `rust-toolchain.toml`) and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): `npm ci`, then `npm run tauri:dev`. See [CONTRIBUTING.md](CONTRIBUTING.md) for development and tests, and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for upstream attribution.
