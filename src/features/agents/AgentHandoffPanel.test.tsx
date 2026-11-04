import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDesktopBridge, type DesktopBridge } from "../../platform/desktop-bridge";
import type { AgentEvidence, AgentEvidenceSummary, AgentHandoff, AgentHandoffSummary, AgentHandoffPreview, AgentMission, AgentOperation, ContextTreeProjection } from "../../shared/contracts";
import { AgentHandoffPanel } from "./AgentHandoffPanel";
import { agentHistoryTime } from "./AgentHistory";
import { AgentTaskDrafts } from "./agent-drafts";

const mission: AgentMission = { runtimeBackend: "herdr-core", id: "mission-1", workspaceId: "workspace-1", repositoryId: "repo", repositoryPath: "/repo", objective: "目标", runId: "run-1", coordinatorHandle: "coordinator", runtimeId: "runtime", status: "ready", error: null, createdAt: "2026-10-06" };
const tree: ContextTreeProjection = { rootId: "root", workspaceId: mission.workspaceId, cursor: { workspaceId: mission.workspaceId, version: 0, activeRunId: null, branchId: null, updatedAt: "2026-10-06" }, draftVersion: 0, nodes: [], edges: [], branches: [], checkpoints: [] };
const preview: AgentHandoffPreview = { title: "下一任务", hash: "preview-hash", text: "完整交接预览", compilerVersion: "1", sourceRunIds: [], evidenceIds: [], sources: [], parentRunId: null, branchId: null, cursorVersion: 0, draftVersion: 0, branchVersion: null };
function evidence(id: string): AgentEvidenceSummary {
  return { id, workspaceId: mission.workspaceId, missionId: mission.id, runId: "run-1", taskId: `task-${id}`, dispatchId: `dispatch-${id}`, handoffOperationId: null, title: `证据 ${id}`, contentHash: `hash-${id}`, source: "terminal", requestCursor: null, cursor: null, hasMore: false, warning: null, observedTaskStatus: "completed", createdAt: "2026-10-06T00:00:00Z" };
}
function handoff(id: string): AgentHandoffSummary {
  return { operationId: id, workspaceId: mission.workspaceId, missionId: mission.id, createdAt: "2026-10-06T00:00:00Z", title: `交接 ${id}`, hash: `hash-${id}`, sourceRunIds: [], evidenceIds: [] };
}
function fixture(overrides: Partial<DesktopBridge> = {}, invoke = async () => { throw new Error("Unexpected IPC"); }): DesktopBridge {
  return {
    ...createDesktopBridge(invoke),
    getContextTree: vi.fn().mockResolvedValue(tree),
    listAgentEvidence: vi.fn(), listAgentHandoffs: vi.fn(),
    listAgentEvidencePage: vi.fn().mockResolvedValue({ items: [evidence("e1")], nextCursor: null }),
    listAgentHandoffPage: vi.fn().mockResolvedValue({ items: [handoff("h1")], nextCursor: null }),
    getAgentEvidence: vi.fn().mockResolvedValue({ ...evidence("e1"), text: "不可变输出正文" }),
    getAgentHandoff: vi.fn().mockResolvedValue({ ...handoff("h1"), preview: { ...preview, text: "不可变交接正文" } }),
    previewAgentHandoff: vi.fn<DesktopBridge["previewAgentHandoff"]>().mockImplementation(async ({ requestedTemplate }) => ({
      ...preview, ...(requestedTemplate === undefined ? {} : { requestedTemplate }),
    })),
    ...overrides,
  };
}
function props(bridge: DesktopBridge) {
  return { templates: [{ name: "reviewer", description: "Review" }, { name: "writer", description: "Write" }], bridge, mission, busy: false, canStart: true, evidenceRevision: 0, onStart: vi.fn<() => Promise<AgentOperation | undefined>>().mockResolvedValue({ id: "op", missionId: mission.id, kind: "start-task", status: "succeeded", receipt: null, error: null, createdAt: "2026-10-06" }) };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
async function makePreview() {
  if (!(screen.getByLabelText("Agent 模板") as HTMLSelectElement).value) fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: "reviewer" } });
  fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "下一任务" } });
  fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "核实来源" } });
  fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
  await screen.findByRole("region", { name: "交接预览" });
}

