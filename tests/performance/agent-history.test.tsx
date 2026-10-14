import { appendFileSync } from "node:fs";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { AgentHandoffPanel } from "../../src/features/agents/AgentHandoffPanel";
import { createDesktopBridge, type DesktopBridge } from "../../src/platform/desktop-bridge";
import type { AgentEvidenceSummary, AgentHandoffSummary, AgentMission, ContextTreeProjection } from "../../src/shared/contracts";

// Isolate history and controlled-input work, as in the recorded before fixture.
// This measures React/jsdom, not ReactFlow, browser layout, IPC or model latency.
vi.mock("../../src/features/agents/AgentWorkflowGraph", () => ({ AgentWorkflowGraph: () => null }));
vi.mock("../../src/features/context-tree", () => ({ ContextTree: () => null }));

const mission: AgentMission = {
  id: "m", workspaceId: "w", repositoryId: "r", repositoryPath: "/fixture",
  objective: "history benchmark", runId: "run", coordinatorHandle: "coordinator",
  runtimeId: "runtime", status: "ready", error: null, createdAt: "2026-10-06T00:00:00Z",
};
const tree: ContextTreeProjection = {
  rootId: "root", workspaceId: "w", draftVersion: 0, nodes: [], edges: [], branches: [], checkpoints: [],
  cursor: { workspaceId: "w", activeRunId: null, branchId: null, version: 0, updatedAt: mission.createdAt },
};
const median = (samples: number[]) => [...samples].sort((a, b) => a - b)[Math.floor(samples.length / 2)];

function fixture(count: number) {
  // Only the first page crosses the bridge. Full output bodies are deliberately
  // absent: loading them into the page mock would conceal a boundary regression.
  const evidence: AgentEvidenceSummary[] = Array.from({ length: Math.min(count, 50) }, (_, offset) => {
    const index = count - offset - 1;
    return {
      id: `e${index}`, workspaceId: "w", missionId: "m", runId: "run", taskId: `t${index}`,
      dispatchId: `d${index}`, handoffOperationId: `h${index}`, title: `task ${index}`,
      contentHash: "test-hash", hasMore: true, warning: null, source: "terminal",
      requestCursor: null, cursor: null, observedTaskStatus: "running", createdAt: mission.createdAt,
    };
  });
  const handoffs: AgentHandoffSummary[] = evidence.map((item) => ({
    operationId: item.handoffOperationId!, workspaceId: "w", missionId: "m", createdAt: item.createdAt,
    title: item.title!, hash: "test-hash", sourceRunIds: [], evidenceIds: [],
  }));
  const contextRead = vi.fn().mockResolvedValue(tree);
  const evidenceRead = vi.fn().mockResolvedValue({ items: evidence, nextCursor: count > 50 ? "next-evidence" : null });
  const handoffRead = vi.fn().mockResolvedValue({ items: handoffs, nextCursor: count > 50 ? "next-handoff" : null });
  const detailRead = vi.fn().mockRejectedValue(new Error("Closed history must not load details"));
  const bridge: DesktopBridge = {
    ...createDesktopBridge(async () => { throw new Error("Unexpected IPC in history benchmark"); }),
    getContextTree: contextRead, listAgentEvidencePage: evidenceRead, listAgentHandoffPage: handoffRead,
    getAgentEvidence: detailRead, getAgentHandoff: detailRead,
  };
  return { bridge, contextRead, evidenceRead, handoffRead, detailRead };
}

test.each([0, 100, 500, 1_000])("history with %i records keeps its initial page and input work bounded", async (count) => {
  const mounts: number[] = [];
  const edits: number[] = [];
  let elementCount = 0;
  for (let round = 0; round < 9; round += 1) {
    const { bridge, contextRead, evidenceRead, handoffRead, detailRead } = fixture(count);
    const started = performance.now();
    await act(async () => {
      render(<AgentHandoffPanel bridge={bridge} mission={mission} busy={false} canStart evidenceRevision={0} onStart={async () => undefined} />);
    });
    const mountMs = performance.now() - started;
    const title = document.querySelector<HTMLInputElement>('input[aria-label="任务标题"]');
    expect(title).not.toBeNull();
    const editing = performance.now();
    fireEvent.change(title!, { target: { value: "next title" } });
    const editMs = performance.now() - editing;
    expect(title?.value).toBe("next title");
    expect(document.querySelectorAll("details:not([open])")).toHaveLength(Math.min(count, 50) * 2);
    expect(document.querySelectorAll("details pre")).toHaveLength(0);
    expect(document.querySelectorAll("input[data-evidence-id]")).toHaveLength(Math.min(count, 50));
    expect(contextRead).toHaveBeenCalledTimes(1);
    expect(evidenceRead).toHaveBeenCalledTimes(1);
    expect(handoffRead).toHaveBeenCalledTimes(1);
    expect(detailRead).not.toHaveBeenCalled();
    elementCount = document.body.querySelectorAll("*").length;
    if (round >= 2) { mounts.push(mountMs); edits.push(editMs); }
    cleanup();
  }
  const record = {
    benchmark: "AgentHandoffPanel-jsdom-history", count, graph: "mocked", contextTree: "mocked",
    warmups: 2, rounds: 7, mountMedianMs: median(mounts), titleEditMedianMs: median(edits),
    mountSamples: mounts, titleEditSamples: edits,
    dom: { closedDetails: Math.min(count, 50) * 2, preNodes: 0, hiddenTextChars: 0, elements: elementCount },
    rpcPerMount: { context: 1, evidencePage: 1, handoffPage: 1, details: 0 }, rpcPerTitleEdit: 0,
  };
  // Optional JSONL recording; normal tests assert work bounds rather than timing.
  // THOUGHTSFLOW_BENCH_OUTPUT=/tmp/agent-history.jsonl npm run test:run -- tests/performance/agent-history.test.tsx
  if (process.env.THOUGHTSFLOW_BENCH_OUTPUT) appendFileSync(process.env.THOUGHTSFLOW_BENCH_OUTPUT, `${JSON.stringify(record)}\n`);
}, 30_000);
