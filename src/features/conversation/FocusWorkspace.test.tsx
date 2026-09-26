import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type {
  ContextPreview,
  ContextTreeProjection,
  RunHandle,
  RunEvent,
  RunSnapshot,
  WorkspaceDetail,
} from "../../shared/contracts";
import {
  FocusWorkspace,
  nextCheckpointOperationId,
} from "./FocusWorkspace";

const workspace = {
  id: "workspace-1",
  name: "AI 分支对话产品定义",
  goal: "确定首版默认导航、上下文透明度与长期使用价值",
  systemPrompt: "你是一名严谨的技术决策协作者。",
  archived: false,
  createdAt: "2026-07-22T09:00:00Z",
  updatedAt: "2026-07-22T10:00:00Z",
};

const detail: WorkspaceDetail = {
  workspace,
  turns: [
    {
      id: "turn-1",
      workspaceId: workspace.id,
      parentRunId: null,
      prompt: "默认界面应该是线性阅读，还是无限画布？",
      createdAt: "2026-07-22T09:10:00Z",
      runs: [
        {
          id: "run-a",
          turnId: "turn-1",
          status: "completed",
          output: "线性阅读降低首分钟认知成本。",
          providerProfileId: "provider-cloud",
          providerName: "OpenAI compatible",
          model: "gpt-4.1",
          baseUrl: "https://api.example.com/v1",
          createdAt: "2026-07-22T09:11:00Z",
          completedAt: "2026-07-22T09:11:12Z",
        },
        {
          id: "run-b",
          turnId: "turn-1",
          status: "completed",
          output: "以专注阅读为主，路线图按需打开。",
          providerProfileId: "provider-local",
          providerName: "Ollama",
          model: "qwen3:14b",
          baseUrl: "http://127.0.0.1:11434",
          createdAt: "2026-07-22T09:12:00Z",
          completedAt: "2026-07-22T09:12:08Z",
        },
      ],
    },
  ],
  selectedRunIds: { "turn-1": "run-a" },
  adjacentBranches: [],
  decisionMarks: [],
  contextCursor: {
    workspaceId: workspace.id,
    activeRunId: "run-a",
    branchId: "branch-a",
    version: 4,
    updatedAt: "2026-07-22T10:00:00Z",
  },
};

const preview: ContextPreview = {
  hash: "sha256:preview",
  estimatedTokens: 842,
  limitTokens: 8192,
  blocked: false,
  warnings: [],
  providerProfileId: "provider-cloud",
  providerName: "OpenAI compatible",
  model: "gpt-4.1",
  baseUrl: "https://api.example.com/v1",
  items: [
    {
      id: "context-system",
      sourceRef: { kind: "workspace-system", id: workspace.id },
      contentBlockId: "block-system",
      contentHash: "sha256:system",
      ordinal: 1,
      role: "system" as const,
      label: "系统说明",
      source: "工作区默认",
      content: "你是一名严谨的 AI 产品设计顾问。",
      reason: "工作区系统说明",
      estimatedTokens: 12,
      included: true,
      pinned: false,
      mandatory: true,
    },
    {
      id: "context-run-a",
      sourceRef: { kind: "model-run", id: "run-a" },
      contentBlockId: "block-run-a",
      contentHash: "sha256:run-a",
      ordinal: 2,
      role: "assistant" as const,
      label: "回答 A",
      source: "run-a",
      content: "线性阅读降低首分钟认知成本。",
      reason: "精确祖先路径",
      estimatedTokens: 18,
      included: true,
      pinned: false,
      mandatory: false,
    },
  ],
  rawItems: [],
  draftVersion: 0,
  appliedCheckpoint: null,
};
preview.rawItems = preview.items;

const contextTree: ContextTreeProjection = {
  workspaceId: workspace.id,
  rootId: `workspace-root:${workspace.id}`,
  draftVersion: 0,
  cursor: {
    workspaceId: workspace.id,
    activeRunId: "run-a",
    branchId: "branch-a",
    version: 4,
    updatedAt: "2026-07-22T10:00:00Z",
  },
  nodes: detail.turns[0].runs.map((run) => ({
    runId: run.id,
    turnId: run.turnId,
    parentRunId: null,
    prompt: detail.turns[0].prompt,
    title: "默认界面",
    outputPreview: run.output,
    model: run.model,
    status: run.status,
    createdAt: run.createdAt,
    canContinue: run.status === "completed",
    isActive: run.id === "run-a",
    isOnActivePath: run.id === "run-a",
    branchIds: [run.id === "run-a" ? "branch-a" : "branch-b"],
    checkpointIds: [],
  })),
  edges: detail.turns[0].runs.map((run) => ({
    id: `edge-${run.id}`,
    sourceRunId: null,
    targetRunId: run.id,
    isOnActivePath: run.id === "run-a",
  })),
  branches: [
    {
      id: "branch-a",
      name: "主路线",
      headRunId: "run-a",
      version: 2,
      isActive: true,
    },
    {
      id: "branch-b",
      name: "备选路线",
      headRunId: "run-b",
      version: 1,
      isActive: false,
    },
  ],
  checkpoints: [],
};

function contextTreeAt(runId: "run-a" | "run-b"): ContextTreeProjection {
  const branchId = runId === "run-a" ? "branch-a" : "branch-b";
  return {
    ...contextTree,
    draftVersion: contextTree.draftVersion + (runId === "run-b" ? 1 : 0),
    cursor: {
      ...contextTree.cursor,
      activeRunId: runId,
      branchId,
      version: contextTree.cursor.version + (runId === "run-b" ? 1 : 0),
    },
    nodes: contextTree.nodes.map((node) => ({
      ...node,
      isActive: node.runId === runId,
      isOnActivePath: node.runId === runId,
    })),
    edges: contextTree.edges.map((edge) => ({
      ...edge,
      isOnActivePath: edge.targetRunId === runId,
    })),
    branches: contextTree.branches.map((branch) => ({
      ...branch,
      isActive: branch.id === branchId,
    })),
  };
}

function maintenanceTreeFixture(): ContextTreeProjection {
  return {
    ...contextTree,
    nodes: [
      {
        ...contextTree.nodes[0],
        runId: "run-parent",
        turnId: "turn-parent",
        parentRunId: null,
        title: "事实基线",
        prompt: "先确认事实",
        isActive: false,
        isOnActivePath: true,
        branchIds: ["branch-a"],
      },
      {
        ...contextTree.nodes[0],
        parentRunId: "run-parent",
        isActive: true,
        isOnActivePath: true,
      },
      contextTree.nodes[1],
    ],
    edges: [
      {
        id: "edge-run-parent",
        sourceRunId: null,
        targetRunId: "run-parent",
        isOnActivePath: true,
      },
      {
        id: "edge-run-a",
        sourceRunId: "run-parent",
        targetRunId: "run-a",
        isOnActivePath: true,
      },
      contextTree.edges[1],
    ],
  };
}

function runHandle(runId: string, turnId = "turn-2"): RunHandle {
  return {
    turnId,
    runId,
    cursorVersion: 5,
    draftVersion: 1,
    branchId: "branch-a",
    branchVersion: 3,
  };
}

