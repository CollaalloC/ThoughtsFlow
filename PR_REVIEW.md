# PR Review — 架构审查修正实施

**Mode:** PR Review
**Scope:** 23 files, +1292/-898 lines (excluding `package-lock.json` and `ARCHITECTURE_REVIEW.md`).
**Skip:** `package-lock.json` (lock file), `ARCHITECTURE_REVIEW.md` (review artifact, not production code).

## Summary

这次变更将审查报告中的 4 个发现落地为代码修正：goal/system_prompt 分离、export destination 移除、端口驱动分层正式化和 Tauri 错误规范化。同时加入了路由级代码分割。整体质量高——18 个前端测试 + 51 个 Rust 测试 + clippy + fmt 全部通过，bundle 从 583KB 降到 388KB 主 chunk。

## Findings

### 🟡 Cognitive Overload — `checkpoint` 和 `persist_terminal_run` 的 read-rehydrate-write 循环

**Symptom:** 每次 checkpoint 现在执行两次数据库往返——先 `get_run` 读出完整 `ModelRun`，用 `rehydrate` 重构一个新 `ModelRun`，再传给 `checkpoint_run` 提取回原始字段。

**Source:** `src-tauri/src/application/service.rs:626-656`（`checkpoint`）和 `service.rs:665-697`（`persist_terminal_run`）。

```rust
async fn checkpoint(...) -> Result<(), RepositoryPortError> {
    let current = repository.get_run(run_id).await?;       // ← SELECT
    let current_state = current.state_snapshot();
    let run = ModelRun::rehydrate(
        RunDraft { id: current.id, turn_id: current.turn_id, ... },
        RunStateSnapshot { status: RunStatus::Streaming, output_markdown: output.into(), ... },
    ).map_err(...)?;
    repository.checkpoint_run(&run).await                   // ← UPDATE
}
```

`checkpoint_run` 的端口签名接收 `&ModelRun`，但 SQL 实现只用其中 5 个字段（`id`、`output_markdown`、`reasoning_markdown`、`usage`、`checkpointed_at`）。调用方必须构造完整领域实体才能传递这 5 个值。

**Consequence:**

1. **每次 checkpoint 多一次 SELECT。** 流式输出每 400ms 或 4KB 触发一次 checkpoint。在高吞吐流中，这使数据库写操作从 1 次/周期变为 2 次/周期，每秒最多增加 2.5 次查询。SQLite 本地仍快，但延迟和 WAL 压力会线性增加。
2. **领域模型短暂不一致。** `checkpoint` 硬编码 `status: RunStatus::Streaming`（`service.rs:644`）。如果 Run 在 event loop tick 之间被取消（`cancel_run` 调用了 `cancellation.cancel()`），`get_run` 返回的是 `Cancelled` 状态，但 `rehydrate` 用 `Streaming` 构造。SQL 守卫（`WHERE status = 'streaming'`）会拒绝写入并返回 `Conflict` 错误，event loop 进入 `finalize_storage_failure`——一个取消的 Run 可能被错误标记为持久化失败。

**Remedy:** 端口方法 `checkpoint_run` 和 `finish_run` 的签名接收 `&ModelRun`，但调用方已经持有所需数据（`output`、`reasoning`、`usage` 是 event loop 的本地变量）。考虑两种方向：

- **A（最小改动）：** 保持当前签名，但在 `checkpoint` 函数中用 `current.state_snapshot().status` 而非硬编码 `Streaming`。如果状态不是 `Streaming`，直接返回 `Ok(())` 跳过 checkpoint（Run 已被取消或已完成，不需要 checkpoint）。
- **B（更深但更干净）：** 收窄端口签名。`checkpoint_run` 改为接收 `run_id: &str` 和一个 `RunCheckpoint` 结构体（5 个字段），不依赖完整 `ModelRun`。`finish_run` 同理。消除 read-rehydrate-write 循环，每次 checkpoint 恢复为单次 UPDATE。

