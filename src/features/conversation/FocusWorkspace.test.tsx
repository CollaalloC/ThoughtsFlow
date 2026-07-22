import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import { FocusWorkspace } from "./FocusWorkspace";

const workspace = {
  id: "workspace-1",
  name: "AI 分支对话产品定义",
  goal: "确定首版默认导航、上下文透明度与长期使用价值",
  systemPrompt: "你是一名严谨的技术决策协作者。",
  archived: false,
  createdAt: "2026-07-22T09:00:00Z",
  updatedAt: "2026-07-22T10:00:00Z",
};

const detail = {
  workspace,
  turns: [
    {
      id: "turn-1",
      workspaceId: workspace.id,
      parentRunId: null,
      prompt: "默认界面应该是线性阅读，还是无限画布？",
      createdAt: "2026-07-22T09:10:00Z",
      runs: [
        {
          id: "run-a",
          turnId: "turn-1",
          status: "completed",
          output: "线性阅读降低首分钟认知成本。",
          providerProfileId: "provider-cloud",
          providerName: "OpenAI compatible",
          model: "gpt-4.1",
          baseUrl: "https://api.example.com/v1",
          createdAt: "2026-07-22T09:11:00Z",
          completedAt: "2026-07-22T09:11:12Z",
        },
        {
          id: "run-b",
          turnId: "turn-1",
          status: "completed",
          output: "以专注阅读为主，路线图按需打开。",
          providerProfileId: "provider-local",
          providerName: "Ollama",
          model: "qwen3:14b",
          baseUrl: "http://127.0.0.1:11434",
          createdAt: "2026-07-22T09:12:00Z",
          completedAt: "2026-07-22T09:12:08Z",
        },
      ],
    },
  ],
  selectedRunIds: { "turn-1": "run-a" },
  adjacentBranches: [],
  decisionMarks: [],
};

const preview = {
  hash: "sha256:preview",
  estimatedTokens: 842,
  limitTokens: 8192,
  blocked: false,
  warnings: [],
  providerProfileId: "provider-cloud",
  providerName: "OpenAI compatible",
  model: "gpt-4.1",
  baseUrl: "https://api.example.com/v1",
  items: [
    {
      id: "context-system",
      ordinal: 1,
      role: "system" as const,
      label: "系统说明",
      source: "工作区默认",
      content: "你是一名严谨的 AI 产品设计顾问。",
      reason: "工作区系统说明",
      estimatedTokens: 12,
      included: true,
      pinned: false,
    },
    {
      id: "context-run-a",
      ordinal: 2,
      role: "assistant" as const,
      label: "回答 A",
      source: "run-a",
      content: "线性阅读降低首分钟认知成本。",
      reason: "精确祖先路径",
      estimatedTokens: 18,
      included: true,
      pinned: false,
    },
  ],
};

function bridgeFixture() {
  return {
    listWorkspaces: vi.fn().mockResolvedValue([workspace]),
    createWorkspace: vi.fn(),
    openWorkspace: vi.fn().mockResolvedValue(detail),
    updateWorkspace: vi.fn(),
    inspectContext: vi.fn().mockResolvedValue(preview),
    createTurnAndStartRun: vi
      .fn()
      .mockResolvedValue({ turnId: "turn-2", runId: "run-2" }),
    retryRun: vi.fn(),
    cancelRun: vi.fn(),
    getRunSnapshot: vi.fn(),
    updateContextOverrides: vi.fn().mockResolvedValue(undefined),
    getRouteProjection: vi.fn(),
    updateViewState: vi.fn(),
    compareRuns: vi.fn(),
    markDecision: vi.fn(),
    exportDecisionPacket: vi.fn(),
    listProviderProfiles: vi.fn().mockResolvedValue([
      {
        id: "provider-cloud",
        name: "OpenAI compatible",
        dialect: "openai-compatible",
        baseUrl: "https://api.example.com/v1",
        model: "gpt-4.1",
        isDefault: true,
      },
    ]),
    saveProviderProfile: vi.fn(),
    setSessionCredential: vi.fn(),
    testProviderConnection: vi.fn(),
    subscribeToRunEvents: vi.fn().mockReturnValue(() => undefined),
  } as unknown as DesktopBridge;
}

