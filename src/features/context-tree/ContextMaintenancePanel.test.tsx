import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type {
  ContextTreeRunNode,
  ProviderProfile,
} from "../../shared/contracts";
import { ContextMaintenancePanel } from "./ContextMaintenancePanel";

const nodes: ContextTreeRunNode[] = [
  {
    runId: "run-root",
    turnId: "turn-root",
    parentRunId: null,
    prompt: "建立事实基线",
    title: "事实基线",
    outputPreview: "已确认事实",
    model: "qwen3",
    status: "completed",
    createdAt: "2026-07-28T09:00:00Z",
    canContinue: true,
    isActive: false,
    isOnActivePath: true,
    branchIds: ["branch-main"],
    checkpointIds: [],
  },
  {
    runId: "run-middle",
    turnId: "turn-middle",
    parentRunId: "run-root",
    prompt: "比较方案",
    title: "方案比较",
    outputPreview: "选择本地优先",
    model: "qwen3",
    status: "completed",
    createdAt: "2026-07-28T09:10:00Z",
    canContinue: true,
    isActive: false,
    isOnActivePath: true,
    branchIds: ["branch-main"],
    checkpointIds: [],
  },
  {
    runId: "run-leaf",
    turnId: "turn-leaf",
    parentRunId: "run-middle",
    prompt: "确认恢复语义",
    title: "恢复语义",
    outputPreview: "保持精确活动叶",
    model: "qwen3",
    status: "completed",
    createdAt: "2026-07-28T09:20:00Z",
    canContinue: true,
    isActive: true,
    isOnActivePath: true,
    branchIds: ["branch-main"],
    checkpointIds: [],
  },
];

const profiles: ProviderProfile[] = [
  {
    id: "provider-local",
    providerId: "ollama",
    name: "Local Provider",
    dialect: "ollama",
    baseUrl: "http://127.0.0.1:11434",
    model: "qwen3",
    isDefault: true,
  },
  {
    id: "provider-cloud",
    providerId: "openai-compatible",
    name: "Cloud Provider",
    dialect: "openai-compatible",
    baseUrl: "https://api.example.com/v1",
    model: "gpt-4.1",
    isDefault: false,
  },
];

