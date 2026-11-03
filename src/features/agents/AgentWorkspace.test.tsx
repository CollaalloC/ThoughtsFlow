import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createDesktopBridge, DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type { AgentEnvironment, AgentEvidence, AgentEvidencePage, AgentEvidenceSummary, AgentHandoff, AgentHandoffSummary, AgentHandoffPreview, AgentMission, AgentOperation, AgentSnapshot, ContextTreeProjection } from "../../shared/contracts";
import { AgentWorkspace } from "./AgentWorkspace";
import { agentHistoryTime } from "./AgentHistory";

const environment: AgentEnvironment = {
  available: true, running: true, runtimeVersion: "1.0", templates: [{ name: "reviewer", description: "Existing Agent" }, { name: "writer", description: "Writer" }],
  runtimeId: "runtime-1", projects: [{ id: "repo-1", name: "Example", path: "/projects/example" }], message: null,
};
const mission: AgentMission = { runtimeBackend: "herdr-core",
  id: "mission-1", workspaceId: "workspace-1", repositoryId: "repo-1", repositoryPath: "/projects/example",
  objective: "实现协作功能", runId: "run-1", coordinatorHandle: "coordinator-1", runtimeId: "runtime-1",
  status: "ready", error: null, createdAt: "2026-09-22T00:00:00Z",
};
const snapshot: AgentSnapshot = {
  mission, connected: true, warning: null, tasks: [], messages: [],
};
const operation: AgentOperation = {
  id: "operation-1", missionId: mission.id, kind: "start-task", status: "succeeded",
  receipt: { dispatchId: "dispatch-1" }, error: null, createdAt: "2026-09-22T00:01:00Z",
};

const tree: ContextTreeProjection = {
  rootId: "root",
  workspaceId: mission.workspaceId,
  cursor: { workspaceId: mission.workspaceId, activeRunId: null, branchId: null, version: 0, updatedAt: "2026-10-06T00:00:00Z" },
  draftVersion: 0, nodes: [], edges: [], branches: [], checkpoints: [],
};
const preview: AgentHandoffPreview = { requestedTemplate: "reviewer",
  hash: "preview-hash", text: "任务与所选上下文", sourceRunIds: [], evidenceIds: [], sources: [],
  parentRunId: null, branchId: null, cursorVersion: 0, draftVersion: 0, branchVersion: null, compilerVersion: "1",
};
const evidence: AgentEvidence = {
  id: "evidence-1", workspaceId: mission.workspaceId, missionId: mission.id, runId: "run-1", taskId: "task-1", dispatchId: "dispatch-1",
  handoffOperationId: null, text: "Saved immutable observation", contentHash: "evidence-hash", source: "terminal", requestCursor: null,
  cursor: "next-page", hasMore: true, warning: "partial page", observedTaskStatus: "running", createdAt: "2026-10-06T00:00:00Z",
};

function evidenceSummary(item: AgentEvidence, title: string | null = null): AgentEvidenceSummary {
  const { text: _text, ...summary } = item;
  return { ...summary, title };
}
function handoffSummary(item: AgentHandoff): AgentHandoffSummary {
  return { operationId: item.operationId, missionId: item.missionId, workspaceId: item.workspaceId, createdAt: item.createdAt,
    title: item.preview.title ?? "", hash: item.preview.hash, sourceRunIds: item.preview.sourceRunIds, evidenceIds: item.preview.evidenceIds };
}

function fixture(overrides: Partial<DesktopBridge> = {}): DesktopBridge {
  return {
    ...createDesktopBridge(async () => { throw new Error("Unexpected IPC in test"); }),
    listWorkspaces: vi.fn().mockResolvedValue([]),
    getContextTree: vi.fn().mockResolvedValue(tree),
    setActiveContext: vi.fn().mockResolvedValue(tree.cursor),
    previewAgentHandoff: vi.fn().mockResolvedValue(preview),
    listAgentHandoffs: vi.fn(),
    listAgentEvidence: vi.fn(),
    listAgentEvidencePage: vi.fn().mockResolvedValue({ items: [], nextCursor: null }),
    listAgentHandoffPage: vi.fn().mockResolvedValue({ items: [], nextCursor: null }),
    getAgentEvidence: vi.fn().mockResolvedValue(evidence),
    getAgentHandoff: vi.fn().mockImplementation(async (input) => ({ ...input, workspaceId: mission.workspaceId, createdAt: "2026-10-06", preview })),
    captureAgentEvidence: vi.fn().mockResolvedValue(evidence),
    agentEnvironment: vi.fn().mockResolvedValue(environment),
    openAgentRuntime: vi.fn().mockResolvedValue(environment),
    listAgentMissions: vi.fn().mockResolvedValue([mission]),
    createAgentMission: vi.fn().mockResolvedValue(mission),
    getAgentSnapshot: vi.fn().mockResolvedValue(snapshot),
    listAgentOperations: vi.fn().mockResolvedValue([]),
    startAgentTask: vi.fn().mockResolvedValue(operation),
    replyToAgent: vi.fn().mockResolvedValue({ ...operation, kind: "reply" }),
    releaseAgentWorker: vi.fn().mockResolvedValue({ ...operation, kind: "release" }),
    reconnectAgentMission: vi.fn().mockResolvedValue({ ...operation, kind: "reconnect" }),
    readAgentOutput: vi.fn().mockResolvedValue({ text: "Tests passed", cursor: null, hasMore: false, source: "terminal" }),
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => { resolve = resolvePromise; reject = rejectPromise; });
  return { promise, resolve, reject };
}

