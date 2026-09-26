# 跨平台架构与验证边界

更新时间：2026-09-26。

ThoughtsFlow 的目标平台是 Windows、Linux 和 macOS。当前已准备三平台编译、单元测试和桌面 WebView 测试配置；Windows/Linux 的真实运行结论必须等待对应系统的 CI 与运行验证。当前仓库未配置 Git remote，本次没有推送或远端 CI 运行记录。

## 平台边界

React 工作台、IPC 数据结构、Rust 业务服务和 SQLite 数据以共用代码实现。操作系统差异集中在 Tauri 系统依赖、原生程序定位和进程启动处。模型厂商由 Provider adapter 处理，不能把模型协议与操作系统绑定。

ThoughtsFlow 的核心推演功能通过自己的 Provider 运行。Agent 协作另依赖本机 Orca/OMP 的实际可用性与协议能力。三平台编译通过不等于 Orca/OMP 在三平台均已可用；运行时诊断必须报告缺失或不兼容，不能将编译目标当成运行能力。

构建和 WebView 测试脚本统一使用 Node 24。`.node-version` 与 `package.json` 的 engines 共同限定该主版本，以直接运行仓库内的 TypeScript 测试入口。`scripts/desktop-toolchain.ts` 构造程序路径、独立 argv 和子进程环境，再由 Node 启动已安装包声明的 JS CLI 入口。它不调用 shell、不执行 `.bin/*.cmd`，路径含空格和参数中的 shell 字符保持原样。

- Tauri 使用 `node_modules/@tauri-apps/cli/tauri.js`。
- WDIO 使用 `node_modules/@wdio/cli/bin/wdio.js`。
- Windows 测试应用使用 `thoughtsflow.exe`，其他平台使用 `thoughtsflow`。
- WebView 构建以绝对 `CARGO_TARGET_DIR` 输出到 `src-tauri/target/webview-e2e`，运行配置与构建目录一致。
- WebView 服务向应用传入可信的 `process.execPath` 作为 `THOUGHSFLOW_NODE_BIN`，避免图形界面启动时 PATH 缺少 Node。
- `scripts/run-native-test.mjs` 保留原有 Cargo 查找顺序与测试行为。

## 开发环境

安装 Node 24、Rust stable 和对应平台的系统依赖后执行：

```text
npm ci
npm run check
npm run test:webview:typecheck
npm run tauri:dev
```

| 平台 | 原生依赖 | 应用 WebView |
| --- | --- | --- |
| Windows | Visual Studio C++ Build Tools，MSVC Rust toolchain，WebView2 Runtime | Microsoft Edge WebView2 |
| Ubuntu 24.04 | libwebkit2gtk-4.1-dev、build-essential、libxdo-dev、libssl-dev、libayatana-appindicator3-dev、librsvg2-dev 等 | WebKitGTK |
| macOS | Xcode Command Line Tools | WKWebView |

依赖以 [Tauri 官方 prerequisites](https://v2.tauri.app/start/prerequisites/) 为准。这里的 Linux CI 基线是 Ubuntu 24.04，不代表已经验证所有 Linux 发行版；安装包、签名、公证、商店发布和 ARM/x64 全排列验收仍需独立执行。

## CI 分层

`.github/workflows/desktop-ci.yml` 在 push、pull request 或手动触发时，为 `ubuntu-24.04`、`windows-latest`、`macos-latest` 分别执行：

1. `npm ci`、`npm run check` 和 WebView TypeScript 检查。
2. Cargo fmt、clippy、全部 target/feature 测试。
3. Tauri debug 构建，不生成安装包、不启用测试 WebDriver。

`.github/workflows/desktop-webview.yml` 仅手动触发，同样使用三平台矩阵。它构建带 `webview-e2e` 功能的独立测试应用，运行本地 Provider smoke、模拟 Orca 双 Agent 旅程和三进程崩溃恢复旅程。Linux 使用 D-Bus session 和 Xvfb；Windows/macOS 使用 runner 的原生桌面环境。测试驱动采用现有 embedded provider，上游说明支持三平台，但本项目的各平台旅程仍需实际跑通。[WDIO 平台说明](https://github.com/webdriverio/desktop-mobile/blob/main/packages/tauri-service/docs/platform-support.md)

原生旅程保留严格的平台真实性检查：macOS 为 `webkit / macos`，Linux 为 `WebKitGTK / linux`，Windows 为 `msedge / windows`。映射依据所安装测试插件的会话响应实现；没有用“任意浏览器均可”替换断言。

两套 workflow 均不调用真实模型、不注入模型凭据。真实 Agent 测试仍要求同时设置 `TF_AGENT_LIVE=1` 与非空 `TF_AGENT_LIVE_REPO_ID`，保留数据库和回执以检查不确定执行结果。真实代理测试也只由显式的 `test:webview:live-proxy` 命令启动，不进入常规或手动 fixture CI。

Actions 使用 2026-09-26 通过官方 API 核验的提交 SHA，而非浮动主版本：

| Action | 对应版本/来源 | 固定提交 |
| --- | --- | --- |
| actions/checkout | [v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) | `3d3c42e5aac5ba805825da76410c181273ba90b1` |
| actions/setup-node | [v7.0.0](https://github.com/actions/setup-node/releases/tag/v7.0.0) | `820762786026740c76f36085b0efc47a31fe5020` |
| dtolnay/rust-toolchain | [master 提交](https://github.com/dtolnay/rust-toolchain/commit/02cb101ec7c40f2c49e1d9714d64511d8e1b74de)，显式选择 Rust stable | `02cb101ec7c40f2c49e1d9714d64511d8e1b74de` |

Node 按 24 主版本更新，Rust 按 stable 更新；依赖树由 `package-lock.json` 与 `src-tauri/Cargo.lock` 固定。工具链更新造成的差异应通过三平台 CI 发现。

## 本轮已验证与未验证

本机 macOS / Node v24.20.0 下已通过：

- `npm run test:toolchain`：3 项测试，覆盖三平台 argv/路径规划、Windows `.exe`、含空格路径、环境隔离、拒绝未知平台、双重 live opt-in、子进程非零退出状态和实际无 shell 参数传递。
- `npm run test:webview:typecheck`。
- 新 runner 执行 `tauri --version` 返回 `tauri-cli 2.11.4`。
- 两份 workflow 的 YAML 解析、三平台矩阵和手动触发字段检查。

以上脚本验证没有调用真实 Agent。Windows/Linux 的原生构建和 WebView 旅程、远端 CI、安装包与真实 Orca/OMP 执行均不能由这些本机测试推断。后续接入 Git remote 并运行 workflow 后，应把运行链接、平台、commit、结果写入验证记录，再提升平台支持状态。
