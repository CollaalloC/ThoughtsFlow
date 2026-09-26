---
status: accepted
---

# 用 Orca 运行时编排 OMP，由 ThoughsFlow 提供工作台

用户在 2026-09-22 确认集成到 ThoughsFlow，并最终选择允许使用 Orca 运行时。ThoughsFlow 通过本机 Orca CLI 的结构化接口管理协作任务；Orca 拥有 Run、Task、Dispatch、工作区和执行器生命周期，OMP 拥有模型、工具、技能、MCP 与 Agent 会话。

不在 ThoughsFlow 中再实现一套调度器，也不同时直接控制同一个 OMP 进程。这样可以复用 Orca 的实际多 Agent 能力，同时保留现有对话、Context Receipt 和决策模块的边界。代价是 Agent 工作面依赖兼容的本机 Orca 与 OMP；普通对话功能继续独立运行。

每个协作目标有专属 Orca 协调者终端。ThoughsFlow 本地只保存关联与操作回执，不复制 Task 状态作为第二份权威数据。外部调用与 SQLite 不能形成原子事务，未知结果必须保留并由人核查，不能以超时为依据重复启动执行器。