function bridgeFixture() {
  return {
    listWorkspaces: vi.fn().mockResolvedValue([workspace]),
    createWorkspace: vi.fn(),
    openWorkspace: vi.fn().mockResolvedValue(detail),
    updateWorkspace: vi.fn(),
    inspectContext: vi.fn().mockResolvedValue(preview),
    previewContextTransition: vi.fn().mockResolvedValue(preview),
    getContextTree: vi.fn().mockResolvedValue(contextTree),
    setActiveContext: vi.fn().mockResolvedValue({
      ...contextTree.cursor,
      version: contextTree.cursor.version + 1,
    }),
    renameBranch: vi.fn(),
    updateContextDraft: vi.fn().mockResolvedValue({ draftVersion: 1 }),
    createContextCheckpoint: vi.fn(),
    summarizeAndSetActiveContext: vi.fn(),
    cancelContextMaintenance: vi.fn(),
    createTurnAndStartRun: vi
      .fn()
      .mockResolvedValue(runHandle("run-2")),
    retryRun: vi.fn(),
    cancelRun: vi.fn(),
    getRunSnapshot: vi.fn(),
    updateContextOverrides: vi.fn().mockResolvedValue(undefined),
    getRouteProjection: vi.fn(),
    updateViewState: vi.fn(),
    compareRuns: vi.fn(),
    markDecision: vi.fn(),
    exportDecisionPacket: vi.fn(),
    listProviderTemplates: vi.fn().mockResolvedValue([]),
    listProviderProfiles: vi.fn().mockResolvedValue([
      {
        id: "provider-cloud",
        providerId: "openai-compatible",
        name: "OpenAI compatible",
        dialect: "openai-compatible",
        baseUrl: "https://api.example.com/v1",
        model: "gpt-4.1",
        isDefault: true,
      },
    ]),
    listProviderModels: vi.fn().mockResolvedValue([]),
    listSessionCredentials: vi.fn().mockResolvedValue([]),
    saveProviderProfile: vi.fn(),
    setSessionCredential: vi.fn().mockResolvedValue([]),
    activateSessionCredential: vi.fn().mockResolvedValue([]),
    reorderSessionCredentials: vi.fn().mockResolvedValue([]),
    removeSessionCredential: vi.fn().mockResolvedValue([]),
    testProviderConnection: vi.fn(),
    subscribeToRunEvents: vi.fn().mockReturnValue(() => undefined),
  } as unknown as DesktopBridge;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function addOtherWorkspace(bridge: DesktopBridge) {
  const other = { ...workspace, id: "workspace-other", name: "另一个工作区" };
  const otherTree: ContextTreeProjection = {
    ...contextTree, workspaceId: other.id, rootId: `workspace-root:${other.id}`,
    cursor: { ...contextTree.cursor, workspaceId: other.id, activeRunId: null, branchId: null },
    nodes: [], edges: [], branches: [], checkpoints: [],
  };
  const otherDetail: WorkspaceDetail = {
    ...detail, workspace: other, turns: [], selectedRunIds: {}, contextCursor: otherTree.cursor,
  };
  vi.mocked(bridge.listWorkspaces).mockResolvedValue([workspace, other]);
  vi.mocked(bridge.openWorkspace).mockImplementation(async (id) => id === other.id ? otherDetail : detail);
  vi.mocked(bridge.getContextTree).mockImplementation(async ({ workspaceId }) => workspaceId === other.id ? otherTree : contextTree);
  return { other, otherDetail, otherTree };
}

async function sendPrompt(bridge: DesktopBridge, prompt = "A 的后台请求") {
  await screen.findByRole("heading", { name: workspace.name });
  fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: prompt } });
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await waitFor(() => expect(bridge.createTurnAndStartRun).toHaveBeenCalled());
}

