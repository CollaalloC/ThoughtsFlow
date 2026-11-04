import { render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import type { AgentEvidenceSummary, AgentHandoffSummary } from "../../src/shared/contracts";
import { AgentWorkflowGraph } from "../../src/features/agents/AgentWorkflowGraph";

// Check projection work and memoization, not ReactFlow/browser rendering speed.
const { flow } = vi.hoisted(() => ({ flow: vi.fn((_props: { nodes: Node[]; edges: Edge[] }) => null) }));
vi.mock("@xyflow/react", () => ({
  ReactFlow: flow, Background: () => null, Controls: () => null,
  MarkerType: { ArrowClosed: "arrowclosed" },
}));

test("a 50-handoff page deduplicates shared sources and keeps stable directed edges across label changes", () => {
  const handoffs: AgentHandoffSummary[] = Array.from({ length: 50 }, (_, index) => ({
    operationId: `h${index}`, workspaceId: "w", missionId: "m", createdAt: "2026-10-06",
    title: `task ${index}`, hash: `hash${index}`, sourceRunIds: ["shared-context", "second-context"], evidenceIds: ["saved-evidence"],
  }));
  const evidence: AgentEvidenceSummary[] = [];
  const runTitles = { "shared-context": "Shared source" };
  const view = (title: string, labels = runTitles) => <>
    <span>{title}</span><AgentWorkflowGraph handoffs={handoffs} evidence={evidence} runTitles={labels} />
  </>;
  const rendered = render(view("draft"));
  expect(flow).toHaveBeenCalledTimes(1);
  const first = flow.mock.calls[0][0];
  expect(first.nodes).toHaveLength(53);
  expect(new Set(first.nodes.map((node) => node.id)).size).toBe(53);
  expect(first.edges).toHaveLength(150);
  const edgeIds = first.edges.map((edge) => edge.id);
  expect(new Set(edgeIds).size).toBe(150);
  expect(first.edges.every((edge) => edge.markerEnd && typeof edge.markerEnd !== "string" && edge.markerEnd.type === "arrowclosed")).toBe(true);
  rendered.rerender(view("updated draft"));
  expect(flow).toHaveBeenCalledTimes(1);
  rendered.rerender(view("updated draft", { "shared-context": "Renamed source" }));
  expect(flow).toHaveBeenCalledTimes(2);
  const renamed = flow.mock.calls[1][0];
  expect(renamed.edges.map((edge) => edge.id)).toEqual(edgeIds);
  expect(renamed.nodes.find((node) => node.id === "context:shared-context")?.data.label).toContain("Renamed source");
});
