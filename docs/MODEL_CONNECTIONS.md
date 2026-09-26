# 模型连接与 OMP Gateway

核查日期：2026-09-26。本页区分可配置的请求协议、模型目录、实际账号可用性。新增模板复用 ThoughsFlow 的 OpenAI Chat Completions 适配器；没有将 Agent RPC 当作模型请求，也没有新增供应商 SDK 依赖。

## 连接方式

ThoughsFlow 保留 OpenAI Chat Completions、Anthropic Messages、Google Generative AI、Ollama Chat 四种已实现协议。普通供应商模板使用 API Key；OMP Gateway 使用网关的 bearer token，供应商凭据由它的 Auth Broker 管理。密钥仍只在 ThoughsFlow 进程内存中传递，不写入 Provider Profile、SQLite 或 Context Receipt。

模板表示已实现该请求协议，不表示每个模型、订阅、地区、专属参数或账户已经实际测试。新增模板本轮使用本地 HTTP 夹具验证，未调用这些厂商的付费模型。Azure 模板仍标记运行不可用；Bedrock、Vertex ADC、厂商 OAuth 没有被伪装成普通 API Key 原生支持。

## 新增模板与官方依据

下列 Base URL 均附聊天协议的官方来源。自动目录只用于获取元数据，不发送工作区上下文或触发模型生成；也不保证模型配额充足。