// jsdom does not implement the matrix used by ReactFlow's resize observer.
beforeEach(() => vi.stubGlobal("DOMMatrixReadOnly", class { m22 = 1; }));
afterEach(() => vi.unstubAllGlobals());

describe("Herdr native template handoff", () => {
  it("preserves existing dotted and localized template names", async () => {
    const bridge = fixture();
    render(<AgentHandoffPanel {...props(bridge)} templates={[{ name: "审查.v2", description: "Native template" }]} />);
    fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: "审查.v2" } });
    await makePreview();
    expect(bridge.previewAgentHandoff).toHaveBeenCalledWith(expect.objectContaining({ requestedTemplate: "审查.v2" }));
  });
  it("uses an existing template without a Provider and freezes it in the preview", async () => {
    const bridge = fixture(); const panel = props(bridge);
    render(<AgentHandoffPanel {...panel} />);
    expect(screen.queryByLabelText("模型选择")).not.toBeInTheDocument();
    await makePreview();
    expect(bridge.previewAgentHandoff).toHaveBeenCalledWith(expect.objectContaining({ requestedTemplate: "reviewer" }));
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    await waitFor(() => expect(panel.onStart).toHaveBeenCalledWith(expect.objectContaining({ requestedTemplate: "reviewer" })));
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("");
  });
  it("invalidates preview when the template changes", async () => {
    render(<AgentHandoffPanel {...props(fixture())} />); await makePreview();
    fireEvent.change(screen.getByLabelText("Agent 模板"), { target: { value: "writer" } });
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
  });
  it("keeps old Orca missions read-only without replacing stored model evidence", async () => {
    const bridge = fixture({ getAgentHandoff: vi.fn().mockResolvedValue({ ...handoff("h1"), preview: { ...preview, requestedModel: "@smol" } }) });
    render(<AgentHandoffPanel {...props(bridge)} mission={{ ...mission, runtimeBackend: "orca" }} />);
    expect(screen.queryByRole("button", { name: "启动 Agent 任务" })).not.toBeInTheDocument();
    const history = await screen.findByRole("region", { name: "历史任务交接" });
    fireEvent.click(within(history).getByText("交接 h1", { exact: false }));
    expect(await within(history).findByText(/请求模型：@smol/)).toBeVisible();
  });
});

