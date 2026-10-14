# Agent 历史读取与输入性能

2026-10-06，macOS arm64，Node 24.20.0、SQLite 3.54.0。优化前基线为 `561a6dc`。本轮将 Agent 输出观察和交接历史改为每页最多 50 条摘要，展开后才读取及挂载正文；翻页仍可访问全部历史。历史与图组件通过 memo 隔离任务输入更新。数据库记录、原始回执及 hash 不改写，不新增 schema。

## SQLite 与传输数据量

`python3 scripts/benchmark-agent-history.py` 在隔离的内存 SQLite 中应用真实迁移，从 `AgentStore::evidence_page`、`handoff_page` 提取当前查询，与原先全量 `body_json` 查询运行在同一个 fixture 上。每个 scope 含相同数量的 evidence 和 handoff，并另设同等数量的无关 scope 记录，evidence 正文为 64,000 字节、handoff 正文为 32,000 字节。

预热两次、计时七次；计时包含 SQL 读取与 Python JSON 解码。分页读取 51 行判断是否还有下一页，只返回和解码 50 条。计时之外，逐页遍历并断言全部摘要与旧查询结果具有相同的 scope、身份、顺序和字段。查询计划使用现有 scope/rowid 索引；证据标题通过交接主键关联取得。

| 每类目标记录数 | 原全量返回 JSON | 当前首页返回 JSON | 原两条查询中位数之和 | 当前两条查询中位数之和 |
|---:|---:|---:|---:|---:|
| 500 | 48,363,384 字节 | 32,600 字节 | 49.84 ms | 2.28 ms |
| 1,000 | 96,727,884 字节 | 32,600 字节 | 98.57 ms | 2.21 ms |

这是“返回全部正文”与“返回第一页摘要”的有意行为调整，不是同等输出量的算法倍数比较。上述时间之和不是一次完整 IPC 的端到端测量，也不包含磁盘 I/O、Rust 序列化或 WebView。摘要查询仍需要 SQLite 读取、解析所访问记录的 JSON；展开正文仍有单条详情的读取成本。完整样本和执行计划见 [SQL 测量](measurements/agent-history-sqlite-20261006.jsonl)，包含 0、100、500、1,000 四组场景。

## React 历史区域

实际 `AgentHandoffPanel` 在 React/jsdom 中挂载，Graph 与 ContextTree 使用空组件隔离，以保持前后测量范围一致。旧 fixture 返回全部历史正文；新 fixture 只通过分页 API 返回第一页摘要，不在 mock 中偷偷预载正文。输入控件查找、断言和清理均在计时之外，输入计时为 `fireEvent.change` 至同步 React 提交结束。

| 每类后台记录数 | 初次挂载：旧 → 新 | 标题修改：旧 → 新 | 闭合正文字符：旧 → 新 |
|---:|---:|---:|---:|
| 0 | 2.50 → 2.84 ms | 0.82 → 0.58 ms | 0 → 0 |
| 100 | 36.12 → 10.36 ms | 13.93 → 0.60 ms | 9,600,000 → 0 |
| 500 | 125.25 → 6.91 ms | 117.21 → 0.26 ms | 48,000,000 → 0 |
| 1,000 | 184.65 → 5.01 ms | 162.12 → 0.24 ms | 96,000,000 → 0 |

每组预热两次、测量七次。1,000 条的旧基线在保存的原组件副本上单独采集。JIT、宿主负载及运行顺序会影响毫秒值，因此不从较大数据集的较小数字推断“记录越多越快”。稳定的结构边界是：当前页面最多 100 个闭合 details、336 个元素、0 个正文 pre；标题输入不增加 Context、摘要或详情 RPC。自动测试断言这些工作量边界，不设置毫秒阈值。

当前测试可重现并将原始数据写入临时文件：

```sh
THOUGHTSFLOW_BENCH_OUTPUT=/tmp/agent-history-current.jsonl \
  npm run test:run -- tests/performance/agent-history.test.tsx
npm run test:run -- tests/performance/agent-workflow-graph.test.tsx
```

输出文件按 JSONL 追加；需要独立新样本时选用新的文件名。历史基线和本轮结果分别见 [before](measurements/agent-history-20261006-before.jsonl)、[after](measurements/agent-history-20261006-after.jsonl)。旧实现需要完整列表 fixture，不能拿当前摘要 fixture 直接运行旧组件作为复现。

图的独立结构检查验证：50 条交接共享两个 Context 和一条 Evidence 时只投影 53 个唯一节点、150 条唯一且有箭头的边；相同输入 props 的父组件更新不会再次调用 ReactFlow，改变来源名称保持边 ID 稳定。这里使用了 ReactFlow mock，没有测量图库绘制、布局、滚动或桌面操作速度。

## 已有快照基线与范围

固定 25 ms CLI 延迟模型仍通过：九轮中位数 83.32 ms，每轮五次 CLI 调用。历史页面不跟随每五秒进展轮询重新读取；首次进入、保存证据、启动任务后及显式重载才读取。此次优化针对历史打开及输入工作量，不据此宣称运行时或模型执行加速。

五项新增性能/结构测试通过，SQL 完整分页断言通过。以上均为本地 macOS fixture 结果，不能替代原生 WebView 交互验收或 Windows/Linux 实机、runner 证据。

## 2026-10-14 最终代码复测

在补齐“查看最新”入口和可订阅草稿状态后，以同样条件重跑基准。1,000 + 1,000 条历史的旧全量结果仍为 96,727,884 字节，当前第一页仍为 32,600 字节；两条 SQL 各自中位数之和为 105.27 → 2.30 ms。实际页内有 338 个元素（增加两个查看最新按钮）、100 个折叠详情、0 个正文节点。

500 + 500 条历史场景的当前 jsdom 挂载中位为 5.71 ms，标题修改为 0.41 ms；输入不增加 RPC，正文仍不预载。与 10 月 6 日旧基线的比较受日期和宿主负载影响，不将毫秒差异表述为稳定的桌面提速倍数。

原始结果：[最终 SQL](measurements/agent-history-sqlite-20261014.jsonl)、[最终 UI](measurements/agent-history-ui-20261014.jsonl)。五项工作量/结构检查及完整分页校验再次通过。桌面交互验收与整体回归结果见[本轮记录](WORKBENCH_QA_PERFORMANCE_20261006.md)。
