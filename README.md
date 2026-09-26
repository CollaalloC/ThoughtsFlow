# ThoughtsFlow

**把 AI 对话变成可以分支、检查和比较的推演过程。**

ThoughtsFlow 是一个本地优先的桌面工作区。你可以从某一次具体回答继续探索，比较不同路线，检查每次请求发送了哪些上下文，再把结论导出为决策文档。需要执行代码任务时，可连接本机 Orca 与 OMP，拆分任务并查看多个 Agent 的进展。

[下载测试版](https://github.com/CollaalloC/ThoughtsFlow/releases/tag/v0.1.0-beta.2) · [模型连接](docs/MODEL_CONNECTIONS.md) · [参与贡献](CONTRIBUTING.md) · [English overview](#english-overview)

> 当前为 `v0.1.0-beta.2` 早期测试版。请先用于可恢复的测试工作；安装包、平台和已知限制以发布页为准。

## 能做什么

- **从准确的回答分支。** 同一问题可以重试多次，每次结果分别保存；从指定回答继续，不会把兄弟分支混入上下文。
- **看清模型收到的内容。** 发送前预览并选择保留或排除的内容；发送后保留不可变 Context Receipt，记录内容、来源、模型和请求设置。
- **整理长对话。** 经你确认后创建摘要或压缩检查点，保留原始历史；不会静默截断上下文。
- **比较与沉淀。** 查看路线图、回答及上下文差异，标记采纳、否决或待验证，导出 Markdown Decision Packet。
- **连接不同模型。** 支持四类流式协议、模型目录发现和手动模型 ID；API Key 仅在应用会话中保存。
- **协作执行任务。** 可选连接 Orca 的多 Agent 编排与 OMP 执行器，人工拆分任务、并行执行、回复协作问题和查看结果。

## 安装与首次使用

从 [Releases](https://github.com/CollaalloC/ThoughtsFlow/releases) 下载与你的操作系统和 CPU 架构匹配的安装包，并核对发布页提供的校验和。只有实际列出的文件才是该版本已发布的安装包；支持构建某个平台不等于已完成该平台的使用验证。

| 安装包 | 使用方式 |
| --- | --- |
| macOS `.dmg` | 打开磁盘映像，将 ThoughtsFlow 拖入 Applications 后启动 |
| Windows `.exe` | 运行安装向导，完成后从开始菜单启动 |
| Linux `.deb` | 在兼容的 Debian/Ubuntu 系统上使用系统包管理器安装 |

早期 macOS/Windows 包未经过正式分发签名，macOS 包也未公证，系统可能提示无法验证开发者。确认下载来源和校验和后，按系统提供的单应用确认流程处理；不要为安装本项目全局关闭系统安全保护。安装包范围、签名状态和构建方式见[发布说明](docs/RELEASING.md)。

1. 启动 ThoughtsFlow，创建一个工作区，填写想探索的问题或目标。
2. 打开 **Provider 设置**，选择模型供应商，检查 Base URL，输入本次会话的 API Key。使用本机 Ollama 时通常不需要密钥。
3. 发现并选择模型，或手动输入模型 ID，保存配置。
4. 发送问题；从某次回答创建分支，发送前检查 Context，完成后比较路线或导出结论。

普通对话不需要安装 Orca 或 OMP。ThoughtsFlow 不附带模型，也不提供模型调用额度。

## 模型连接

| 连接方式 | 当前状态 |
| --- | --- |
| OpenAI / 通用 OpenAI-compatible、OpenRouter | Chat Completions 流式请求 |
| Anthropic、Google Gemini、Ollama | 各自的原生流式协议 |
| DeepSeek、xAI、Mistral、Groq、Together、Moonshot、SiliconFlow | 预置兼容端点，复用 Chat Completions 协议 |
| Qwen 北京 / 新加坡、Z.AI | 预置兼容端点，当前手动填写模型 ID |
| OMP Gateway | 可选连接，需另行配置 OMP Auth Broker 与网关 token |
| Azure OpenAI | 仅有模板，当前不可运行 |

共 **18 个模板**，其中包含地区变体与尚不可运行的 Azure 模板。模板并不代表每家供应商的所有模型、订阅、地区和参数都经过真实账号验证。OAuth、Bedrock、Vertex ADC 目前没有原生接入。

OMP Gateway 不会自动继承本机 OMP 登录或 `models.yml`。通过网关请求时，Receipt 记录 ThoughtsFlow 发给网关的内容，不代表网关转换后发给最终供应商的请求。配置条件、默认端点和协议边界见[模型连接指南](docs/MODEL_CONNECTIONS.md)。

## 可选：Orca + OMP Agent 协作

先分别安装并配置 [Orca](https://github.com/stablyai/orca) 和 [oh-my-pi / OMP](https://github.com/can1357/oh-my-pi)，在 Orca 中登记代码项目，并为 OMP 配好模型与认证。

在 ThoughtsFlow 的 **Agent 协作** 中连接本机 Orca，选择项目与协作目标，再为任务填写范围、约束和验收条件。任务由 Orca 在独立工作区中启动 OMP；你可以查看进度、读取输出并回复协作问题。

- Agent 使用 OMP 自己的配置，不复用 ThoughtsFlow 的模型密钥，也不会自动附带普通对话历史。
- 任务由人工拆分和审查；当前不会自动合并代码或把结果自动采纳到 Decision Packet。
- 工具审批及停止正在运行的任务继续在 Orca/OMP 原生界面处理。
- Git worktree 提供代码隔离，**不提供操作系统权限沙箱**。
- 连接中断或操作结果不明时，不会自动重复派发任务。

CLI 路径、运行时版本与恢复语义见 [Agent 架构](docs/AGENT_ARCHITECTURE.md)、[跨平台说明](docs/CROSS_PLATFORM.md)和[上游兼容跟踪](docs/upstream/README.md)。Orca 与 OMP 是分别安装的外部运行时，本项目安装包不捆绑它们。

## 数据与隐私

工作区、回答、设置和操作回执保存在本机 SQLite；导出文件也保存在本机。数据库及导出文件**没有透明加密**，请按需要使用操作系统磁盘加密和备份。

API Key 在输入时短暂停留于界面状态，提交后仅存于当前 Rust 进程内存，不写入数据库、Receipt 或导出文件；退出应用后需要重新输入。多个命名凭据由用户显式切换，不会自动轮换。

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
        ├─ SQLite：工作区、精确回答树与不可变回执
        ├─ Provider adapters：模型请求与目录
        ├─ Filesystem adapter：决策文档导出
        └─ Orca adapter → 本机 Orca → OMP workers
```

模型对话与 Agent 任务使用独立的状态、凭据和执行边界。界面不直接访问数据库或执行任意 shell 命令；旧回执不会按当前配置重新计算。

当前还没有云同步、多人协作、附件/RAG、完整全文搜索、可恢复的工作区导入包、自动任务依赖调度或 OS 密钥库。普通对话为文本模型请求，Agent 工具能力来自另行安装的 Orca/OMP。

进一步阅读：[产品构想](PRODUCT_BLUEPRINT.md) · [构想与实现对照](docs/VISION_ALIGNMENT.md) · [架构路线](docs/PLATFORM_MODEL_ROADMAP.md) · [最近的性能测量](docs/PERFORMANCE_REFRESH_STORAGE_20260926.md) · [变更记录](CHANGELOG.md)

## 致谢与许可证

ThoughtsFlow 的设计深入学习了 **[oh-my-pi](https://github.com/can1357/oh-my-pi)** 的 Agent 执行、模型接入、会话树与上下文管理，以及 **[Orca](https://github.com/stablyai/orca)** 的多 Agent 编排和工作区协作方式。感谢这些项目及其贡献者公开设计与实现。

ThoughtsFlow 是独立项目，与上述项目没有官方隶属或背书关系。代码许可见 [LICENSE](LICENSE)；上游许可证、参考来源和第三方声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。外部 Orca、OMP 运行时仍分别遵循各自的许可证与使用条款。

## English overview

ThoughtsFlow is a local-first desktop workspace for branching AI conversations and technical decisions. Branch from an exact model response, inspect the context sent to a model, compare alternatives, and export a Markdown decision packet. An optional integration connects separately installed Orca orchestration with OMP agent execution.

Built with Tauri 2, React, TypeScript, Rust and SQLite. It supports four streaming model protocols and 18 provider templates, including regional presets and an Azure template that is not yet runnable. API keys are session-only; local databases are not encrypted. Remote model calls send the selected context to your configured endpoint.

This is an early beta. Download available platform packages from [Releases](https://github.com/CollaalloC/ThoughtsFlow/releases), or build with Node.js 24, Rust 1.97.1 (pinned in `rust-toolchain.toml`) and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/): `npm ci`, then `npm run tauri:dev`. See [CONTRIBUTING.md](CONTRIBUTING.md) for development and tests, and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for upstream attribution.
