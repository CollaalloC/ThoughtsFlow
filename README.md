# ThoughsFlow

ThoughsFlow 是一个本地优先的 AI 推演与技术决策桌面工作区。它把对话保存为由精确回答版本连接的路线：重试会新增 `ModelRun`，分支会绑定选定回答的 `parent_run_id`，实际发出的 Context 会作为不可变 Receipt 保留，最后可以比较路线、标记判断并导出 Markdown Decision Packet。

当前生产入口使用 Tauri 2、React/TypeScript、Rust/Tokio 与 SQLite。`src/prototype/` 及 `output/prototype-screenshots/` 仅保留为设计证据，不在正式运行路径中。

## 已实现的核心闭环

- 创建、打开、重命名和归档本地工作区；
- 工作区目标与模型 `system prompt` 分开保存；未设置目标时的界面提示不会进入模型 Context；
- Generic OpenAI-compatible Chat Completions（SSE）、Ollama `/api/chat`（NDJSON）、Anthropic Messages（SSE）与 Google Gemini `streamGenerateContent`（SSE）真实流式请求；
- Provider 模型发现：OpenAI-compatible、OpenRouter 与 OpenAI 使用模型列表 API，Ollama 使用 `/api/tags`，Google 使用 `/v1beta/models`，Anthropic 使用 Rust 内置的审核列表；
- 每个已保存 Provider Profile 可在本次应用会话中维护多个命名 API Key，并显式选择当前首选凭据；
- 对结构化且可重试的配额/限流失败提供显式凭据切换与重试入口，失败或部分输出的旧 Run 保持不变；
- 同一 Turn 多个不可覆盖的 Run、精确回答分支与兄弟分支 Context 隔离；
- 以精确 `ModelRun` 为节点的持久化 Context Tree：活动 Run、可选分支指针和版本在重启后恢复，历史节点继续或重试会显式 fork；
- 发送前 Context 检查、pin/exclude、超限阻断和 preview hash 复核；
- pin/exclude 作为持久化的“下一次发送”草稿保存，切换 Context 时与活动游标用双 CAS 原子重基，发送事务成功后才消费；
- 用户确认来源范围与保留边界后，保存不可变 compaction/branch-summary checkpoint；失败、取消和版本冲突不会激活 checkpoint 或移动游标；
- 发送后不可变 Context Snapshot/Receipt，包含有序内容、来源、Provider、Model、Base URL、参数与 canonical hash；
- 取消、失败、批量 checkpoint，以及启动时把未终结 Run 恢复为 `interrupted` 并保留部分输出；
- 真实会话树投影的轻量路线图、回答与 Context Diff、采纳/否决/待验证标记；
- Markdown Decision Packet/ADR 导出。

## 开发运行

需要 Node.js 20.19+、Rust stable，以及当前平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)。

```bash
npm install
npm run tauri:dev
```

正式前端通过 Tauri IPC 工作；单独运行 `npm run dev` 只能加载界面资源，不能替代 Rust Core。构建当前平台安装包：

```bash
npm run tauri -- build
```

## Provider 配置

在“Provider 设置”中选择：

- `OpenAI`、`OpenRouter` 或 `Generic OpenAI-compatible`：模板提供默认端点、Bearer 认证位置与 SSE 协议；
- `Ollama`：模板默认 `http://127.0.0.1:11434`，本地端点使用原生 `/api/chat` NDJSON，不需要 API Key；改为远端或代理端点时可设置 Bearer 会话凭据，只有内存中存在非空凭据才会发送认证头；
- `Anthropic`：使用 `/v1/messages`、`x-api-key` 与固定的 `anthropic-version: 2023-06-01`；未显式配置 `max_output_tokens` 时，Rust 会在 Context 预览与 Receipt hash 生成前冻结有效默认值 `4096`；
- `Google Gemini`：使用 `/v1beta/models/{model}:streamGenerateContent?alt=sse` 与 `x-goog-api-key`，模型 ID 可来自发现结果或手动输入；
- `Azure OpenAI`：模板仍只展示目标协议与认证要求，当前没有可运行的部署/版本化端点适配器。

Rust Core 内置并唯一维护 7 个权威模板：OpenAI、Generic OpenAI-compatible、Ollama、Anthropic、Google、Azure OpenAI 和 OpenRouter；前端不能改写其协议或认证位置。选择模板会填入默认 Base URL，用户仍可覆盖为代理或自托管端点。新的 Context Receipt 会锁定模板 ID/revision、实际协议、非敏感认证位置、静态头与最终生效参数；API Key 不进入 Receipt。迁移前生成的历史 Receipt 保留其原有参数，新增模板元数据明确显示为 `legacy/unknown`，不会用当前模板反向推断或伪造历史事实。

