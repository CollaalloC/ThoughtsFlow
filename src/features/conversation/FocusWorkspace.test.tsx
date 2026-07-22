import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type { WorkspaceDetail } from "../../shared/contracts";
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

const detail: WorkspaceDetail = {
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
    listProviderTemplates: vi.fn().mockResolvedValue([]),
    listProviderProfiles: vi.fn().mockResolvedValue([
      {
        id: "provider-cloud",
        providerId: "openai-compatible",
        name: "OpenAI compatible",
        dialect: "openai-compatible",
        baseUrl: "https://api.example.com/v1",
        model: "gpt-4.1",
        isDefault: true,
      },
    ]),
    listProviderModels: vi.fn().mockResolvedValue([]),
    listSessionCredentials: vi.fn().mockResolvedValue([]),
    saveProviderProfile: vi.fn(),
    setSessionCredential: vi.fn().mockResolvedValue([]),
    activateSessionCredential: vi.fn().mockResolvedValue([]),
    reorderSessionCredentials: vi.fn().mockResolvedValue([]),
    removeSessionCredential: vi.fn().mockResolvedValue([]),
    testProviderConnection: vi.fn(),
    subscribeToRunEvents: vi.fn().mockReturnValue(() => undefined),
  } as unknown as DesktopBridge;
}