Source: Ousterhout, *A Philosophy of Software Design*, Ch. 4 — "Deep Modules"; Feathers, *Working Effectively with Legacy Code*, Ch. 4 — Seam Model.

---

### 🟡 Dependency Disorder — `RepositoryPort` 接口过大（ISP 违反）

**Symptom:** `RepositoryPort` trait 从 12 个方法增长到约 25 个方法。`spawn_run` 的 event loop 只用 3 个（`mark_run_streaming`、`checkpoint_run`、`finish_run`），`inspect_context_impl` 只用 4 个，但两者都依赖完整 trait。

**Source:** `src-tauri/src/ports/mod.rs:80-142`。

**Consequence:** 测试 `spawn_run` 的 event loop 需要实现或 mock 全部 25 个方法。当前测试通过真实 SQLite 绕过了这个问题，但如果未来需要用 fake repository 测试 event loop（如模拟 checkpoint 冲突），mock 表面积太大。`spawn_run` 的签名接收 `Arc<dyn RepositoryPort>`，暗示它可能使用任何仓储方法——但实际只用了 3 个。

**Remedy:** 不需要立刻拆分。当前只有一个适配器（`SqliteRepository`），拆分接口在第二个适配器出现前属于 YAGNI。但如果要测试 event loop 的存储失败路径，可考虑引入一个 `RunStore` 子 trait（`mark_run_streaming` + `checkpoint_run` + `finish_run` + `get_run`），让 `spawn_run` 依赖它而非完整 `RepositoryPort`。标注为 "when second adapter or fake appears, split here"。

Source: Martin, *Agile Software Development*, Principles, Patterns, and Practices — Interface Segregation Principle.

---

### 🟢 Knowledge Duplication — 前端 catch 块未利用 `DesktopBridgeError` 的 `code` 和 `retryable`

**Symptom:** `DesktopBridgeError` 正确保留了 `code`、`retryable` 和 `details`，但 13 个 catch 块仍然只取 `reason.message`。

**Source:** `src/features/conversation/FocusWorkspace.tsx:232,252,282,417,467,488,502,519,532,545`；`src/app/App.tsx:111,149`；`src/features/settings/ProviderSettings.tsx:62,133,158`。

```ts
// 当前：丢弃 code 和 retryable
setError(reason instanceof Error ? reason.message : "发送失败。");
```

**Consequence:** 用户看到错误消息（因为 `DesktopBridgeError extends Error`，`instanceof Error` 返回 true），但无法区分可重试错误和永久错误。例如 `preview_hash_mismatch`（不可重试，需要重新检查 Context）和 `provider_unreachable`（可重试，网络恢复后可重试）显示效果完全相同——一段文字，没有重试按钮。

**Remedy:** 在组件中检查 `reason instanceof DesktopBridgeError`，利用 `retryable` 决定是否显示重试按钮。不需要改所有 13 处——集中在最关键的 3 个（`startTurn` 的 `send failed`、`retryRun` 的 `retry failed`、`inspectContext` 的 `context check failed`）。

```ts
import { DesktopBridgeError } from "../../platform/desktop-bridge";
// ...
} catch (reason) {
  setBusy(false);
  if (reason instanceof DesktopBridgeError) {
    setError(reason.message);
    if (reason.retryable) setCanRetry(true);
  } else {
    setError(reason instanceof Error ? reason.message : "发送失败。");
  }
}
```

Source: Feathers, *Working Effectively with Legacy Code*, Ch. 4 — the value of a seam is what you can observe through it.

---

### 🟢 Coverage Illusion — `DesktopBridgeError` 的 Tauri 真实拒绝路径未测试

**Symptom:** `desktop-bridge.test.ts` 测试了 `normalizeDesktopBridgeError` 对 mock 对象的转换，但没有验证 Tauri 真实 `invoke` 拒绝时返回的 value 形状。现有前端测试用 `new Error(...)` 或 `mockRejectedValueOnce({ code, message })` 模拟拒绝，但前者已被 `normalizeDesktopBridgeError` 的 Error 分支覆盖，后者已被对象分支覆盖——两条路径都有测试。

