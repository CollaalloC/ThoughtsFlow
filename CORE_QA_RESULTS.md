# ThoughtsFlow Core QA Results

验证日期：2026-07-29（Asia/Shanghai）

## 最终自动化门槛

| 检查 | 结果 | 最终证据 |
|---|---:|---|
| React / TypeScript | 通过 | `npm run check`：13 个 Vitest 文件、91 项测试全部通过 |
| Provider fixture | 通过 | 9/9；验证真实出站 JSON、流分片、失败、断流与凭据脱敏 |
| TypeScript / production web build | 通过 | `tsc --noEmit`；主 JS 422.78 kB（gzip 130.47 kB），Route Map 181.65 kB（gzip 58.95 kB） |
| Rust 全目标/全 feature | 通过 | `cargo test --all-targets --all-features`：lib 220/220，native 3/3 |
| Rust 静态检查与格式 | 通过 | `cargo clippy --all-targets --all-features -- -D warnings`；`cargo fmt --all -- --check` |
| v1–v4 真实旧库升级 | 通过 | 4 个冻结 SQLite fixture 经 production migrator 升至 v5；原 SQLx checksum、Receipt、外键和不可变 trigger 保留 |
| SQLite Context Tree 仓储 | 通过 | 36/36；含 cursor/draft 双 CAS、单 snapshot、8 次逻辑 SELECT、checkpoint 顺序与完整回滚 |
| 前端 1,000 Run Context Tree | 通过 | 打开并点击精确 `run-999`：325 ms，门槛 5 s |
| 前端 1,000 Turn Route Map | 通过 | 打开并点击精确 `run-999`：2,797 ms，门槛 5 s |
| 后端 1,000 Turn / 2,000 Run 投影 | 通过 | 204.823 ms、8 次逻辑 SELECT，门槛 500 ms / 8 次 |
| 深度 1,000 Context 重建 | 通过 | 50.536 ms，门槛 100 ms |
| Tauri MockRuntime 原生旅程 | 通过 | `npm run test:native` 单 worker 3/3；真实 IPC、AppState 与文件 SQLite |
| 真实 macOS WKWebView | 通过 | `npm run test:webview`：smoke + 三个独立进程的崩溃窗口/重启旅程 |
| 本机 VibeProxy 实际模型 | 通过 | WKWebView → `http://localhost:8317/v1` → `gpt-5.6-sol`，固定提示精确返回 `TF_APP_OK` |
| Playwright 旅程发现 | 如实跳过 | Chromium/WebKit 共发现 18 条；无外部 Tauri URL 时 18 条默认 skip，不计作原生通过 |
| macOS release | 通过 | `.app` 6.6 MB；arm64 `.dmg` 3.4 MB |

## 真实 WKWebView 三进程证据

最终 `npm run test:webview` 使用独立应用标识符、临时数据目录和内嵌 macOS WebDriver：

- smoke 在 WebKit 605.1.15 中完成真实 Provider fixture 请求、精确 Run 选择和 reload 后 cursor 恢复；
- restart journey ID：`1785308684497-67390`；
- 三个 WDIO launcher PID：`67391 / 67422 / 67508`；
- crash operation ID：`cca5d12c-3c0d-46c7-87b8-6cab24ec3d8c`；
- 退出码严格为 `[0, 1, 0]`：第二进程的 `1` 是测试 feature 在 SQLite 提交后、IPC 响应前执行的预期 abort；
- 继续到第三进程前，外层 runner 已验证固定审计文件包含同一 operation ID，且摘要 Provider 请求恰好一次；
- 第三进程从同一数据库恢复已提交 checkpoint，原样重放输入后没有新增 checkpoint 或第二次 Provider 请求；
- checkpoint 前的 Receipt 在提交、abort、重开与重放后保持完全相等；
- checkpoint 后真实 Provider payload 包含摘要、保留尾部和当前 prompt，并排除已压缩 root/leaf；
- fixture 共收到 8 次预期请求。

