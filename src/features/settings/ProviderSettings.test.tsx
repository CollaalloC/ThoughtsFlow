import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DesktopBridgeError, type DesktopBridge } from "../../platform/desktop-bridge";
import type { ProviderModelInfo, ProviderTemplate } from "../../shared/contracts";
import { ProviderSettings } from "./ProviderSettings";

const openAiTemplate: ProviderTemplate = {
  providerId: "openai-compatible",
  revision: 1,
  displayName: "Generic OpenAI-compatible",
  defaultBaseUrl: "http://127.0.0.1:8000/v1",
  protocol: {
    streamProtocol: "openai_sse",
    authPlacement: "bearer_header",
    authHeaderName: "Authorization",
    modelsEndpoint: "/models",
    requiresAdditionalHeaders: false,
    additionalHeaders: {},
  },
  runtimeAvailable: true,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

function createProviderSettingsBridge(overrides: Partial<DesktopBridge> = {}): DesktopBridge {
  return {
    listWorkspaces: vi.fn(),
    createWorkspace: vi.fn(),
    openWorkspace: vi.fn(),
    updateWorkspace: vi.fn(),
    inspectContext: vi.fn(),
    createTurnAndStartRun: vi.fn(),
    retryRun: vi.fn(),
    cancelRun: vi.fn(),
    getRunSnapshot: vi.fn(),
    updateContextOverrides: vi.fn(),
    getRouteProjection: vi.fn(),
    updateViewState: vi.fn(),
    compareRuns: vi.fn(),
    markDecision: vi.fn(),
    exportDecisionPacket: vi.fn(),
    listProviderTemplates: vi.fn().mockResolvedValue([]),
    listProviderProfiles: vi.fn().mockResolvedValue([]),
    listProviderModels: vi.fn().mockResolvedValue([]),
    listSessionCredentials: vi.fn().mockResolvedValue([]),
    saveProviderProfile: vi.fn(),
    setSessionCredential: vi.fn().mockResolvedValue([]),
    activateSessionCredential: vi.fn().mockResolvedValue([]),
    reorderSessionCredentials: vi.fn().mockResolvedValue([]),
    removeSessionCredential: vi.fn().mockResolvedValue([]),
    testProviderConnection: vi.fn(),
    subscribeToRunEvents: vi.fn(() => () => undefined),
    ...overrides,
  };
}

describe("ProviderSettings", () => {
  it("selects a Rust-owned template, fills its protocol defaults, and keeps Base URL overridable", async () => {
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([
        {
          providerId: "openai",
          revision: 1,
          displayName: "OpenAI",
          defaultBaseUrl: "https://api.openai.com/v1",
          protocol: {
            streamProtocol: "openai_sse",
            authPlacement: "bearer_header",
            authHeaderName: "Authorization",
            modelsEndpoint: "/models",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
        {
          providerId: "anthropic",
          revision: 2,
          displayName: "Anthropic",
          defaultBaseUrl: "https://api.anthropic.com",
          protocol: {
            streamProtocol: "anthropic_sse",
            authPlacement: "api_key_header",
            authHeaderName: "x-api-key",
            requiresAdditionalHeaders: true,
            additionalHeaders: { "anthropic-version": "2023-06-01" },
          },
          runtimeAvailable: true,
        },
        {
          providerId: "ollama",
          revision: 2,
          displayName: "Ollama",
          defaultBaseUrl: "http://127.0.0.1:11434",
          protocol: {
            streamProtocol: "ollama_ndjson",
            authPlacement: "bearer_header",
            authHeaderName: "Authorization",
            modelsEndpoint: "/api/tags",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      listProviderModels: vi.fn().mockResolvedValue([
        {
          id: "gpt-4.1",
          displayName: "gpt-4.1",
          contextWindow: null,
          supportsTools: null,
        },
      ] satisfies ProviderModelInfo[]),
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...input,
        id: "provider-openai",
        dialect: "openai-compatible",
      })),
      setSessionCredential: vi.fn().mockResolvedValue(undefined),
      testProviderConnection: vi.fn(),
    });

    render(<ProviderSettings bridge={bridge} />);

    const selector = await screen.findByLabelText("Provider 模板");
    const anthropicOption = screen.getByRole("option", { name: /Anthropic/ });
    expect(anthropicOption).toBeEnabled();
    fireEvent.change(selector, { target: { value: "anthropic" } });
    expect(screen.getByText("x-api-key: …")).toBeVisible();
    expect(screen.getByRole("button", { name: "保存 Provider" })).toBeEnabled();
    fireEvent.change(selector, { target: { value: "openai" } });
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://api.openai.com/v1");
    expect(screen.getByText(/openai_sse/)).toBeVisible();
    expect(screen.getByText("Authorization: Bearer …")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    await screen.findByText(/已发现 1 个模型/);
    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "gpt-4.1" } });
    fireEvent.change(screen.getByLabelText("API Key（仅本次会话）"), {
      target: { value: "also-cleared" },
    });
    fireEvent.change(selector, { target: { value: "ollama" } });
    expect(screen.getByLabelText("Base URL")).toHaveValue("http://127.0.0.1:11434");
    expect(screen.getByLabelText("模型")).toHaveValue("");
    expect(screen.getByLabelText("API Key（仅本次会话）")).toHaveValue("");
    expect(screen.getByText("Authorization: Bearer …")).toBeVisible();

    fireEvent.change(selector, { target: { value: "openai" } });

    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "OpenAI 代理" } });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://gateway.example.com/openai/v1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    await screen.findByText(/已发现 1 个模型/);
    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "gpt-4.1" } });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));

    await waitFor(() =>
      expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
        expect.objectContaining({
          providerId: "openai",
          baseUrl: "https://gateway.example.com/openai/v1",
        }),
      ),
    );
    expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
      expect.not.objectContaining({ dialect: expect.anything() }),
    );
  });

  it("saves credentials only to the Rust session and explains the outbound boundary", async () => {
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([
        {
          providerId: "openai-compatible",
          revision: 1,
          displayName: "Generic OpenAI-compatible",
          defaultBaseUrl: "http://127.0.0.1:8000/v1",
          protocol: {
            streamProtocol: "openai_sse",
            authPlacement: "bearer_header",
            authHeaderName: "Authorization",
            modelsEndpoint: "/models",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      listProviderModels: vi.fn().mockResolvedValue([
        {
          id: "gpt-4.1",
          displayName: "gpt-4.1",
          contextWindow: null,
          supportsTools: null,
        },
      ] satisfies ProviderModelInfo[]),
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...input,
        id: "provider-1",
        dialect: "openai-compatible",
      })),
      setSessionCredential: vi.fn().mockResolvedValue(undefined),
      testProviderConnection: vi.fn().mockResolvedValue({
        ok: true,
        message: "连接成功",
      }),
    });

    render(<ProviderSettings bridge={bridge} />);
    await screen.findByRole("option", { name: "Generic OpenAI-compatible" });

    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "团队网关" },
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://llm.example.com/v1" },
    });
    fireEvent.change(screen.getByLabelText("初始凭据标签"), {
      target: { value: "Team primary" },
    });
    fireEvent.change(screen.getByLabelText("API Key（仅本次会话）"), {
      target: { value: "secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    await screen.findByText(/已发现 1 个模型/);
    fireEvent.change(screen.getByLabelText("模型"), {
      target: { value: "gpt-4.1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));

    await waitFor(() =>
      expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
        expect.not.objectContaining({ apiKey: expect.anything() }),
        { label: "Team primary", credential: "secret-value" },
      ),
    );
    expect(bridge.setSessionCredential).not.toHaveBeenCalled();
    expect(screen.getByText(/工作区内容保存在本机/)).toBeVisible();
    expect(screen.getByText(/选中的 Context 会发送到 llm\.example\.com/)).toBeVisible();
    expect(screen.getByText(/API Key 会短暂存在 WebView 表单/)).toBeVisible();
    expect(screen.getByText(/不写入 SQLite 或前端持久状态/)).toBeVisible();
    expect(
      screen.getByText(/命名凭据“Team primary”已载入 Rust 进程内存/),
    ).toBeVisible();
    await waitFor(() =>
      expect(bridge.listSessionCredentials).toHaveBeenCalledWith({
        providerProfileId: "provider-1",
      }),
    );
    expect(screen.queryByText("secret-value")).not.toBeInTheDocument();
  });

  it("treats only loopback hosts as local even when the template uses no authentication", async () => {
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([
        {
          providerId: "synthetic-no-auth-local",
          revision: 1,
          displayName: "Synthetic no-auth local",
          defaultBaseUrl: "http://127.0.0.1:11434",
          protocol: {
            streamProtocol: "ollama_ndjson",
            authPlacement: "none",
            modelsEndpoint: "/api/tags",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      listProviderModels: vi.fn().mockResolvedValue([
        {
          id: "qwen3",
          displayName: "qwen3",
          contextWindow: null,
          supportsTools: null,
        },
      ] satisfies ProviderModelInfo[]),
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...input,
        id: "provider-remote-no-auth",
        dialect: "ollama",
      })),
      setSessionCredential: vi.fn().mockResolvedValue(undefined),
      testProviderConnection: vi.fn(),
    });

    render(<ProviderSettings bridge={bridge} />);
    await screen.findByRole("option", { name: "Synthetic no-auth local" });

    expect(screen.getByText(/本机 · 未命名 Provider · 127\.0\.0\.1:11434/)).toBeVisible();
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://ollama.example.com" },
    });

    expect(screen.getByText(/外发 · 未命名 Provider · ollama\.example\.com/)).toBeVisible();
    expect(screen.getByText("无认证")).toBeVisible();
    expect(screen.queryByLabelText("API Key（仅本次会话）")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Remote no-auth endpoint" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    await screen.findByText(/已发现 1 个模型/);
    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "qwen3" } });
    expect(bridge.listProviderModels).toHaveBeenCalledWith({
      draft: {
        providerId: "synthetic-no-auth-local",
        baseUrl: "https://ollama.example.com",
      },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));
    await waitFor(() => expect(bridge.saveProviderProfile).toHaveBeenCalled());
    expect(bridge.setSessionCredential).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("该模板无需会话凭据");
  });

  it("locks a saved profile template and blocks stale connection tests until edits are saved", async () => {
    const profile = {
      id: "provider-saved",
      providerId: "openai",
      name: "Saved OpenAI",
      dialect: "openai-compatible" as const,
      baseUrl: "https://api.openai.com/v1",
      model: "gpt-4.1",
      isDefault: true,
      parameters: {},
    };
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([
        {
          providerId: "openai",
          revision: 1,
          displayName: "OpenAI",
          defaultBaseUrl: "https://api.openai.com/v1",
          protocol: {
            streamProtocol: "openai_sse",
            authPlacement: "bearer_header",
            authHeaderName: "Authorization",
            modelsEndpoint: "/models",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([profile]),
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...profile,
        ...input,
      })),
      setSessionCredential: vi.fn(),
      testProviderConnection: vi.fn().mockResolvedValue({ ok: true, message: "连接成功" }),
    });

    render(<ProviderSettings bridge={bridge} />);
    fireEvent.click(await screen.findByRole("button", { name: /Saved OpenAI/ }));

    expect(screen.getByLabelText("Provider 模板")).toBeDisabled();
    expect(screen.getByText(/已保存 Profile 的模板不可更改/)).toBeVisible();
    expect(screen.getByText(/名称、模型和参数更新会保留现有会话凭据/)).toBeVisible();
    expect(screen.getByText(/Base URL.*保存成功后清除旧凭据/)).toBeVisible();
    expect(screen.getByRole("button", { name: "测试连接" })).toBeEnabled();

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://gateway.example.com/v1" },
    });

    const saveFirst = screen.getByRole("button", { name: "请先保存更改" });
    expect(saveFirst).toBeDisabled();
    fireEvent.click(saveFirst);
    expect(bridge.testProviderConnection).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));
    await waitFor(() => expect(bridge.saveProviderProfile).toHaveBeenCalled());
    expect(
      screen.getByText(/端点身份已变更，旧会话凭据已在保存成功后清除/),
    ).toBeVisible();
  });

  it("rejects insecure non-loopback HTTP endpoints before saving", async () => {
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([
        {
          providerId: "openai-compatible",
          revision: 1,
          displayName: "Generic OpenAI-compatible",
          defaultBaseUrl: "http://127.0.0.1:8000/v1",
          protocol: {
            streamProtocol: "openai_sse",
            authPlacement: "bearer_header",
            authHeaderName: "Authorization",
            modelsEndpoint: "/models",
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      saveProviderProfile: vi.fn(),
      setSessionCredential: vi.fn(),
      testProviderConnection: vi.fn(),
    });
    render(<ProviderSettings bridge={bridge} />);
    await screen.findByRole("option", { name: "Generic OpenAI-compatible" });

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "http://llm.example.com/v1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));

    expect(screen.getByRole("alert")).toHaveTextContent("远程端点必须使用 HTTPS");
    expect(bridge.saveProviderProfile).not.toHaveBeenCalled();
  });

  it("discovers models from a new draft without saving or rendering Provider names as HTML", async () => {
    const models: ProviderModelInfo[] = [
      {
        id: "safe-model",
        displayName: "Safe model",
        contextWindow: 128_000,
        supportsTools: true,
      },
      {
        id: "evil-model",
        displayName: '<img src="x" onerror="alert(1)">',
        contextWindow: null,
        supportsTools: null,
      },
    ];
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([openAiTemplate]),
      listProviderModels: vi.fn().mockResolvedValue(models),
    });

    render(<ProviderSettings bridge={bridge} />);
    await screen.findByRole("option", { name: openAiTemplate.displayName });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://llm.example.com/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key（仅本次会话）"), {
      target: { value: "session-only-secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));

    await waitFor(() =>
      expect(bridge.listProviderModels).toHaveBeenCalledWith({
        draft: {
          providerId: "openai-compatible",
          baseUrl: "https://llm.example.com/v1",
          sessionCredential: "session-only-secret",
        },
      }),
    );
    await screen.findByText(/已发现 2 个模型/);
    const maliciousSuggestion = document.querySelector<HTMLDataListElement>(
      '#provider-model-options option[value="evil-model"]',
    );
    expect(maliciousSuggestion).not.toBeNull();
    expect(maliciousSuggestion).toHaveTextContent(
      '<img src="x" onerror="alert(1)"> (evil-model)',
    );
    expect(document.querySelector('img[src="x"]')).toBeNull();
    expect(screen.getByText(/只读取模型目录元数据，不发送工作区 Context/)).toBeVisible();
    expect(screen.getByText(/远程目录仅向 llm\.example\.com.*发起 GET/)).toBeVisible();
    expect(screen.getByText(/内置审核列表不会联网/)).toBeVisible();

    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "safe-model" } });
    expect(screen.getByLabelText("模型")).toHaveValue("safe-model");
    expect(screen.getByText(/目录元数据：Safe model/)).toHaveTextContent(
      "Context 128,000 · Tools 支持",
    );
    expect(bridge.saveProviderProfile).not.toHaveBeenCalled();
    expect(screen.queryByText("session-only-secret")).not.toBeInTheDocument();
  });

  it("explains that an audited static model catalog does not issue a remote GET", async () => {
    const anthropicTemplate: ProviderTemplate = {
      providerId: "anthropic",
      revision: 2,
      displayName: "Anthropic",
      defaultBaseUrl: "https://api.anthropic.com",
      protocol: {
        streamProtocol: "anthropic_sse",
        authPlacement: "api_key_header",
        authHeaderName: "x-api-key",
        modelsEndpoint: "/v1/models",
        requiresAdditionalHeaders: true,
        additionalHeaders: { "anthropic-version": "2023-06-01" },
      },
      runtimeAvailable: true,
    };
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([anthropicTemplate]),
      listProviderModels: vi.fn().mockResolvedValue([
        {
          id: "claude-static",
          displayName: "Claude static catalog entry",
          contextWindow: null,
          supportsTools: true,
        },
      ] satisfies ProviderModelInfo[]),
    });

    render(<ProviderSettings bridge={bridge} />);
    fireEvent.change(await screen.findByLabelText("Provider 模板"), {
      target: { value: "anthropic" },
    });

    expect(screen.getByText(/只读取模型目录元数据，不发送工作区 Context/)).toBeVisible();
    expect(screen.getByText(/远程目录仅向 api\.anthropic\.com.*发起 GET/)).toBeVisible();
    expect(screen.getByText(/内置审核列表不会联网/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    expect(await screen.findByText(/已发现 1 个模型/)).toBeVisible();
  });

  it("uses a saved profile for discovery, preserves an unlisted current model, and saves only explicitly", async () => {
    const profile = {
      id: "provider-saved",
      providerId: "openai-compatible",
      name: "Saved gateway",
      dialect: "openai-compatible" as const,
      baseUrl: "https://llm.example.com/v1",
      model: "legacy-model",
      isDefault: true,
      parameters: {},
    };
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([openAiTemplate]),
      listProviderProfiles: vi.fn().mockResolvedValue([profile]),
      listProviderModels: vi.fn().mockResolvedValue([
        {
          id: "new-model",
          displayName: "New model",
          contextWindow: null,
          supportsTools: null,
        },
      ] satisfies ProviderModelInfo[]),
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...profile,
        ...input,
      })),
      listSessionCredentials: vi.fn().mockResolvedValue([
        {
          credentialId: "credential-primary",
          label: "Primary production",
          order: 0,
          isActive: true,
        },
      ]),
    });

    render(<ProviderSettings bridge={bridge} />);
    fireEvent.click(await screen.findByRole("button", { name: /Saved gateway/ }));
    expect(await screen.findByText("Primary production")).toBeVisible();
    expect(screen.getByRole("heading", { name: "会话凭据" })).toBeVisible();
    expect(screen.queryByLabelText("初始凭据标签")).not.toBeInTheDocument();
    expect(document.body).not.toHaveTextContent("stored-secret-that-must-not-return");
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));

    await waitFor(() =>
      expect(bridge.listProviderModels).toHaveBeenCalledWith({
        providerProfileId: "provider-saved",
      }),
    );
    await screen.findByText(/已发现 1 个模型/);
    expect(screen.getByLabelText("模型")).toHaveValue("legacy-model");

    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "new-model" } });
    expect(bridge.saveProviderProfile).not.toHaveBeenCalled();
    expect(screen.getByText(/选择后仍需保存 Provider 才会生效/)).toBeVisible();

    fireEvent.change(screen.getByLabelText("名称"), { target: { value: "Renamed gateway" } });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    await waitFor(() => expect(bridge.listProviderModels).toHaveBeenCalledTimes(2));
    expect(bridge.listProviderModels).toHaveBeenLastCalledWith({
      providerProfileId: "provider-saved",
    });

    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));
    await waitFor(() =>
      expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
        expect.objectContaining({ model: "new-model" }),
      ),
    );
    expect(
      screen.getByText(/端点身份未变，现有会话凭据已保留/),
    ).toBeVisible();
  });

  it("opens the requested saved profile directly and loads only its safe credential summaries", async () => {
    const profiles = [
      {
        id: "provider-a",
        providerId: "openai-compatible",
        name: "Gateway A",
        dialect: "openai-compatible" as const,
        baseUrl: "https://a.example.com/v1",
        model: "model-a",
        isDefault: true,
        parameters: {},
      },
      {
        id: "provider-b",
        providerId: "openai-compatible",
        name: "Gateway B",
        dialect: "openai-compatible" as const,
        baseUrl: "https://b.example.com/v1",
        model: "model-b",
        isDefault: false,
        parameters: {},
      },
    ];
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([openAiTemplate]),
      listProviderProfiles: vi.fn().mockResolvedValue(profiles),
      listSessionCredentials: vi.fn().mockResolvedValue([
        {
          credentialId: "credential-b",
          label: "B backup",
          order: 0,
          isActive: true,
        },
      ]),
    });

    render(
      <ProviderSettings
        bridge={bridge}
        initialProviderProfileId="provider-b"
      />,
    );

    expect(await screen.findByLabelText("名称")).toHaveValue("Gateway B");
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://b.example.com/v1");
    expect(screen.getByLabelText("模型")).toHaveValue("model-b");
    expect(await screen.findByText("B backup")).toBeVisible();
    expect(bridge.listSessionCredentials).toHaveBeenCalledWith({
      providerProfileId: "provider-b",
    });
  });

  it("shows discovery loading, retryable errors, retry, and a valid empty catalog", async () => {
    const firstRequest = deferred<ProviderModelInfo[]>();
    const listProviderModels = vi
      .fn()
      .mockReturnValueOnce(firstRequest.promise)
      .mockResolvedValueOnce([]);
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([openAiTemplate]),
      listProviderModels,
      saveProviderProfile: vi.fn().mockImplementation(async (input) => ({
        ...input,
        id: "provider-manual",
        dialect: "openai-compatible" as const,
      })),
    });

    render(<ProviderSettings bridge={bridge} />);
    await screen.findByRole("option", { name: openAiTemplate.displayName });
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    expect(screen.getByRole("button", { name: "发现中" })).toBeDisabled();

    firstRequest.reject(
      new DesktopBridgeError({
        code: "provider_timeout",
        message: "模型目录请求超时",
        retryable: true,
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("模型目录请求超时");
    expect(screen.getByRole("alert")).toHaveTextContent("临时错误，可以重试");

    fireEvent.click(screen.getByRole("button", { name: "重试发现模型" }));
    expect(await screen.findByRole("status")).toHaveTextContent("端点返回了空模型列表");
    expect(listProviderModels).toHaveBeenCalledTimes(2);

    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Manual fallback gateway" },
    });
    fireEvent.change(screen.getByLabelText("模型"), {
      target: { value: "manual-model-id" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));
    await waitFor(() =>
      expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
        expect.objectContaining({ model: "manual-model-id" }),
      ),
    );
  });

  it("ignores a late discovery response after switching profiles", async () => {
    const firstRequest = deferred<ProviderModelInfo[]>();
    const profiles = [
      {
        id: "provider-a",
        providerId: "openai-compatible",
        name: "Gateway A",
        dialect: "openai-compatible" as const,
        baseUrl: "https://a.example.com/v1",
        model: "a-current",
        isDefault: true,
        parameters: {},
      },
      {
        id: "provider-b",
        providerId: "openai-compatible",
        name: "Gateway B",
        dialect: "openai-compatible" as const,
        baseUrl: "https://b.example.com/v1",
        model: "b-current",
        isDefault: false,
        parameters: {},
      },
    ];
    const listProviderModels = vi
      .fn()
      .mockReturnValueOnce(firstRequest.promise)
      .mockResolvedValueOnce([
        {
          id: "b-discovered",
          displayName: "B discovered",
          contextWindow: null,
          supportsTools: null,
        },
      ] satisfies ProviderModelInfo[]);
    const bridge = createProviderSettingsBridge({
      listProviderTemplates: vi.fn().mockResolvedValue([openAiTemplate]),
      listProviderProfiles: vi.fn().mockResolvedValue(profiles),
      listProviderModels,
    });

    render(<ProviderSettings bridge={bridge} />);
    fireEvent.click(await screen.findByRole("button", { name: /Gateway A/ }));
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));
    fireEvent.click(screen.getByRole("button", { name: /Gateway B/ }));
    fireEvent.click(screen.getByRole("button", { name: "发现模型" }));

    await screen.findByText(/已发现 1 个模型/);
    expect(
      document.querySelector('#provider-model-options option[value="b-discovered"]'),
    ).not.toBeNull();
    firstRequest.resolve([
      {
        id: "a-discovered",
        displayName: "A discovered",
        contextWindow: null,
        supportsTools: null,
      },
    ]);
    await waitFor(() => expect(listProviderModels).toHaveBeenCalledTimes(2));
    expect(
      document.querySelector('#provider-model-options option[value="a-discovered"]'),
    ).toBeNull();
    expect(screen.getByLabelText("模型")).toHaveValue("b-current");
  });
});
