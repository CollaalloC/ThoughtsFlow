# 原始构想与当前实现的对应关系

核查日期：2026-09-22。核查对象为当前工作树的生产代码，包括前一轮 Orca / OMP 接入改动；不将 `src/prototype/` 计入功能实现。

## 判断

ThoughsFlow 已实现原始构想最重要的工程内核：从准确回答版本分支、隔离各路线的上下文、检查实际发送内容、保存不可变凭证，以及本地持久化和故障恢复。后续定位提出的“比较路线 → 记录采纳或否决理由 → 导出 Decision Packet”也已有生产实现。

但原始 P0 仍有未完成项，尤其是完整全文搜索、可恢复的数据导出与备份恢复。新增 Agent 工作面能够承载执行控制，却尚未与路线依据和决策证据连通。因此当前版本可视为具备核心能力的可用实现，不能认定全部产品构想已经达成。

工程能力与产品价值需要分别判断。仓库里的自动化及原生 QA 记录不能证明用户会持续回访分支、在第二个真实任务中主动使用产品，或愿意付费。本报告不计算完成百分比，也不将未开展的用户研究当作成功。

## 核查方法与证据边界

- 需求基线为根目录的 `AI分支对话产品需求与架构调研报告.md`、`PRODUCT_BLUEPRINT.md` 与 `产品市场定位与切入策略调研.md`。
- 交叉检查生产 React / TypeScript、Rust 服务、SQLite 迁移、IPC 契约和测试源码。
- `CORE_QA_RESULTS.md` 记录的是 2026-07-29 的历史验证；`docs/AGENT_QA_RESULTS.md` 记录的是此前 2026-09-22 的 Agent 验证。达成度表首先由静态核查得到，同轮后续实际回归和真实 OMP 单任务结果另见 `docs/ARCHITECTURE_QA_RESULTS.md` 与 `docs/LIVE_AGENT_QA_RESULTS.md`，不以旧 QA 数字冒充新结果。
- 下表采用相对仓库根目录的路径与核查时行号，并补充符号名，便于代码调整后继续检索。未找到的能力表述为“当前生产入口与契约未提供”，不以文档中的规划替代实现证据。

## 目标追踪

| 层次与原始目标 | 当前生产实现 | 未完成或尚未验证的部分 | 可查证据（仓库相对路径与行号） |
| --- | --- | --- | --- |
| 原始 P0：从精确回答分支，旁支不污染当前输入 | `Turn.parent_run_id` 绑定准确 Model Run；Context Tree 保存活动 cursor、branch pointer 和草稿；领域性质测试检查兄弟分支隔离 | 已完成回答可以分支；原规划中“停止后显式接受部分回答再分支”的入口尚未提供。真实用户能否理解回答版本和分支的区别，仍需可用性验证 | 需求报告第 352–356、402–406 行；`src-tauri/src/domain/tests.rs:1240` 的 `branch_cannot_target_an_unfinished_run`、`:1278` 的 `property_wide_branches_never_leak_sibling_context`；`CORE_QA_RESULTS.md:63` |
| 原始 P0：可检查、可调整、可审计的 Context | Inspector、固定和排除、typed source / hash、运行快照与 checkpoint 来源均已有实现和测试 | ThoughsFlow 的发送凭证不覆盖 OMP 内部模型和工具调用；不能扩大对可见性的承诺 | 需求报告第 360–364 行；`CORE_QA_RESULTS.md:66`、`:70`、`:75`；`docs/AGENT_ARCHITECTURE.md:64` |
| 原始 P0：本地持久化、流式失败恢复和模型连接 | SQLite 自动保存、启动时恢复中断运行；历史 QA 包含真实 macOS WebView 跨进程恢复与本机代理模型探针 | 没有 Windows / Linux 实机验证；其它真实云模型未验；数据库持久化不能替代用户可操作的备份恢复 | `src-tauri/src/application/service.rs:80` 的 `initialize`；`CORE_QA_RESULTS.md:28`、`:43`、`:84` |
| 原始 P0：工作区管理、导航和搜索 | 创建、重命名、归档；路线图和 Context Tree 导航；Context Tree 可按当前投影字段筛选 | Focus 的“搜索工作区”按钮没有事件处理；树筛选只检查回答预览，未提供 Prompt / 回答全文的跨工作区搜索；路线图未保存视口 | 需求报告第 342–347、375–378 行；`src/features/conversation/FocusWorkspace.tsx:1466`、`:1414`；`src/features/context-tree/ContextTree.tsx:112` 的 `visibleNodes`；`src/features/route-map/RouteMap.tsx:298` 的 `ReactFlow` |
| 原始 P0：开放导出、手动备份与恢复 | 已有 Decision Packet Markdown 导出 | Decision Packet 不包含完整工作区可恢复状态；生产 IPC 未提供完整工作区 JSON 导出、导入或备份恢复；未见完整删除 / 归档恢复入口 | 需求报告第 373–379 行；`src/platform/desktop-bridge.ts:64` 的 `DesktopBridge` 工作区接口、`:120` 的 `exportDecisionPacket`；`src-tauri/src/application/service.rs:4462` 的 `export_decision_packet_impl` |
| 后续定位：精确分支 → Context Diff → 路线比较 → 采纳 / 否决 → Decision Packet | 决策工作面比较两个 Model Run 的回答及上下文，保存判断理由，导出回答、Receipt hash 和 checkpoint 来源 | 决策主体仅能指向 Model Run，尚无 Agent 结果或外部证据身份；结果是否方便同事审查仍未有用户验证 | `PRODUCT_BLUEPRINT.md:32`；`src/features/decision/DecisionWorkspace.test.tsx:107`、`:167`、`:207`；`src-tauri/src/application/service.rs:4437` 的 `mark_decision_impl`、`:4512` 的导出循环 |
| 用户新增范围：Orca 多 Agent 编排与 OMP 执行 | 工作区中创建 Mission，人工拆分任务，查看状态和输出、回复问题、释放执行器及重新连接；持久化控制操作回执；后续真实 OMP 单任务已跑通 | Mission 只关联工作区，没有冻结精确路线依据；任务结果未进入 Decision Packet；真实多 Agent 并发、依赖图和长任务未验；停止和工具审批仍在 Orca / OMP | `src/shared/contracts/agents.ts:17` 的 `AgentMission`、`:79` 的 `CreateAgentMissionInput`；`src/features/agents/AgentWorkspace.tsx:118`；`src-tauri/migrations/0006_agents.sql:2`；`docs/LIVE_AGENT_QA_RESULTS.md` |
| 用户验证：跨天恢复、30+ 节点定位、第二次任务复用、留存与付费 | 有工程性能、恢复和原生界面的历史 QA 记录 | 没找到按计划开展的 8–12 名用户、7–14 天真实项目研究及留存 / 付款结果；长期同步、协作、RAG 等能力不能记为已实现 | 需求报告第 1191–1216 行；`PRODUCT_BLUEPRINT.md:34`；`CORE_QA_RESULTS.md:16`、`:86` |

