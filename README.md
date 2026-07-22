# ThoughsFlow

ThoughsFlow 是一个本地优先的 AI 推演与技术决策桌面工作区。它把对话保存为由精确回答版本连接的路线：重试会新增 `ModelRun`，分支会绑定选定回答的 `parent_run_id`，实际发出的 Context 会作为不可变 Receipt 保留，最后可以比较路线、标记判断并导出 Markdown Decision Packet。

当前生产入口使用 Tauri 2、React/TypeScript、Rust/Tokio 与 SQLite。`src/prototype/` 及 `output/prototype-screenshots/` 仅保留为设计证据，不在正式运行路径中。

## 已实现的核心闭环

- 创建、打开、重命名和归档本地工作区；
- 工作区目标与模型 `system prompt` 分开保存；未设置目标时的界面提示不会进入模型 Context；
- Generic OpenAI-compatible Chat Completions（SSE）与 Ollama `/api/chat`（NDJSON）真实流式请求；
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
- Anthropic、Google 与 Azure OpenAI 模板会展示其协议和认证要求，但在对应流式协议完成前不可保存为可运行 Profile。

Rust Core 内置并唯一维护 7 个权威模板：OpenAI、Generic OpenAI-compatible、Ollama、Anthropic、Google、Azure OpenAI 和 OpenRouter；前端不能改写其协议或认证位置。选择模板会填入默认 Base URL，用户仍可覆盖为代理或自托管端点。新的 Context Receipt 会锁定模板 ID/revision、实际协议、非敏感认证位置、静态头与最终生效参数；API Key 不进入 Receipt。迁移前生成的历史 Receipt 保留其原有参数，新增模板元数据明确显示为 `legacy/unknown`，不会用当前模板反向推断或伪造历史事实。

远程端点必须使用 HTTPS；HTTP 只允许 `localhost`、`127.0.0.1` 或 `::1`。Base URL 不允许包含用户名或密码，Provider 请求也不会跟随 3xx 重定向。API Key 只保存在当前 Rust 进程内存，退出应用后清除，不写入 SQLite、前端持久状态、日志或导出文件。若 Provider 原样回显当前会话凭据，Rust 会在内容进入 `RunEvent` 或 SQLite 前进行跨 delta 的精确脱敏；该防线只匹配已知凭据原文，不能识别经过变形或编码的泄露。

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

Decision Packet 只能由 Rust 文件适配器在上述 `exports` 目录创建新文件；WebView 命令不接受目标路径，也不会覆盖已有文件。

## 验证

```bash
npm run test:run
npm run build
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

应用服务只依赖 `RepositoryPort`、运行热路径专用的 `RunPersistencePort`、`ProviderGateway`、`ProviderConnectionTester` 与 `DecisionPacketWriter`；SQLite、Reqwest 和本地文件系统实现由 Tauri 组合根注入。Tauri 结构化命令错误在 DesktopBridge 统一转换为 `DesktopBridgeError`，保留 `code`、`retryable` 和 `details`。

## 当前限制

- 仅支持 OpenAI-compatible Chat Completions 与 Ollama native 两种 dialect；
- 没有登录、云同步、多人协作、移动端、Agent/MCP、工具执行、RAG、附件或完整知识库；
- Context token 数为保守估算，不是 Provider tokenizer 的精确计数；超限会阻止发送，不做静默截断或摘要；
- Context pin/exclude 是“下一次发送”的会话态调整；已锁定 Receipt 永远不变；
- 当前只在本仓库的 macOS 环境做原生构建/冒烟，不声称 Windows 或 Linux 已实机验证。

产品定位、架构证据和原型结论分别见 [PRODUCT_BLUEPRINT.md](./PRODUCT_BLUEPRINT.md)、[产品市场定位与切入策略调研.md](./产品市场定位与切入策略调研.md)、[跨平台技术路线与产品技术架构调研.md](./跨平台技术路线与产品技术架构调研.md) 与 [AI分支对话产品需求与架构调研报告.md](./AI分支对话产品需求与架构调研报告.md)。

## 项目级 MCP 配置

`.codex/config.toml` 中的 TikHub MCP 仅作用于本仓库。启动 Codex 前通过本地环境或密钥管理器提供 `TIKHUB_API_KEY`，不要把 token 写入 Git。修改 MCP 配置后需要重启 Codex 并新建任务，配置才会重新加载。
