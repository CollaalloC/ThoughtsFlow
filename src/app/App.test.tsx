import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { DesktopBridge } from "../platform/desktop-bridge";
import type {
  CompareRunsResult,
  ProviderProfile,
  RouteProjection,
  WorkspaceDetail,
  WorkspaceSummary,
} from "../shared/contracts";
import { App } from "./App";

const workspace: WorkspaceSummary = {
  id: "workspace-1",
  name: "技术路线评审",
  goal: "选择迁移策略",
  systemPrompt: "你是一名严谨的技术决策协作者。",
  archived: false,
  createdAt: "2026-07-22T08:00:00Z",
  updatedAt: "2026-07-22T08:00:00Z",
};

const provider: ProviderProfile = {
  id: "provider-1",
  providerId: "ollama",
  name: "Local fixture",
  dialect: "ollama",
  baseUrl: "http://127.0.0.1:11434",
  model: "fixture-model",
  isDefault: true,
};

const detail: WorkspaceDetail = {
  workspace,
  turns: [
    {
      id: "turn-root",
      workspaceId: workspace.id,
      parentRunId: null,
      prompt: "先建立共同事实",
      title: "事实基线",
      createdAt: "2026-07-22T08:01:00Z",
      runs: [
        {
          id: "run-root-a",
          turnId: "turn-root",
          status: "completed",
          output: "基线回答 A",
          providerProfileId: provider.id,
          providerName: provider.name,
          model: provider.model,
          baseUrl: provider.baseUrl,
          createdAt: "2026-07-22T08:02:00Z",
          completedAt: "2026-07-22T08:03:00Z",
        },
        {
          id: "run-root-b",
          turnId: "turn-root",
          status: "completed",
          output: "基线回答 B",
          providerProfileId: provider.id,
          providerName: provider.name,
          model: provider.model,
          baseUrl: provider.baseUrl,
          createdAt: "2026-07-22T08:04:00Z",
          completedAt: "2026-07-22T08:05:00Z",
        },
      ],
    },
  ],
  selectedRunIds: { "turn-root": "run-root-b" },
  adjacentBranches: [],
  decisionMarks: [],
};

const routeProjection: RouteProjection = {
  workspaceId: workspace.id,
  nodes: [
    {
      id: "node-root",
      turnId: "turn-root",
      title: "事实基线",
      summary: "先建立共同事实",
      status: "completed",
      x: 40,
      y: 80,
      isCurrent: true,
      isOnCurrentLineage: true,
      runs: [
        {
          runId: "run-root-a",
          label: "回答 A",
          model: provider.model,
          status: "completed",
          canBranch: true,
        },
        {
          runId: "run-root-b",
          label: "回答 B",
          model: provider.model,
          status: "completed",
          canBranch: true,
        },
      ],
    },
  ],
  edges: [],
};

const comparison: CompareRunsResult = {
  left: { runId: "run-root-a", model: provider.model, status: "completed" },
  right: { runId: "run-root-b", model: provider.model, status: "completed" },
  answer: { leftMarkdown: "回答 A", rightMarkdown: "回答 B" },
  contextDiff: { onlyLeft: [], onlyRight: [], shared: [] },
};

function bridgeFixture(overrides: Partial<DesktopBridge> = {}): DesktopBridge {
  return {
    listWorkspaces: vi.fn().mockResolvedValue([workspace]),
    createWorkspace: vi.fn().mockResolvedValue(workspace),
    openWorkspace: vi.fn().mockResolvedValue(detail),
    updateWorkspace: vi.fn().mockResolvedValue(workspace),
    inspectContext: vi.fn().mockResolvedValue({
      hash: "preview-hash",
      estimatedTokens: 0,
      limitTokens: 10_000,
      blocked: false,
      warnings: [],
      providerProfileId: provider.id,
      providerName: provider.name,
      model: provider.model,
      baseUrl: provider.baseUrl,
      items: [],
    }),
    createTurnAndStartRun: vi.fn(),
    retryRun: vi.fn(),
    cancelRun: vi.fn().mockResolvedValue(undefined),
    getRunSnapshot: vi.fn(),
    updateContextOverrides: vi.fn().mockResolvedValue(undefined),
    getRouteProjection: vi.fn().mockResolvedValue(routeProjection),
    updateViewState: vi.fn().mockResolvedValue(undefined),
    compareRuns: vi.fn().mockResolvedValue(comparison),
    markDecision: vi.fn(),
    exportDecisionPacket: vi.fn(),
    listProviderTemplates: vi.fn().mockResolvedValue([]),
    listProviderProfiles: vi.fn().mockResolvedValue([provider]),
    saveProviderProfile: vi.fn().mockResolvedValue(provider),
    setSessionCredential: vi.fn().mockResolvedValue(undefined),
    testProviderConnection: vi.fn().mockResolvedValue({ ok: true, message: "ok" }),
    subscribeToRunEvents: vi.fn().mockReturnValue(() => undefined),
    ...overrides,
  };
}