describe("FocusWorkspace", () => {
  it("labels a remote Ollama endpoint as outbound instead of local", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([
      {
        id: "provider-remote-ollama",
        providerId: "ollama",
        name: "Remote Ollama",
        dialect: "ollama",
        baseUrl: "https://ollama.example.com",
        model: "qwen3",
        isDefault: true,
        parameters: {},
      },
    ]);
    vi.mocked(bridge.inspectContext).mockResolvedValue({
      ...preview,
      providerProfileId: "provider-remote-ollama",
      providerName: "Remote Ollama",
      model: "qwen3",
      baseUrl: "https://ollama.example.com",
    });

    render(<FocusWorkspace bridge={bridge} />);

    const outboundLabels = await screen.findAllByText("外发 · ollama.example.com");
    expect(outboundLabels.length).toBeGreaterThan(0);
    expect(outboundLabels[0]).toBeVisible();
    expect(screen.queryByText("本机 · ollama.example.com")).not.toBeInTheDocument();
  });

  it("creates an untitled-goal workspace without turning placeholder copy into model context", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.createWorkspace).mockResolvedValue({
      ...workspace,
      id: "workspace-new",
      name: "新工作区",
      goal: "",
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "添加工作区" }));
    fireEvent.change(screen.getByRole("textbox", { name: "工作区名称" }), {
      target: { value: "新工作区" },
    });
    fireEvent.click(screen.getByRole("button", { name: "确认创建" }));

    await waitFor(() =>
      expect(bridge.createWorkspace).toHaveBeenCalledWith({ name: "新工作区", goal: "" }),
    );
  });

  it("buffers early stream events and surfaces an uncommitted persistence failure", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.createTurnAndStartRun).mockImplementation(async (_input, onEvent) => {
      onEvent({
        apiVersion: 1,
        type: "text-delta",
        runId: "run-early",
        text: "已先到达的部分输出",
        at: "2026-07-22T10:10:00Z",
      });
      onEvent({
        apiVersion: 1,
        type: "persistence-failed",
        runId: "run-early",
        error: { code: "storage_failure_uncommitted", message: "磁盘不可写" },
        at: "2026-07-22T10:10:01Z",
      });
      return { turnId: "turn-early", runId: "run-early" };
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "验证事件竞态" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("已先到达的部分输出")).toBeVisible();
    expect(await screen.findByRole("alert")).toHaveTextContent("磁盘不可写");
    expect(screen.queryByText("请求凭证已锁定，正在等待 Provider 返回。")).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "可以继续编辑" },
    });
    expect(screen.getByRole("button", { name: "发送" })).not.toBeDisabled();
  });

  it("offers an explicit retry when sending fails with a retryable bridge error", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.createTurnAndStartRun)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "provider_unreachable",
        message: "Provider 暂时不可达",
        retryable: true,
      }))
      .mockResolvedValueOnce({ turnId: "turn-2", runId: "run-2" });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "重新连接后继续" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    expect(await screen.findByText("Provider 暂时不可达")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重试发送" }));

    await waitFor(() => expect(bridge.createTurnAndStartRun).toHaveBeenCalledTimes(2));
  });

  it("offers an explicit retry when answer-version retry is temporarily unavailable", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.retryRun)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "provider_timeout",
        message: "Provider 响应超时",
        retryable: true,
      }))
      .mockResolvedValueOnce({ turnId: "turn-1", runId: "run-c" });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "重试回答" }));

    expect(await screen.findByText("Provider 响应超时")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重试回答请求" }));

    await waitFor(() => expect(bridge.retryRun).toHaveBeenCalledTimes(2));
  });

  it("offers an explicit retry when Context inspection fails temporarily", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.inspectContext)
      .mockRejectedValueOnce(new DesktopBridgeError({
        code: "repository_busy",
        message: "Context 暂时无法读取",
        retryable: true,
      }))
      .mockResolvedValue(preview as never);
    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("Context 暂时无法读取")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重新检查 Context" }));

    await waitFor(() => expect(bridge.inspectContext).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: /2 项 Context/ })).toBeVisible();
  });

  it("opens a route-map departure at the exact selected Run", async () => {
    const bridge = bridgeFixture();
    render(<FocusWorkspace bridge={bridge} initialRunId="run-b" />);

    expect(await screen.findByText("以专注阅读为主，路线图按需打开。")).toBeVisible();
    expect(screen.getByText(/从精确 Run run-b 继续/)).toBeVisible();
  });

  it("creates a branch from the exact selected answer version", async () => {
    const bridge = bridgeFixture();
    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByRole("heading", { name: workspace.name })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /回答 B.*qwen3:14b/i }));
    fireEvent.click(screen.getByRole("button", { name: "从此回答创建分支" }));

    const branchForm = screen.getByRole("form", { name: "创建精确回答分支" });
    fireEvent.change(within(branchForm).getByRole("textbox"), {
      target: { value: "如果入口改成画布，会带来哪些恢复成本？" },
    });
    fireEvent.submit(branchForm);

    await waitFor(() =>
      expect(bridge.createTurnAndStartRun).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: "workspace-1",
          parentRunId: "run-b",
          prompt: "如果入口改成画布，会带来哪些恢复成本？",
          previewHash: "sha256:preview",
        }),
        expect.any(Function),
      ),
    );
  });

  it("retries a root Turn against its original null parent instead of its own selected Run", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.retryRun).mockResolvedValue({ turnId: "turn-1", runId: "run-c" });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "重试回答" }));

    await waitFor(() =>
      expect(bridge.inspectContext).toHaveBeenCalledWith(
        expect.objectContaining({
          workspaceId: "workspace-1",
          parentRunId: null,
          prompt: "默认界面应该是线性阅读，还是无限画布？",
        }),
      ),
    );
    expect(bridge.retryRun).toHaveBeenCalledWith(
      expect.objectContaining({
        runId: "run-a",
        previewHash: "sha256:preview",
      }),
      expect.any(Function),
    );
  });

  it("keeps provider destination visible and blocks sending an over-limit context", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.inspectContext).mockResolvedValue({
      ...preview,
      estimatedTokens: 9000,
      blocked: true,
      warnings: ["上下文超过模型限制，请排除内容后重试。"],
    } as never);
    render(<FocusWorkspace bridge={bridge} />);

    expect((await screen.findAllByText(/api\.example\.com/))[0]).toBeVisible();
    expect(screen.getByText("数据保存在本机")).toBeVisible();
    expect((await screen.findAllByText(/上下文超过模型限制/))[0]).toBeVisible();

    fireEvent.change(screen.getByRole("textbox", { name: "消息" }), {
      target: { value: "继续分析" },
    });
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
  });

  it("updates next-send exclusions without mutating the locked snapshot", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getRunSnapshot).mockResolvedValue({
      id: "snapshot-a",
      runId: "run-a",
      canonicalHash: "sha256:locked",
      createdAt: "2026-07-22T09:11:00Z",
      providerId: "openai-compatible",
      templateRevision: 3,
      providerName: "OpenAI compatible",
      streamProtocol: "openai_sse",
      authPlacement: "bearer_header",
      authHeaderName: "Authorization",
      additionalHeaders: { "x-client-revision": "2026-07-22" },
      model: "gpt-4.1",
      baseUrl: "https://api.example.com/v1",
      parameters: { temperature: 0.2, stop: ["DONE"] },
      items: preview.items,
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(await screen.findByRole("button", { name: "排除 回答 A" }));

    await waitFor(() =>
      expect(bridge.updateContextOverrides).toHaveBeenCalledWith({
        workspaceId: "workspace-1",
        parentRunId: "run-a",
        itemId: "context-run-a",
        included: false,
        pinned: false,
      }),
    );

    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));
    expect(await screen.findByText("sha256:locked")).toBeVisible();
    expect(screen.getByText("openai-compatible r3")).toBeVisible();
    expect(screen.getByText("OpenAI SSE")).toBeVisible();
    expect(screen.getByText("Authorization: Bearer …")).toBeVisible();
    expect(screen.getByText("x-client-revision")).toBeVisible();
    expect(screen.getByText("2026-07-22")).toBeVisible();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.2")).toBeVisible();
    expect(screen.getByText('["DONE"]')).toBeVisible();
    expect(screen.getByRole("button", { name: "排除 回答 A" })).toBeDisabled();
  });

  it("falls back atomically for a legacy snapshot instead of rendering partial provider metadata", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.getRunSnapshot).mockResolvedValue({
      id: "snapshot-legacy",
      runId: "run-a",
      canonicalHash: "sha256:legacy",
      createdAt: "2026-07-22T09:11:00Z",
      providerName: "Legacy Provider",
      model: "legacy-model",
      baseUrl: "https://legacy.example.com/v1",
      additionalHeaders: {},
      parameters: { temperature: 0.7 },
      items: preview.items,
    });
    render(<FocusWorkspace bridge={bridge} />);

    await screen.findByRole("heading", { name: workspace.name });
    fireEvent.click(screen.getByRole("button", { name: "打开上下文检查器" }));
    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));

    expect(await screen.findByText("旧版快照：Provider 协议与认证元数据未记录")).toBeVisible();
    expect(screen.queryByText(/undefined|rundefined/)).not.toBeInTheDocument();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.7")).toBeVisible();
  });
});