WDIO 会打印“未安装外部 `tauri-driver`”诊断，但配置明确使用 `driverProvider: embedded`；真实 WebKit session 和所有断言均完成。测试驱动、固定 IPC 审计文件和 abort hook 只在 `webview-e2e` feature/测试构建变量下编译。最终 production 二进制已检查，不含对应环境变量、审计文件名或 WebDriver marker。

## 本机 `gpt-5.6-sol` 证据

显式 opt-in 的 `test:webview:live-proxy:run` 只发送代码中固定的提示：

```text
Reply with exactly TF_APP_OK and nothing else.
```

最终结果：

- Endpoint：`http://localhost:8317/v1`
- Model：`gpt-5.6-sol`
- Response：`TF_APP_OK`
- Run ID：`10aa2824-dca1-44f2-9cc0-ce52eed35d8a`
- Receipt hash：`97486f45f6ca99b2bbd126a9f1ca3db8d784ea4cc5055ecca4b110da7883a92d`

该探针没有发送仓库、用户工作区或其他项目内容，也没有在日志中输出 Receipt items 或凭据。

## 已验证的 Context Tree 语义

- `Turn.parent_run_id` 仍是唯一拓扑；活动 cursor 只投影精确 root→Run 路径。
- cursor、Branch Pointer 和 Context Draft 均持久化并带版本；切换路径时 cursor/draft 双 CAS 原子重基。
- 从当前 branch head 继续才推进原分支；历史节点发送或 retry 创建新分支，兄弟分支不泄漏。
- pin 使用 `sourceRef + contentBlockId + contentHash` 精确身份；system/current prompt 不可排除。
- draft 只在发送事务成功时消费；Provider 失败不消费；任一 cursor/draft/branch stale guard 会使整次 Run start 回滚。
- 原始路径始终可检查，`ContextPreview.items` 只表示实际有效 Context，`rawItems` 不混入 checkpoint 投影。
- compaction 必须选择精确 root-prefix 来源和 `firstKeptRunId`；branch summary 禁止压缩边界，也不能引用 anchor 之后的 Run。
- checkpoint 来源、hash、边界、摘要结果和真实 Provider snapshot 不可修改或删除；人工摘要的 Provider provenance 为 `null`。
- 同毫秒连续 checkpoint 以 workspace 内严格单调的持久时间保留提交顺序，不由随机 UUID 决定。
- 连续 compaction 只应用后写 checkpoint；旧 checkpoint 仍可审计；文件数据库重连后结论不变。
- checkpoint 前创建的分支不追溯继承；之后 fork 通过显式 inheritance 继承，并可传递到后续 fork。
- Context Tree marker 只挂载当前 branch 显式可见或 branchless 的 checkpoint；全量 checkpoint 仍保留在审计投影中。
- 旧 `StoredRunReceipt` 和公开 Snapshot 在连续 checkpoint、失败、abort、幂等 replay 与数据库重连前后保持完整对象相等。
- 摘要失败、取消或 CAS 冲突不激活 checkpoint、不移动 cursor；提交后响应前崩溃可按 operation ID 幂等恢复。

## Release 产物

- `src-tauri/target/release/bundle/macos/ThoughtsFlow.app`
- `src-tauri/target/release/bundle/dmg/ThoughtsFlow_0.1.0_aarch64.dmg`
- DMG SHA-256：`eafca5358f5722c0abd8787451beb5ef2f0ae139a18e46266f611018d7c4310c`

## 边界

- 1,000 节点前端门槛是 jsdom CPU/交互 gate；真实 WKWebView 旅程验证了原生可用性，但没有在 WebView 内批量生成 1,000 个节点。
- 没有 Windows/Linux 实机安装证据。
- Playwright 的 18 条旅程在缺少外部 Tauri URL 时明确跳过；它们没有被包装成“已通过”。
- 本轮只验证用户提供的本机 OpenAI-compatible 代理；没有使用真实 Anthropic、Google 或其他外部付费凭据。
