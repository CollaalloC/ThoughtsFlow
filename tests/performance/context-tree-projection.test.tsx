import { fireEvent, render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import {
  ContextTree,
  type ContextTreeProjectionView,
} from "../../src/features/context-tree";

const runCount = 1_000;

function fixedProjection(): ContextTreeProjectionView {
  return {
    workspaceId: "context-tree-performance-workspace",
    rootId: "workspace-root:context-tree-performance-workspace",
    draftVersion: 0,
    cursor: {
      workspaceId: "context-tree-performance-workspace",
      activeRunId: `run-${runCount - 1}`,
      branchId: "branch-main",
      version: 1,
      updatedAt: "2026-07-28T00:00:00Z",
    },
    nodes: Array.from({ length: runCount }, (_, index) => ({
      runId: `run-${index}`,
      turnId: `turn-${index}`,
      parentRunId: index === 0 ? null : `run-${index - 1}`,
      prompt: `上下文性能问题 ${index + 1}`,
      title: `性能节点 ${index + 1}`,
      outputPreview: `精确 Run ${index + 1} 的稳定输出摘要。`,
      model: "fixture-model",
      status: "completed",
      createdAt: "2026-07-28T00:00:00Z",
      canContinue: true,
      isActive: index === runCount - 1,
      isOnActivePath: true,
      branchIds: index === runCount - 1 ? ["branch-main"] : [],
      checkpointIds: [],
    })),
    edges: Array.from({ length: runCount }, (_, index) => ({
      id: `edge-${index}`,
      sourceRunId: index === 0 ? null : `run-${index - 1}`,
      targetRunId: `run-${index}`,
      isOnActivePath: true,
    })),
    branches: [
      {
        id: "branch-main",
        name: "性能基线",
        headRunId: `run-${runCount - 1}`,
        version: 1,
        isActive: true,
      },
    ],
    checkpoints: [],
  };
}

test(
  "a 1,000-Run Context Tree opens and keeps exact leaf selection under five seconds",
  () => {
    const onSelect = vi.fn();
    const startedAt = performance.now();
    const rendered = render(<ContextTree onSelect={onSelect} projection={fixedProjection()} />);

    const exactLeaf = rendered.container.querySelector<HTMLButtonElement>(
      'button[role="treeitem"][aria-label*="run-999"]',
    );
    expect(exactLeaf).not.toBeNull();
    fireEvent.click(exactLeaf!);
    expect(onSelect).toHaveBeenCalledWith("run-999", "branch-main");
    expect(performance.now() - startedAt).toBeLessThan(5_000);
  },
  10_000,
);