describe("Agent handoff paged history", () => {
  it("loads only the first summary pages and mounts no hidden bodies for a large history", async () => {
    const allEvidence = Array.from({ length: 1000 }, (_, index) => evidence(`e${index}`));
    const allHandoffs = Array.from({ length: 1000 }, (_, index) => handoff(`h${index}`));
    const bridge = fixture({
      listAgentEvidencePage: vi.fn().mockResolvedValue({ items: allEvidence.slice(0, 50), nextCursor: "e-next" }),
      listAgentHandoffPage: vi.fn().mockResolvedValue({ items: allHandoffs.slice(0, 50), nextCursor: "h-next" }),
    });
    const rendered = render(<AgentHandoffPanel {...props(bridge)} />);
    await screen.findByRole("checkbox", { name: "使用证据 证据 e49" });
    expect(screen.queryByRole("checkbox", { name: "使用证据 证据 e50" })).not.toBeInTheDocument();
    expect(bridge.listAgentEvidencePage).toHaveBeenCalledExactlyOnceWith({ workspaceId: mission.workspaceId, cursor: undefined });
    expect(bridge.listAgentHandoffPage).toHaveBeenCalledExactlyOnceWith({ missionId: mission.id, cursor: undefined });
    expect(bridge.listAgentEvidence).not.toHaveBeenCalled(); expect(bridge.listAgentHandoffs).not.toHaveBeenCalled();
    expect(bridge.getAgentEvidence).not.toHaveBeenCalled(); expect(bridge.getAgentHandoff).not.toHaveBeenCalled();
    expect(rendered.container.querySelectorAll("details")).toHaveLength(100);
    expect(rendered.container.querySelectorAll("pre")).toHaveLength(0);
    expect(screen.queryByRole("tree")).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("包含当前图谱路径"));
    expect(screen.getByText("当前工作区还没有对话路径，将只交接任务说明和已选证据。")).toBeVisible();
    fireEvent.change(screen.getByLabelText("任务标题"), { target: { value: "只修改任务输入" } });
    expect(rendered.container.querySelectorAll("pre")).toHaveLength(0);
    expect(bridge.listAgentEvidencePage).toHaveBeenCalledTimes(1);
  });

  it("reads immutable bodies only on expansion, unmounts on close, and reuses the cached detail", async () => {
    const bridge = fixture();
    render(<AgentHandoffPanel {...props(bridge)} />);
    const saved = screen.getByRole("region", { name: "已保存的 Agent 证据" });
    const summary = await within(saved).findByText(`证据 e1 · ${agentHistoryTime(evidence("e1").createdAt)}`);
    fireEvent.click(summary);
    expect(await within(saved).findByText("不可变输出正文")).toBeVisible();
    expect(bridge.getAgentEvidence).toHaveBeenCalledExactlyOnceWith({ workspaceId: mission.workspaceId, id: "e1" });
    fireEvent.click(summary);
    await waitFor(() => expect(within(saved).queryByText("不可变输出正文")).not.toBeInTheDocument());
    fireEvent.click(summary);
    expect(await within(saved).findByText("不可变输出正文")).toBeVisible();
    expect(bridge.getAgentEvidence).toHaveBeenCalledTimes(1);
    const history = screen.getByRole("region", { name: "历史任务交接" });
    fireEvent.click(within(history).getByText(`交接 h1 · ${agentHistoryTime(handoff("h1").createdAt)}`));
    expect(await within(history).findByText("不可变交接正文")).toBeVisible();
    expect(bridge.getAgentHandoff).toHaveBeenCalledExactlyOnceWith({ missionId: mission.id, operationId: "h1" });
  });

  it("retains cross-page selections and reloads without discarding them, invalidating preview on selection changes", async () => {
    const bridge = fixture({ listAgentEvidencePage: vi.fn().mockImplementation(async ({ cursor }) => cursor
      ? { items: [evidence("e2")], nextCursor: null } : { items: [evidence("e1")], nextCursor: "opaque-next" }) });
    render(<AgentHandoffPanel {...props(bridge)} />);
    fireEvent.click(await screen.findByRole("checkbox", { name: "使用证据 证据 e1" }));
    await makePreview();
    fireEvent.click(screen.getByRole("button", { name: "下一页证据" }));
    fireEvent.click(await screen.findByRole("checkbox", { name: "使用证据 证据 e2" }));
    expect(screen.getByRole("button", { name: "取消证据 证据 e1" })).toBeVisible();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    await makePreview();
    expect(bridge.previewAgentHandoff).toHaveBeenLastCalledWith(expect.objectContaining({ context: { includeActiveContext: false, evidenceIds: ["e1", "e2"] } }));
    fireEvent.click(screen.getByRole("button", { name: "重新加载上下文与证据" }));
    await screen.findByRole("checkbox", { name: "使用证据 证据 e2" });
    expect(screen.getByRole("button", { name: "取消证据 证据 e1" })).toBeVisible();
    expect(screen.getByRole("checkbox", { name: "使用证据 证据 e2" })).toBeChecked();
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "上一页证据" }));
    expect(await screen.findByRole("checkbox", { name: "使用证据 证据 e1" })).toBeChecked();
    expect(bridge.getAgentEvidence).not.toHaveBeenCalled();
  });

  it("limits selected evidence to 32 while letting the user remove an earlier choice", async () => {
    const bridge = fixture({ listAgentEvidencePage: vi.fn().mockResolvedValue({ items: Array.from({ length: 40 }, (_, i) => evidence(`e${i}`)), nextCursor: null }) });
    render(<AgentHandoffPanel {...props(bridge)} />);
    await screen.findByRole("checkbox", { name: "使用证据 证据 e39" });
    for (let i = 0; i < 32; i++) fireEvent.click(screen.getByRole("checkbox", { name: `使用证据 证据 e${i}` }));
    expect(screen.getByRole("checkbox", { name: "使用证据 证据 e32" })).toBeDisabled();
    expect(screen.getByText(/已选 32 \/ 32 条/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "取消证据 证据 e0" }));
    expect(screen.getByRole("checkbox", { name: "使用证据 证据 e32" })).toBeEnabled();
  });

  it("ignores late detail and page results after switching the owning workspace or mission", async () => {
    const oldDetail = deferred<AgentEvidence>();
    const oldPage = deferred<{ items: AgentHandoffSummary[]; nextCursor: string | null }>();
    const bridge = fixture({ getAgentEvidence: vi.fn().mockReturnValue(oldDetail.promise),
      listAgentEvidencePage: vi.fn().mockImplementation(async ({ workspaceId }) => ({ items: workspaceId === mission.workspaceId ? [evidence("e1")] : [], nextCursor: null })),
      listAgentHandoffPage: vi.fn().mockImplementation(({ missionId }) => missionId === mission.id ? oldPage.promise : Promise.resolve({ items: [], nextCursor: null })),
    });
    const rendered = render(<AgentHandoffPanel {...props(bridge)} />);
    fireEvent.click(await screen.findByText(`证据 e1 · ${agentHistoryTime(evidence("e1").createdAt)}`));
    await waitFor(() => expect(bridge.getAgentEvidence).toHaveBeenCalledTimes(1));
    rendered.rerender(<AgentHandoffPanel {...props(bridge)} mission={{ ...mission, id: "mission-2", workspaceId: "workspace-2" }} />);
    await act(async () => { oldDetail.resolve({ ...evidence("e1"), text: "旧工作区不能泄露的正文" }); oldPage.resolve({ items: [handoff("old")], nextCursor: null }); });
    expect(screen.queryByText("旧工作区不能泄露的正文")).not.toBeInTheDocument();
    expect(screen.queryByText(/交接 old/)).not.toBeInTheDocument();
  });

  it("rejects a detail from the wrong owner and retries only after an explicit click", async () => {
    const bridge = fixture({ getAgentEvidence: vi.fn().mockResolvedValueOnce({ ...evidence("e1"), workspaceId: "wrong", text: "不得显示" }).mockResolvedValue({ ...evidence("e1"), text: "正确详情" }) });
    render(<AgentHandoffPanel {...props(bridge)} />);
    fireEvent.click(await screen.findByText(`证据 e1 · ${agentHistoryTime(evidence("e1").createdAt)}`));
    expect(await screen.findByText("证据详情与所选记录不一致，请重新加载历史记录。")).toBeVisible();
    expect(screen.queryByText("不得显示")).not.toBeInTheDocument();
    expect(bridge.getAgentEvidence).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "重试读取" }));
    expect(await screen.findByText("正确详情")).toBeVisible();
  });

  it("restores only draft choices across remounts, keeps unknown drafts, and clears confirmed successful drafts", async () => {
    const drafts = new AgentTaskDrafts([[mission.id, { title: "保留标题", spec: "保留说明", requestedTemplate: "reviewer", includeActiveContext: true, evidenceIds: ["older-unloaded"] }]]);
    const bridge = fixture();
    const start = vi.fn().mockResolvedValueOnce({ id: "op", status: "unknown" }).mockResolvedValueOnce({ id: "op2", status: "succeeded" });
    const rendered = render(<AgentHandoffPanel {...props(bridge)} drafts={drafts} onStart={start} />);
    expect(screen.getByLabelText("任务标题")).toHaveValue("保留标题");
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("reviewer");
    expect(screen.getByRole("button", { name: "取消证据 older-unload" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("任务说明"), { target: { value: "最后一次输入" } });
    expect(drafts.get(mission.id)?.spec).toBe("最后一次输入");
    expect(drafts.get(mission.id)?.requestedTemplate).toBe("reviewer");
    fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
    await screen.findByRole("region", { name: "交接预览" });
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(drafts.get(mission.id)?.spec).toBe("最后一次输入");
    rendered.unmount();
    render(<AgentHandoffPanel {...props(bridge)} drafts={drafts} onStart={start} />);
    expect(screen.getByLabelText("任务说明")).toHaveValue("最后一次输入");
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("reviewer");
    expect(screen.getByRole("button", { name: "启动 Agent 任务" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
    await screen.findByRole("region", { name: "交接预览" });
    fireEvent.click(screen.getByRole("button", { name: "启动 Agent 任务" }));
    await waitFor(() => expect(drafts.has(mission.id)).toBe(false));
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("");
    expect(bridge.getAgentEvidence).not.toHaveBeenCalled();
  });

  it("keeps malformed historical timestamps readable", () => {
    expect(agentHistoryTime("legacy timestamp")).toBe("legacy timestamp");
  });
  it("returns directly to newest summaries after a capture on an older page while retaining selected evidence", async () => {
    let captured = false;
    const bridge = fixture({
      listAgentEvidencePage: vi.fn().mockImplementation(async ({ cursor }) => cursor
        ? { items: [evidence("older")], nextCursor: null }
        : { items: [evidence(captured ? "newly-saved" : "e1")], nextCursor: "older-evidence" }),
      listAgentHandoffPage: vi.fn().mockImplementation(async ({ cursor }) => cursor
        ? { items: [handoff("older")], nextCursor: null }
        : { items: [handoff("latest")], nextCursor: "older-handoff" }),
    });
    const panel = props(bridge);
    const rendered = render(<AgentHandoffPanel {...panel} />);
    fireEvent.click(await screen.findByRole("checkbox", { name: "使用证据 证据 e1" }));
    fireEvent.click(screen.getByRole("button", { name: "下一页证据" }));
    await screen.findByRole("checkbox", { name: "使用证据 证据 older" });
    fireEvent.click(screen.getByRole("button", { name: "下一页交接" }));
    await screen.findByText(`交接 older · ${agentHistoryTime(handoff("older").createdAt)}`);
    captured = true;
    rendered.rerender(<AgentHandoffPanel {...panel} evidenceRevision={1} />);
    await screen.findByRole("checkbox", { name: "使用证据 证据 older" });
    expect(screen.getByRole("button", { name: "取消证据 证据 e1" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "查看最新证据" }));
    await screen.findByRole("checkbox", { name: "使用证据 证据 newly-saved" });
    expect(bridge.listAgentEvidencePage).toHaveBeenLastCalledWith({ workspaceId: mission.workspaceId, cursor: undefined });
    expect(screen.getByRole("button", { name: "取消证据 证据 e1" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "查看最新交接" }));
    await screen.findByText(`交接 latest · ${agentHistoryTime(handoff("latest").createdAt)}`);
    expect(bridge.listAgentHandoffPage).toHaveBeenLastCalledWith({ missionId: mission.id, cursor: undefined });
    expect(bridge.getAgentEvidence).not.toHaveBeenCalled();
    expect(bridge.getAgentHandoff).not.toHaveBeenCalled();
  });

  it("reconciles a late confirmed draft deletion after remount and cleans StrictMode subscriptions", async () => {
    const saved = { title: "尚未结束的提交", spec: "执行说明", requestedTemplate: "writer", includeActiveContext: true, evidenceIds: ["e1"] };
    const drafts = new AgentTaskDrafts([[mission.id, saved]]);
    const subscribers = new Set<() => void>();
    const subscribe = drafts.subscribe.bind(drafts);
    vi.spyOn(drafts, "subscribe").mockImplementation((listener) => {
      subscribers.add(listener);
      const unsubscribe = subscribe(listener);
      return () => { subscribers.delete(listener); unsubscribe(); };
    });
    const bridge = fixture();
    const first = render(<StrictMode><AgentHandoffPanel {...props(bridge)} drafts={drafts} /></StrictMode>);
    expect(subscribers.size).toBe(1);
    first.unmount();
    expect(subscribers.size).toBe(0);
    const returned = render(<StrictMode><AgentHandoffPanel {...props(bridge)} drafts={drafts} /></StrictMode>);
    expect(screen.getByLabelText("任务标题")).toHaveValue(saved.title);
    fireEvent.click(screen.getByRole("button", { name: "预览交接内容" }));
    await screen.findByRole("region", { name: "交接预览" });
    const readCount = vi.mocked(bridge.listAgentEvidencePage).mock.calls.length;
    act(() => { drafts.set("another-mission", saved); drafts.delete("another-mission"); });
    expect(screen.getByLabelText("任务标题")).toHaveValue(saved.title);
    act(() => { drafts.delete(mission.id); });
    expect(screen.getByLabelText("任务标题")).toHaveValue("");
    expect(screen.getByLabelText("任务说明")).toHaveValue("");
    expect(screen.getByLabelText("Agent 模板")).toHaveValue("");
    expect(screen.getByLabelText("包含当前图谱路径")).not.toBeChecked();
    expect(screen.queryByRole("region", { name: "交接预览" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /取消证据/ })).not.toBeInTheDocument();
    expect(bridge.listAgentEvidencePage).toHaveBeenCalledTimes(readCount);
    returned.unmount();
    expect(subscribers.size).toBe(0);
  });

});