“发现模型”既可使用已保存 Profile，也可在保存前检查当前 draft。前者由 Rust 从 SQLite 与会话凭据存储解析权威目标；后者只把模板 ID、Base URL 和可选的本次会话凭据交给 Rust，由内置模板决定认证头、路径和响应格式。远程目录只向界面显示的 Host 发送模型元数据 GET，Anthropic 的内置审核列表不会联网；两者都不携带工作区 Context。原始目录响应和发现结果不写入 SQLite；只有用户选中模型并显式保存 Profile 后，模型 ID 才会持久化。Azure OpenAI 没有可移植的模型目录，当前会明确提示不支持发现。

已保存的 Provider Profile 可以维护多个带标签的会话 API Key。API Key 在提交前会短暂停留于 WebView 密码输入状态；交接后，Secret 只存在于当前 Rust 进程内存，WebView 只能读取凭据的标签、顺序和当前首选状态。退出应用会清除全部会话凭据；Secret 不写入 SQLite、前端持久状态、日志、Receipt 或导出文件。凭据顺序用于人工管理，不触发静默自动轮换：切换当前首选凭据和再次运行都必须由用户明确操作；存在备用项时也不能直接删除当前首选，必须先显式激活替代项。

保存 Provider 时，名称、模型或参数等不改变端点身份的更新会保留现有会话凭据；Provider 身份或 Base URL 的变更只会在保存成功后清除旧凭据。保存失败不会预先清除或替换原有凭据。模型发现所需的 draft 凭据只通过一次命令进入 Rust；已保存 Profile 的发现则复用 Rust 内存中的当前首选凭据。

远程端点必须使用 HTTPS；HTTP 只允许 `localhost`、`127.0.0.1` 或 `::1`。Base URL 不允许包含用户名或密码、query 或 fragment，Provider 请求也不会跟随 3xx 重定向。若 Provider 原样回显当前会话凭据，Rust 会在内容进入 `RunEvent`、错误、模型列表或 SQLite 前进行精确脱敏；该防线只匹配已知凭据原文，不能识别经过变形或编码的泄露。

## 数据、隐私与恢复

权威工作区数据位于 Tauri 的应用数据目录：

```text
<app_data_dir>/thoughsflow.sqlite3
<app_data_dir>/exports/decision-packet-<uuid>.md
```

macOS 的默认位置通常是：

```text
~/Library/Application Support/io.thoughsflow.desktop/
```

“数据保存在本机”只描述 SQLite 和导出文件的位置。每轮发送前，Composer 与 Inspector 会另行显示本轮 Context 将发往的 Provider、Model 和 Host；调用远程 Provider 时，相应 Context 会离开本机。

Context Tree 的原始历史路径始终可检查。人工 checkpoint 的摘要文本只在本机保存；选择 Provider 生成摘要时，压缩预览中列出的来源范围和摘要请求会发送到所选 Provider。应用不会静默摘要、自动压缩或在失败后自动重试。

Run、Manifest 与 Snapshot 在 Provider I/O 前由同一数据库事务落盘。流式输出约每 400ms 或累计 4KB 做 checkpoint；应用启动时，数据库中的 `connecting` 或 `streaming` Run 会变为 `interrupted`，已有部分输出不会丢失。首版没有数据库透明加密，也不承诺删除后物理不可恢复。

只有同时带有 `retryable: true` 且机器码精确为 `quota_exhausted` 或 `rate_limited` 的失败，界面才提供凭据恢复入口；不会根据错误文案、普通 5xx、断流或网络故障猜测并切换凭据。用户显式选择另一个命名凭据后，Rust 会在同一个 Provider Profile 串行命令中用该精确凭据创建新的 `ModelRun`；只有创建成功才把它设为当前首选，创建失败会保持原首选不变。重试继续使用失败 Run 绑定的精确 Provider Profile；原失败 Run、部分输出和 Context Receipt 保持不可变。不同 API Key 可能仍共享同一个 Project、Organization 或其他配额作用域，因此人工切换不保证恢复可用额度。

Decision Packet 只能由 Rust 文件适配器在上述 `exports` 目录创建新文件；WebView 命令不接受目标路径，也不会覆盖已有文件。