| 模板 ID | Base URL | 模型目录与来源 |
| --- | --- | --- |
| `omp-gateway` | `http://127.0.0.1:4000/v1` | `/models`；[OMP 18.2.10 网关源码](https://github.com/can1357/oh-my-pi/blob/v18.2.10/packages/ai/src/auth-gateway/server.ts) |
| `deepseek` | `https://api.deepseek.com/v1` | `/models`；[聊天 endpoint](https://api-docs.deepseek.com/quick_start/agent_integrations/workbuddy/) / [目录](https://api-docs.deepseek.com/api/list-models/) |
| `xai` | `https://api.x.ai/v1` | `/models`；[聊天与流式协议](https://docs.x.ai/developers/model-capabilities/text/streaming) / [目录](https://docs.x.ai/developers/rest-api-reference/inference/models) |
| `mistral` | `https://api.mistral.ai/v1` | `/models`；[聊天兼容](https://docs.mistral.ai/resources/migration-guides) / [目录](https://docs.mistral.ai/api/endpoint/models) |
| `groq` | `https://api.groq.com/openai/v1` | `/models`；[聊天协议](https://console.groq.com/docs/api-reference) / [目录](https://console.groq.com/docs/models) |
| `together` | `https://api.together.ai/v1` | `/models`；[官方兼容矩阵，包含聊天与目录](https://docs.together.ai/docs/inference/openai-compatibility) |
| `moonshot` | `https://api.moonshot.cn/v1` | 中国区 `/models`；[聊天配置](https://platform.kimi.com/docs/get-api-key) / [目录](https://platform.kimi.com/docs/api/list-models) |
| `qwen-beijing` | `https://dashscope.aliyuncs.com/compatible-mode/v1` | 北京区手工输入模型 ID；[地区与 Base URL](https://help.aliyun.com/zh/model-studio/base-url) / [聊天协议](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope) |
| `qwen-singapore` | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | 新加坡区手工输入模型 ID；同上官方来源 |
| `zai` | `https://api.z.ai/api/paas/v4` | 普通 API，手工输入模型 ID；[官方 Quick Start](https://docs.z.ai/guides/overview/quick-start) |
| `siliconflow` | `https://api.siliconflow.cn/v1` | 中国区 `/models`；[聊天协议](https://docs.siliconflow.cn/docs/api/chat-completions-post) / [目录](https://docs.siliconflow.cn/docs/api/models-get) |

Qwen 的地区、业务空间和 API Key 必须匹配；用户可将对应模板的 Base URL 改成官方提供的业务空间专属域名。本模板使用普通按量 API，不默认路由到 Coding Plan 或 Token Plan。官方模型目录是另一套 `/api/v1/models` 接口与分页结构，本版尚未接入，不猜测 `compatible-mode/v1/models`。[Qwen 目录规范](https://help.aliyun.com/zh/model-studio/list-models)

Z.AI 的普通 API 和 GLM Coding Plan 端点不同。本模板使用普通 API；自动目录未接入。Qwen 与 Z.AI 的目录刷新及基于目录的连接测试会明确返回 `provider_model_discovery_unsupported`，手填模型后仍可使用已实现的聊天协议。

## OMP Gateway 的前置条件与证据边界

1. 已部署并配置 OMP Auth Broker。`omp auth-gateway serve` 要求 `OMP_AUTH_BROKER_URL` 或 `auth.broker.url/token`。
2. 启动网关，例如 `omp auth-gateway serve --bind=127.0.0.1:4000`。ThoughsFlow 不会自动启动、升级或配置 broker，也不会默认关闭网关认证。
3. 在 ThoughsFlow 选择 OMP Gateway，输入网关 bearer token，刷新目录。模型 ID 保持 `provider/model-id`，避免不同供应商的同名模型冲突。

该网关只使用 broker 提供的凭据，构建目录时忽略本机 `models.yml`，不直接继承本机 `~/.omp/agent/agent.db`。目录仅覆盖 broker 凭据可路由的模型；它与 `omp models --json` 的本机可用目录不是同一来源。[网关启动与目录构建源码](https://github.com/can1357/oh-my-pi/blob/v18.3.2/packages/coding-agent/src/cli/auth-gateway-cli.ts)

Context Receipt 证明 ThoughsFlow 编译并发送给配置网关的上下文、模型 ID 与协议。OMP 可能再将请求转换为供应商原生协议；本版没有捕获该第二跳的 payload，因此 Receipt 不宣称包含最终供应商 wire payload。Orca 管理的 Agent 会话继续属于 Agent Mission；这里不会建立第二条 RPC 来争用它。

## 模型目录行为

- 保留 qualified ID、显示名和提供者声明的 context length。没有元数据时保持 unknown，不硬编码模型名单或推断上下文长度。
- OpenAI-compatible 目录排除 OMP 已知非聊天 `kind`（`tiny`、`image`、`tts`、`stt`、`search`、`judge`、`embedding`、`rerank`、`video`）、输入模态明确不含 text、或 `capabilities.completion_chat=false` 的记录。第三方自定义 `kind`（例如 `llm`）与缺失元数据保留为 unknown，模型 ID 的名称不作为能力证据；供应商仍可能在实际调用时拒绝不适用的模型。[OMP catalog kinds](https://github.com/can1357/oh-my-pi/blob/v18.3.2/packages/catalog/src/types.ts)
- 目录响应只提取允许字段，保留既有凭据脱敏、长度限制、禁止重定向规则，不持久化原始响应。
- Anthropic 改为带 `x-api-key` 与 `anthropic-version` 的真实 `/v1/models` 查询，按 `has_more` / `last_id` 翻页。整次查询共用 20 秒、4 MiB、5,000 条记录、最多 10 页的上限；游标重复或缺失返回错误，不返回假装完整的部分名单。下一页始终请求同一已验证 endpoint，游标只作为编码后的 query 参数。[Anthropic 官方目录规范](https://platform.claude.com/docs/en/api/models/list)
- Anthropic 模板 revision 为 3；历史 Receipt 的冻结快照不修改。

## 后续 SDK 接入条件

`@oh-my-pi/pi-ai` 提供更广的鉴权和 provider transport，`pi-catalog` 提供动态目录与兼容规则；其 TS 包依赖 Bun/原生模块，尚未作为 ThoughsFlow 自带 sidecar 发布。当前 OMP Gateway 是已有模型端口上的可选连接方式。独立 OMP Agent runtime、OAuth 登录 UI、自带 Auth Broker、计费及 reasoning 参数全集不属于本轮完成范围。[OMP SDK](https://github.com/can1357/oh-my-pi/blob/v18.3.2/packages/ai/README.md)
