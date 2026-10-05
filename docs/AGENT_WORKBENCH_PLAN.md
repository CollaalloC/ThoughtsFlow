# Agent 工作台与上下文交接

起点：`5d3e28b9e9840cd8412d23ad171ab0f109b89486`。2026-10-06 用户明确调整定位：ThoughtsFlow 的核心是对话上下文的图形化管理和工作流编排，复用现有 Agent 的模型与 harness，不再要求用户重建 Provider 配置或 Agent 生态。

## 架构决定

ThoughtsFlow 管理工作区、可视上下文、用户选择的交接内容和执行证据。Orca 管理执行器、隔离工作区和任务生命周期。现有 Agent 管理模型连接、工具循环、Skills、MCP、审批和内部会话。模型默认值继续由 Agent 自身配置决定；运行时未确认的实际模型不显示为已知。

主入口改为 Agent 工作台，可直接创建和选择工作区。已有 Provider 直连对话保留为兼容入口，配置和历史 Context Receipt 不迁移为 Agent 配置，不更改既有 hash。工作台的新交接快照表示实际提交给 harness 的文本，不能冒充完整模型请求记录。

## 本轮纵向行为

1. 创建或打开工作区，选择 Orca 已登记项目与协作目标；无需 Provider。
2. 为任务显式选择当前图谱路径和已保存的 Agent 输出证据。默认不隐式发送全部历史。
3. 预览最终交接文本与来源；后端使用现有 ContextCompiler 的 Provider 无关模式，保留固定、排除和分支检查点规则。
4. 启动前重新核验预览 hash、工作区归属及所选 cursor/draft/branch 版本、系统提示和检查点/继承集合，先持久化交接证据与操作身份，再调用现有 Orca worker-start。预览过期要求重新查看，不自动重试。新任务缺少预览确认时直接拒绝；旧请求只允许精确重放已有操作，不借兼容路径派发新任务。
5. 显式保存某次 Dispatch 的输出观察，保留运行、任务、Dispatch、来源、时间、文本 hash 与分页/完整性信息。输出与历史交接可离线读取；部分输出不能显示为完整答案。
6. 选择这些证据作为后续任务输入，展示来源关联，形成可追溯的人工编排工作流。

## 接口与测试边界

复用 AgentService 控制接口与 ContextCompiler，不创建新的通用 Agent SDK。新增接口集中于交接预览和不可变输出证据。测试穿越 AgentService / SQLite 与 DesktopBridge / 用户界面两个既有接口，使用固定 CLI fixture；覆盖无 Provider、跨工作区拒绝、预览过期、派发前落盘、未知结果不重派、幂等证据保存和离线读取。

验收使用组件测试、Rust 契约/数据库测试、类型检查、构建与三平台工作流配置检查。性能基准继续使用同样的固定 CLI 延迟，不把 fixture 结果声称为真实模型或跨平台运行结果。

## 已核实的上游边界

- Orca 1.4.220 公开 CLI 仍支持现有 supervised worker 链路，OMP worker 沿用自身模型配置，未声明 `--model` / `--effort` 覆盖。
- Orca 的结构化 `agentSession.create` 当前限定 Claude/Codex；没有通用的公开 native-chat CLI。因此不拼接猜测的会话命令。
- OMP 18.6.1 的 Direct RPC 可复用已有配置和完整 harness，支持会话与分支；接入需要独立的会话归属、事件相关性、审批与恢复闭环，后续按完整纵向功能实现。
- OMP 新 `auth-gateway stdio` 可复用已有认证，但只提供推理服务，且响应缓冲，不代表 Agent harness。

当前 Orca 路径面向已登记代码仓库与隔离工作区。通用无仓库聊天、跨 Agent 会话导入、自动 DAG 调度和自动重派不在本轮实现中；图上证据依赖不表示系统已经自动调度。原生 Agent 工具审批继续在 Orca / Agent 中处理。

## 回归记录

实施前：固定 fixture 10 项、工具链 3 项、Agent 契约 42 项通过；性能基准 9 轮中位数 81.218875 ms，每快照 5 次 CLI 调用。三份 CI 工作流 YAML、只读默认权限、三平台 / 四安装目标矩阵检查通过。初次 fixture 的沙箱回环监听限制与 Cargo 锁定依赖下载限制在获准执行后解除。

独立审查分别检查代码标准和本轮规格，发现并要求修复两个问题：检查点发布不一定推进 cursor/draft/branch 版本，必须在交接事务额外核验；全新 start-task 不得通过缺少 handoff 的旧输入绕过预览。输出证据另按正式 worker-read 契约验证 Dispatch、运行时和可选 projection 身份，且按声明的 source 读取对应内容。

实施后：147 项前端组件/Bridge 测试、10 项固定 fixture、3 项工具链检查、TypeScript 与生产构建通过。Rust formatting、Clippy `--all-targets --all-features -D warnings` 通过；完整 Rust 检查为 290 项库测试和 4 项原生 MockRuntime IPC/性能测试通过，1 项独立基准忽略。新增 schema 9→10 升级验证旧历史内容、hash、旧迁移元数据不变。第三方许可证清单与锁文件检查通过。

原生 WebView 测试二进制已构建成功；实际工作台旅程尚未通过。初次运行已完成无 Provider 工作区与 Mission 创建，但窗口处于后台，产品正确暂停刷新；WebDriver 的 switchWindow 只选择窗口，不激活它，因此测试专用配置和 helper 增加真实 setFocus。继续诊断确认窗口存在且未最小化，但未获焦点，WebView 为 hidden；获准的 CoreGraphics 控制台检查返回 `screenLocked=1`。已停止 GUI 重试，等待解锁后续跑，不覆写 document.visibilityState、不删除后台暂停保护。实验性的 app_show 权限已撤回，发布构建权限没有增加。

待解锁后的明确命令：`node tests/webview/run-agent-journey.mjs`，随后串行运行 `npm run test:webview:smoke` 和 `npm run test:webview:restart:run`。工作台测试涵盖预览与实际 argv 文本一致、保存输出、证据复用、离线重载 hash 不变及离线零派发；目前只能说明测试已准备，不能宣称这些原生步骤已执行成功。

本轮证据均来自本机 macOS，不提升 Windows/Linux 新版或真实 Agent 验证状态；未运行真实模型。最新已发布安装包仍为 v0.1.0-beta.2，本轮改动没有推送或发布。

最终另通过 `npm run tauri -- build --debug --no-bundle`，生成不含 WebDriver 的本机调试程序 `src-tauri/target/debug/thoughtsflow`；该程序尚未启动，不代表锁屏期间完成了交互验收。测试专用构建也已按最终权限配置重新构建，可在解锁后直接续跑上述旅程。