表中的“需求报告”均指仓库根目录的 [AI分支对话产品需求与架构调研报告.md](../AI分支对话产品需求与架构调研报告.md)。

## Agent 扩展如何延续原始构想

原始文件曾把 Agent 列为首期后置项目。这表达了当时的验证顺序；用户此后已明确要求融合 Orca 与 OMP，并允许依赖 Orca 运行时，应以新的产品范围为准。

新增执行能力与“让每条思路都有来路”相容。真正需要修补的是两段流程之间的关联：当前 Agent 面板仅从应用获取 `workspaceId`，Mission 创建输入只有仓库与目标文本；Decision Packet 则只遍历 Model Run 的决策标记和发送快照。用户无法从一个任务直接核对它依据哪条准确路线，也无法把任务成果连同来源提交为可审查证据。

对应证据是 `src/app/App.tsx:296`、`src/features/agents/AgentWorkspace.tsx:118` 和 `src-tauri/src/application/service.rs:4512`。原始市场定位在 `产品市场定位与切入策略调研.md:260–269` 已提出未来 Agent 关键步骤可检查、可分支、可批准和可回放；前一轮实施计划也将“结果进入决策闭环”明确保留为后续（`docs/AGENT_ARCHITECTURE.md:77`）。

最值得优先设计的连接是：**明确路线依据 → 执行任务 → 捕获带来源的结果 → 用户审阅 → Decision Packet**。界面可以保留不同工作面，领域模型必须能说明这些环节的关系。

## 按产品价值排序的缺口路线

下列顺序以“用户可以找到、保留和核对自己的推演”为依据。真实用户验证从第一项开始并行开展，无需等全部功能完成。

| 顺序 | 交付 | 用户得到什么 | 验收标准 |
| --- | --- | --- | --- |
| 1 | 完整全文搜索与准确定位 | 跨天后能找回结论及其准确回答来源，检验产品最初的导航价值 | 搜索覆盖工作区标题、Prompt 和回答全文；命中长回答中不在预览内的词句；点击结果打开对应 Model Run，不误选另一回答版本 |
| 2 | 可恢复的工作区数据包与备份恢复 | 本地数据可迁移、可验证恢复；能够放心把长期项目保存在产品里 | 明确版本的导出格式包含路线、回答、决策和必要凭证；导入检查引用完整性；在全新数据库恢复后保持 Model Run、Receipt 与分支关系一致；外部 Agent 运行状态不能被导入操作当作新的派发指令 |
| 3 | Agent 依据与结果证据进入决策闭环 | 能说明“依据什么执行、实际观察到什么、为什么采纳”，减少手工复制与出处丢失 | 从准确 Model Run 显式选择任务依据；冻结授权传出的内容及 hash；按 Mission / Task / Dispatch 捕获结果内容与时间；用户作采纳 / 否决 / 待验证判断后，Decision Packet 保留来源。CLI 成功、worker 退出和任务完成保持不同语义 |
| 4 | 真实用户验证与一次真实 OMP 任务的完整旅程 | 确定哪些能力值得继续投入，验证接入真实环境后的可用性 | 工程验收单独记录真实 OMP 的派发、结果、证据捕获和恢复；用户研究按自己的项目观察分支回访、24 小时后定位、30+ 节点误入率和第二个任务主动复用。付费意向与实际付款分别记录 |

其中第 3 项是下一步最关键的产品架构优化。应保留 `ModelRun`、Orca `Run`、`Dispatch`、控制 `Operation` 和结果证据各自的身份，不把所有状态塞进同一个 Run，也不为 OMP 输出伪造 ThoughsFlow 的 Provider Receipt。它可以沿用现有 Orca 运行时和操作回执模块逐步实现，无需先重写执行平台。
