import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import type { ProviderTemplate } from "../../shared/contracts";
import { ProviderSettings } from "./ProviderSettings";

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
    saveProviderProfile: vi.fn(),
    setSessionCredential: vi.fn(),
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
          revision: 1,
          displayName: "Anthropic",
          defaultBaseUrl: "https://api.anthropic.com",
          protocol: {
            streamProtocol: "anthropic_sse",
            authPlacement: "api_key_header",
            authHeaderName: "x-api-key",
            requiresAdditionalHeaders: true,
            additionalHeaders: { "anthropic-version": "2023-06-01" },
          },
          runtimeAvailable: false,
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
    expect(screen.getByRole("button", { name: "协议即将支持" })).toBeDisabled();
    fireEvent.change(selector, { target: { value: "openai" } });
    expect(screen.getByLabelText("Base URL")).toHaveValue("https://api.openai.com/v1");
    expect(screen.getByText(/openai_sse/)).toBeVisible();
    expect(screen.getByText("Authorization: Bearer …")).toBeVisible();

    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "will-be-cleared" } });
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
    fireEvent.change(screen.getByLabelText("模型"), {
      target: { value: "gpt-4.1" },
    });
    fireEvent.change(screen.getByLabelText("API Key（仅本次会话）"), {
      target: { value: "secret-value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));

    await waitFor(() =>
      expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
        expect.not.objectContaining({ apiKey: expect.anything() }),
        "secret-value",
      ),
    );
    expect(bridge.setSessionCredential).not.toHaveBeenCalled();
    expect(screen.getByText(/工作区内容保存在本机/)).toBeVisible();
    expect(screen.getByText(/选中的 Context 会发送到 llm\.example\.com/)).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("新 API Key 已载入本次应用会话");
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
            requiresAdditionalHeaders: false,
            additionalHeaders: {},
          },
          runtimeAvailable: true,
        },
      ] satisfies ProviderTemplate[]),
      listProviderProfiles: vi.fn().mockResolvedValue([]),
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
    expect(screen.getByLabelText("API Key（仅本次会话）")).toBeDisabled();

    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Remote no-auth endpoint" },
    });
    fireEvent.change(screen.getByLabelText("模型"), { target: { value: "qwen3" } });
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
    expect(screen.getByRole("status")).toHaveTextContent("之前的会话 API Key 已清除");
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
});