## Orca + OMP Agent 协作

“Agent 协作”工作面把 ThoughsFlow 的目标工作台、Orca 的多 Agent 编排和 OMP 的任务执行连接起来。架构与分阶段实施说明见 [AGENT_ARCHITECTURE.md](docs/AGENT_ARCHITECTURE.md)。本机协议核查基线为 Orca 1.4.206、OMP 18.2.8。

1. 安装 Orca 和 OMP，在 Orca 中登记代码项目，并配置好 OMP 的模型与认证。
2. 打开一个 ThoughsFlow 工作区，进入“Agent 协作”，检测或启动本机 Orca。
3. 选择代码项目，填写协作目标；每个目标拥有独立的 Orca Run 与专用协调者终端。
4. 为每个可独立完成的任务填写标题、范围、约束与验收条件，点击“启动 OMP 任务”。任务在 Orca 新建的独立工作区中运行，可以并行执行；不会自动合并代码。
5. 查看任务进展、读取输出、回复协调问题；任务结算后可以释放执行器，Orca 保留其输出存档。

Agent 使用本机 OMP 设置，不复用 ThoughsFlow 的 Provider 会话凭据，也不自动附带对话历史。停止运行中任务和 OMP 工具审批继续在 Orca/OMP 的原生界面处理；ThoughsFlow 的问题回复只处理协作消息。独立 worktree 从 Orca 项目默认 base 创建，不包含当前未提交改动，也不是操作系统权限沙箱。

关联和每次操作的回执保存在现有 SQLite。任务状态与执行器存活分别显示；命令接受不代表任务完成。连接中断或操作结果未知时，不会自动重发或启动替代执行器。重新连接只恢复原协作身份；未知派发仍需根据回执和 Orca 实际状态核查。当前版本提供人工任务拆分、并行执行和人工审查，自动依赖调度与结果采纳到 Decision Packet 属于后续阶段。

CLI 不在默认路径时，可在启动应用的进程环境中设置绝对路径 `THOUGHSFLOW_ORCA_BIN`、`THOUGHSFLOW_OMP_BIN`。WebView 不提供任意命令执行入口。Agent 功能仅连接本机 Orca；缺少运行环境不会影响普通对话功能。

## 验证

```bash
npm run check
npm run test:fixtures
npm run test:e2e
npm run test:native
npm run test:webview
npm run test:webview:agents

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

`npm run test:webview` 会构建独立标识符、独立数据目录且仅测试构建启用 WebDriver 的 macOS 应用，然后在真实 WKWebView 中执行冒烟和三进程重启旅程。旅程覆盖活动叶切换与重开、运行中断恢复、摘要失败/取消、checkpoint 提交后 IPC 响应前崩溃，以及同一 operation ID 的幂等重放。测试专用驱动、审计捕获和故障注入均受 `webview-e2e` feature 限制，不进入普通 production build。

`npm run test:webview:agents` 在真实 WKWebView 中走 Agent 表单、Tauri IPC、SQLite 和 CLI 子进程，使用隔离的模拟 Orca 可执行程序验证创建、派发、提问回复、输出、释放和页面重载恢复，不联系真实模型。它与本机 Orca 控制链路核查分别记录，不能作为真实 OMP 模型任务已完成的证据。

真实 OMP 验证是独立的显式 opt-in：设置 `TF_AGENT_LIVE=1` 和从 `orca repo list --json` 取得的 `TF_AGENT_LIVE_REPO_ID` 后运行 `npm run test:webview:agents:live`。该测试会使用现有 OMP 模型配置，在 Orca 独立工作区创建一个带随机标记的验证文件，读取结果并释放已结算执行器；保留临时应用数据库与回执路径以便失败后核查，不会在普通测试中自动运行，也不会在超时后自动重派。

本机 OpenAI-compatible 端点可以用固定、无项目数据的提示做显式 opt-in 探针：

```bash
npm run test:webview:live-proxy
```

该命令会真实联系配置在测试中的本机端点，必须由操作者明确运行；测试只发送代码中固定的 `TF_APP_OK` 提示，不发送仓库或工作区内容。

`npm run test:e2e` 保留 Playwright 旅程发现；没有外部 Tauri URL 时会明确跳过，不作为原生通过证据。`npm run test:native` 则以 Tauri 内置 MockRuntime 走真实 IPC、AppState 与文件 SQLite，适合确定性检查，但同样不替代上面的真实 WKWebView 旅程。固定 1,000 Turn 的路线图、1,000 Run Context Tree 组件基准，以及后端 1,000 Turn/2,000 Run 和深度 1,000 重建基准分别位于 `tests/performance/` 与 `src-tauri/tests/native_context_tree.rs`。最新实测与平台边界记录在 `CORE_QA_RESULTS.md`。

## 架构边界

```text
React UI
  -> versioned DesktopBridge
  -> Tauri commands / Channel
  -> Rust Application Service
  -> Domain + Ports
  -> SQLite / Provider / filesystem adapters
