# Contributing to ThoughtsFlow

欢迎提交问题、改进文档、补充平台验证和贡献代码。中文或英文均可。开始较大的功能改动前，请先在 [Issues](https://github.com/CollaalloC/ThoughtsFlow/issues) 描述使用场景和计划，以便明确范围。

## 开发环境

安装 Node.js 24、Rust 1.97.1 和 [Tauri 2 对应平台依赖](https://v2.tauri.app/start/prerequisites/)，然后：

```bash
npm ci
npm run tauri:dev
```

请使用锁文件安装依赖。不要提交 API Key、`.env`、个人路径、模型对话、工作区数据库、运行时日志或本地工具的认证配置。

## 提交一个改动

1. 从 `main` 创建分支，让每个改动有明确的使用场景或缺陷复现步骤。
2. 保持改动集中；涉及行为变化时加入能够证明该行为的测试。
3. 更新受影响的文档和 `CHANGELOG.md` 的 `Unreleased` 部分。
4. 运行与改动相关的检查，在 PR 中写明结果、验证平台和未覆盖的边界。
5. 用可独立审查的提交描述“改变了什么、为什么”。不要把自动格式化或依赖升级混入无关功能。

提交贡献即表示你有权提供这些内容，并同意贡献遵循项目 [LICENSE](LICENSE)。引入上游代码或资源时，保留版权、许可证和必要声明，同时更新 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)；不要只写“灵感来自”来替代许可证义务。

## 检查与测试

常规检查不需要模型账号：

```bash
npm run check
npm run test:webview:typecheck
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked --all-targets --all-features
```

`npm run check` 包含前端测试、HTTP/Orca fixture、桌面启动器测试和前端构建。Rust 测试覆盖领域行为、SQLite 事务和 Tauri IPC。只运行浏览器界面不能证明原生桌面功能通过。

在有图形会话的平台上，可另外运行原生 WebView 旅程：

```bash
npm run test:webview
npm run test:webview:agents
```

这些测试使用独立应用标识、测试数据目录和本地 fixture；测试驱动与故障注入仅在测试 feature 下启用。Linux 的 CI 图形运行需要 Xvfb/DBus。平台依赖、浏览器引擎和 CI 层次见 [CROSS_PLATFORM.md](docs/CROSS_PLATFORM.md)。

真实模型与真实 Agent 测试是显式 opt-in。`test:webview:agents:live` 会使用现有 Orca/OMP 配置并派发任务，`test:webview:live-proxy` 会调用其配置的本机模型端点；它们可能产生费用和文件写入。运行前阅读测试脚本与[真实 Agent 验证说明](docs/LIVE_AGENT_QA_RESULTS.md)，不要向普通 PR 检查注入私人账号凭据。

## 需要保持的架构约束

- UI 通过 `src/platform/desktop-bridge.ts` 访问 Rust，业务规则和外部 I/O 边界留在后端。
- 分支指向精确的 Model Run；重试创建新结果，不覆盖历史。
- Context Receipt 一经保存即不可变，不根据后续配置或树状态重新计算。
- 异步响应必须核验工作区、任务和操作身份；迟到结果不能成为新操作的授权。
- Agent 命令接受、任务完成和执行器存活是不同事实。未知操作不能自动重发。
- 数据迁移兼容历史库；不要为性能优化删除事务边界或遗漏共享内容。
- 凭据不进入数据库、日志、快照或导出。外部进程使用明确的可执行文件和参数，不把任务文本拼接为 shell 命令。

领域术语见 [CONTEXT.md](CONTEXT.md)，演进记录见 [docs/adr](docs/adr) 和 [ARCHITECTURE_EVOLUTION.md](docs/ARCHITECTURE_EVOLUTION.md)。性能改动应附可复现输入、测量范围与前后结果；单条 SQL 基准不能代表整个应用的提速。

## 报告问题

请包含应用版本、操作系统/架构、最小复现步骤、预期与实际结果。模型问题可补充供应商、协议和模型 ID；Agent 问题可补充 Orca/OMP 版本。分享截图和日志前删去密钥、私有代码、目录和对话内容。

安全漏洞请按 [SECURITY.md](SECURITY.md) 私密报告，不要直接公开可利用细节。

## English

Contributions and issue reports are welcome in Chinese or English. Use Node.js 24, Rust 1.97.1 (rust-toolchain.toml) and Tauri's platform prerequisites. Start from `main`, keep changes focused, add meaningful behavioral tests, and document the platforms and commands you actually verified. Contributions are made under the project's license. Preserve attribution for third-party code. Never commit credentials or personal workspace data; report security issues privately as described in `SECURITY.md`.
