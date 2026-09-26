# Agent 刷新与 Context 读取优化

日期：2026-09-26。起点：`ee0cf0c87054b0838fd7066b38002fe6c1ede6eb`。

用户要求继续完善架构和性能，并已授权按功能提交。沿用 ask-matt 的架构调查 → 有界实现 → 两轴审查流程。本轮使用现有 AgentWorkspace 和 Repository 公开 Interface 验证，保留 ADR 0001、0002 的生命周期与结果归属规则。

## 选定的改动

### Agent 刷新

当前刷新若发现已有读请求就直接返回，操作完成后的刷新也会被丢弃。固定定时器还会在隐藏页面发起读取。

- 任何时刻最多执行一组快照与操作回执读取。
- 合并读请求期间的显式刷新、操作完成刷新和恢复可见刷新，当前请求结束后至多补一轮；定时 tick 不积压。
- 操作进行中不启动自动刷新；旧 revision 的结果不得覆盖操作后的状态。
- 页面隐藏时暂停周期读取，恢复可见时立即读取。保留旧进展供查看，但恢复后的控制操作须等待新状态确认。
- Mission / Workspace 切换与卸载不接收旧读结果，也不取消后端已提交任务。React StrictMode 重新挂载后仍能完成首次有效刷新。
- 不新增调度框架、共享缓存或运行时状态权威。

通过公开 AgentWorkspace 行为测试验证并发、延迟结果、可见性和卸载；用 native fixture 旅程验证桌面集成。隐藏页节省的调用数属于结构性改进，不声称为整机 CPU 或能耗实测。

### Context 内容读取

当前查询从全库 content_block 出发，逐项核查四种引用。工作区自身数据不变时，其他工作区增长也会拖慢读取。

- 从当前工作区的 Turn Prompt、Context Manifest、Context Checkpoint、Context Draft Override 引用出发，再按主键读取内容。
- 保留共享内容、去重和 `created_at, id` 排序；保留现有快照事务以及八次逻辑读取。
- 如查询计划证实缺少工作区索引，仅新增索引迁移，不修改历史迁移或已有内容、Receipt 和 hash。
- 无须改变 DesktopBridge、RepositoryPort、数据库事实模型或 Context 编译规则。

通过真实 SQLite 的公开 Repository Interface 验证四种来源、工作区隔离、重复引用和顺序。基准须包含无关工作区的 Turn 与 Manifest 历史，记录 SQLite 版本、规模、样本数以及执行计划。SQL 基准不等同于完整页面延迟。

## 暂缓

Branch revision 与 checkpoint inheritance 的联接可能放大中间行数，另开测量后再决定。回执折叠渲染、全文搜索、可恢复导出和 OMP SDK 接入不在本轮范围。跨平台 CI 的实际运行状态沿用现有记录，不以本机结果代替 Windows/Linux 验收。

## 完成条件

目标回归、现有相关套件、类型检查、Rust fmt/clippy 和原生 fixture 旅程通过；Standards 与 Spec 独立审查；更新结果记录；按功能提交当前分支，不推送。
