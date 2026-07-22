import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { RouteMap, type RouteProjection } from "../../src/features/route-map";

const turnCount = 1_000;

function fixedProjection(): RouteProjection {
  return {
    workspaceId: "performance-workspace",
    nodes: Array.from({ length: turnCount }, (_, index) => ({
      id: `node-${index}`,
      turnId: `turn-${index}`,
      title: `问题 ${index + 1}`,
      summary: `固定性能样本中的第 ${index + 1} 个摘要节点`,
      status: "completed",
      x: (index % 20) * 300,
      y: Math.floor(index / 20) * 230,
      isCurrent: index === turnCount - 1,
      isOnCurrentLineage: true,
      runs: [
        {
          runId: `run-${index}`,
          label: "回答 1",
          model: "fixture-model",
          status: "completed",
          canBranch: true,
        },
      ],
    })),
    edges: Array.from({ length: turnCount - 1 }, (_, index) => ({
      id: `edge-${index}`,
      sourceRunId: `run-${index}`,
      targetTurnId: `turn-${index + 1}`,
      isOnCurrentLineage: true,
    })),
  };
}

test(
  "a fixed 1,000-turn projection opens and preserves exact run interaction",
  () => {
    const onSelectRun = vi.fn();
    const startedAt = performance.now();

    const rendered = render(
      <RouteMap
        onCreateBranch={vi.fn()}
        onSelectRun={onSelectRun}
        projection={fixedProjection()}
      />,
    );

    const openedInMs = performance.now() - startedAt;
    expect(openedInMs).toBeLessThan(5_000);
    expect(screen.getByRole("heading", { name: "当前路线 · 1000 个问题" })).toBeVisible();
    const exactRunButton = rendered.container.querySelector<HTMLButtonElement>(
      'button[data-run-id="run-999"]',
    );
    expect(exactRunButton).not.toBeNull();
    fireEvent.click(exactRunButton!);
    expect(onSelectRun).toHaveBeenCalledWith("run-999");
  },
  10_000,
);
