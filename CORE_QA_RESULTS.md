# ThoughsFlow Core QA Results

验证日期：2026-07-22（Asia/Shanghai）

## 自动化验证

| 检查 | 结果 | 证据 |
|---|---:|---|
| React / TypeScript 组件与性能测试 | 通过 | `npm run test:run`：6 个测试文件、18 个测试通过 |
| TypeScript 与 production build | 通过 | `npm run build`；Vite 生产包成功生成 |
| Rust 格式 | 通过 | `cargo fmt --all -- --check` |
| Rust 静态检查 | 通过 | `cargo clippy --all-targets -- -D warnings` |
| Rust 测试 | 通过 | `cargo test --all-targets`：45 个测试通过 |
| 1,000 Turn 路线图 | 通过 | 固定数据集在 jsdom 中打开并选择精确 `run-999`；单跑 2.13 秒 |
| Playwright 旅程发现 | 通过 | Chromium / WebKit 各发现 8 条核心旅程，共 16 条 |
| Playwright 原生旅程执行 | 未在当前环境执行 | 默认命令明确跳过；需要 `THOUGHSFLOW_E2E_NATIVE=1` 与 `THOUGHSFLOW_E2E_BASE_URL`，重启旅程还需要 `THOUGHSFLOW_E2E_RESTART_URL` |

## macOS 原生冒烟

使用 debug `.app` 验证真实 Tauri → Rust → SQLite 路径：

- 成功创建本地工作区 `核心闭环原生冒烟`。
- Generic/Ollama Provider 设置界面可以编辑；远程 `http://models.example.com/v1` 被拒绝，loopback HTTP 保持允许。
- 向未运行的本地 Ollama `http://127.0.0.1:11434/api/chat` 发出真实请求后，Run 进入失败态；Prompt、Run、错误和 Context Receipt 均已可靠落盘。
- 关闭并重新打开 `.app` 后，工作区、失败 Run、Provider/Model/Host、内容顺序及 canonical SHA-256 hash 恢复。
- Context Receipt 显示本轮实际发送的 System Prompt 与 Current Prompt；失败后不再残留“正在等待 Provider”提示。
- Focus 的失败输出正常占满正文列，不再落入 44px 路线编号列。
- 全局打开路线图后，服务端从最近持久化 BranchPointer 恢复当前路线；失败 Run 正确显示为 `CURRENT`，路线计数为 1。
- 使用 loopback OpenAI-compatible 协议 fixture 完整手工驱动 8 条核心旅程；fixture 只提供 SSE 网络响应，工作区、Run、Snapshot、决策和导出仍由真实 Rust/SQLite 生产路径处理。
- 同一根 Turn 重试生成回答 A/B；从精确回答 B 建立“路线 A：渐进迁移”和“路线 B：一次替换”两个同父分支，路线 B 的锁定 Receipt 不包含路线 A。
- pin Ancestor Prompt、exclude Exact Ancestor Answer 后完成下一轮发送；锁定 Receipt 与预览的包含顺序一致，并保存新的 canonical hash。
- `[slow]` 运行取消后保留部分输出并记为 `cancelled`；`[disconnect]` 断流保留部分输出并记为 `failed`。
- `[hang]` 运行产生 checkpoint 后关闭并重启 `.app`；恢复为 `interrupted`，部分输出和恢复原因保留。
- 对两个真实 Run 执行回答与 Context Diff，分别保存采纳/否决理由，并成功导出 Markdown Decision Packet；未设置工作区目标时，`Problem` 回落到被比较的真实 Turn Prompt。
- 初始窗口配置为 1440×900，并向 1280×800 方向手工缩放检查；Focus、Inspector 和 Composer 未出现横向滚动条。当前 Computer Use 截图服务会把窗口归一化为 1229×768，因此这里记录的是原生视觉冒烟，不宣称像素级截图比对。

原生构建产物：

`src-tauri/target/debug/bundle/macos/ThoughsFlow.app`

## 已验证的核心语义

- 精确 `parent_run_id` 分支与兄弟分支 Context 隔离。
- 重试创建新 Run，不覆盖历史回答或 Snapshot。
- preview hash 变化阻止发送；Snapshot 保存实际内容和顺序。
- Provider 发送前的事务落盘、checkpoint、取消、断流和启动恢复。
- OpenAI-compatible SSE 与 Ollama NDJSON 的任意分片、错误体、usage 和非正常结束。
- SQLite STRICT schema、外键、事务回滚、不可变触发器和 terminal Run 保护。
- 远程 HTTP、内嵌凭据和非 HTTP(S) scheme 拒绝；API Key 仅保存在 Rust 进程内存。
- Run Compare、Context Diff、决策标记与 Markdown Decision Packet 导出契约。

## 未声称的验证

- 没有真实云 Provider 凭据，因此没有对外部付费模型做现场请求。
- 当前 macOS 环境没有可驱动 Tauri WebView 且支持硬重启的 Playwright/WebDriver harness；8 条旅程已实现但默认诚实跳过。
- 没有 Windows/Linux 实机运行证据；不声称三平台安装验证。
- production JavaScript 主包约 583 kB，Vite 会给出 chunk-size warning；它不阻塞当前核心闭环，但后续可按工作面做懒加载。