```

组件不直接调用 SQL、Provider、API Key 或任意文件系统；所有 IPC 通过 `src/platform/desktop-bridge.ts`。唯一业务拓扑是 `Turn.parent_run_id`；持久 `ContextCursor` 只选择其中一条精确 root→Run 路径，路线图位置只属于 `ViewState`，不能改变 Context 编译结果。旧 Receipt 从不按当前树或当前 Provider 设置重算。

应用服务只依赖 `RepositoryPort`、运行热路径专用的 `RunPersistencePort`、`ProviderGateway`、`ProviderConnectionTester`、只读 `ProviderModelCatalog` 与 `DecisionPacketWriter`；SQLite、Reqwest 和本地文件系统实现由 Tauri 组合根注入。Tauri 结构化命令错误在 DesktopBridge 统一转换为 `DesktopBridgeError`，保留 `code`、`retryable` 和 `details`。

Context Tree 的设计参考固定在 oh-my-pi commit [`d16c6168`](https://github.com/can1357/oh-my-pi/commit/d16c6168c86f40fc44f25118c2fd06fe160fcb93)：复用活动叶/树投影与非破坏式压缩重建的设计思想，没有复制其实质代码，也没有移植通用 SessionEntry 日志、自动压缩、workspace 克隆或 Snapcompact。

## 当前限制

- 当前运行 dialect 为 OpenAI-compatible Chat Completions、Ollama native、Anthropic Messages 与 Google Gemini `streamGenerateContent`；Azure OpenAI 尚不可运行；
- Google 的 `thoughtSignature` 会被识别为不透明协议元数据且不会误显示为 reasoning，但当前不持久化或回送；纯文本多轮通常仍可调用，复杂推理质量可能受影响，工具调用所要求的签名连续性也不在本轮范围内；
- 没有登录、云同步、多人协作、移动端、RAG、附件或完整知识库；Agent、工具和 MCP 执行由本机 Orca/OMP 提供，普通对话仍为纯文本模型请求；
- Context token 数为保守估算，不是 Provider tokenizer 的精确计数；超限会阻止发送，不做静默截断或摘要；
- Context pin/exclude 是持久化的“下一次发送”草稿；成功发送后消费，失败不消费，切换路径时原子清空并重基；已锁定 Receipt 永远不变；
- 会话凭据当前没有 OS Credential Store、OAuth 或远程 Secret broker；退出应用后必须重新提供；
- 命名凭据只支持人工激活与显式重试，不做静默自动轮换；共享 Project/Organization 配额时，更换 Key 可能无效；
- 当前只在本仓库的 macOS 环境做原生构建/冒烟，不声称 Windows 或 Linux 已实机验证。

产品定位、架构证据和原型结论分别见 [PRODUCT_BLUEPRINT.md](./PRODUCT_BLUEPRINT.md)、[产品市场定位与切入策略调研.md](./产品市场定位与切入策略调研.md)、[跨平台技术路线与产品技术架构调研.md](./跨平台技术路线与产品技术架构调研.md) 与 [AI分支对话产品需求与架构调研报告.md](./AI分支对话产品需求与架构调研报告.md)。

当前实现与最初构想的逐项对应见 [VISION_ALIGNMENT.md](docs/VISION_ALIGNMENT.md)。工作区异步归属、Agent 回执原子提交与后续证据闭环的设计见 [ARCHITECTURE_EVOLUTION.md](docs/ARCHITECTURE_EVOLUTION.md)。核心推演流程已具备生产实现，但完整全文搜索、可恢复工作区数据包、Agent 结果进入决策和真实用户验证仍是明确的待完成项。

## 项目级 MCP 配置

`.codex/config.toml` 中的 TikHub MCP 仅作用于本仓库。启动 Codex 前通过本地环境或密钥管理器提供 `TIKHUB_API_KEY`，不要把 token 写入 Git。修改 MCP 配置后需要重启 Codex 并新建任务，配置才会重新加载。
