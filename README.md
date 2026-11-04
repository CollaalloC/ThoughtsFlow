# ThoughtsFlow

**管理上下文，让现有 Agent 在可追溯的工作流中协作。**

ThoughtsFlow 是一个本地优先的 Agent 工作台。它负责图形化选择上下文、向 Agent 交接任务、保存输出证据并组织后续工作；模型连接、工具、Skills、MCP 和审批沿用现有 Agent harness。当前执行路径以本机 herdr-core 为基座，复用已有 Agent 模板，无需在 ThoughtsFlow 再配置 Provider。

[下载测试版](https://github.com/CollaalloC/ThoughtsFlow/releases/tag/v0.1.0-beta.3) · [工作台架构](docs/adr/0004-herdr-core-workbench.md) · [参与贡献](CONTRIBUTING.md) · [English overview](#english-overview)

> 本版本为 `v0.1.0-beta.3`：Agent 协作改用 herdr-core，已有 Orca 协作保留为只读历史。安装包、平台检查结果和签名限制以对应发布页为准。升级前请备份原数据库；迁移后不要直接用旧版打开同一数据库。

## 能做什么

- **复用已有 Agent。** 连接 herdr-core，选择已有模板，继承其中的 Agent、模型、认证与工具配置。
- **显式交接上下文。** 在图上选择准确的历史路径，为任务选择当前上下文或已保存的 Agent 证据，预览后再派发。内容或版本变化后需要重新预览。
- **保存可追溯的输出。** 保存准确执行尝试的屏幕观察及 hash，分页浏览摘要，展开时读取正文，离线查看并用于后续任务。部分输出和任务成功分别标明。
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

1. 安装 [herdr-core](https://github.com/herdrdev/herdr-core)，使用其原生模板配置已有 Agent；无需在 ThoughtsFlow 配置 Provider。
2. 在 **Agent 工作台** 创建本地工作区，点击“连接 herdr-core”，使用 ThoughtsFlow 专用会话。
3. 填写已有项目或独立 checkout 的绝对工作目录和协作目标。选择 Agent 模板、上下文与已有证据，预览交接后启动。
4. 查看准确终端的状态，保存屏幕观察，审阅后显式复用到下一任务。权限和信任对话在原生 Agent 终端处理。

模型与启动参数继承 Herdr 模板，不另建模型目录或认证配置。任务在选定目录运行，可能共享文件；herdr-core 不自动创建 Git worktree，需要隔离时请选择独立 checkout。现有 Orca 协作保留为只读历史，原始回执与 hash 不变。

### 准备一个原生 Agent 模板

需要单独安装 **herdr-core 0.1.17 或更新的兼容版本**，以及已能独立运行的 Agent。以本机已有 OMP 为例，在 Herdr 原生模板目录创建 `omp.md`：

```markdown
---
description: 使用本机 OMP 的现有模型、工具和认证
command: omp
---
按工作台交接的范围执行任务，报告结果和验证证据。
```

macOS/Linux 默认目录为 `~/.config/herdr-core/templates/`；Windows 默认为 `%APPDATA%\herdr-core\templates\`，设置 `XDG_CONFIG_HOME` 时以该目录为准。Windows 原生模板还需要 `sh`（例如 Git for Windows）。模板不创建新的模型账号，OMP 应先在自己的原生界面完成配置；其它 Agent 可使用自己的启动命令和就绪约定，参见 [Herdr 模板文档](https://github.com/herdrdev/herdr-core/blob/v0.1.17/docs/templates.md)。

重新检测或连接后，在工作台选择模板。若找不到 core，可通过 `THOUGHTSFLOW_HERDR_BIN` 指定其原生可执行文件的绝对路径。模型不会随安装包附带，现有认证、Skills 与审批仍归 Agent 管理。完整路径与原生终端介入方式见 [Agent 架构](docs/AGENT_ARCHITECTURE.md)。

macOS 已完成真实 OMP 的配置继承、上下文交接、文件执行和证据复用验证，见 [真实 Agent 测试记录](docs/upstream/2026-11-04-real-agent.md)。三平台 CI、原生 WebView fixture 和安装包构建分别验证；Windows/Linux 的真实模型执行不因构建成功而自动视为通过。

## 兼容入口：直连模型

已有直连对话与 Provider 设置继续保留，用于原有工作区和明确需要的独立模型请求；它们不承担 Agent harness。此路径可独立于 Agent 运行时运行，需要配置相应 Provider。

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

## herdr-core 与工作台的职责

ThoughtsFlow 拥有上下文图谱、任务意图、交接和证据；herdr-core 拥有持久终端、原生模板、状态与消息；所选 Agent 拥有模型、工具、Skills、认证和审批。

- 交接 hash 冻结工作台提交的文本、来源和请求模板，不声称包含原生模板正文、Agent 内部提示或所有模型请求。
- 任务送达、Agent 自报完成、进程退出与成果验收分别表达；结果未知时不会自动重派。
- 屏幕观察是有界证据，不代表完整转录或任务成功。只释放已经退出且准确属于本工作台的终端。
- 任务由人工拆分和审查，不自动调度依赖、合并代码或采纳决策。

协议、专用会话和恢复说明见 [Agent 架构](docs/AGENT_ARCHITECTURE.md)、[跨平台说明](docs/CROSS_PLATFORM.md)和[上游跟踪](docs/upstream/README.md)。安装包不捆绑 herdr-core 或任何 Agent。

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
        └─ herdr-core JSON-RPC → 原生模板 → Agent harness
```

上下文图谱通过显式交接连接 Agent 执行。界面不直接访问数据库或执行任意 shell 命令；旧回执不会按当前配置重新计算，输出证据不会伪造为历史 Model Run。

当前还没有通用 Agent 原生聊天界面、云同步、多人协作、附件/RAG、完整全文搜索、可恢复的工作区导入包或自动任务依赖调度。图上的证据关联用于人工编排。完整会话与模型目录接入以 OMP Direct RPC 等已验证接口为后续路线，Agent 工具能力继续来自原生 harness。

进一步阅读：[产品构想](PRODUCT_BLUEPRINT.md) · [构想与实现对照](docs/VISION_ALIGNMENT.md) · [架构路线](docs/PLATFORM_MODEL_ROADMAP.md) · [工作台性能测量](docs/PERFORMANCE_AGENT_HISTORY_20261006.md) · [变更记录](CHANGELOG.md)

## 致谢与许可证

ThoughtsFlow 的设计深入学习了 **[oh-my-pi](https://github.com/can1357/oh-my-pi)** 的 Agent 执行、模型接入、会话树与上下文管理，以及 **[Orca](https://github.com/stablyai/orca)** 的多 Agent 编排和工作区协作方式。感谢这些项目及其贡献者公开设计与实现。

ThoughtsFlow 是独立项目，与上述项目没有官方隶属或背书关系。代码许可见 [LICENSE](LICENSE)；上游许可证、参考来源和第三方声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。外部 herdr-core 和各 Agent 仍分别遵循自己的许可证与使用条款。Orca 是历史参考来源，已退出当前执行路径。

## English overview

ThoughtsFlow is a local-first Agent workbench for graphical context management. Its execution foundation is herdr-core: select an existing native Agent template, preview a context handoff, run in an explicitly chosen directory, and save immutable screen observations. Models, authentication, tools and approvals remain with the Agent. Legacy Orca history stays read-only; no Orca runtime is required. Task delivery, reported completion and process exit are distinct from acceptance. Worktrees and automatic DAG scheduling are not provided.

Built with Tauri 2, React, TypeScript, Rust and SQLite. Existing direct-model conversations and four streaming protocols remain compatibility features. Historical receipts remain unchanged; a handoff records the input delivered to an Agent harness, not all internal model requests. Saved evidence is a bounded output observation, not proof of success. Native general-purpose Agent chat and automatic DAG scheduling are not implemented. Local databases are not encrypted.

This is an early beta. Download available platform packages from [Releases](https://github.com/CollaalloC/ThoughtsFlow/releases), or build with Node.js 24, Rust 1.97.1 (pinned in `rust-toolchain.toml`) and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): `npm ci`, then `npm run tauri:dev`. See [CONTRIBUTING.md](CONTRIBUTING.md) for development and tests, and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for upstream attribution.