**Source:** `src/platform/desktop-bridge.test.ts:17-20`。

**Consequence:** 如果 Tauri 在某些环境下将 reject value 包裹为 `{ data: { code, message } }` 或其他非预期形状，当前 normalization 不会捕获。这是一个理论风险，不是确认 bug。

**Remedy:** 在 E2E 测试中（当原生 harness 可用时）增加一个用例：触发一个必然失败的 Rust 命令（如向不存在的 workspace_id 导出），验证前端收到 `DesktopBridgeError` 且 `code` 字段正确。标注为 "when native E2E harness exists"。

Source: Feathers, *Working Effectively with Legacy Code*, Ch. 1 — untested integration points are the riskiest seams.

---

### 🟢 Accidental Complexity — `update_workspace` 用 `save_workspace` 实现 upsert

**Symptom:** `update_workspace`（`service.rs:1195-1213`）从 `save_workspace` 重新构建整个 `Workspace` 结构体，而非调用目标化的 update 方法。

**Source:** `src-tauri/src/application/service.rs:1195-1213`。

```rust
fn update_workspace(&self, input: UpdateWorkspaceInput) {
    let current = self.repository.get_workspace(&input.id).await...;
    self.repository.save_workspace(Workspace {
        id: current.id,
        title: input.name.unwrap_or(current.title),
        goal: input.goal.unwrap_or(current.goal),
        system_prompt: input.system_prompt.unwrap_or(current.system_prompt),
        ...
    }).await...
}
```

`save_workspace` 的 SQLite 实现先做 `get_workspace` 检查存在性，再 INSERT 或 UPDATE。所以 `update_workspace` 实际执行了 **两次** `get_workspace`：一次在应用层（读 current），一次在仓储层（检查存在性）。

**Consequence:** 每次工作区更新多一次 SELECT。功能正确，但产生冗余查询。更重要的是，`save_workspace` 的 upsert 语义（可能 INSERT）对于 `update_workspace` 用例是多余的——更新一个已打开的工作区永远不会 INSERT。

**Remedy:** 在 `RepositoryPort` 上增加 `update_workspace` 方法（目标化 UPDATE），或在 `save_workspace` 实现中消除冗余检查。优先级低——工作区更新不是热路径。

Source: Fowler, *Refactoring*, Ch. 3 — Middle Man smell; Ousterhout, Ch. 9 — "Different layers should have different abstractions."

## Quick Test Check

| 信号 | 结果 |
|---|---|
| 新行为是否有测试 | ✓ `workspace_goal_migration_repairs_v1_goal_as_system_prompt_rows`、`structured_run_failure_survives_rehydration`、`creates an untitled-goal workspace without turning placeholder copy into model context`、`decision_packet_contract_rejects_webview_supplied_paths` |
| Mock 滥用 | ✓ 未发现 |
| 测试名称清晰度 | ✓ 测试名称表达场景和预期结果 |

新行为有良好测试覆盖，无 mock 滥用，测试名称清晰。

## Ponytail Review

```
service.rs:633: extra SELECT in checkpoint. get_run + rehydrate + checkpoint_run. Pass checkpoint fields directly to port, 1 query.
service.rs:674: same pattern in persist_terminal_run. Extra get_run before finish_run.
service.rs:1202: save_workspace does its own get_workspace inside update_workspace. Redundant SELECT.
net: ~3 queries removable per checkpoint/terminal/update cycle.
```

## Recommended fix order

1. `checkpoint` 硬编码 `Streaming` → 用实际状态或提前返回（防取消后误标 storage failure）
2. 消除 `checkpoint` / `persist_terminal_run` 中的 `get_run` 往返
3. 利用 `DesktopBridgeError.retryable` 显示重试 UI
4. 消除 `update_workspace` 中的冗余 `get_workspace`
