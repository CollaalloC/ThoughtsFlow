import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ContextInspector, type LockedSnapshot } from "./ContextInspector";
import type { ContextCheckpointView, ContextItem } from "../../shared/contracts";

const snapshot: LockedSnapshot = {
  canonicalHash: "sha256:receipt",
  createdAt: "2026-07-22T09:11:00Z",
  providerMetadataStatus: "resolved",
  providerId: "anthropic",
  templateRevision: 1,
  providerName: "Anthropic",
  streamProtocol: "anthropic_sse",
  authPlacement: "api_key_header",
  authHeaderName: "x-api-key",
  additionalHeaders: {
    "anthropic-version": "2023-06-01",
    Authorization: "must-not-render",
  },
  model: "claude-sonnet",
  baseUrl: "https://api.anthropic.com",
  parameters: {
    temperature: 0.1,
    stop: ["END"],
  },
  items: [],
};

describe("ContextInspector locked receipt", () => {
  it("renders resolved protocol, auth, non-secret headers, and effective parameters", () => {
    render(
      <ContextInspector
        open
        items={[]}
        estimatedTokens={0}
        limitTokens={8192}
        warnings={[]}
        blocked={false}
        provider={{
          name: "Anthropic",
          model: "claude-sonnet",
          baseUrl: "https://api.anthropic.com",
          local: false,
        }}
        snapshot={snapshot}
        runs={[]}
        onClose={vi.fn()}
        onToggleIncluded={vi.fn()}
        onTogglePinned={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));

    expect(screen.getByText("anthropic r1")).toBeVisible();
    expect(screen.getByText("Anthropic SSE")).toBeVisible();
    expect(screen.getByText("x-api-key: API Key …")).toBeVisible();
    expect(screen.getByText("anthropic-version")).toBeVisible();
    expect(screen.getByText("2023-06-01")).toBeVisible();
    expect(screen.queryByText("must-not-render")).not.toBeInTheDocument();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.1")).toBeVisible();
    expect(screen.getByText('["END"]')).toBeVisible();
  });

  it("separates the raw path from effective next-send Context and protects mandatory items", () => {
    const systemItem: ContextItem = {
      id: "item-system",
      sourceRef: { kind: "workspace-system", id: "workspace-1" },
      contentBlockId: "block-system",
      contentHash: "sha256:system",
      ordinal: 1,
      role: "system",
      label: "系统说明",
      source: "工作区默认",
      content: "不可排除的系统约束",
      reason: "工作区系统说明",
      estimatedTokens: 12,
      included: true,
      pinned: false,
      mandatory: true,
    };
    const omittedRawItem: ContextItem = {
      id: "item-old-answer",
      sourceRef: { kind: "model-run", id: "run-old" },
      contentBlockId: "block-old",
      contentHash: "sha256:old",
      ordinal: 2,
      role: "assistant",
      label: "旧回答",
      source: "run-old",
      content: "仍可检查但不进入下一轮",
      reason: "已被压缩检查点覆盖",
      estimatedTokens: 20,
      included: false,
      pinned: false,
      mandatory: false,
    };
    const excludedCheckpointItem: ContextItem = {
      id: "checkpoint-summary:checkpoint-old",
      sourceRef: { kind: "checkpoint-summary", id: "checkpoint-old" },
      contentBlockId: "block-system-checkpoint-old",
      contentHash: "sha256:checkpoint-old",
      ordinal: 3,
      role: "system",
      label: "旧压缩摘要",
      source: "checkpoint-old",
      content: "被明确排除、但仍可重新纳入的不可变摘要",
      reason: "下一轮明确排除",
      estimatedTokens: 10,
      included: false,
      pinned: false,
      mandatory: false,
    };
    const checkpoint: ContextCheckpointView = {
      id: "checkpoint-1",
      workspaceId: "workspace-1",
      branchId: "branch-main",
      branchVersion: 2,
      kind: "compaction",
      anchorRunId: "run-current",
      sourceRunIds: ["run-old"],
      sourceHash: "sha256:source-range",
      firstKeptRunId: "run-current",
      summary: "旧路线已压缩为可审计摘要",
      provider: null,
      status: "completed",
      createdAt: "2026-07-28T10:00:00Z",
    };

    const onToggleIncluded = vi.fn();
    render(
      <ContextInspector
        open
        items={[systemItem]}
        rawItems={[systemItem, omittedRawItem, excludedCheckpointItem]}
        draftVersion={7}
        appliedCheckpoint={checkpoint}
        estimatedTokens={12}
        limitTokens={8192}
        warnings={[]}
        blocked={false}
        provider={{
          name: "Local Provider",
          model: "qwen3",
          baseUrl: "http://127.0.0.1:11434",
          local: true,
        }}
        snapshot={null}
        runs={[]}
        onClose={vi.fn()}
        onToggleIncluded={onToggleIncluded}
        onTogglePinned={vi.fn()}
      />,
    );

    expect(screen.getByText("草稿版本 v7")).toBeVisible();
    expect(screen.getByText("旧路线已压缩为可审计摘要")).toBeVisible();
    expect(screen.getByText("sha256:source-range")).toBeVisible();
    expect(screen.getByText("人工摘要")).toBeVisible();
    expect(screen.getByText("不可排除的系统约束")).toBeVisible();
    expect(screen.getByRole("button", { name: "排除 系统说明" })).toBeDisabled();

    fireEvent.click(screen.getByRole("tab", { name: "原始路径" }));
    expect(screen.getByText("仍可检查但不进入下一轮")).toBeVisible();
    expect(screen.getByText(
      "原始内容只读；检查点不会删除历史内容，下方操作只调整下一轮 Context 草稿。",
    )).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重新纳入 旧压缩摘要" }));
    expect(onToggleIncluded).toHaveBeenCalledWith(excludedCheckpointItem);
  });
});