describe("App", () => {
  it("opens the persisted route projection and returns to Focus", async () => {
    const user = userEvent.setup();
    const bridge = bridgeFixture();

    render(<App bridge={bridge} />);
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();

    const navigation = screen.getByRole("navigation", { name: "工作面" });
    await user.click(within(navigation).getByRole("button", { name: "路线图" }));

    expect(await screen.findByRole("region", { name: "对话路线图" })).toBeVisible();
    expect(bridge.getRouteProjection).toHaveBeenCalledWith({
      workspaceId: workspace.id,
      currentRunId: undefined,
    });
    expect(bridge.openWorkspace).toHaveBeenCalledWith(workspace.id);

    await user.click(screen.getByRole("button", { name: "返回 Focus" }));
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
  });

  it("connects comparison, decision marking, and export to the DesktopBridge", async () => {
    const user = userEvent.setup();
    const bridge = bridgeFixture({
      markDecision: vi.fn().mockResolvedValue({
        id: "decision-1",
        workspaceId: workspace.id,
        runId: "run-root-a",
        status: "accepted",
        reason: "保留回滚窗口。",
        createdAt: "2026-07-22T09:00:00Z",
      }),
      exportDecisionPacket: vi.fn().mockResolvedValue({
        path: "/tmp/decision.md",
        bytesWritten: 128,
      }),
    });

    render(<App bridge={bridge} />);
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    const navigation = screen.getByRole("navigation", { name: "工作面" });
    await user.click(within(navigation).getByRole("button", { name: "决策" }));

    expect(
      await screen.findByRole("heading", { name: "比较路线，形成可审查的判断" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "比较两条路线" }));
    expect(bridge.compareRuns).toHaveBeenCalledWith({
      leftRunId: "run-root-a",
      rightRunId: "run-root-b",
    });

    const editor = screen.getByRole("article", { name: "事实基线 · 回答 A的决策" });
    await user.click(within(editor).getByRole("button", { name: "采纳" }));
    await user.type(within(editor).getByRole("textbox"), "保留回滚窗口。");
    await user.click(within(editor).getByRole("button", { name: "保存决策标记" }));
    expect(bridge.markDecision).toHaveBeenCalledWith({
      workspaceId: workspace.id,
      runId: "run-root-a",
      status: "accepted",
      reason: "保留回滚窗口。",
    });

    await user.click(screen.getByRole("button", { name: "导出 Decision Packet" }));
    expect(bridge.exportDecisionPacket).toHaveBeenCalledWith({ workspaceId: workspace.id });
    expect(await screen.findByText("Decision Packet 已导出：/tmp/decision.md")).toBeVisible();
  });

  it("reports a route loading failure and retries through the public navigation", async () => {
    const user = userEvent.setup();
    const getRouteProjection = vi
      .fn()
      .mockRejectedValueOnce(new Error("路线投影暂时不可用"))
      .mockResolvedValue(routeProjection);
    const bridge = bridgeFixture({ getRouteProjection });

    render(<App bridge={bridge} />);
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    const navigation = screen.getByRole("navigation", { name: "工作面" });
    await user.click(within(navigation).getByRole("button", { name: "路线图" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("路线投影暂时不可用");
    await user.click(screen.getByRole("button", { name: "重试加载" }));
    expect(await screen.findByRole("region", { name: "对话路线图" })).toBeVisible();
    expect(getRouteProjection).toHaveBeenCalledTimes(2);
  });

  it("opens Provider settings and exposes an accessible return to Focus", async () => {
    const user = userEvent.setup();
    const bridge = bridgeFixture();

    render(<App bridge={bridge} />);
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    const navigation = screen.getByRole("navigation", { name: "工作面" });
    await user.click(within(navigation).getByRole("button", { name: "Provider 设置" }));

    expect(await screen.findByRole("heading", { name: "Providers" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "返回 Focus" }));
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
  });

  it("reloads for an exact run selection and persists route view movement", async () => {
    const user = userEvent.setup();
    const bridge = bridgeFixture();

    render(<App bridge={bridge} />);
    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    const navigation = screen.getByRole("navigation", { name: "工作面" });
    await user.click(within(navigation).getByRole("button", { name: "路线图" }));
    const route = await screen.findByRole("region", { name: "对话路线图" });

    await user.click(within(route).getByRole("button", { name: "选择事实基线的回答 A" }));
    await waitFor(() =>
      expect(bridge.getRouteProjection).toHaveBeenCalledWith({
        workspaceId: workspace.id,
        currentRunId: "run-root-a",
      }),
    );
    await waitFor(() => expect(route).not.toBeInTheDocument());
    const refreshedRoute = await screen.findByRole("region", { name: "对话路线图" });

    const node = within(refreshedRoute).getByRole("article", { name: "事实基线" }).closest(
      ".react-flow__node",
    );
    expect(node).not.toBeNull();
    const eventWindow = (node as Element).ownerDocument.defaultView;
    expect(eventWindow).not.toBeNull();
    const dispatchMouse = (
      target: Document | Element | Node | Window,
      type: "mousedown" | "mousemove" | "mouseup",
      init: MouseEventInit,
    ) => {
      const event = new MouseEvent(type, init);
      Object.defineProperty(event, "view", { value: eventWindow });
      fireEvent(target, event);
    };
    dispatchMouse(node as Element, "mousedown", { button: 0, clientX: 80, clientY: 100 });
    dispatchMouse(eventWindow as Window, "mousemove", {
      buttons: 1,
      clientX: 180,
      clientY: 180,
    });
    dispatchMouse(eventWindow as Window, "mouseup", {
      button: 0,
      clientX: 180,
      clientY: 180,
    });

    await waitFor(() => expect(bridge.updateViewState).toHaveBeenCalled());
    expect(bridge.updateViewState).toHaveBeenLastCalledWith(
      expect.objectContaining({ workspaceId: workspace.id, turnId: "turn-root", collapsed: false }),
    );
  });
});