async function ready() {
  expect(await screen.findByText("已连接 · 每 5 秒更新进展")).toBeVisible();
}

function fillTask() {
  if (!(screen.getByLabelText("Agent 模板") as HTMLSelectElement).value) fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: "reviewer" } });
  fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "实现任务列表" } });
  fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "仅修改任务列表，保持现有接口，运行组件测试验证。" } });
}

async function previewTask() {
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "预览交接内容" })); });
}

async function prepareTask() { fillTask(); await previewTask(); }

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

describe("AgentWorkspace", () => {
  it("keeps a newly created workspace when the initial workspace list arrives late", async () => {
    const initial = deferred<Awaited<ReturnType<DesktopBridge["listWorkspaces"]>>>();
    const created = { id: "workspace-new", name: "我的新工作区", goal: "保留任务证据", archived: false, createdAt: "2026-10-06" };
    const bridge = fixture({
      listWorkspaces: vi.fn().mockReturnValue(initial.promise),
      createWorkspace: vi.fn().mockResolvedValue(created),
      listAgentMissions: vi.fn().mockResolvedValue([]),
    });
    render(<AgentWorkspace bridge={bridge} />);
    fireEvent.change(screen.getByLabelText("工作区名称"), { target: { value: created.name } });
    fireEvent.click(screen.getByRole("button", { name: "创建工作区" }));
    expect(await screen.findByRole("option", { name: created.name })).toBeInTheDocument();
    await act(async () => initial.resolve([]));
    expect(screen.getByRole("option", { name: created.name })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "本地工作区" })).toHaveValue(created.id);
  });

  it("keeps a manual environment refresh when the initial inspection arrives late", async () => {
    const initial = deferred<AgentEnvironment>();
    const bridge = fixture({
      agentEnvironment: vi.fn().mockReturnValueOnce(initial.promise).mockResolvedValue(environment),
      listAgentMissions: vi.fn().mockResolvedValue([]),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    expect(await screen.findByText("herdr-core 1.0 · 已连接")).toBeVisible();
    await act(async () => initial.resolve({ ...environment, running: false, projects: [] }));
    expect(screen.getByText("herdr-core 1.0 · 已连接")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "工作目录" })).toBeEnabled();
  });

  it("keeps the newly created mission selected when initial history arrives late", async () => {
    const history = deferred<AgentMission[]>();
    const bridge = fixture({ listAgentMissions: vi.fn().mockReturnValue(history.promise) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    await screen.findByText("herdr-core 1.0 · 已连接");
    fireEvent.change(screen.getByLabelText("工作目录"), { target: { value: "repo-1" } });
    fireEvent.change(screen.getByLabelText("协作目标"), { target: { value: mission.objective } });
    fireEvent.click(screen.getByRole("button", { name: "创建协作" }));
    expect(await screen.findByRole("heading", { name: mission.objective })).toBeVisible();
    await act(async () => history.resolve([]));
    expect(screen.getByRole("heading", { name: mission.objective })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "历史协作" })).toHaveValue(mission.id);
  });

  it("rejects a seventy-character Chinese title by UTF-8 size and permits a corrected title", async () => {
    const bridge = fixture();
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    await prepareTask();
    fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "中".repeat(70) } });
    const submit = screen.getByRole("button", { name: "启动 Agent 任务" });
    expect(screen.getByText("任务标题超过 200 字节上限（当前 210 字节），请缩短后再提交。")).toBeVisible();
    expect(submit).toBeDisabled();
    fireEvent.submit(submit.closest("form")!);
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "实现任务列表" } });
    await previewTask();
    expect(submit).toBeEnabled();
    fireEvent.click(submit);
    expect(await screen.findByText("任务已提交。执行进展会自动更新。")).toBeVisible();
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/操作结果尚未确认/)).not.toBeInTheDocument();
  });

  it("validates objective, specification and reply byte limits before dispatching commands", async () => {
    const bridge = fixture({ getAgentSnapshot: vi.fn().mockResolvedValue({
      ...snapshot,
      messages: [{ id: "question-1", type: "question", body: "是否继续？", taskId: "task-1", dispatchId: "dispatch-1", requiresReply: true }],
    }) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.change(screen.getByLabelText("工作目录"), { target: { value: "repo-1" } });
    fireEvent.change(screen.getByLabelText("协作目标"), { target: { value: "目".repeat(10_667) } });
    expect(screen.getByText(/协作目标超过 32000 字节上限/)).toBeVisible();
    const create = screen.getByRole("button", { name: "创建协作" });
    expect(create).toBeDisabled();
    fireEvent.submit(create.closest("form")!);
    expect(bridge.createAgentMission).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("协作目标"), { target: { value: "目标" } });
    expect(create).toBeEnabled();

    await prepareTask();
    fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "述".repeat(21_334) } });
    expect(screen.getByText(/任务说明超过 64000 字节上限/)).toBeVisible();
    const start = screen.getByRole("button", { name: "启动 Agent 任务" });
    expect(start).toBeDisabled();
    fireEvent.submit(start.closest("form")!);
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "执行后运行测试" } });
    await previewTask();
    expect(start).toBeEnabled();

    fireEvent.change(screen.getByLabelText("回复问题 question-1"), { target: { value: "复".repeat(10_667) } });
    expect(screen.getByText(/回复内容超过 32000 字节上限/)).toBeVisible();
    const reply = screen.getByRole("button", { name: "发送回复" });
    expect(reply).toBeDisabled();
    fireEvent.submit(reply.closest("form")!);
    expect(bridge.replyToAgent).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("回复问题 question-1"), { target: { value: "继续" } });
    expect(reply).toBeEnabled();
  });

  it("translates known attention categories and retains unfamiliar details for inspection", async () => {
    const bridge = fixture({ getAgentSnapshot: vi.fn().mockResolvedValue({
      ...snapshot,
      tasks: [{ id: "task-1", title: "需要查看的任务", spec: "检查结果", status: "dispatched", dispatchId: null, terminalState: null, liveness: "live", attention: "root_completion, custom_attention", canRelease: false }],
    }) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    expect(screen.getByText("已派发")).toBeVisible();
    expect(screen.getByText("Agent 报告完成，请审查成果。 请在原生终端中检查：custom_attention")).toBeVisible();
    expect(screen.queryByText(/root_completion/)).not.toBeInTheDocument();
  });

  it("creates a persisted mission and starts only an explicitly submitted self-contained task", async () => {
    const user = userEvent.setup();
    const bridge = fixture({ listAgentMissions: vi.fn().mockResolvedValue([]) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await waitFor(() => expect(screen.getByLabelText("工作目录")).toBeEnabled());
    expect(bridge.createAgentMission).not.toHaveBeenCalled();
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText("工作目录"), "repo-1");
    await user.type(screen.getByLabelText("协作目标"), mission.objective);
    await user.click(screen.getByRole("button", { name: "创建协作" }));
    await ready();
    expect(bridge.createAgentMission).toHaveBeenCalledWith({ id: expect.any(String), workspaceId: "workspace-1", repositoryId: "repo-1", objective: mission.objective });
    expect(screen.getByText(/不会自动创建 worktree/)).toBeVisible();
    await prepareTask();
    await user.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    expect(bridge.startAgentTask).toHaveBeenCalledWith({ requestedTemplate: "reviewer", operationId: expect.any(String), missionId: mission.id, title: "实现任务列表", spec: "仅修改任务列表，保持现有接口，运行组件测试验证。", handoff: {
      context: { includeActiveContext: false, evidenceIds: [] }, previewHash: preview.hash,
      expectedCursorVersion: 0, expectedDraftVersion: 0, expectedBranchId: null, expectedBranchVersion: null,
    } });
    expect(await screen.findByText("任务已提交。执行进展会自动更新。")).toBeVisible();
    expect(screen.getByLabelText("任务标题")).toHaveValue("");
  });

  it("opens Orca explicitly and prevents work without a ThoughtsFlow workspace", async () => {
    const bridge = fixture({ agentEnvironment: vi.fn().mockResolvedValue({ ...environment, running: false }) });
    render(<AgentWorkspace bridge={bridge} />);
    expect(await screen.findByRole("button", { name: "连接 herdr-core" })).toBeEnabled();
    expect(bridge.openAgentRuntime).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "连接 herdr-core" }));
    await waitFor(() => expect(bridge.openAgentRuntime).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "创建协作" })).toBeDisabled();
    expect(bridge.listAgentMissions).not.toHaveBeenCalled();
  });

  it.each(["reviewer", "writer"])("does not repeat an in-flight task submission or claim an unknown result succeeded (model %s)", async (requestedTemplate) => {
    const pending = deferred<AgentOperation>();
    const bridge = fixture({ startAgentTask: vi.fn().mockReturnValue(pending.promise),
      previewAgentHandoff: vi.fn().mockResolvedValue({ ...preview, requestedTemplate }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    if (requestedTemplate !== undefined) {
      fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: requestedTemplate } });
    }
    await prepareTask();
    const form = screen.getByRole("button", { name: "启动 Agent 任务" }).closest("form")!;
    fireEvent.submit(form);
    fireEvent.submit(form);
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(1);
    if (requestedTemplate !== undefined) expect(bridge.startAgentTask).toHaveBeenCalledWith(expect.objectContaining({ requestedTemplate }));
    await act(async () => pending.resolve({ ...operation, status: "unknown", error: "Orca timed out" }));
    expect(screen.getByText(/操作结果尚未确认，请勿重复提交/)).toBeVisible();
    expect(screen.queryByText("任务已提交。执行进展会自动更新。")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    await waitFor(() => expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(3));
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    expect(screen.getByLabelText("任务标题")).toHaveValue("实现任务列表");
    if (requestedTemplate !== undefined) expect(screen.getByLabelText("Agent 模板")).toHaveValue(requestedTemplate);
  });

  it("keeps verified replies and ended-worker release available with an unrelated unknown start", async () => {
    const user = userEvent.setup();
    const bridge = fixture({
      listAgentOperations: vi.fn().mockResolvedValue([{ ...operation, status: "unknown" }]),
      getAgentSnapshot: vi.fn().mockResolvedValue({
        ...snapshot,
        tasks: [{ id: "task-1", title: "组件测试", spec: "验证交互", status: "completed", dispatchId: "dispatch-1", terminalState: "exited", liveness: "dead", attention: null, canRelease: true }],
        messages: [{ id: "question-1", type: "question", body: "是否需要兼容旧接口？", taskId: "task-1", dispatchId: "dispatch-1", requiresReply: true }],
      }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    await prepareTask();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    await user.type(screen.getByLabelText("回复问题 question-1"), "保留兼容性。");
    await user.click(screen.getByRole("button", { name: "发送回复" }));
    expect(bridge.replyToAgent).toHaveBeenCalledWith({ operationId: expect.any(String), missionId: mission.id, messageId: "question-1", body: "保留兼容性。" });
    await waitFor(() => expect(screen.getByRole("button", { name: "发送回复" })).toBeDisabled());
    await user.click(screen.getByRole("button", { name: "释放已结束的 Agent" }));
    expect(bridge.releaseAgentWorker).toHaveBeenCalledWith({ operationId: expect.any(String), missionId: mission.id, dispatchId: "dispatch-1" });
    const task = screen.getByRole("article", { name: "任务 组件测试" });
    await user.click(within(task).getByText("Agent 输出"));
    await user.click(within(task).getByRole("button", { name: "读取输出" }));
    expect(await within(task).findByText("Tests passed")).toBeVisible();
  });

  it("ignores a stale snapshot after selecting another mission", async () => {
    const user = userEvent.setup();
    const oldSnapshot = deferred<AgentSnapshot>();
    const otherMission = { ...mission, id: "mission-2", objective: "新的协作目标" };
    const bridge = fixture({
      listAgentMissions: vi.fn().mockResolvedValue([mission, otherMission]),
      getAgentSnapshot: vi.fn().mockImplementation((id: string) => id === mission.id ? oldSnapshot.promise : Promise.resolve({ ...snapshot, mission: otherMission })),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await waitFor(() => expect(bridge.getAgentSnapshot).toHaveBeenCalledWith(mission.id));
    await user.selectOptions(screen.getByLabelText("历史协作"), otherMission.id);
    await ready();
    await act(async () => oldSnapshot.resolve({ ...snapshot, warning: "旧协作过期警告" }));
    expect(screen.getByRole("heading", { name: otherMission.objective })).toBeVisible();
    expect(screen.queryByText("旧协作过期警告")).not.toBeInTheDocument();
    await prepareTask();
    await user.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    expect(bridge.startAgentTask).toHaveBeenCalledWith(expect.objectContaining({ missionId: otherMission.id }));
  });

  it("preserves the latest snapshot on read failure and only reconnects on explicit request", async () => {
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce({ ...snapshot, tasks: [{ id: "task-1", title: "已知任务", spec: "继续处理", status: "running", dispatchId: null, terminalState: null, liveness: "alive", attention: null, canRelease: false }] })
      .mockRejectedValueOnce(new Error("Runtime unavailable"))
      .mockResolvedValue({ ...snapshot, connected: false, warning: "运行时绑定已失效" }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    expect(await screen.findByText("Runtime unavailable")).toBeVisible();
    expect(screen.getByText("连接已断开 · 保留最近一次进展")).toBeVisible();
    expect(screen.getByRole("article", { name: "任务 已知任务" })).toBeVisible();
    await prepareTask();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    expect(await screen.findByText("运行时绑定已失效")).toBeVisible();
    expect(bridge.reconnectAgentMission).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "重新连接协作" }));
    expect(bridge.reconnectAgentMission).toHaveBeenCalledWith({ operationId: expect.any(String), missionId: mission.id });
  });

  it("polls read-only state every five seconds and stops after unmount", async () => {
    vi.useFakeTimers();
    const bridge = fixture();
    const rendered = render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await act(async () => { await Promise.resolve(); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
    rendered.unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
  });

  it("coalesces a refresh after mutation and rejects the older in-flight snapshot", async () => {
    const olderRead = deferred<AgentSnapshot>();
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce(snapshot)
      .mockReturnValueOnce(olderRead.promise)
      .mockResolvedValue({ ...snapshot, tasks: [{ id: "task-new", title: "刚提交的任务", spec: "检查结果", status: "running", dispatchId: null, terminalState: null, liveness: "alive", attention: null, canRelease: false }] }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    await prepareTask();
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    await screen.findByText("任务已提交。执行进展会自动更新。");
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    await act(async () => olderRead.resolve({ ...snapshot, warning: "操作前的旧进展" }));
    expect(await screen.findByRole("article", { name: "任务 刚提交的任务" })).toBeVisible();
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(3);
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("操作前的旧进展")).not.toBeInTheDocument();
  });

  it("skips polling during a mutation and drains one queued read after its result", async () => {
    vi.useFakeTimers();
    const pending = deferred<AgentOperation>();
    const bridge = fixture({ startAgentTask: vi.fn().mockReturnValue(pending.promise) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await act(async () => { await Promise.resolve(); });
    await prepareTask();
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve(operation));
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(1);
  });

  it("pauses hidden-window polls and requires fresh state before commands after returning", async () => {
    vi.useFakeTimers();
    let visibility: DocumentVisibilityState = "visible";
    vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
    const resumedRead = deferred<AgentSnapshot>();
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce(snapshot)
      .mockReturnValueOnce(resumedRead.promise),
    });
    const rendered = render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await act(async () => { await Promise.resolve(); });
    await prepareTask();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeEnabled();
    visibility = "hidden";
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(1);
    expect(bridge.listAgentOperations).toHaveBeenCalledTimes(1);
    visibility = "visible";
    fireEvent(document, new Event("visibilitychange"));
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    await act(async () => resumedRead.resolve(snapshot));
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeEnabled();
    rendered.unmount();
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
  });

  it("starts a fresh read after StrictMode replays the mount effect", async () => {
    const discardedRead = deferred<AgentSnapshot>();
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockReturnValueOnce(discardedRead.promise)
      .mockResolvedValue(snapshot),
    });
    render(<StrictMode><AgentWorkspace bridge={bridge} workspaceId="workspace-1" /></StrictMode>);
    await waitFor(() => expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(1));
    await act(async () => discardedRead.resolve({ ...snapshot, warning: "已清理 effect 的旧响应" }));
    await ready();
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("已清理 effect 的旧响应")).not.toBeInTheDocument();
  });

  it("does not unlock commands with a read started before the window became hidden", async () => {
    let visibility: DocumentVisibilityState = "visible";
    vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
    const oldRead = deferred<AgentSnapshot>();
    const resumedRead = deferred<AgentSnapshot>();
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce({ ...snapshot, messages: [{ id: "message-1", type: "message", body: "上次确认的进展", taskId: null, dispatchId: null, requiresReply: false }] })
      .mockReturnValueOnce(oldRead.promise)
      .mockReturnValueOnce(resumedRead.promise),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    await prepareTask();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    visibility = "hidden";
    fireEvent(document, new Event("visibilitychange"));
    visibility = "visible";
    fireEvent(document, new Event("visibilitychange"));
    await act(async () => oldRead.resolve({ ...snapshot, warning: "后台之前的旧响应" }));
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(3);
    expect(screen.getByText("上次确认的进展")).toBeVisible();
    expect(screen.queryByText("后台之前的旧响应")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    await act(async () => resumedRead.resolve(snapshot));
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeEnabled();
  });

  it.each(["disconnected", "failed"])("waits for the resumed read before reconnecting when it returns %s", async (outcome) => {
    let visibility: DocumentVisibilityState = "visible";
    vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
    const disconnected = { ...snapshot, connected: false };
    const resumedRead = deferred<AgentSnapshot>();
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce(disconnected)
      .mockReturnValueOnce(resumedRead.promise)
      .mockResolvedValue(snapshot),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    const reconnect = await screen.findByRole("button", { name: "重新连接协作" });
    expect(reconnect).toBeEnabled();
    visibility = "hidden";
    fireEvent(document, new Event("visibilitychange"));
    expect(reconnect).toBeDisabled();
    visibility = "visible";
    fireEvent(document, new Event("visibilitychange"));
    expect(bridge.getAgentSnapshot).toHaveBeenCalledTimes(2);
    expect(reconnect).toBeDisabled();
    fireEvent.click(reconnect);
    expect(bridge.reconnectAgentMission).not.toHaveBeenCalled();
    await act(async () => {
      if (outcome === "failed") resumedRead.reject(new Error("Resume read failed"));
      else resumedRead.resolve(disconnected);
    });
    expect(reconnect).toBeEnabled();
    fireEvent.click(reconnect);
    expect(bridge.reconnectAgentMission).toHaveBeenCalledTimes(1);
    await ready();
  });

  it("retains tasks and messages when a disconnected snapshot returns empty collections", async () => {
    const bridge = fixture({ getAgentSnapshot: vi.fn()
      .mockResolvedValueOnce({
        ...snapshot,
        tasks: [{ id: "task-1", title: "断开前的任务", spec: "保留状态", status: "running", dispatchId: null, terminalState: null, liveness: "alive", attention: null, canRelease: false }],
        messages: [{ id: "question-1", type: "question", body: "断开前的问题", taskId: "task-1", dispatchId: null, requiresReply: true }],
      })
      .mockResolvedValue({ ...snapshot, connected: false, warning: "运行时身份已变化", mission: { ...mission, status: "needs-attention" } }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    expect(await screen.findByText("运行时身份已变化")).toBeVisible();
    expect(screen.getByText("连接已断开 · 保留最近一次进展")).toBeVisible();
    expect(screen.getByRole("article", { name: "任务 断开前的任务" })).toBeVisible();
    expect(screen.getByText("断开前的问题")).toBeVisible();
    expect(screen.getByLabelText("回复问题 question-1")).toBeDisabled();
    await prepareTask();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重新连接协作" })).toBeEnabled();
  });

  it("clears the historical mission error when a fresh snapshot confirms recovery", async () => {
    const previousMission: AgentMission = { ...mission, status: "needs-attention", error: "之前的运行时已断开" };
    const bridge = fixture({
      listAgentMissions: vi.fn().mockResolvedValue([previousMission]),
      getAgentSnapshot: vi.fn()
        .mockResolvedValueOnce({ ...snapshot, connected: false, mission: previousMission })
        .mockResolvedValue(snapshot),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    expect(await screen.findByText("之前的运行时已断开")).toBeVisible();
    await screen.findByRole("button", { name: "重新连接协作" });
    fireEvent.click(screen.getByRole("button", { name: "刷新进展" }));
    await ready();
    expect(screen.queryByText("之前的运行时已断开")).not.toBeInTheDocument();
  });
  it("requires preview and invalidates it after every editable task or selected source changes", async () => {
    const bridge = fixture({ listAgentEvidencePage: vi.fn().mockResolvedValue({ items: [evidenceSummary(evidence)], nextCursor: null }) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fillTask();
    const start = screen.getByRole("button", { name: "启动 Agent 任务" });
    fireEvent.submit(start.closest("form")!);
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
    await previewTask();
    expect(start).toBeEnabled();
    fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "新标题" } });
    expect(start).toBeDisabled();
    await previewTask();
    fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "新说明" } });
    expect(start).toBeDisabled();
    await previewTask();
    fireEvent.click(screen.getByLabelText("包含当前图谱路径"));
    expect(start).toBeDisabled();
    await previewTask();
    fireEvent.click(screen.getByLabelText("使用证据 task-1"));
    expect(start).toBeDisabled();
    await previewTask();
    expect(bridge.previewAgentHandoff).toHaveBeenLastCalledWith({
      missionId: mission.id, requestedTemplate: "reviewer", title: "新标题", spec: "新说明", context: { includeActiveContext: true, evidenceIds: [evidence.id] },
    });
    fireEvent.click(start);
    await screen.findByText("任务已提交。执行进展会自动更新。");
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(1);
  });

  it("discards an asynchronous preview when the draft changes before it returns", async () => {
    const pending = deferred<AgentHandoffPreview>();
    const bridge = fixture({ previewAgentHandoff: vi.fn().mockReturnValue(pending.promise) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fillTask();
    fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
    fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "已修改说明" } });
    await act(async () => pending.resolve(preview));
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
  });

  it.each(["agent_handoff_stale", "agent_handoff_required", "agent_handoff", "agent_model", "agent_model_unsupported"])("requires a fresh preview after pre-dispatch rejection %s without inventing an unknown dispatch", async (code) => {
    const bridge = fixture({ startAgentTask: vi.fn()
      .mockRejectedValueOnce(new DesktopBridgeError({ code, message: "上下文需要重新确认" }))
      .mockResolvedValue(operation),
      previewAgentHandoff: vi.fn().mockResolvedValue({ ...preview, requestedTemplate: "reviewer" }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: "reviewer" } });
    await prepareTask();
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    expect(await screen.findByText("上下文需要重新确认 请重新预览后再启动。")).toBeVisible();
    expect(screen.queryByText(/操作结果尚未确认/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("reviewer");
    await previewTask();
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    await screen.findByText("任务已提交。执行进展会自动更新。");
    expect(bridge.startAgentTask).toHaveBeenCalledTimes(2);
  });

  it("changes the graph cursor with optimistic versions and invalidates the handoff", async () => {
    const activeTree: ContextTreeProjection = { ...tree, nodes: [{ runId: "context-1", turnId: "turn-1", parentRunId: null, prompt: "上下文", title: "上下文", outputPreview: "内容", model: "legacy", status: "completed", createdAt: "2026-10-06", canContinue: true, isActive: true, isOnActivePath: true, branchIds: [], checkpointIds: [] }], cursor: { ...tree.cursor, activeRunId: "context-1", version: 7 }, draftVersion: 3 };
    const bridge = fixture({ getContextTree: vi.fn().mockResolvedValue(activeTree) });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready(); await prepareTask();
    fireEvent.click(screen.getByLabelText("包含当前图谱路径"));
    await previewTask();
    fireEvent.click(screen.getByRole("treeitem", { name: /工作区起点/ }));
    await waitFor(() => expect(bridge.setActiveContext).toHaveBeenCalledWith({
      workspaceId: mission.workspaceId, runId: null, branchId: null, expectedCursorVersion: 7, expectedDraftVersion: 3,
    }));
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    expect(bridge.getContextTree).toHaveBeenCalledTimes(2);
  });

  it("saves a server-read page with one stable capture id, then reuses its evidence", async () => {
    const capture = deferred<AgentEvidence>();
    const task = { id: "task-1", title: "收集事实", spec: "检查", status: "running", dispatchId: "dispatch-1", terminalState: null, liveness: "alive", attention: null, canRelease: false };
    const bridge = fixture({
      getAgentSnapshot: vi.fn().mockResolvedValue({ ...snapshot, tasks: [task] }),
      readAgentOutput: vi.fn().mockResolvedValueOnce({ text: "page one", cursor: "page-2", hasMore: true, source: "terminal" })
        .mockResolvedValueOnce({ text: "page two", cursor: null, hasMore: false, source: "terminal" }),
      captureAgentEvidence: vi.fn().mockRejectedValueOnce(new Error("response lost")).mockReturnValue(capture.promise),
      listAgentEvidencePage: vi.fn().mockResolvedValueOnce({ items: [], nextCursor: null }).mockResolvedValue({ items: [evidenceSummary(evidence)], nextCursor: null }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready();
    fireEvent.click(screen.getByText("Agent 输出"));
    fireEvent.click(screen.getByRole("button", { name: "读取输出" }));
    await screen.findByText("page one");
    fireEvent.click(screen.getByRole("button", { name: "读取后续输出" }));
    await screen.findByText("page onepage two");
    const save = screen.getByRole("button", { name: "保存当前页观察（重新读取）" });
    fireEvent.click(save);
    await screen.findByText(/response lost/);
    fireEvent.click(save); fireEvent.click(save);
    expect(bridge.captureAgentEvidence).toHaveBeenCalledTimes(2);
    const first = vi.mocked(bridge.captureAgentEvidence).mock.calls[0][0];
    expect(first).toEqual({ id: expect.any(String), missionId: mission.id, dispatchId: "dispatch-1", cursor: "page-2" });
    expect(vi.mocked(bridge.captureAgentEvidence).mock.calls[1][0]).toEqual(first);
    await act(async () => capture.resolve(evidence));
    await screen.findByText(/输出观察已保存/);
    await screen.findByLabelText("使用证据 task-1");
    fillTask(); fireEvent.click(screen.getByLabelText("使用证据 task-1")); await previewTask();
    expect(bridge.previewAgentHandoff).toHaveBeenLastCalledWith(expect.objectContaining({ context: { includeActiveContext: false, evidenceIds: [evidence.id] } }));
  });

  it("reads immutable handoffs and page evidence while the runtime is offline", async () => {
    const bridge = fixture({
      agentEnvironment: vi.fn().mockResolvedValue({ ...environment, running: false }),
      getAgentSnapshot: vi.fn().mockResolvedValue({ ...snapshot, connected: false }),
      listAgentEvidencePage: vi.fn().mockResolvedValue({ items: [evidenceSummary(evidence)], nextCursor: null }),
      listAgentHandoffPage: vi.fn().mockResolvedValue({ items: [handoffSummary({ operationId: "handoff-1", workspaceId: mission.workspaceId, missionId: mission.id, createdAt: "2026-10-06", preview })], nextCursor: null }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    const history = await screen.findByRole("region", { name: "历史任务交接" });
    fireEvent.click(await within(history).findByText(`handoff-1 · ${agentHistoryTime("2026-10-06")}`));
    expect(await within(history).findByText(preview.text)).toBeVisible();
    const saved = screen.getByRole("region", { name: "已保存的 Agent 证据" });
    fireEvent.click(within(saved).getByText(`task-1 · ${agentHistoryTime(evidence.createdAt)}`));
    expect(await within(saved).findByText(evidence.text)).toBeVisible();
    expect(within(saved).getByText(evidence.contentHash)).toBeVisible();
    expect(within(saved).getByText("partial page")).toBeVisible();
    expect(screen.getByRole("region", { name: "人工编排来源图" })).toBeVisible();
    expect(bridge.openAgentRuntime).not.toHaveBeenCalled();
    expect(bridge.startAgentTask).not.toHaveBeenCalled();
  });

  it("does not show a previous workspace's saved evidence or preview after selection changes", async () => {
    const oldEvidence = deferred<AgentEvidencePage>();
    const oldPreview = deferred<AgentHandoffPreview>();
    const otherMission = { ...mission, id: "mission-2", workspaceId: "workspace-2", objective: "另一个工作区" };
    const bridge = fixture({
      listAgentMissions: vi.fn().mockImplementation((id: string) => Promise.resolve([id === "workspace-1" ? mission : otherMission])),
      listAgentEvidencePage: vi.fn().mockImplementation(({ workspaceId }: { workspaceId: string }) => workspaceId === "workspace-1" ? oldEvidence.promise : Promise.resolve({ items: [], nextCursor: null })),
      previewAgentHandoff: vi.fn().mockReturnValue(oldPreview.promise),
    });
    const rendered = render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    await ready(); fillTask();
    fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
    rendered.rerender(<AgentWorkspace bridge={bridge} workspaceId="workspace-2" />);
    await screen.findByRole("heading", { name: otherMission.objective });
    await act(async () => { oldEvidence.resolve({ items: [evidenceSummary(evidence)], nextCursor: null }); oldPreview.resolve(preview); });
    expect(screen.queryByLabelText("使用证据 task-1")).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("任务标题")).toHaveValue("");
  });

  it("shows task and context titles in the graph and saved evidence without hiding audit identities", async () => {
    const titledPreview = { ...preview, title: "审查接口兼容性", sourceRunIds: ["context-source"] };
    const sourceTree: ContextTreeProjection = { ...tree, nodes: [{
      runId: "context-source", turnId: "turn-source", parentRunId: null, prompt: "原始问题", title: "已确认的接口约束",
      outputPreview: "保留兼容字段", model: "legacy", status: "completed", createdAt: "2026-10-06",
      canContinue: true, isActive: false, isOnActivePath: false, branchIds: [], checkpointIds: [],
    }] };
    const bridge = fixture({
      getContextTree: vi.fn().mockResolvedValue(sourceTree),
      listAgentHandoffPage: vi.fn().mockResolvedValue({ items: [handoffSummary({ operationId: "handoff-readable", workspaceId: mission.workspaceId, missionId: mission.id, createdAt: "2026-10-06", preview: titledPreview })], nextCursor: null }),
      listAgentEvidencePage: vi.fn().mockResolvedValue({ items: [evidenceSummary({ ...evidence, handoffOperationId: "handoff-readable" }, titledPreview.title)], nextCursor: null }),
    });
    render(<AgentWorkspace bridge={bridge} workspaceId="workspace-1" />);
    const checkbox = await screen.findByRole("checkbox", { name: "使用证据 审查接口兼容性" });
    expect(checkbox).toHaveAttribute("data-evidence-id", evidence.id);
    const graph = screen.getByRole("region", { name: "人工编排来源图" });
    expect(within(graph).getByText("审查接口兼容性")).toBeInTheDocument();
    expect(within(graph).getByText("上下文回答 · 已确认的接口约束")).toBeInTheDocument();
    const saved = screen.getByRole("region", { name: "已保存的 Agent 证据" });
    fireEvent.click(within(saved).getByText(`审查接口兼容性 · ${agentHistoryTime(evidence.createdAt)}`));
    expect(await within(saved).findByText(/证据 evidence-1 · 任务 task-1/)).toBeVisible();
    const history = screen.getByRole("region", { name: "历史任务交接" });
    fireEvent.click(within(history).getByText(`审查接口兼容性 · ${agentHistoryTime("2026-10-06")}`));
    expect(await within(history).findByText(/操作 handoff-readable/)).toBeVisible();
  });

});
