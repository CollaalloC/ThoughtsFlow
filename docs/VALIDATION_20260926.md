# 跨平台与模型接入验证记录

验证日期：2026-09-26。本机 macOS，Node 24；Windows/Linux CI 按用户选择仅配置，尚未接入远端执行。没有在本轮启动付费模型任务或修改凭据。

## 本轮通过的检查

| 检查 | 结果 |
| --- | --- |
| 前端组件与性能保护测试 | 14 个文件，127 项通过 |
| 本机 HTTP / Orca CLI fixtures | 10 项通过 |
| 跨平台 Node 工具链 | 3 项通过，包含真实 argv 原样传递 |
| TypeScript / Vite 构建 | 通过 |
| WebView TypeScript | 通过 |
| Rust 全 target / feature | 267 项库测试、3 项原生 IPC 测试通过；1 个性能基准明确 ignored，已单独执行 |
| Cargo fmt / clippy `-D warnings` | 通过 |
| 模型目录未知 kind 的最终兼容回归 | 精确目标通过 |
| macOS 原生 Context Tree 与三进程恢复 | 通过；中间进程退出码 1 为预期故障注入 |
| macOS 原生双 Agent fixture 流程 | 通过，40.6 秒；用新 Node 启动计划执行 fixture |
| 独立代码审查 | 已修复发现问题，最终无遗留 P1/P2 |

## 实测发现并修复

- 三平台 GUI workflow 所调用的旧测试仍只接受 macOS。现统一按插件实际返回值检查各平台 WebView，保留真实性断言。
- 1080px 窄窗口中 Inspector 覆盖发送按钮。几何诊断证明按钮未禁用且位于窗口内，中心点却命中 Inspector footer。现窄屏初始收起、可手动开关，覆盖层限定在工作面内；发送与 Context 语义未改，原生旅程通过。
- 通用模型目录不能将任意未知 `kind` 当成非聊天。现仅排除已知非聊天类型及明确能力不符项，未知类型保留 unknown。

## 性能证据与边界

固定每 CLI 25ms 的延迟模型中，快照中位数从 136.99ms 降至 81.97ms，约减少 40.2%，仍是 5 次 CLI 调用。这个数值不是实际 Orca 网络、跨平台或整机速度承诺。原始样本与复现方法见 [PERFORMANCE_20260926.md](PERFORMANCE_20260926.md)。

模型候选列表最多渲染 100 项，但保留完整目录供输入筛选；500 模型的组件测试验证最后一个模型仍能找到，输入筛选不重复请求目录。

## 仍须后续验收

- Windows/Linux 的实际构建、进程和 GUI 行为必须等待对应 CI 或实机记录，不能由本机条件测试推断。
- 18 个模板包含地区配置及仍未启用的 Azure，不等于 18 家厂商都已用真实账号测试。
- Qwen / Z.AI 目前手工模型 ID；OMP Gateway 需要已配置 Auth Broker，不自动继承本机 OMP 登录。
- 本机 OMP 开始核查为 18.2.10，收尾复核为 18.3.2；本任务没有执行运行时升级命令。最新版本的真实 Agent 测试与 SDK 全鉴权接入尚未完成。
- 当前无 Git remote，未推送源码、未发布安装包、未宣称远端 CI 已通过。

功能说明见 [MODEL_CONNECTIONS.md](MODEL_CONNECTIONS.md)、[CROSS_PLATFORM.md](CROSS_PLATFORM.md) 与 [上游跟踪基线](upstream/README.md)。
