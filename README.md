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
- 发送前 Context 检查、pin/exclude、超限阻断和 preview hash 复核；
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

Run、Manifest 与 Snapshot 在 Provider I/O 前由同一数据库事务落盘。流式输出约每 400ms 或累计 4KB 做 checkpoint；应用启动时，数据库中的 `connecting` 或 `streaming` Run 会变为 `interrupted`，已有部分输出不会丢失。首版没有数据库透明加密，也不承诺删除后物理不可恢复。

只有同时带有 `retryable: true` 且机器码精确为 `quota_exhausted` 或 `rate_limited` 的失败，界面才提供凭据恢复入口；不会根据错误文案、普通 5xx、断流或网络故障猜测并切换凭据。用户显式选择另一个命名凭据后，Rust 会在同一个 Provider Profile 串行命令中用该精确凭据创建新的 `ModelRun`；只有创建成功才把它设为当前首选，创建失败会保持原首选不变。重试继续使用失败 Run 绑定的精确 Provider Profile；原失败 Run、部分输出和 Context Receipt 保持不可变。不同 API Key 可能仍共享同一个 Project、Organization 或其他配额作用域，因此人工切换不保证恢复可用额度。

Decision Packet 只能由 Rust 文件适配器在上述 `exports` 目录创建新文件；WebView 命令不接受目标路径，也不会覆盖已有文件。

## 验证

```bash
npm run check
npm run test:fixtures
npm run test:e2e

cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Playwright 的桌面旅程需要一个可驱动 Tauri WebView 的外部 harness；默认执行会明确跳过这些原生旅程，而不会把浏览器 mock 当成桌面验证：

```bash
THOUGHSFLOW_E2E_NATIVE=1 \
THOUGHSFLOW_E2E_BASE_URL=<tauri-webdriver-url> \
npm run test:e2e
```

固定 1,000 Turn 的路线图组件基准在 `tests/performance/route-projection.test.tsx`。最新实测与平台边界记录在 `CORE_QA_RESULTS.md`。

## 架构边界

```text
React UI
  -> versioned DesktopBridge
  -> Tauri commands / Channel
  -> Rust Application Service
  -> Domain + Ports
  -> SQLite / Provider / filesystem adapters
```

组件不直接调用 SQL、Provider、API Key 或任意文件系统；所有 IPC 通过 `src/platform/desktop-bridge.ts`。唯一业务拓扑是 `Turn.parent_run_id`，路线图位置只属于 `ViewState`，不能改变 Context 编译结果。

应用服务只依赖 `RepositoryPort`、运行热路径专用的 `RunPersistencePort`、`ProviderGateway`、`ProviderConnectionTester`、只读 `ProviderModelCatalog` 与 `DecisionPacketWriter`；SQLite、Reqwest 和本地文件系统实现由 Tauri 组合根注入。Tauri 结构化命令错误在 DesktopBridge 统一转换为 `DesktopBridgeError`，保留 `code`、`retryable` 和 `details`。

## 当前限制

- 当前运行 dialect 为 OpenAI-compatible Chat Completions、Ollama native、Anthropic Messages 与 Google Gemini `streamGenerateContent`；Azure OpenAI 尚不可运行；
- Google 的 `thoughtSignature` 会被识别为不透明协议元数据且不会误显示为 reasoning，但当前不持久化或回送；纯文本多轮通常仍可调用，复杂推理质量可能受影响，工具调用所要求的签名连续性也不在本轮范围内；
- 没有登录、云同步、多人协作、移动端、Agent/MCP、工具执行、RAG、附件或完整知识库；
- Context token 数为保守估算，不是 Provider tokenizer 的精确计数；超限会阻止发送，不做静默截断或摘要；
- Context pin/exclude 是“下一次发送”的会话态调整；已锁定 Receipt 永远不变；
- 会话凭据当前没有 OS Credential Store、OAuth 或远程 Secret broker；退出应用后必须重新提供；
- 命名凭据只支持人工激活与显式重试，不做静默自动轮换；共享 Project/Organization 配额时，更换 Key 可能无效；
- 当前只在本仓库的 macOS 环境做原生构建/冒烟，不声称 Windows 或 Linux 已实机验证。

产品定位、架构证据和原型结论分别见 [PRODUCT_BLUEPRINT.md](./PRODUCT_BLUEPRINT.md)、[产品市场定位与切入策略调研.md](./产品市场定位与切入策略调研.md)、[跨平台技术路线与产品技术架构调研.md](./跨平台技术路线与产品技术架构调研.md) 与 [AI分支对话产品需求与架构调研报告.md](./AI分支对话产品需求与架构调研报告.md)。

## 项目级 MCP 配置

`.codex/config.toml` 中的 TikHub MCP 仅作用于本仓库。启动 Codex 前通过本地环境或密钥管理器提供 `TIKHUB_API_KEY`，不要把 token 写入 Git。修改 MCP 配置后需要重启 Codex 并新建任务，配置才会重新加载。
