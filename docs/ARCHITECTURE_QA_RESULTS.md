# 架构优化验证记录

日期：2026-09-22。本轮基于已有 Orca / OMP 集成工作树继续改进；没有覆盖或撤销前轮功能。产品达成度与后续能力设计分别见 [VISION_ALIGNMENT.md](VISION_ALIGNMENT.md) 和 [ARCHITECTURE_EVOLUTION.md](ARCHITECTURE_EVOLUTION.md)。

## 本轮验证过的行为

- Focus 新增 17 项回归，累计 43 项：导航响应倒序、旧发送成功和失败、ACK 前后事件、返回运行中的工作区、事件不重复累加、同工作区旧刷新、草稿保存与恢复、旧维护与预览、StrictMode 空库首次创建、旧 Run 终态不解除新发送锁。
- Agent 新增 12 项回归，累计 28 项：确切当前 Dispatch、真实任务结算后的空活动 ID、同 ID 重放、错误 Run 身份、缺失成功证据、绑定提交失败和两阶段创建回滚、原始回执保留、跨 Mission 并发与同 Mission 重复请求。
- 实现前先得到失败证据：旧 A 读取抢回 B 的界面；worker 顺序决定当前 Dispatch；不匹配的重连留下 succeeded；绑定写入失败仍留下成功；全局锁阻塞不相关 Mission。修复后相应用例通过。
- 独立代码审查在本轮修改范围内未发现遗留 P1/P2。

## 自动化结果

| 检查 | 本轮结果 |
| --- | --- |
| `npm run test:run` | 14 个文件，123 项通过 |
| `npm run test:fixtures` | 10 项通过，包括真实形状的 Orca 回复关联字段和未读消息语义 |
| TypeScript / Vite 构建 | 通过 |
| WebView 测试 TypeScript | 通过 |
| `cargo test --all-targets --all-features` | 248 项库测试、3 项原生 IPC 测试通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 通过 |
| Rust fmt / Git whitespace | 通过 |

## 原生对话与崩溃恢复

`npm run test:webview` 完成真实 macOS WKWebView 的 Context Tree 冒烟和三进程崩溃恢复旅程。父级旅程记录结果为 `three-process WKWebView crash-window journey passed`。三次进程退出码为 `0, 1, 0`；中间的 `1` 是预期的提交后故障注入，不是遗漏的失败测试，最后一程核验重启后的持久化与幂等重放。

原生验收也暴露了既有布局问题：发送按钮 `disabled=false`，但矩形 top=1030、bottom=1064，视口高度仅为 1035，中心点命中为空。应用壳重新分配可用高度后，同一原生点击断言通过；没有增加等待来掩盖，也没有放宽断言。

Agent 双任务模拟 CLI 原生旅程也通过。用户随后授权的真实 OMP 单任务旅程耗时 1 分 46.2 秒，包含文件独立校验和真实执行器释放，详见 [LIVE_AGENT_QA_RESULTS.md](LIVE_AGENT_QA_RESULTS.md)。

## 适用范围

本轮没有改变 Provider 请求协议、Context Receipt 含义或数据库 schema。工作区草稿隔离保存在本次 Focus 挂载的内存中，不新增跨应用退出的草稿持久化。切换界面不会取消已发出的后台 Run。

常规自动化使用本机协议 fixture；真实 opt-in 只验证一个范围受限的 OMP 短任务，不代替真实多 Agent 长任务与用户研究。搜索、可恢复工作区数据包与执行证据进入 Decision Packet 仍属于后续产品功能。
