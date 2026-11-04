# 2026-11-04 真实 Agent 工作流验证

用户明确要求使用真实 Agent 验证配置继承、上下文交接、任务执行和证据复用。本轮在 `refactor/herdr-core-workbench` 的 `697de96` 基础上添加显式 opt-in 原生旅程；没有用 fixture 冒充模型执行，也没有修改生产派发、回执或 unknown 规则。

测试入口与修正提交为 `ac0119a`。成功旅程的原始本机证据目录为 `/var/folders/c3/wx18vm150r570n3088tz94bc0000gn/T/tf-agent-live-oszauZ`；临时路径不作为可移植存储保证。

## 结果与证据

macOS arm64 / WKWebView 605.1.15 原生桌面旅程通过，完整成功运行耗时 **91.7 秒**。执行链是 ThoughtsFlow 的 UI → Tauri / SQLite → herdr-core 0.1.17 → 真实交互式 OMP 18.16.0。

本机未发现已有 Herdr 模板，因此在临时 Herdr 配置中创建 `live-producer` / `live-consumer`，不宣称继承了一个原先存在的 Herdr 模板。两者都直接启动用户已有 OMP，使用独立 `--session-dir`、`--max-time 300` 和 `--max-cost 2`。没有 `--model`、`--provider`、`--profile`、`--no-extensions`、`--auto-approve` 或 `--approval-mode` 覆盖。HOME 与原生 OMP 配置/认证路径保持继承。

测试前只读取非凭据的配置元数据，默认角色是 `openai-codex/gpt-6-astra:xhigh`。成功运行的两个原生会话均记录了 `openai-codex/gpt-6-astra` 的真实 assistant 消息、`thinking_level_change: xhigh` 和 read/eval 工具调用；eval 内调用原生 write，随后原生 read 校验文件。认证有效的结论来自真实模型回复，不来自读取、复制或记录认证内容。

| 阶段 | 明确输入 | 独立验证 |
| --- | --- | --- |
| producer / t1 | 显式勾选工作区 system context，携带随机 context_code 与 `[11,17,23]` | Agent 用工具生成随机 UUID、求和 51、写入并读回 artifact.json；模型最终输出包含 TF_RESULT_A |
| consumer / t2 | 新 Mission、新目录，仅勾选 producer 保存的屏幕证据 | Agent 从证据提取相同 nonce、Evidence ID 与 SHA256，计算 102，写入并读回独立 artifact.json |
| 持久化 | 保存两个不可变交接和输出观察，再重载 WebView | 原证据正文和 contentHash 原样读取；consumer 交接的 evidenceIds 精确包含 producer 证据 |

producer 证据 ID 为 `8353cddf-e4f4-46b8-9c24-4d87d38a653c`，内容 hash 为 `523647d3031ea5da6376a019423dd669f4426a1ace4da913dcd390cf515457e0`。consumer 文件逐字匹配该 ID/hash 和随机 nonce，且其工具调用只读取自身目录和 artifact.json，没有打开 producer 文件来代替证据复用。

两个启动操作都收到 `first_message.sent: true` / `started: true`，分别绑定准确 t1/t2。任务成功由测试独立核对文件与引用数据，不把 core 的 live/finished 或 Agent 的 done 自动当作验收通过。

[可移植的证据摘要](../../output/herdr-real-agent-20261104.json)包含两阶段 artifact、操作/交接/证据标识与 hash、原生模型及工具摘要。[桌面截图](../../output/herdr-real-agent-20261104.png)记录真实 core 连接后的工作台。原始 session JSONL、完整操作回执、SQLite 和日志保留在本机测试输出目录，不提交可能包含原生 harness 上下文的完整会话。

## 发现并修正的问题

- 旧真实测试开关仍要求 Orca repository ID；改为显式提供已有 OMP 可执行文件路径。默认 fixture / CI 不会调用模型。
- 首轮 producer 正常完成，但测试错误地拒绝大写 UUID。已改为接受合法大小写 UUID，并重新完整运行两阶段。首轮不是未知派发；其已完成任务和 core 清理后，才启动新的明确测试。
- 原生会话 model_change 已携带完整 provider/model 选择器，摘要器不再重复加上不存在的 provider 前缀。
- 为原生 WebView 子进程显式传递测试用 XDG 配置/状态目录，确保模板目录、core 和保留证据属于同一隔离环境。

未发现本次场景中的生产派发、上下文交接或证据复用故障，因此没有为了测试通过而放宽产品身份核验、审批或 unknown 保护。

## 复现与边界

```sh
TF_AGENT_LIVE=1 \
TF_AGENT_LIVE_AGENT_BIN=/absolute/path/to/omp \
THOUGHTSFLOW_HERDR_BIN=/absolute/path/to/herdr-core \
npm run test:webview:agents:live
```

该入口会进行真实模型调用。300 秒 / 2 USD 是 OMP 原生会话限制，不是费用账单保证；测试不自行实现计费或认证。两次成功阶段及首轮断言失败的测试终端均已清理，隔离 core 已停止；没有停止用户共享 core。

本轮同时通过 WebView TypeScript 与工具链测试。基础产品的完整单元/fixture 回归仍见 [架构重构记录](2026-11-04.md)，生产源码本轮未改。完整真实用例未加入自动 CI。

本轮真实上下文是显式选中的工作区 system context；不声称已经用真实模型覆盖所有历史 Model Run 分支、裁剪或摘要组合。重载检查证明持久化读取，不把它称作断网实测。没有在 Windows/Linux 或其他 Agent/模型上执行本轮用例，也不扩大这些平台的验证结论。

OMP 原生 OSC 7501 就绪/完成信号核对来源：[18.16.0 run-status.ts](https://github.com/can1357/oh-my-pi/blob/v18.16.0/packages/coding-agent/src/utils/run-status.ts)。模板与首条消息语义沿用 [herdr-core 0.1.17](https://github.com/herdrdev/herdr-core/blob/ce87721557176f6cddaac1c7aefbf76929923263/crates/herdr-core/src/templates.rs)；没有使用 quiet 模式推断就绪或强制批准输入。