describe("FocusWorkspace", () => {
  describe("responsive Inspector", () => {
    afterEach(() => vi.restoreAllMocks());

    it("starts closed on narrow screens and can be opened and closed explicitly", async () => {
      const media = { ...window.matchMedia("(max-width: 1180px)"), matches: true };
      vi.spyOn(window, "matchMedia").mockReturnValue(media);
      render(<FocusWorkspace bridge={bridgeFixture()} />);
      await screen.findByRole("heading", { name: workspace.name });
      expect(screen.queryByRole("complementary", { name: "Context Inspector" })).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
      expect(screen.getByRole("complementary", { name: "Context Inspector" })).toBeVisible();
      fireEvent.click(screen.getByRole("button", { name: "关闭上下文检查器" }));
      expect(screen.queryByRole("complementary", { name: "Context Inspector" })).not.toBeInTheDocument();
    });

    it("collapses when entering narrow layout without reopening on expansion and removes its listener", async () => {
      const media = {
        ...window.matchMedia("(max-width: 1180px)"),
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      };
      vi.spyOn(window, "matchMedia").mockReturnValue(media);
      const rendered = render(<FocusWorkspace bridge={bridgeFixture()} />);
      expect(await screen.findByRole("complementary", { name: "Context Inspector" })).toBeVisible();
      expect(media.addEventListener).toHaveBeenCalledWith("change", expect.any(Function));
      const listener = media.addEventListener.mock.calls[0][1];
      act(() => listener({ matches: true }));
      expect(screen.queryByRole("complementary", { name: "Context Inspector" })).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
      expect(screen.getByRole("complementary", { name: "Context Inspector" })).toBeVisible();
      fireEvent.click(screen.getByRole("button", { name: "关闭上下文检查器" }));
      act(() => listener({ matches: false }));
      expect(screen.queryByRole("complementary", { name: "Context Inspector" })).not.toBeInTheDocument();
      rendered.unmount();
      expect(media.removeEventListener).toHaveBeenCalledWith("change", listener);
    });
  });

  it("can create the first workspace after StrictMode replays mount effects", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.listWorkspaces).mockResolvedValue([]);
    vi.mocked(bridge.createWorkspace).mockResolvedValue(workspace);
    render(<StrictMode><FocusWorkspace bridge={bridge} /></StrictMode>);
    await screen.findByRole("button", { name: "添加工作区" });
    fireEvent.click(screen.getByRole("button", { name: "添加工作区" }));
    fireEvent.change(screen.getByRole("textbox", { name: "工作区名称" }), { target: { value: workspace.name } });
    fireEvent.change(screen.getByRole("textbox", { name: "工作区目标" }), { target: { value: workspace.goal } });
    fireEvent.click(screen.getByRole("button", { name: "确认创建" }));
    await waitFor(() => expect(bridge.createWorkspace).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
  });

  it("keeps each workspace's unfinished draft across navigation", async () => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    render(<FocusWorkspace bridge={bridge} />);
    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "A 的草稿" } });
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("");
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "B 的草稿" } });
    fireEvent.click(screen.getByRole("button", { name: workspace.name }));
    await screen.findByRole("heading", { name: workspace.name });
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("A 的草稿");
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("B 的草稿");
    expect(bridge.createTurnAndStartRun).not.toHaveBeenCalled();
  });

  it("adopts a reopened Run's broadcast without duplicating direct stream deltas", async () => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    let broadcast!: (event: RunEvent) => void;
    let direct!: (event: RunEvent) => void;
    vi.mocked(bridge.subscribeToRunEvents).mockImplementation((listener) => { broadcast = listener; return () => undefined; });
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation(async (_input, onEvent) => { direct = onEvent; return runHandle("run-live"); });
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    await screen.findByText(/从精确 Run run-live 继续/);
    const delta: RunEvent = { apiVersion: 1, at: workspace.createdAt, type: "text-delta", runId: "run-live", text: "第一段" };
    act(() => { direct(delta); broadcast(delta); });
    expect(screen.getByText("第一段")).toBeVisible();
    expect(screen.queryByText("第一段第一段")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail, turns: [{ ...detail.turns[0], runs: [{ ...detail.turns[0].runs[0], id: "run-live", status: "streaming", output: "第一段" }] }],
      selectedRunIds: { "turn-1": "run-live" },
    });
    vi.mocked(bridge.getContextTree).mockResolvedValue({
      ...contextTree, cursor: { ...contextTree.cursor, activeRunId: "run-live" },
      nodes: [{ ...contextTree.nodes[0], runId: "run-live", status: "streaming", canContinue: false }],
    });
    fireEvent.click(screen.getByRole("button", { name: workspace.name }));
    await screen.findByRole("heading", { name: workspace.name });
    act(() => {
      const next = { ...delta, text: "第二段" };
      direct(next);
      broadcast(next);
    });
    expect(screen.getByText("第一段第二段")).toBeVisible();
    expect(screen.queryByText("第一段第二段第二段")).not.toBeInTheDocument();
    const reads = vi.mocked(bridge.openWorkspace).mock.calls.length;
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail, turns: [{ ...detail.turns[0], runs: [{ ...detail.turns[0].runs[0], id: "run-live", output: "第一段第二段" }] }],
      selectedRunIds: { "turn-1": "run-live" },
    });
    vi.mocked(bridge.getContextTree).mockResolvedValue({
      ...contextTree, cursor: { ...contextTree.cursor, activeRunId: "run-live" },
      nodes: [{ ...contextTree.nodes[0], runId: "run-live", status: "completed", canContinue: true }],
    });
    act(() => {
      const completed: RunEvent = { apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-live" };
      direct(completed);
      broadcast(completed);
    });
    await waitFor(() => expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads + 1));
    expect(screen.getByText("第一段第二段")).toBeVisible();
    expect(screen.queryByRole("button", { name: "停止生成" })).not.toBeInTheDocument();
  });

  it("does not reinitialize when the shell echoes the selected workspace", async () => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    const view = render(<FocusWorkspace bridge={bridge} initialWorkspaceId={workspace.id} />);
    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    view.rerender(<FocusWorkspace bridge={bridge} initialWorkspaceId={other.id} />);
    expect(bridge.listProviderProfiles).toHaveBeenCalledTimes(1);
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(2);
  });

  it("keeps the active send locked when an adopted older Run finishes in the same workspace", async () => {
    const bridge = bridgeFixture();
    const pending = deferred<RunHandle>();
    let broadcast!: (event: RunEvent) => void;
    vi.mocked(bridge.subscribeToRunEvents).mockImplementation((listener) => { broadcast = listener; return () => undefined; });
    vi.mocked(bridge.createTurnAndStartRun).mockReturnValue(pending.promise);
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    const reads = vi.mocked(bridge.openWorkspace).mock.calls.length;
    act(() => broadcast({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-b" }));
    expect(screen.getByRole("button", { name: "发送中" })).toBeDisabled();
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads);
    await act(async () => { pending.resolve(runHandle("run-current")); await pending.promise; });
    await screen.findByText(/从精确 Run run-curren 继续/);
    act(() => broadcast({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-b" }));
    expect(screen.getByRole("button", { name: "发送中" })).toBeDisabled();
    expect(screen.getByText("请求凭证已锁定，正在等待 Provider 返回。")).toBeVisible();
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads);
  });
  it("keeps the latest workspace selection when older reads finish last", async () => {
    const bridge = bridgeFixture();
    const other = { ...workspace, id: "workspace-other", name: "另一个工作区" };
    const oldRead = deferred<WorkspaceDetail>();
    vi.mocked(bridge.listWorkspaces).mockResolvedValue([workspace, other]);
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockReturnValueOnce(oldRead.promise)
      .mockResolvedValueOnce({ ...detail, workspace: other });
    const onWorkspaceChange = vi.fn();
    render(<FocusWorkspace bridge={bridge} onWorkspaceChange={onWorkspaceChange} />);
    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(within(screen.getByRole("navigation", { name: "工作区" })).getByRole("button", { name: workspace.name }));
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    await act(async () => { oldRead.resolve(detail); await oldRead.promise; });
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(onWorkspaceChange).toHaveBeenLastCalledWith(other.id);
  });

  it.each(["success", "failure"] as const)("ignores a previous workspace's late send %s and stream events", async (outcome) => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    const pending = deferred<RunHandle>();
    let emit!: (event: RunEvent) => void;
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation((_input, onEvent) => {
      emit = onEvent;
      return pending.promise;
    });
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    act(() => emit({ apiVersion: 1, at: workspace.createdAt, type: "text-delta", runId: "run-old", text: "A 的早到输出" }));
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "B 的新草稿" } });
    const reads = vi.mocked(bridge.openWorkspace).mock.calls.length;
    await act(async () => {
      if (outcome === "success") pending.resolve(runHandle("run-old"));
      else pending.reject(new Error("A 的延迟错误"));
      await pending.promise.catch(() => undefined);
      emit({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-old" });
    });
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("B 的新草稿");
    expect(screen.getByRole("button", { name: "发送" })).toBeEnabled();
    expect(screen.queryByText("A 的后台请求")).not.toBeInTheDocument();
    expect(screen.queryByText("A 的早到输出")).not.toBeInTheDocument();
    expect(screen.queryByText("A 的延迟错误")).not.toBeInTheDocument();
    expect(screen.getByText("创建根 Turn")).toBeVisible();
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads);
    expect(bridge.cancelRun).not.toHaveBeenCalled();
  });

  it("ignores an old stream completion while the new workspace is sending", async () => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    let oldStream!: (event: RunEvent) => void;
    vi.mocked(bridge.createTurnAndStartRun)
      .mockImplementationOnce(async (_input, onEvent) => { oldStream = onEvent; return runHandle("run-old"); })
      .mockReturnValueOnce(new Promise(() => undefined));
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    await screen.findByText(/从精确 Run run-old 继续/);
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "B 的请求" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(bridge.createTurnAndStartRun).toHaveBeenCalledTimes(2));
    const reads = vi.mocked(bridge.openWorkspace).mock.calls.length;
    act(() => oldStream({ apiVersion: 1, at: workspace.createdAt, type: "run-failed", runId: "run-old", error: { code: "old", message: "A 的流失败" } }));
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(screen.getByRole("button", { name: "发送中" })).toBeDisabled();
    expect(screen.queryByText("A 的流失败")).not.toBeInTheDocument();
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads);
  });

  it("does not let an old terminal refresh replace a newly opened workspace", async () => {
    const bridge = bridgeFixture();
    const { other, otherDetail } = addOtherWorkspace(bridge);
    const oldRefresh = deferred<WorkspaceDetail>();
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockReturnValueOnce(oldRefresh.promise)
      .mockResolvedValueOnce(otherDetail);
    let emit!: (event: RunEvent) => void;
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation(async (_input, onEvent) => {
      emit = onEvent;
      return runHandle("run-old");
    });
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    await screen.findByText(/从精确 Run run-old 继续/);
    act(() => emit({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-old" }));
    await waitFor(() => expect(bridge.openWorkspace).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    await act(async () => { oldRefresh.resolve(detail); await oldRefresh.promise; });
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(screen.getByText("创建根 Turn")).toBeVisible();
  });

  it("preserves a draft edited during send and rejects reads from before the new Run", async () => {
    const bridge = bridgeFixture();
    const oldRead = deferred<WorkspaceDetail>();
    const send = deferred<RunHandle>();
    vi.mocked(bridge.openWorkspace).mockResolvedValueOnce(detail).mockReturnValueOnce(oldRead.promise);
    vi.mocked(bridge.createTurnAndStartRun).mockReturnValue(send.promise);
    render(<FocusWorkspace bridge={bridge} />);
    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(within(screen.getByRole("navigation", { name: "工作区" })).getByRole("button", { name: workspace.name }));
    await sendPrompt(bridge, "发出的草稿");
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "后续仍在编辑" } });
    await act(async () => { send.resolve(runHandle("run-new")); await send.promise; });
    await screen.findByText(/从精确 Run run-new 继续/);
    await act(async () => { oldRead.resolve(detail); await oldRead.promise; });
    expect(screen.getByText(/从精确 Run run-new 继续/)).toBeVisible();
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("后续仍在编辑");
  });

  it("does not let an automatic preview supersede the send's slower context check", async () => {
    const bridge = bridgeFixture();
    const checked = deferred<ContextPreview>();
    vi.mocked(bridge.inspectContext).mockReturnValueOnce(checked.promise).mockResolvedValue(preview);
    render(<FocusWorkspace bridge={bridge} />);
    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "慢速校验仍需发送" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalledTimes(2));
    await act(async () => { checked.resolve(preview); await checked.promise; });
    await waitFor(() => expect(bridge.createTurnAndStartRun).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("无法生成发送前凭证。")).not.toBeInTheDocument();
  });

  it("keeps the newest same-workspace refresh when an earlier refresh arrives last", async () => {
    const bridge = bridgeFixture();
    const oldRefresh = deferred<WorkspaceDetail>();
    const nextTree = contextTreeAt("run-b");
    vi.mocked(bridge.openWorkspace).mockResolvedValueOnce(detail).mockReturnValueOnce(oldRefresh.promise)
      .mockResolvedValue({ ...detail, selectedRunIds: { "turn-1": "run-b" }, contextCursor: nextTree.cursor });
    vi.mocked(bridge.getContextTree).mockResolvedValueOnce(contextTree).mockResolvedValueOnce(contextTree).mockResolvedValue(nextTree);
    let emit!: (event: RunEvent) => void;
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation(async (_input, onEvent) => { emit = onEvent; return runHandle("run-old"); });
    render(<FocusWorkspace bridge={bridge} />);
    await sendPrompt(bridge);
    await screen.findByText(/从精确 Run run-old 继续/);
    act(() => emit({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-old" }));
    await waitFor(() => expect(bridge.openWorkspace).toHaveBeenCalledTimes(2));
    act(() => emit({ apiVersion: 1, at: workspace.createdAt, type: "run-completed", runId: "run-old" }));
    await screen.findByText(/从精确 Run run-b 继续/);
    await act(async () => { oldRefresh.resolve(detail); await oldRefresh.promise; });
    expect(screen.getByText(/从精确 Run run-b 继续/)).toBeVisible();
  });

  it.each(["success", "failure"] as const)("discards a previous workspace's delayed maintenance %s", async (outcome) => {
    const bridge = bridgeFixture();
    const { other, otherTree } = addOtherWorkspace(bridge);
    const checkpoint = deferred<Awaited<ReturnType<DesktopBridge["createContextCheckpoint"]>>>();
    vi.mocked(bridge.createContextCheckpoint).mockReturnValue(checkpoint.promise);
    vi.mocked(bridge.getContextTree).mockImplementation(async ({ workspaceId }) => workspaceId === other.id ? otherTree : maintenanceTreeFixture());
    render(<FocusWorkspace bridge={bridge} />);
    await screen.findByRole("heading", { name: workspace.name });
    const open = screen.getByRole("button", { name: "准备 Context 压缩" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);
    fireEvent.change(screen.getByRole("textbox", { name: "人工摘要" }), { target: { value: "A 的摘要" } });
    fireEvent.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));
    await waitFor(() => expect(bridge.createContextCheckpoint).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), { target: { value: "B 在继续编辑" } });
    const reads = vi.mocked(bridge.openWorkspace).mock.calls.length;
    await act(async () => {
      if (outcome === "success") checkpoint.resolve({
        id: "checkpoint-old", workspaceId: workspace.id, branchId: "branch-a", branchVersion: 2,
        kind: "compaction", anchorRunId: "run-a", sourceRunIds: ["run-parent"], sourceHash: "old",
        firstKeptRunId: "run-a", summary: "A 的摘要", provider: null, status: "completed", createdAt: workspace.createdAt,
      });
      else checkpoint.reject(new Error("A 的维护错误"));
      await checkpoint.promise.catch(() => undefined);
    });
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("B 在继续编辑");
    expect(screen.queryByText("Context 检查点已保存并激活。")).not.toBeInTheDocument();
    expect(screen.queryByText("A 的维护错误")).not.toBeInTheDocument();
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(reads);
  });

  it.each(["success", "failure"] as const)("discards delayed preview and snapshot %s from the previous workspace", async (outcome) => {
    const bridge = bridgeFixture();
    const { other } = addOtherWorkspace(bridge);
    const oldPreview = deferred<ContextPreview>();
    const oldSnapshot = deferred<RunSnapshot>();
    vi.mocked(bridge.inspectContext).mockImplementation((input) => input.workspaceId === workspace.id
      ? oldPreview.promise : Promise.resolve({ ...preview, estimatedTokens: 123 }));
    vi.mocked(bridge.getRunSnapshot).mockReturnValue(oldSnapshot.promise);
    render(<FocusWorkspace bridge={bridge} />);
    await screen.findByRole("heading", { name: workspace.name });
    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));
    await waitFor(() => expect(bridge.getRunSnapshot).toHaveBeenCalled());
    fireEvent.click(screen.getByRole("button", { name: other.name }));
    await screen.findByRole("heading", { name: other.name });
    await act(async () => {
      if (outcome === "success") {
        oldPreview.resolve({ ...preview, estimatedTokens: 999 });
        oldSnapshot.resolve({ id: "old", runId: "run-a", canonicalHash: "old-workspace-hash", createdAt: workspace.createdAt,
          providerName: "Old provider", model: "old", baseUrl: "https://old.example.com", additionalHeaders: {}, parameters: {}, items: preview.items });
      } else {
        oldPreview.reject(new Error("A 的预览错误"));
        oldSnapshot.reject(new Error("A 的快照错误"));
      }
      await Promise.allSettled([oldPreview.promise, oldSnapshot.promise]);
    });
    expect(screen.getByRole("heading", { name: other.name })).toBeVisible();
    expect(screen.queryByText("old-workspace-hash")).not.toBeInTheDocument();
    expect(screen.queryByText(/999 tokens/)).not.toBeInTheDocument();
    expect(screen.queryByText("A 的预览错误")).not.toBeInTheDocument();
    expect(screen.queryByText("A 的快照错误")).not.toBeInTheDocument();
  });

  it("keeps random UUID generation unless a valid WebView E2E operation ID is supplied", () => {
    const randomId = "11111111-1111-4111-8111-111111111111";
    const forcedId = "22222222-2222-4222-8222-222222222222";
    const randomUuid = vi.fn(() => randomId);

    expect(nextCheckpointOperationId(undefined, randomUuid)).toBe(randomId);
    expect(nextCheckpointOperationId("not-a-uuid", randomUuid)).toBe(randomId);
    expect(nextCheckpointOperationId(forcedId, randomUuid)).toBe(forcedId);
    expect(randomUuid).toHaveBeenCalledTimes(2);
  });

  it("recovers a quota failure with the failed Run's exact Provider and creates a new version", async () => {
    const bridge = bridgeFixture();
    const composerProfile = {
      id: "provider-composer",
      providerId: "ollama",
      name: "Current composer",
      dialect: "ollama" as const,
      baseUrl: "http://127.0.0.1:11434",
      model: "qwen3:14b",
      isDefault: true,
      parameters: {},
    };
    const failedProfile = {
      id: "provider-failed",
      providerId: "openai-compatible",
      name: "Failed exact provider",
      dialect: "openai-compatible" as const,
      baseUrl: "https://failed.example.com/v1",
      model: "gpt-exact",
      isDefault: false,
      parameters: {},
    };
    const failedRun = {
      ...detail.turns[0].runs[0],
      status: "failed" as const,
      output: "已保留的失败部分输出",
      providerProfileId: failedProfile.id,
      providerName: failedProfile.name,
      model: failedProfile.model,
      baseUrl: failedProfile.baseUrl,
      error: {
        code: "quota_exhausted",
        message: "当前凭据额度已耗尽。",
        retryable: true,
        status: 429,
      },
    };
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([
      composerProfile,
      failedProfile,
    ]);
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{ ...detail.turns[0], runs: [failedRun] }],
      selectedRunIds: { "turn-1": failedRun.id },
    });
    vi.mocked(bridge.listSessionCredentials).mockResolvedValue([
      { credentialId: "primary", label: "Primary", order: 0, isActive: true },
      { credentialId: "backup", label: "Backup", order: 1, isActive: false },
    ]);
    const previewForProvider = async (input: { providerProfileId: string }) => ({
      ...preview,
      hash: input.providerProfileId === failedProfile.id
        ? "sha256:failed-exact-profile"
        : "sha256:composer-profile",
      providerProfileId: input.providerProfileId,
    });
    vi.mocked(bridge.inspectContext).mockImplementation(previewForProvider);
    vi.mocked(bridge.previewContextTransition).mockImplementation(previewForProvider);
    vi.mocked(bridge.retryRun).mockResolvedValue(
      runHandle("run-recovered", "turn-1"),
    );

    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("已保留的失败部分输出")).toBeVisible();
    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalled());
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue(
      composerProfile.id,
    );
    expect(screen.getByRole("region", { name: "凭据恢复" })).toBeVisible();
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole(
      "combobox",
      { name: "备用凭据" },
      { timeout: 5_000 },
    );
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    await waitFor(() => expect(bridge.retryRun).toHaveBeenCalledTimes(1));
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    expect(bridge.previewContextTransition).toHaveBeenCalledWith(
      expect.objectContaining({
        workspaceId: workspace.id,
        parentRunId: null,
        prompt: detail.turns[0].prompt,
        providerProfileId: failedProfile.id,
      }),
    );
    expect(bridge.retryRun).toHaveBeenCalledWith(
      expect.objectContaining({
        runId: failedRun.id,
        providerProfileId: failedProfile.id,
        previewHash: "sha256:failed-exact-profile",
        credentialId: "backup",
        expectedCursorVersion: 4,
        expectedDraftVersion: 0,
      }),
      expect.any(Function),
    );
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue(
      composerProfile.id,
    );
    expect(await screen.findByRole("button", { name: /回答 B · gpt-exact/ })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /回答 A · gpt-exact/ }));
    expect(await screen.findByText("已保留的失败部分输出")).toBeVisible();
  });

  it("keeps an atomic credential retry rejected when Run creation fails", async () => {
    const bridge = bridgeFixture();
    const failedRun = {
      ...detail.turns[0].runs[0],
      status: "failed" as const,
      output: "已保留的部分输出",
      error: {
        code: "rate_limited",
        message: "当前凭据被限流。",
        retryable: true,
        status: 429,
      },
    };
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{ ...detail.turns[0], runs: [failedRun] }],
      selectedRunIds: { "turn-1": failedRun.id },
    });
    vi.mocked(bridge.listSessionCredentials).mockResolvedValue([
      { credentialId: "primary", label: "Primary", order: 0, isActive: true },
      { credentialId: "backup", label: "Backup", order: 1, isActive: false },
    ]);
    vi.mocked(bridge.retryRun).mockRejectedValue(new DesktopBridgeError({
      code: "context_preview_stale",
      message: "Context 已变化，请重新确认。",
      retryable: false,
    }));

    render(<FocusWorkspace bridge={bridge} />);

    const recovery = await screen.findByRole("region", { name: "凭据恢复" });
    fireEvent.click(within(recovery).getByRole("button", { name: "选择备用凭据" }));
    await within(recovery).findByRole("combobox", { name: "备用凭据" });
    fireEvent.click(within(recovery).getByRole("button", { name: "切换并新增回答版本" }));

    expect(await within(recovery).findByRole("alert")).toHaveTextContent(
      "Context 已变化，请重新确认。",
    );
    expect(bridge.retryRun).toHaveBeenCalledWith(
      expect.objectContaining({ credentialId: "backup" }),
      expect.any(Function),
    );
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /回答 B/ })).not.toBeInTheDocument();
  });

  it.each([
    ["a different provider error", "provider_unreachable", true],
    ["a non-retryable rate limit", "rate_limited", false],
  ])("does not offer credential recovery for %s", async (_label, code, retryable) => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          status: "failed",
          error: { code, message: "不可使用凭据恢复。", retryable },
        }],
      }],
    } as never);

    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("不可使用凭据恢复。")).toBeVisible();
    expect(screen.queryByRole("region", { name: "凭据恢复" })).not.toBeInTheDocument();
    expect(bridge.listSessionCredentials).not.toHaveBeenCalled();
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
  });

  it("labels a remote Ollama endpoint as outbound instead of local", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          providerProfileId: "provider-remote-ollama",
          providerName: "Remote Ollama",
          model: "qwen3",
          baseUrl: "https://ollama.example.com",
        }],
      }],
    });
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([
      {
        id: "provider-remote-ollama",
        providerId: "ollama",
        name: "Remote Ollama",
        dialect: "ollama",
        baseUrl: "https://ollama.example.com",
        model: "qwen3",
        isDefault: true,
        parameters: {},
      },
    ]);
    vi.mocked(bridge.inspectContext).mockResolvedValue({
      ...preview,
      providerProfileId: "provider-remote-ollama",
      providerName: "Remote Ollama",
      model: "qwen3",
      baseUrl: "https://ollama.example.com",
    });

    render(<FocusWorkspace bridge={bridge} />);

    const outboundLabels = await screen.findAllByText("外发 · ollama.example.com");
    expect(outboundLabels.length).toBeGreaterThan(0);
    expect(outboundLabels[0]).toBeVisible();
    expect(screen.queryByText("本机 · ollama.example.com")).not.toBeInTheDocument();
  });

  it("renders a Run's immutable historical endpoint after its Profile is edited", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([{
      id: "provider-cloud",
      providerId: "openai-compatible",
      name: "Renamed current Profile",
      dialect: "openai-compatible",
      baseUrl: "https://new-endpoint.example.com/v1",
      model: "gpt-current",
      isDefault: true,
      parameters: {},
    }]);
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          providerName: "Historical Profile",
          baseUrl: "https://historical.example.com/v1",
        }],
      }],
    });

    render(<FocusWorkspace bridge={bridge} />);

    const output = await screen.findByText("线性阅读降低首分钟认知成本。");
    const answer = output.closest(".focus-turn__answer");
    expect(answer).not.toBeNull();
    expect(within(answer as HTMLElement).getByText("外发 · historical.example.com")).toBeVisible();
    expect(
      within(answer as HTMLElement).queryByText("外发 · new-endpoint.example.com"),
    ).not.toBeInTheDocument();
  });

  it("notifies the shell only after a workspace opens successfully", async () => {
    const bridge = bridgeFixture();
    const onWorkspaceChange = vi.fn();
    const other = { ...workspace, id: "workspace-other", name: "另一个工作区" };
    vi.mocked(bridge.listWorkspaces).mockResolvedValue([workspace, other]);
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockRejectedValueOnce(new Error("打开失败"))
      .mockResolvedValueOnce({ ...detail, workspace: other });
    render(<FocusWorkspace bridge={bridge} onWorkspaceChange={onWorkspaceChange} />);
    await waitFor(() => expect(onWorkspaceChange).toHaveBeenLastCalledWith(workspace.id));
    const otherButton = screen.getByRole("button", { name: other.name });
    fireEvent.click(otherButton);
    await screen.findByText("打开失败");
    expect(onWorkspaceChange).toHaveBeenCalledTimes(1);
    fireEvent.click(otherButton);
    await waitFor(() => expect(onWorkspaceChange).toHaveBeenLastCalledWith(other.id));
  });

  it("requires and persists an explicit workspace goal", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.createWorkspace).mockResolvedValue({
      ...workspace,
      id: "workspace-new",
      name: "新工作区",
      goal: "验证 Context Tree",
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "添加工作区" }));
    fireEvent.change(screen.getByRole("textbox", { name: "工作区名称" }), {
      target: { value: "新工作区" },
    });
    expect(screen.getByRole("button", { name: "确认创建" })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox", { name: "工作区目标" }), {
      target: { value: "验证 Context Tree" },
    });
    fireEvent.click(screen.getByRole("button", { name: "确认创建" }));

    await waitFor(() =>
      expect(bridge.createWorkspace).toHaveBeenCalledWith({
        name: "新工作区",
        goal: "验证 Context Tree",
      }),
    );
  });

  it("buffers early stream events and surfaces an uncommitted persistence failure", async () => {
    const bridge = bridgeFixture();
    const interruptedRun = {
      ...detail.turns[0].runs[0],
      id: "run-early",
      turnId: "turn-early",
      status: "interrupted" as const,
      output: "已先到达的部分输出",
      createdAt: "2026-07-22T10:10:00Z",
      completedAt: "2026-07-22T10:10:01Z",
      error: { code: "storage_failure_uncommitted", message: "磁盘不可写" },
    };
    const authoritativeDetail: WorkspaceDetail = {
      ...detail,
      turns: [
        ...detail.turns,
        {
          id: "turn-early",
          workspaceId: workspace.id,
          parentRunId: "run-a",
          prompt: "验证事件竞态",
          createdAt: "2026-07-22T10:10:00Z",
          runs: [interruptedRun],
        },
      ],
      selectedRunIds: {
        ...detail.selectedRunIds,
        "turn-early": "run-early",
      },
      contextCursor: {
        ...detail.contextCursor!,
        activeRunId: "run-early",
        branchId: "branch-fork",
        version: 5,
      },
    };
    const authoritativeTree: ContextTreeProjection = {
      ...contextTree,
      draftVersion: 1,
      cursor: {
        ...contextTree.cursor,
        activeRunId: "run-early",
        branchId: "branch-fork",
        version: 5,
      },
      nodes: [
        ...contextTree.nodes.map((node) => ({
          ...node,
          isActive: false,
          isOnActivePath: node.runId === "run-a",
          branchIds: node.runId === "run-a"
            ? ["branch-a", "branch-fork"]
            : node.branchIds,
        })),
        {
          runId: "run-early",
          turnId: "turn-early",
          parentRunId: "run-a",
          prompt: "验证事件竞态",
          title: "验证事件竞态",
          outputPreview: interruptedRun.output,
          model: interruptedRun.model,
          status: interruptedRun.status,
          createdAt: interruptedRun.createdAt,
          canContinue: false,
          isActive: true,
          isOnActivePath: true,
          branchIds: ["branch-fork"],
          checkpointIds: [],
        },
      ],
      edges: [
        ...contextTree.edges.map((edge) => ({
          ...edge,
          isOnActivePath: edge.targetRunId === "run-a",
        })),
        {
          id: "edge-run-early",
          sourceRunId: "run-a",
          targetRunId: "run-early",
          isOnActivePath: true,
        },
      ],
      branches: [
        ...contextTree.branches.map((branch) => ({ ...branch, isActive: false })),
        {
          id: "branch-fork",
          name: "故障分支",
          headRunId: "run-early",
          version: 0,
          isActive: true,
        },
      ],
    };
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockResolvedValue(authoritativeDetail);
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(authoritativeTree);
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation(async (_input, onEvent) => {
      onEvent({
        apiVersion: 1,
        type: "text-delta",
        runId: "run-early",
        text: "已先到达的部分输出",
        at: "2026-07-22T10:10:00Z",
      });
      onEvent({
        apiVersion: 1,
        type: "persistence-failed",
        runId: "run-early",
        error: { code: "storage_failure_uncommitted", message: "磁盘不可写" },
        at: "2026-07-22T10:10:01Z",
      });
      return {
        ...runHandle("run-early", "turn-early"),
        branchId: "branch-fork",
        branchVersion: 0,
      };
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "验证事件竞态" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("已先到达的部分输出")).toBeVisible();
    expect(await screen.findByRole("alert")).toHaveTextContent("磁盘不可写");
    await waitFor(() => expect(bridge.openWorkspace).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("请求凭证已锁定，正在等待 Provider 返回。")).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "可以继续编辑" },
    });
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
    expect(screen.getByText(/当前 Run 不可继续/)).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "打开 Context Tree" }));
    fireEvent.click(screen.getByRole("treeitem", { name: /run-a/ }));
    await waitFor(() =>
      expect(bridge.setActiveContext).toHaveBeenCalledWith({
        workspaceId: workspace.id,
        runId: "run-a",
        branchId: "branch-fork",
        expectedCursorVersion: 5,
        expectedDraftVersion: 1,
      }),
    );
  });

  it("offers an explicit retry when sending fails with a retryable bridge error", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.createTurnAndStartRun)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "provider_unreachable",
        message: "Provider 暂时不可达",
        retryable: true,
      }))
      .mockResolvedValueOnce(runHandle("run-2"));
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "重新连接后继续" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("Provider 暂时不可达")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重试发送" }));

    await waitFor(() => expect(bridge.createTurnAndStartRun).toHaveBeenCalledTimes(2));
  });

  it("offers an explicit retry when answer-version retry is temporarily unavailable", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.retryRun)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "provider_timeout",
        message: "Provider 响应超时",
        retryable: true,
      }))
      .mockResolvedValueOnce(runHandle("run-c", "turn-1"));
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "重试回答" }));

    expect(await screen.findByText("Provider 响应超时")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重试回答请求" }));

    await waitFor(() => expect(bridge.retryRun).toHaveBeenCalledTimes(2));
  });

  it("offers an explicit retry when Context inspection fails temporarily", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.inspectContext)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "repository_busy",
        message: "Context 暂时无法读取",
        retryable: true,
      }))
      .mockResolvedValue(preview as never);
    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("Context 暂时无法读取")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重新检查 Context" }));

    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: /2 项 Context/ })).toBeVisible();
  });

  it("opens a route-map departure at the exact selected Run", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(contextTreeAt("run-b"));
    vi.mocked(bridge.setActiveContext).mockResolvedValue(contextTreeAt("run-b").cursor);
    render(<FocusWorkspace bridge={bridge} initialRunId="run-b" />);

    expect(await screen.findByText("以专注阅读为主，路线图按需打开。")).toBeVisible();
    expect(bridge.setActiveContext).toHaveBeenCalledWith({
      workspaceId: workspace.id,
      runId: "run-b",
      branchId: "branch-b",
      expectedCursorVersion: 4,
      expectedDraftVersion: 0,
    });
    expect(screen.getByText(/从精确 Run run-b 继续/)).toBeVisible();
  });

  it("creates a branch from the exact selected answer version", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(contextTreeAt("run-b"));
    vi.mocked(bridge.setActiveContext).mockResolvedValue(contextTreeAt("run-b").cursor);
    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /回答 B.*qwen3:14b/i }));
    await waitFor(() => expect(bridge.setActiveContext).toHaveBeenCalled());
    await screen.findByText(/从精确 Run run-b 继续/);
    fireEvent.click(screen.getByRole("button", { name: "从此回答创建分支" }));

    const branchForm = screen.getByRole("form", { name: "创建精确回答分支" });
    fireEvent.change(within(branchForm).getByRole("textbox"), {
      target: { value: "如果入口改成画布，会带来哪些恢复成本？" },
    });
    fireEvent.submit(branchForm);

    await waitFor(() =>
      expect(bridge.createTurnAndStartRun).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: "workspace-1",
          parentRunId: "run-b",
          prompt: "如果入口改成画布，会带来哪些恢复成本？",
          previewHash: "sha256:preview",
        }),
        expect.any(Function),
      ),
    );
  });

  it("retries a root Turn against its original null parent instead of its own selected Run", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.retryRun).mockResolvedValue({
      ...runHandle("run-c", "turn-1"),
      branchId: "branch-retry",
      branchVersion: 0,
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "重试回答" }));

    await waitFor(() =>
      expect(bridge.inspectContext).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: "workspace-1",
          parentRunId: null,
          prompt: "默认界面应该是线性阅读，还是无限画布？",
        }),
      ),
    );
    expect(bridge.retryRun).toHaveBeenCalledWith(
      {
        runId: "run-a",
        providerProfileId: "provider-cloud",
        previewHash: "sha256:preview",
        branchId: "branch-a",
        expectedCursorVersion: 4,
        expectedBranchVersion: 2,
        expectedDraftVersion: 0,
      },
      expect.any(Function),
    );

    fireEvent.click(screen.getByRole("button", { name: "打开 Context Tree" }));
    expect(screen.getByRole("treeitem", { name: /run-c.*当前 Context/ })).toBeVisible();
    expect(screen.queryByRole("treeitem", { name: /run-a/ })).not.toBeInTheDocument();
    expect(screen.getByText("重试分支")).toBeVisible();
  });

  it("keeps provider destination visible and blocks sending an over-limit context", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.inspectContext).mockResolvedValue({
      ...preview,
      estimatedTokens: 9000,
      blocked: true,
      warnings: ["上下文超过模型限制，请排除内容后重试。"],
    } as never);
    render(<FocusWorkspace bridge={bridge} />);

    expect((await screen.findAllByText(/api\.example\.com/))[0]).toBeVisible();
    expect(screen.getByText("数据保存在本机")).toBeVisible();
    expect((await screen.findAllByText(/上下文超过模型限制/))[0]).toBeVisible();

    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "继续分析" },
    });
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("updates next-send exclusions without mutating the locked snapshot", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getRunSnapshot).mockResolvedValue({
      id: "snapshot-a",
      runId: "run-a",
      canonicalHash: "sha256:locked",
      createdAt: "2026-07-22T09:11:00Z",
      providerId: "openai-compatible",
      templateRevision: 3,
      providerName: "OpenAI compatible",
      streamProtocol: "openai_sse",
      authPlacement: "bearer_header",
      authHeaderName: "Authorization",
      additionalHeaders: { "x-client-revision": "2026-07-22" },
      model: "gpt-4.1",
      baseUrl: "https://api.example.com/v1",
      parameters: { temperature: 0.2, stop: ["DONE"] },
      items: preview.items,
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(await screen.findByRole("button", { name: "排除 回答 A" }));

    await waitFor(() =>
      expect(bridge.updateContextDraft).toHaveBeenCalledWith({
        workspaceId: "workspace-1",
        parentRunId: "run-a",
        expectedDraftVersion: 0,
        items: expect.arrayContaining([
          {
            sourceRef: { kind: "model-run", id: "run-a" },
            contentBlockId: "block-run-a",
            included: false,
            pinned: false,
          },
        ]),
      }),
    );

    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));
    expect(await screen.findByText("sha256:locked")).toBeVisible();
    expect(screen.getByText("openai-compatible r3")).toBeVisible();
    expect(screen.getByText("OpenAI SSE")).toBeVisible();
    expect(screen.getByText("Authorization: Bearer …")).toBeVisible();
    expect(screen.getByText("x-client-revision")).toBeVisible();
    expect(screen.getByText("2026-07-22")).toBeVisible();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.2")).toBeVisible();
    expect(screen.getByText('["DONE"]')).toBeVisible();
    expect(screen.getByRole("button", { name: "排除 回答 A" })).toBeDisabled();
  });

  it("atomically rebases a persisted draft when moving to root and sends with the authoritative version", async () => {
    const bridge = bridgeFixture();
    const persistedDraftPreview = deferred<ContextPreview>();
    const rebasedTree: ContextTreeProjection = {
      ...contextTree,
      draftVersion: 2,
      cursor: {
        ...contextTree.cursor,
        activeRunId: null,
        branchId: null,
        version: 5,
      },
      nodes: contextTree.nodes.map((node) => ({
        ...node,
        isActive: false,
        isOnActivePath: false,
      })),
      edges: contextTree.edges.map((edge) => ({
        ...edge,
        isOnActivePath: false,
      })),
      branches: contextTree.branches.map((branch) => ({
        ...branch,
        isActive: false,
      })),
    };
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockResolvedValue({
        ...detail,
        contextCursor: rebasedTree.cursor,
      });
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(rebasedTree);
    vi.mocked(bridge.setActiveContext).mockResolvedValue(rebasedTree.cursor);
    vi.mocked(bridge.updateContextDraft).mockResolvedValue({ draftVersion: 1 });
    vi.mocked(bridge.inspectContext)
      .mockResolvedValueOnce(preview)
      .mockResolvedValue({ ...preview, draftVersion: 2 });
    vi.mocked(bridge.previewContextTransition).mockImplementation(async (input) => (
      input.parentRunId === "run-a" && input.draftVersion === 1
        ? persistedDraftPreview.promise
        : { ...preview, draftVersion: input.draftVersion }
    ));
    vi.mocked(bridge.createTurnAndStartRun).mockResolvedValue({
      ...runHandle("run-from-root", "turn-from-root"),
      draftVersion: 3,
    });

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(await screen.findByRole("button", { name: "排除 回答 A" }));
    await waitFor(() =>
      expect(bridge.previewContextTransition).toHaveBeenCalledWith(
        expect.objectContaining({
          parentRunId: "run-a",
          draftVersion: 1,
        }),
      ),
    );

    // The persisted-draft scenario starts after the authoritative preview is applied,
    // not merely after its asynchronous bridge request has begun.
    await act(async () => {
      const excludedItems = preview.items.map((item) => ({
        ...item,
        included: item.sourceRef.id === "run-a" ? false : item.included,
      }));
      persistedDraftPreview.resolve({
        ...preview,
        items: excludedItems,
        rawItems: excludedItems,
        draftVersion: 1,
      });
    });
    expect(await screen.findByRole("button", { name: "重新纳入 回答 A" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "打开 Context Tree" }));
    fireEvent.click(screen.getByRole("treeitem", { name: /工作区起点/ }));

    await waitFor(() =>
      expect(bridge.setActiveContext).toHaveBeenCalledWith({
        workspaceId: workspace.id,
        runId: null,
        branchId: null,
        expectedCursorVersion: 4,
        expectedDraftVersion: 1,
      }),
    );
    await waitFor(() =>
      expect(bridge.inspectContext).toHaveBeenLastCalledWith(
        expect.objectContaining({
          workspaceId: workspace.id,
          parentRunId: null,
        }),
      ),
    );

    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "从根节点重新开始" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() =>
      expect(bridge.createTurnAndStartRun).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: workspace.id,
          parentRunId: null,
          expectedCursorVersion: 5,
          expectedDraftVersion: 2,
        }),
        expect.any(Function),
      ),
    );
  });

  it("falls back atomically for a legacy snapshot instead of rendering partial provider metadata", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getRunSnapshot).mockResolvedValue({
      id: "snapshot-legacy",
      runId: "run-a",
      canonicalHash: "sha256:legacy",
      createdAt: "2026-07-22T09:11:00Z",
      providerName: "Legacy Provider",
      model: "legacy-model",
      baseUrl: "https://legacy.example.com/v1",
      additionalHeaders: {},
      parameters: { temperature: 0.7 },
      items: preview.items,
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));

    expect(await screen.findByText("旧版快照：Provider 协议与认证元数据未记录")).toBeVisible();
    expect(screen.queryByText(/undefined|rundefined/)).not.toBeInTheDocument();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.7")).toBeVisible();
  });

  it("discards a delayed snapshot after the persisted cursor moves to another Run", async () => {
    const bridge = bridgeFixture();
    const movedTree = contextTreeAt("run-b");
    let resolveSnapshot!: (snapshot: RunSnapshot) => void;
    const delayedSnapshot = new Promise<RunSnapshot>((resolve) => {
      resolveSnapshot = resolve;
    });
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(movedTree);
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockResolvedValue({
        ...detail,
        selectedRunIds: { "turn-1": "run-b" },
        contextCursor: movedTree.cursor,
      });
    vi.mocked(bridge.setActiveContext).mockResolvedValue(movedTree.cursor);
    vi.mocked(bridge.getRunSnapshot).mockReturnValue(delayedSnapshot);

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));
    await waitFor(() => expect(bridge.getRunSnapshot).toHaveBeenCalledWith("run-a"));

    fireEvent.click(screen.getByRole("button", { name: /回答 B.*qwen3:14b/i }));
    await screen.findByText(/从精确 Run run-b 继续/);

    await act(async () => {
      resolveSnapshot({
        id: "snapshot-stale",
        runId: "run-a",
        canonicalHash: "sha256:must-not-cross-runs",
        createdAt: "2026-07-22T09:11:00Z",
        providerName: "OpenAI compatible",
        additionalHeaders: {},
        model: "gpt-4.1",
        baseUrl: "https://api.example.com/v1",
        parameters: {},
        items: preview.items,
      });
      await delayedSnapshot;
    });

    expect(screen.queryByText("sha256:must-not-cross-runs")).not.toBeInTheDocument();
  });

  it("moves the persisted Context cursor to an exact Run and keeps the composer draft", async () => {
    const bridge = bridgeFixture();
    const movedTree = contextTreeAt("run-b");
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(movedTree);
    vi.mocked(bridge.setActiveContext).mockResolvedValue(movedTree.cursor);

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "这段草稿不能丢" },
    });
    fireEvent.click(screen.getByRole("button", { name: "打开 Context Tree" }));
    fireEvent.click(screen.getByRole("button", { name: "全部节点" }));
    fireEvent.click(screen.getByRole("treeitem", { name: /run-b/ }));

    await waitFor(() =>
      expect(bridge.setActiveContext).toHaveBeenCalledWith({
        workspaceId: workspace.id,
        runId: "run-b",
        branchId: "branch-b",
        expectedCursorVersion: 4,
        expectedDraftVersion: 0,
      }),
    );
    expect((await screen.findAllByText("以专注阅读为主，路线图按需打开。")).length).toBeGreaterThan(0);
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("这段草稿不能丢");
    expect(screen.getByText(/从精确 Run run-b 继续/)).toBeVisible();
  });

  it("refreshes a stale cursor without losing the composer draft", async () => {
    const bridge = bridgeFixture();
    const remoteRun = {
      ...detail.turns[0].runs[0],
      id: "run-remote",
      turnId: "turn-remote",
      output: "另一窗口已经落盘的权威回答。",
      createdAt: "2026-07-22T10:20:00Z",
      completedAt: "2026-07-22T10:20:05Z",
    };
    const remoteDetail: WorkspaceDetail = {
      ...detail,
      turns: [
        ...detail.turns,
        {
          id: "turn-remote",
          workspaceId: workspace.id,
          parentRunId: "run-a",
          prompt: "另一窗口继续了这条路线",
          createdAt: "2026-07-22T10:19:00Z",
          runs: [remoteRun],
        },
      ],
      selectedRunIds: {
        ...detail.selectedRunIds,
        "turn-remote": remoteRun.id,
      },
      contextCursor: {
        ...detail.contextCursor!,
        activeRunId: remoteRun.id,
        version: 5,
      },
    };
    const remoteTree: ContextTreeProjection = {
      ...contextTree,
      cursor: {
        ...contextTree.cursor,
        activeRunId: remoteRun.id,
        version: 5,
      },
      nodes: [
        ...contextTree.nodes.map((node) => ({
          ...node,
          isActive: false,
          isOnActivePath: node.runId === "run-a",
        })),
        {
          runId: remoteRun.id,
          turnId: remoteRun.turnId,
          parentRunId: "run-a",
          prompt: "另一窗口继续了这条路线",
          title: "另一窗口继续",
          outputPreview: remoteRun.output,
          model: remoteRun.model,
          status: remoteRun.status,
          createdAt: remoteRun.createdAt,
          canContinue: true,
          isActive: true,
          isOnActivePath: true,
          branchIds: ["branch-a"],
          checkpointIds: [],
        },
      ],
      edges: [
        ...contextTree.edges.map((edge) => ({
          ...edge,
          isOnActivePath: edge.targetRunId === "run-a",
        })),
        {
          id: "edge-run-remote",
          sourceRunId: "run-a",
          targetRunId: remoteRun.id,
          isOnActivePath: true,
        },
      ],
      branches: contextTree.branches.map((branch) => branch.id === "branch-a"
        ? { ...branch, headRunId: remoteRun.id, version: 3, isActive: true }
        : { ...branch, isActive: false }),
    };
    vi.mocked(bridge.openWorkspace)
      .mockResolvedValueOnce(detail)
      .mockResolvedValue(remoteDetail);
    vi.mocked(bridge.getContextTree)
      .mockResolvedValueOnce(contextTree)
      .mockResolvedValue(remoteTree);
    vi.mocked(bridge.setActiveContext).mockRejectedValue(
      new DesktopBridgeError({
        code: "context_cursor_conflict",
        message: "Context 位置已在其他窗口变化。",
        retryable: true,
      }),
    );

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "保留冲突前草稿" },
    });
    fireEvent.click(screen.getByRole("button", { name: "打开 Context Tree" }));
    fireEvent.click(screen.getByRole("treeitem", { name: /工作区起点/ }));

    expect(await screen.findByText("Context 位置已在其他窗口变化。")).toBeVisible();
    expect(bridge.getContextTree).toHaveBeenCalledTimes(2);
    expect(bridge.openWorkspace).toHaveBeenCalledTimes(2);
    expect((await screen.findAllByText("另一窗口已经落盘的权威回答。")).length)
      .toBeGreaterThan(0);
    expect(screen.getByText(/从精确 Run run-remote 继续/)).toBeVisible();
    expect(screen.getByRole("textbox", { name: "消息" })).toHaveValue("保留冲突前草稿");
  });

  it("creates a manual checkpoint only after confirming an exact source range", async () => {
    const bridge = bridgeFixture();
    const maintenanceTree: ContextTreeProjection = {
      ...contextTree,
      nodes: [
        {
          ...contextTree.nodes[0],
          runId: "run-parent",
          turnId: "turn-parent",
          parentRunId: null,
          title: "事实基线",
          prompt: "先确认事实",
          isActive: false,
          isOnActivePath: true,
          branchIds: ["branch-a"],
        },
        {
          ...contextTree.nodes[0],
          parentRunId: "run-parent",
          isActive: true,
          isOnActivePath: true,
        },
        contextTree.nodes[1],
      ],
      edges: [
        {
          id: "edge-run-parent",
          sourceRunId: null,
          targetRunId: "run-parent",
          isOnActivePath: true,
        },
        {
          id: "edge-run-a",
          sourceRunId: "run-parent",
          targetRunId: "run-a",
          isOnActivePath: true,
        },
        contextTree.edges[1],
      ],
    };
    vi.mocked(bridge.getContextTree).mockResolvedValue(maintenanceTree);
    vi.mocked(bridge.createContextCheckpoint).mockResolvedValue({
      id: "checkpoint-manual",
      workspaceId: workspace.id,
      branchId: "branch-a",
      branchVersion: 2,
      kind: "compaction",
      anchorRunId: "run-a",
      sourceRunIds: ["run-parent"],
      sourceHash: "sha256:sources",
      firstKeptRunId: "run-a",
      summary: "保留已验证事实。",
      provider: null,
      status: "completed",
      createdAt: "2026-07-28T10:00:00Z",
    });

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    const openMaintenance = screen.getByRole("button", { name: "准备 Context 压缩" });
    await waitFor(() => expect(openMaintenance).toBeEnabled());
    fireEvent.click(openMaintenance);
    fireEvent.change(screen.getByRole("textbox", { name: "人工摘要" }), {
      target: { value: "保留已验证事实。" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));

    await waitFor(() =>
      expect(bridge.createContextCheckpoint).toHaveBeenCalledWith({
        clientOperationId: expect.any(String),
        workspaceId: workspace.id,
        branchId: "branch-a",
        kind: "compaction",
        sourceRunIds: ["run-parent"],
        firstKeptRunId: "run-a",
        summary: "保留已验证事实。",
        expectedCursorVersion: 4,
        expectedBranchVersion: 2,
      }),
    );
    expect(await screen.findByText("Context 检查点已保存并激活。")).toBeVisible();
    expect(screen.queryByRole("complementary", { name: "Context 压缩预览" }))
      .not.toBeInTheDocument();
  });

  it("summarizes and moves Context atomically with cursor, branch, and draft CAS versions", async () => {
    const bridge = bridgeFixture();
    const maintenanceTree = maintenanceTreeFixture();
    vi.mocked(bridge.getContextTree).mockResolvedValue(maintenanceTree);
    vi.mocked(bridge.summarizeAndSetActiveContext).mockResolvedValue({
      cursor: { ...maintenanceTree.cursor, version: 5 },
      checkpoint: null,
    });

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalled());
    const openMaintenance = screen.getByRole("button", { name: "准备 Context 压缩" });
    await waitFor(() => expect(openMaintenance).toBeEnabled());
    fireEvent.click(openMaintenance);
    fireEvent.click(screen.getByRole("button", { name: "Provider 生成摘要" }));
    fireEvent.change(screen.getByRole("textbox", { name: "摘要请求" }), {
      target: { value: "只总结已验证事实。" },
    });
    fireEvent.click(screen.getByRole("button", { name: "确认生成并切换 Context" }));

    await waitFor(() =>
      expect(bridge.summarizeAndSetActiveContext).toHaveBeenCalledWith({
        clientOperationId: expect.any(String),
        workspaceId: workspace.id,
        targetRunId: "run-a",
        branchId: "branch-a",
        sourceRunIds: ["run-parent"],
        firstKeptRunId: "run-a",
        summaryPrompt: "只总结已验证事实。",
        providerProfileId: "provider-cloud",
        expectedCursorVersion: 4,
        expectedBranchVersion: 2,
        expectedDraftVersion: 0,
      }),
    );
    expect(await screen.findByText("摘要已生成，Context 已原子切换。")).toBeVisible();
  });

  it("can cancel an in-flight Provider summary with the stable maintenance operation id", async () => {
    const bridge = bridgeFixture();
    const maintenanceTree = maintenanceTreeFixture();
    let rejectSummary: ((reason?: unknown) => void) | undefined;
    vi.mocked(bridge.getContextTree).mockResolvedValue(maintenanceTree);
    vi.mocked(bridge.summarizeAndSetActiveContext).mockImplementation(
      () => new Promise((_resolve, reject) => {
        rejectSummary = reject;
      }),
    );
    vi.mocked(bridge.cancelContextMaintenance).mockImplementation(async () => {
      rejectSummary?.(new DesktopBridgeError({
        code: "context_maintenance_cancelled",
        message: "Context summary generation was cancelled",
        retryable: false,
      }));
    });

    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalled());
    const openMaintenance = screen.getByRole("button", { name: "准备 Context 压缩" });
    await waitFor(() => expect(openMaintenance).toBeEnabled());
    fireEvent.click(openMaintenance);
    fireEvent.click(screen.getByRole("button", { name: "Provider 生成摘要" }));
    fireEvent.click(screen.getByRole("button", { name: "确认生成并切换 Context" }));

    await waitFor(() => expect(bridge.summarizeAndSetActiveContext).toHaveBeenCalled());
    const operationId = vi.mocked(bridge.summarizeAndSetActiveContext)
      .mock.calls[0]?.[0].clientOperationId;
    fireEvent.click(
      screen.getByRole("button", { name: "取消正在生成的 Context 摘要" }),
    );

    await waitFor(() =>
      expect(bridge.cancelContextMaintenance).toHaveBeenCalledWith(operationId),
    );
    expect(await screen.findByText("已请求取消摘要；检查点和当前 Context 不会移动。"))
      .toBeVisible();
    const submit = screen.getByRole("button", { name: "确认生成并切换 Context" });
    await waitFor(() => expect(submit).toBeEnabled());
    fireEvent.click(submit);
    await waitFor(() =>
      expect(bridge.summarizeAndSetActiveContext).toHaveBeenCalledTimes(2),
    );
    expect(
      vi.mocked(bridge.summarizeAndSetActiveContext).mock.calls[1]?.[0].clientOperationId,
    ).not.toBe(operationId);
  });
});
