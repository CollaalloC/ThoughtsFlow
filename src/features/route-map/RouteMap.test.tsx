import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RouteMap, type RouteProjection } from "./RouteMap";

const projection: RouteProjection = {
  workspaceId: "workspace-1",
  nodes: [
    {
      id: "node-root",
      turnId: "turn-root",
      title: "根因基线",
      summary: "先确认可复现事实",
      status: "completed",
      x: 40,
      y: 80,
      isCurrent: false,
      isOnCurrentLineage: true,
      runs: [
        {
          runId: "run-root-a",
          label: "回答 A",
          model: "local-model",
          status: "completed",
          canBranch: true,
        },
        {
          runId: "run-root-b",
          label: "回答 B",
          model: "cloud-model",
          status: "completed",
          canBranch: true,
        },
      ],
    },
    {
      id: "node-child",
      turnId: "turn-child",
      title: "候选路线",
      summary: "验证第二个假设",
      status: "completed",
      x: 420,
      y: 80,
      isCurrent: true,
      isOnCurrentLineage: true,
      runs: [
        {
          runId: "run-child",
          label: "回答 1",
          model: "local-model",
          status: "completed",
          canBranch: true,
        },
      ],
    },
    {
      id: "node-sibling",
      turnId: "turn-sibling",
      title: "旁支",
      summary: "不会进入当前上下文",
      status: "completed",
      x: 420,
      y: 300,
      isCurrent: false,
      isOnCurrentLineage: false,
      runs: [],
    },
  ],
  edges: [
    {
      id: "edge-child",
      sourceRunId: "run-root-b",
      targetTurnId: "turn-child",
      isOnCurrentLineage: true,
    },
    {
      id: "edge-sibling",
      sourceRunId: "run-root-a",
      targetTurnId: "turn-sibling",
      isOnCurrentLineage: false,
    },
  ],
};

describe("RouteMap", () => {
  it("shows the current lineage and branches from the exact selected run", async () => {
    const user = userEvent.setup();
    const onSelectRun = vi.fn();
    const onCreateBranch = vi.fn();

    render(
      <RouteMap
        selectedRunIds={{
          "turn-root": "run-root-b",
          "turn-child": "run-child",
        }}
        projection={projection}
        onSelectRun={onSelectRun}
        onCreateBranch={onCreateBranch}
      />,
    );

    expect(screen.getByRole("heading", { name: "当前路线 · 2 个问题" })).toBeVisible();
    expect(screen.getByRole("article", { name: "候选路线" })).toHaveAttribute(
      "data-current",
      "true",
    );
    expect(screen.getByRole("article", { name: "旁支" })).toHaveAttribute(
      "data-lineage",
      "false",
    );
    expect(screen.getByRole("button", { name: "选择根因基线的回答 B" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "选择根因基线的回答 A" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("button", { name: "选择候选路线的回答 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(screen.getByRole("button", { name: "选择根因基线的回答 B" }));
    expect(onSelectRun).toHaveBeenCalledWith("run-root-b");

    await user.click(screen.getByRole("button", { name: "从根因基线的回答 B创建分支" }));
    expect(onCreateBranch).toHaveBeenCalledWith("run-root-b");
  });
});