describe("ContextMaintenancePanel", () => {
  it("previews and submits a manual compaction without requiring a Provider", async () => {
    const user = userEvent.setup();
    const onCreateManual = vi.fn();

    render(
      <ContextMaintenancePanel
        estimatedTokens={4800}
        nodes={nodes}
        onCancel={vi.fn()}
        onCreateManual={onCreateManual}
        onSummarize={vi.fn()}
        profiles={[]}
      />,
    );

    expect(screen.getByText("2 个来源 Run")).toBeVisible();
    expect(screen.getByText("从 run-leaf 开始保留原文")).toBeVisible();
    expect(screen.getByText("预计预算 4,800 tokens")).toBeVisible();
    expect(
      within(screen.getByRole("region", { name: "压缩影响预览" }))
        .getByText("人工摘要"),
    ).toBeVisible();
    expect(
      screen.queryByRole("combobox", { name: "摘要 Provider" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("sha256 将由后端对精确来源清单计算")).toBeVisible();

    await user.type(screen.getByRole("textbox", { name: "人工摘要" }), "此前已确认本地优先与恢复语义。");
    await user.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));

    expect(onCreateManual).toHaveBeenCalledWith({
      kind: "compaction",
      sourceRunIds: ["run-root", "run-middle"],
      firstKeptRunId: "run-leaf",
      summary: "此前已确认本地优先与恢复语义。",
    });
  });

  it("never summarizes silently and sends an explicit prompt only after confirmation", async () => {
    const user = userEvent.setup();
    const onSummarize = vi.fn();

    render(
      <ContextMaintenancePanel
        estimatedTokens={4800}
        nodes={nodes}
        onCancel={vi.fn()}
        onCreateManual={vi.fn()}
        onSummarize={onSummarize}
        profiles={profiles}
      />,
    );

    expect(onSummarize).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Provider 生成摘要" }));
    expect(screen.getByRole("combobox", { name: "摘要 Provider" })).toBeVisible();
    await user.selectOptions(screen.getByRole("combobox", { name: "摘要 Provider" }), "provider-cloud");
    const prompt = screen.getByRole("textbox", { name: "摘要请求" });
    await user.clear(prompt);
    await user.type(prompt, "只总结已验证事实和未决风险。");
    await user.click(screen.getByRole("button", { name: "确认生成并切换 Context" }));

    expect(onSummarize).toHaveBeenCalledWith({
      sourceRunIds: ["run-root", "run-middle"],
      firstKeptRunId: "run-leaf",
      providerProfileId: "provider-cloud",
      summaryPrompt: "只总结已验证事实和未决风险。",
    });
  });

  it("prevents checkpoint sources from overlapping or crossing the retained boundary", async () => {
    const user = userEvent.setup();
    const onCreateManual = vi.fn();

    render(
      <ContextMaintenancePanel
        estimatedTokens={4800}
        nodes={nodes}
        onCancel={vi.fn()}
        onCreateManual={onCreateManual}
        onSummarize={vi.fn()}
        profiles={profiles}
      />,
    );

    await user.selectOptions(
      screen.getByRole("combobox", { name: "保留原文边界" }),
      "run-middle",
    );

    expect(screen.getByRole("checkbox", { name: /事实基线/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /方案比较/ })).toBeDisabled();
    expect(screen.getByRole("checkbox", { name: /恢复语义/ })).toBeDisabled();
    expect(screen.getByText("1 个来源 Run")).toBeVisible();

    await user.type(
      screen.getByRole("textbox", { name: "人工摘要" }),
      "只总结边界之前的事实。",
    );
    await user.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));

    expect(onCreateManual).toHaveBeenCalledWith({
      kind: "compaction",
      sourceRunIds: ["run-root"],
      firstKeptRunId: "run-middle",
      summary: "只总结边界之前的事实。",
    });
  });

  it("keeps compaction sources as an exact root-to-boundary prefix", async () => {
    const user = userEvent.setup();
    const onCreateManual = vi.fn();

    render(
      <ContextMaintenancePanel
        estimatedTokens={4800}
        nodes={nodes}
        onCancel={vi.fn()}
        onCreateManual={onCreateManual}
        onSummarize={vi.fn()}
        profiles={profiles}
      />,
    );

    const boundary = screen.getByRole("combobox", { name: "保留原文边界" });
    expect(
      within(boundary).queryByRole("option", { name: "不保留原文尾部" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: /事实基线/ }));
    expect(screen.getByRole("checkbox", { name: /事实基线/ })).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: /方案比较/ })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "保存人工压缩检查点" })).toBeDisabled();

    await user.selectOptions(boundary, "run-leaf");
    expect(screen.getByRole("checkbox", { name: /事实基线/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /方案比较/ })).toBeChecked();
    expect(boundary).toHaveValue("run-leaf");

    await user.type(
      screen.getByRole("textbox", { name: "人工摘要" }),
      "只允许连续来源前缀。",
    );
    await user.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));

    expect(onCreateManual).toHaveBeenCalledWith({
      kind: "compaction",
      sourceRunIds: ["run-root", "run-middle"],
      firstKeptRunId: "run-leaf",
      summary: "只允许连续来源前缀。",
    });
  });

  it("omits the retained boundary for branch summaries", async () => {
    const user = userEvent.setup();
    const onCreateManual = vi.fn();

    render(
      <ContextMaintenancePanel
        estimatedTokens={4800}
        nodes={nodes}
        onCancel={vi.fn()}
        onCreateManual={onCreateManual}
        onSummarize={vi.fn()}
        profiles={profiles}
      />,
    );

    await user.selectOptions(
      screen.getByRole("combobox", { name: "维护类型" }),
      "branch-summary",
    );
    expect(
      screen.queryByRole("combobox", { name: "保留原文边界" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("分支摘要不使用保留边界")).toBeVisible();

    await user.click(screen.getByRole("checkbox", { name: /方案比较/ }));
    await user.type(
      screen.getByRole("textbox", { name: "人工摘要" }),
      "只总结连续分支来源。",
    );
    await user.click(screen.getByRole("button", { name: "保存人工压缩检查点" }));

    expect(onCreateManual).toHaveBeenCalledWith({
      kind: "branch-summary",
      sourceRunIds: ["run-root"],
      firstKeptRunId: null,
      summary: "只总结连续分支来源。",
    });
  });
});
