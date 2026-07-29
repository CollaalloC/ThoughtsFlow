import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  ContextTree,
  type ContextTreeProjectionView,
} from "./ContextTree";

const projection: ContextTreeProjectionView = {
  workspaceId: "workspace-1",
  rootId: "workspace-root:workspace-1",
  draftVersion: 3,
  cursor: {
    workspaceId: "workspace-1",
    activeRunId: "run-child",
    branchId: "branch-main",
    version: 4,
    updatedAt: "2026-07-28T11:00:00Z",
  },
  nodes: [
    {
      runId: "run-root-a",
      turnId: "turn-root",
      parentRunId: null,
      prompt: "先确认产品基线",
      title: "产品基线",
      outputPreview: "上下文必须精确可审计。",
      model: "gpt-4.1",
      status: "completed",
      canContinue: true,
      isOnActivePath: false,
      isActive: false,
      branchIds: ["branch-alt"],
      checkpointIds: [],
      createdAt: "2026-07-28T10:00:00Z",
    },
    {
      runId: "run-root-b",
      turnId: "turn-root",
      parentRunId: null,
      prompt: "先确认产品基线",
      title: "产品基线",
      outputPreview: "先做持久化活动叶。",
      model: "qwen3",
      status: "completed",
      canContinue: true,
      isOnActivePath: true,
      isActive: false,
      branchIds: ["branch-main"],
      checkpointIds: [],
      createdAt: "2026-07-28T10:01:00Z",
    },
    {
      runId: "run-child",
      turnId: "turn-child",
      parentRunId: "run-root-b",
      prompt: "如何恢复上下文？",
      title: "恢复上下文",
      outputPreview: "从精确活动叶恢复。",
      model: "qwen3",
      status: "interrupted",
      canContinue: false,
      isOnActivePath: true,
      isActive: true,
      branchIds: ["branch-main"],
      checkpointIds: ["checkpoint-1"],
      createdAt: "2026-07-28T10:02:00Z",
    },
  ],
  edges: [
    {
      id: "edge-root-a",
      sourceRunId: null,
      targetRunId: "run-root-a",
      isOnActivePath: false,
    },
    {
      id: "edge-root-b",
      sourceRunId: null,
      targetRunId: "run-root-b",
      isOnActivePath: true,
    },
    {
      id: "edge-child",
      sourceRunId: "run-root-b",
      targetRunId: "run-child",
      isOnActivePath: true,
    },
  ],
  branches: [
    {
      id: "branch-alt",
      name: "备选答案",
      headRunId: "run-root-a",
      version: 1,
      isActive: false,
    },
    {
      id: "branch-main",
      name: "主路线",
      headRunId: "run-child",
      version: 2,
      isActive: true,
    },
  ],
  checkpoints: [
    {
      id: "checkpoint-1",
      workspaceId: "workspace-1",
      branchId: "branch-main",
      branchVersion: 2,
      kind: "compaction",
      anchorRunId: "run-child",
      sourceRunIds: ["run-root-b"],
      sourceHash: "sha256:sources",
      firstKeptRunId: "run-child",
      summary: "保留最近 1 轮",
      provider: null,
      status: "completed",
      createdAt: "2026-07-28T10:03:00Z",
    },
  ],
};

describe("ContextTree", () => {
  it("keeps exact Model Runs separate and selects the virtual root or an exact Run", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();

    render(<ContextTree projection={projection} onSelect={onSelect} />);

    expect(screen.getByRole("treeitem", { name: /工作区起点/ })).toBeVisible();
    expect(screen.getByRole("treeitem", { name: /run-root-b/ })).toBeVisible();
    expect(screen.queryByRole("treeitem", { name: /run-root-a/ })).not.toBeInTheDocument();
    expect(screen.getByText("CURRENT CONTEXT")).toBeVisible();

    await user.click(screen.getByRole("treeitem", { name: /工作区起点/ }));
    expect(onSelect).toHaveBeenLastCalledWith(null, null);

    await user.click(screen.getByRole("treeitem", { name: /run-child/ }));
    expect(onSelect).toHaveBeenLastCalledWith("run-child", "branch-main");
  });

  it("filters the complete tree with case-insensitive AND terms and shows checkpoint provenance", async () => {
    const user = userEvent.setup();

    render(<ContextTree projection={projection} onSelect={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "全部节点" }));
    expect(screen.getByRole("treeitem", { name: /run-root-a/ })).toBeVisible();

    await user.type(screen.getByRole("searchbox", { name: "搜索 Context Tree" }), "GPT 审计");
    expect(screen.getByRole("treeitem", { name: /run-root-a/ })).toBeVisible();
    expect(screen.queryByRole("treeitem", { name: /run-root-b/ })).not.toBeInTheDocument();

    await user.clear(screen.getByRole("searchbox", { name: "搜索 Context Tree" }));
    expect(screen.getByText("压缩检查点 · 保留最近 1 轮")).toBeVisible();
    expect(screen.getByText("已中断")).toBeVisible();
    expect(screen.getByText("不可继续")).toBeVisible();
  });

  it("renames the branch through an explicit inline edit", async () => {
    const user = userEvent.setup();
    const onRenameBranch = vi.fn();

    render(
      <ContextTree
        projection={projection}
        onRenameBranch={onRenameBranch}
        onSelect={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "重命名分支 主路线" }));
    const input = screen.getByRole("textbox", { name: "分支名称" });
    await user.clear(input);
    await user.type(input, "恢复路线");
    await user.click(screen.getByRole("button", { name: "保存分支名称" }));

    expect(onRenameBranch).toHaveBeenCalledWith("branch-main", "恢复路线", 2);
  });

  it("provides roving tree focus and keyboard navigation without changing selection", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();

    render(<ContextTree projection={projection} onSelect={onSelect} />);

    const root = screen.getByRole("treeitem", { name: /工作区起点/ });
    const parent = screen.getByRole("treeitem", { name: /run-root-b/ });
    const child = screen.getByRole("treeitem", { name: /run-child/ });

    expect(child).toHaveAttribute("tabindex", "0");
    expect(root).toHaveAttribute("tabindex", "-1");
    expect(parent).toHaveAttribute("tabindex", "-1");

    child.focus();
    await user.keyboard("{ArrowUp}");
    expect(parent).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(root).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(parent).toHaveFocus();
    await user.keyboard("{End}");
    expect(child).toHaveFocus();
    await user.keyboard("{Home}");
    expect(root).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(parent).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(child).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(parent).toHaveFocus();

    expect(onSelect).not.toHaveBeenCalled();
    expect(parent).toHaveAttribute("tabindex", "0");
    expect(root).toHaveAttribute("tabindex", "-1");
    expect(child).toHaveAttribute("tabindex", "-1");
  });
});
