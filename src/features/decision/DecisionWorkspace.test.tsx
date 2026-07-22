import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  DecisionWorkspace,
  type CompareRunsResult,
  type DecisionRunOption,
} from "./DecisionWorkspace";

const runs: DecisionRunOption[] = [
  {
    runId: "run-a",
    label: "路线 A · 保守迁移",
    model: "local-model",
    status: "completed",
  },
  {
    runId: "run-b",
    label: "路线 B · 一次重构",
    model: "cloud-model",
    status: "completed",
  },
];

const comparison: CompareRunsResult = {
  left: { runId: "run-a", model: "local-model", status: "completed" },
  right: { runId: "run-b", model: "cloud-model", status: "completed" },
  answer: {
    leftMarkdown: "先双写并观察，再逐步切流。",
    rightMarkdown: "冻结需求后一次完成替换。",
  },
  contextDiff: {
    onlyLeft: [
      {
        id: "ctx-slo",
        ordinal: 2,
        role: "user",
        source: "运行约束",
        preview: "迁移期间错误预算不得超过 0.1%",
      },
    ],
    onlyRight: [
      {
        id: "ctx-deadline",
        ordinal: 2,
        role: "user",
        source: "交付约束",
        preview: "必须在本周内完成",
      },
    ],
    shared: [
      {
        id: "ctx-system",
        ordinal: 1,
        role: "system",
        source: "工作区默认",
        preview: "给出可审查的技术方案",
      },
    ],
  },
};

describe("DecisionWorkspace", () => {
  it("compares two runs and exposes answer and context differences", async () => {
    const user = userEvent.setup();
    const onCompare = vi.fn().mockResolvedValue(comparison);

    render(
      <DecisionWorkspace
        existingMarks={[]}
        onCompare={onCompare}
        onExport={vi.fn()}
        onMarkDecision={vi.fn()}
        runs={runs}
        workspaceId="workspace-1"
      />,
    );

    await user.selectOptions(screen.getByLabelText("路线 A"), "run-a");
    await user.selectOptions(screen.getByLabelText("路线 B"), "run-b");
    await user.click(screen.getByRole("button", { name: "比较两条路线" }));

    expect(onCompare).toHaveBeenCalledWith({ leftRunId: "run-a", rightRunId: "run-b" });
    const answerDiff = screen.getByRole("region", { name: "回答差异" });
    expect(within(answerDiff).getByText("先双写并观察，再逐步切流。")).toBeVisible();
    expect(within(answerDiff).getByText("冻结需求后一次完成替换。")).toBeVisible();

    const contextDiff = screen.getByRole("region", { name: "Context Diff" });
    expect(within(contextDiff).getByText("迁移期间错误预算不得超过 0.1%")).toBeVisible();
    expect(within(contextDiff).getByText("必须在本周内完成")).toBeVisible();
    expect(within(contextDiff).getByText("给出可审查的技术方案")).toBeVisible();
  });

  it("persists a decision status with its review reason", async () => {
    const user = userEvent.setup();
    const onMarkDecision = vi.fn().mockResolvedValue({
      id: "mark-a",
      workspaceId: "workspace-1",
      runId: "run-a",
      status: "accepted",
      reason: "迁移风险更低，且保留回滚窗口。",
      createdAt: "2026-07-22T10:00:00Z",
    });

    render(
      <DecisionWorkspace
        existingMarks={[]}
        onCompare={vi.fn().mockResolvedValue(comparison)}
        onExport={vi.fn()}
        onMarkDecision={onMarkDecision}
        runs={runs}
        workspaceId="workspace-1"
      />,
    );

    await user.click(screen.getByRole("button", { name: "比较两条路线" }));
    const editor = screen.getByRole("article", { name: "路线 A · 保守迁移的决策" });
    await user.click(within(editor).getByRole("button", { name: "采纳" }));
    await user.type(
      within(editor).getByRole("textbox", { name: "路线 A · 保守迁移的判断理由" }),
      "迁移风险更低，且保留回滚窗口。",
    );
    await user.click(within(editor).getByRole("button", { name: "保存决策标记" }));

    expect(onMarkDecision).toHaveBeenCalledWith({
      workspaceId: "workspace-1",
      runId: "run-a",
      status: "accepted",
      reason: "迁移风险更低，且保留回滚窗口。",
    });
    expect(await within(editor).findByText("已保存")).toBeVisible();
  });

  it("exports the workspace decision packet after a comparison", async () => {
    const user = userEvent.setup();
    const onExport = vi.fn().mockResolvedValue({
      path: "/tmp/decision-packet.md",
      markdown: "# Architecture Decision Record",
      bytesWritten: 30,
    });

    render(
      <DecisionWorkspace
        existingMarks={[]}
        onCompare={vi.fn().mockResolvedValue(comparison)}
        onExport={onExport}
        onMarkDecision={vi.fn()}
        runs={runs}
        workspaceId="workspace-1"
      />,
    );

    const exportButton = screen.getByRole("button", { name: "导出 Decision Packet" });
    expect(exportButton).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "比较两条路线" }));
    expect(exportButton).toBeEnabled();
    await user.click(exportButton);

    expect(onExport).toHaveBeenCalledWith({ workspaceId: "workspace-1" });
    expect(await screen.findByText("Decision Packet 已导出：/tmp/decision-packet.md")).toBeVisible();
  });
});