describe("FocusWorkspace", () => {
  it("recovers a quota failure with the failed Run's exact Provider and creates a new version", async () => {
    const bridge = bridgeFixture();
    const composerProfile = {
      id: "provider-composer",
      providerId: "ollama",
      name: "Current composer",
      dialect: "ollama" as const,
      baseUrl: "http://127.0.0.1:11434",
      model: "qwen3:14b",
      isDefault: true,
      parameters: {},
    };
    const failedProfile = {
      id: "provider-failed",
      providerId: "openai-compatible",
      name: "Failed exact provider",
      dialect: "openai-compatible" as const,
      baseUrl: "https://failed.example.com/v1",
      model: "gpt-exact",
      isDefault: false,
      parameters: {},
    };
    const failedRun = {
      ...detail.turns[0].runs[0],
      status: "failed" as const,
      output: "已保留的失败部分输出",
      providerProfileId: failedProfile.id,
      providerName: failedProfile.name,
      model: failedProfile.model,
      baseUrl: failedProfile.baseUrl,
      error: {
        code: "quota_exhausted",
        message: "当前凭据额度已耗尽。",
        retryable: true,
        status: 429,
      },
    };
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([
      composerProfile,
      failedProfile,
    ]);
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{ ...detail.turns[0], runs: [failedRun] }],
      selectedRunIds: { "turn-1": failedRun.id },
    });
    vi.mocked(bridge.listSessionCredentials).mockResolvedValue([
      { credentialId: "primary", label: "Primary", order: 0, isActive: true },
      { credentialId: "backup", label: "Backup", order: 1, isActive: false },
    ]);
    vi.mocked(bridge.inspectContext).mockImplementation(async (input) => ({
      ...preview,
      hash: input.providerProfileId === failedProfile.id
        ? "sha256:failed-exact-profile"
        : "sha256:composer-profile",
      providerProfileId: input.providerProfileId,
    }));
    vi.mocked(bridge.retryRun).mockResolvedValue({
      turnId: "turn-1",
      runId: "run-recovered",
    });

    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("已保留的失败部分输出")).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue(
      composerProfile.id,
    );
    expect(screen.getByRole("region", { name: "凭据恢复" })).toBeVisible();
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole("combobox", { name: "备用凭据" });
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    await waitFor(() => expect(bridge.retryRun).toHaveBeenCalledTimes(1));
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    expect(bridge.inspectContext).toHaveBeenCalledWith({
      workspaceId: workspace.id,
      parentRunId: null,
      prompt: detail.turns[0].prompt,
      providerProfileId: failedProfile.id,
    });
    expect(bridge.retryRun).toHaveBeenCalledWith(
      {
        runId: failedRun.id,
        providerProfileId: failedProfile.id,
        previewHash: "sha256:failed-exact-profile",
        credentialId: "backup",
      },
      expect.any(Function),
    );
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue(
      composerProfile.id,
    );
    expect(await screen.findByRole("button", { name: /回答 B · gpt-exact/ })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /回答 A · gpt-exact/ }));
    expect(screen.getByText("已保留的失败部分输出")).toBeVisible();
  });

  it("keeps an atomic credential retry rejected when Run creation fails", async () => {
    const bridge = bridgeFixture();
    const failedRun = {
      ...detail.turns[0].runs[0],
      status: "failed" as const,
      output: "已保留的部分输出",
      error: {
        code: "rate_limited",
        message: "当前凭据被限流。",
        retryable: true,
        status: 429,
      },
    };
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{ ...detail.turns[0], runs: [failedRun] }],
      selectedRunIds: { "turn-1": failedRun.id },
    });
    vi.mocked(bridge.listSessionCredentials).mockResolvedValue([
      { credentialId: "primary", label: "Primary", order: 0, isActive: true },
      { credentialId: "backup", label: "Backup", order: 1, isActive: false },
    ]);
    vi.mocked(bridge.retryRun).mockRejectedValue(new DesktopBridgeError({
      code: "context_preview_stale",
      message: "Context 已变化，请重新确认。",
      retryable: false,
    }));

    render(<FocusWorkspace bridge={bridge} />);

    const recovery = await screen.findByRole("region", { name: "凭据恢复" });
    fireEvent.click(within(recovery).getByRole("button", { name: "选择备用凭据" }));
    await within(recovery).findByRole("combobox", { name: "备用凭据" });
    fireEvent.click(within(recovery).getByRole("button", { name: "切换并新增回答版本" }));

    expect(await within(recovery).findByRole("alert")).toHaveTextContent(
      "Context 已变化，请重新确认。",
    );
    expect(bridge.retryRun).toHaveBeenCalledWith(
      expect.objectContaining({ credentialId: "backup" }),
      expect.any(Function),
    );
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /回答 B/ })).not.toBeInTheDocument();
  });

  it.each([
    ["a different provider error", "provider_unreachable", true],
    ["a non-retryable rate limit", "rate_limited", false],
  ])("does not offer credential recovery for %s", async (_label, code, retryable) => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          status: "failed",
          error: { code, message: "不可使用凭据恢复。", retryable },
        }],
      }],
    } as never);

    render(<FocusWorkspace bridge={bridge} />);

    expect(await screen.findByText("不可使用凭据恢复。")).toBeVisible();
    expect(screen.queryByRole("region", { name: "凭据恢复" })).not.toBeInTheDocument();
    expect(bridge.listSessionCredentials).not.toHaveBeenCalled();
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
  });

  it("labels a remote Ollama endpoint as outbound instead of local", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          providerProfileId: "provider-remote-ollama",
          providerName: "Remote Ollama",
          model: "qwen3",
          baseUrl: "https://ollama.example.com",
        }],
      }],
    });
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

  it("renders a Run's immutable historical endpoint after its Profile is edited", async () => {
    const bridge = bridgeFixture();
    vi.mocked(bridge.listProviderProfiles).mockResolvedValue([{
      id: "provider-cloud",
      providerId: "openai-compatible",
      name: "Renamed current Profile",
      dialect: "openai-compatible",
      baseUrl: "https://new-endpoint.example.com/v1",
      model: "gpt-current",
      isDefault: true,
      parameters: {},
    }]);
    vi.mocked(bridge.openWorkspace).mockResolvedValue({
      ...detail,
      turns: [{
        ...detail.turns[0],
        runs: [{
          ...detail.turns[0].runs[0],
          providerName: "Historical Profile",
          baseUrl: "https://historical.example.com/v1",
        }],
      }],
    });

    render(<FocusWorkspace bridge={bridge} />);

    const output = await screen.findByText("线性阅读降低首分钟认知成本。");
    const answer = output.closest(".focus-turn__answer");
    expect(answer).not.toBeNull();
    expect(within(answer as HTMLElement).getByText("外发 · historical.example.com")).toBeVisible();
    expect(
      within(answer as HTMLElement).queryByText("外发 · new-endpoint.example.com"),
    ).not.toBeInTheDocument();
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
