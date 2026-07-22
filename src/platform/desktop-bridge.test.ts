import { describe, expect, it } from "vitest";

import type { ProviderModelInfo, ProviderTemplate } from "../shared/contracts";
import { DesktopBridgeError, createDesktopBridge } from "./desktop-bridge";

describe("DesktopBridge errors", () => {
  it("normalizes a plain structured Tauri rejection into DesktopBridgeError", async () => {
    const rejection = {
      code: "provider_timeout",
      message: "Provider timed out",
      retryable: true,
      details: { providerProfileId: "provider-1" },
    };
    const invokeCommand = async <T,>(): Promise<T> => Promise.reject(rejection);

    let error: unknown;
    try {
      await createDesktopBridge(invokeCommand).listWorkspaces();
    } catch (reason) {
      error = reason;
    }

    expect(error instanceof DesktopBridgeError).toBe(true);
    const bridgeError = error as DesktopBridgeError;
    expect(bridgeError.name).toBe("DesktopBridgeError");
    expect(bridgeError.code).toBe("provider_timeout");
    expect(bridgeError.message).toBe("Provider timed out");
    expect(bridgeError.retryable).toBe(true);
    expect(bridgeError.details).toEqual({ providerProfileId: "provider-1" });
  });

  it("loads Provider Templates from the Rust catalog command", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
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
        ] satisfies ProviderTemplate[],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).listProviderTemplates();

    expect(calls).toEqual([{ command: "list_provider_templates", args: undefined }]);
    expect(result[0]?.providerId).toBe("openai");
    expect(result[0]?.protocol.authPlacement).toBe("bearer_header");
  });

  it("saves a Provider Profile and its named session credential through one IPC command", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: {
          id: "provider-1",
          providerId: "openai",
          name: "OpenAI",
          dialect: "openai-compatible",
          baseUrl: "https://api.openai.com/v1",
          model: "gpt-4.1",
          isDefault: true,
          parameters: {},
        },
      } as T;
    };

    await createDesktopBridge(invokeCommand).saveProviderProfile(
      {
        providerId: "openai",
        name: "OpenAI",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4.1",
        isDefault: true,
        parameters: {},
      },
      { label: "Primary", credential: "session-only-secret" },
    );

    expect(calls).toEqual([
      {
        command: "save_provider_profile",
        args: {
          input: {
            providerId: "openai",
            name: "OpenAI",
            baseUrl: "https://api.openai.com/v1",
            model: "gpt-4.1",
            isDefault: true,
            parameters: {},
            sessionCredentialLabel: "Primary",
            sessionCredential: "session-only-secret",
          },
        },
      },
    ]);
    expect(calls.some(({ command }) => command === "set_session_credential")).toBe(false);
  });

  it("omits both initial credential fields when saving a Provider Profile without one", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: {
          id: "provider-1",
          providerId: "openai",
          name: "OpenAI",
          dialect: "openai-compatible",
          baseUrl: "https://api.openai.com/v1",
          model: "gpt-4.1",
          isDefault: true,
          parameters: {},
        },
      } as T;
    };

    await createDesktopBridge(invokeCommand).saveProviderProfile({
      providerId: "openai",
      name: "OpenAI",
      baseUrl: "https://api.openai.com/v1",
      model: "gpt-4.1",
      isDefault: true,
      parameters: {},
    });

    expect(calls).toEqual([
      {
        command: "save_provider_profile",
        args: {
          input: {
            providerId: "openai",
            name: "OpenAI",
            baseUrl: "https://api.openai.com/v1",
            model: "gpt-4.1",
            isDefault: true,
            parameters: {},
          },
        },
      },
    ]);
  });

  it("lists only safe named credential summaries through the versioned command envelope", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
          {
            credentialId: "credential-primary",
            label: "Primary",
            order: 0,
            isActive: true,
            credential: "must-not-escape",
            secret: "must-not-escape-either",
          },
        ],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).listSessionCredentials({
      providerProfileId: "provider-1",
    });

    expect(calls).toEqual([
      {
        command: "list_session_credentials",
        args: { input: { providerProfileId: "provider-1" } },
      },
    ]);
    expect(result).toEqual([
      {
        credentialId: "credential-primary",
        label: "Primary",
        order: 0,
        isActive: true,
      },
    ]);
    expect(result[0]).not.toHaveProperty("credential");
    expect(result[0]).not.toHaveProperty("secret");
  });

  it("sets a named credential and returns only safe summaries", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
          {
            credentialId: "credential-backup",
            label: "Backup",
            order: 1,
            isActive: false,
            credential: "must-not-escape",
          },
        ],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).setSessionCredential({
      providerProfileId: "provider-1",
      credentialId: "credential-backup",
      credentialLabel: "Backup",
      credential: "session-only-secret",
    });

    expect(calls).toEqual([
      {
        command: "set_session_credential",
        args: {
          input: {
            providerProfileId: "provider-1",
            credentialId: "credential-backup",
            credentialLabel: "Backup",
            credential: "session-only-secret",
          },
        },
      },
    ]);
    expect(result).toEqual([
      {
        credentialId: "credential-backup",
        label: "Backup",
        order: 1,
        isActive: false,
      },
    ]);
  });

  it("preserves a structured credential command rejection as DesktopBridgeError", async () => {
    const rejection = {
      code: "credential_not_found",
      message: "Session credential was not found",
      retryable: false,
      details: { credentialId: "missing" },
    };
    const invokeCommand = async <T,>(): Promise<T> => Promise.reject(rejection);

    await expect(
      createDesktopBridge(invokeCommand).activateSessionCredential({
        providerProfileId: "provider-1",
        credentialId: "missing",
      }),
    ).rejects.toMatchObject({
      name: "DesktopBridgeError",
      code: "credential_not_found",
      message: "Session credential was not found",
      retryable: false,
      details: { credentialId: "missing" },
    });
  });

  it("activates a named credential using the snake_case command", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
          {
            credentialId: "credential-backup",
            label: "Backup",
            order: 0,
            isActive: true,
            secret: "must-not-escape",
          },
        ],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).activateSessionCredential({
      providerProfileId: "provider-1",
      credentialId: "credential-backup",
    });

    expect(calls).toEqual([
      {
        command: "activate_session_credential",
        args: {
          input: {
            providerProfileId: "provider-1",
            credentialId: "credential-backup",
          },
        },
      },
    ]);
    expect(result).toEqual([
      {
        credentialId: "credential-backup",
        label: "Backup",
        order: 0,
        isActive: true,
      },
    ]);
  });

  it("reorders named credentials using credential identifiers only", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
          {
            credentialId: "credential-backup",
            label: "Backup",
            order: 0,
            isActive: false,
          },
          {
            credentialId: "credential-primary",
            label: "Primary",
            order: 1,
            isActive: true,
          },
        ],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).reorderSessionCredentials({
      providerProfileId: "provider-1",
      orderedCredentialIds: ["credential-backup", "credential-primary"],
    });

    expect(calls).toEqual([
      {
        command: "reorder_session_credentials",
        args: {
          input: {
            providerProfileId: "provider-1",
            orderedCredentialIds: ["credential-backup", "credential-primary"],
          },
        },
      },
    ]);
    expect(result.map(({ credentialId, order }) => ({ credentialId, order }))).toEqual([
      { credentialId: "credential-backup", order: 0 },
      { credentialId: "credential-primary", order: 1 },
    ]);
  });

  it("removes a named credential and returns the remaining safe summaries", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data: [
          {
            credentialId: "credential-primary",
            label: "Primary",
            order: 0,
            isActive: true,
            apiKey: "must-not-escape",
          },
        ],
      } as T;
    };

    const result = await createDesktopBridge(invokeCommand).removeSessionCredential({
      providerProfileId: "provider-1",
      credentialId: "credential-backup",
    });

    expect(calls).toEqual([
      {
        command: "remove_session_credential",
        args: {
          input: {
            providerProfileId: "provider-1",
            credentialId: "credential-backup",
          },
        },
      },
    ]);
    expect(result).toEqual([
      {
        credentialId: "credential-primary",
        label: "Primary",
        order: 0,
        isActive: true,
      },
    ]);
  });

  it("discovers models for a saved profile through the versioned command envelope", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const models: ProviderModelInfo[] = [
      {
        id: "gpt-4.1",
        displayName: "GPT-4.1",
        contextWindow: 1_000_000,
        supportsTools: true,
      },
    ];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return { apiVersion: 1, data: models } as T;
    };

    const result = await createDesktopBridge(invokeCommand).listProviderModels({
      providerProfileId: "provider-1",
    });

    expect(calls).toEqual([
      {
        command: "list_provider_models",
        args: { input: { providerProfileId: "provider-1" } },
      },
    ]);
    expect(result).toEqual(models);
  });

  it("discovers models from an unsaved draft without persisting its session credential", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return { apiVersion: 1, data: [] } as T;
    };

    await createDesktopBridge(invokeCommand).listProviderModels({
      draft: {
        providerId: "openai-compatible",
        baseUrl: "https://llm.example.com/v1",
        sessionCredential: "session-only-secret",
      },
    });

    expect(calls).toEqual([
      {
        command: "list_provider_models",
        args: {
          input: {
            draft: {
              providerId: "openai-compatible",
              baseUrl: "https://llm.example.com/v1",
              sessionCredential: "session-only-secret",
            },
          },
        },
      },
    ]);
    expect(calls.some(({ command }) => command === "save_provider_profile")).toBe(false);
    expect(calls.some(({ command }) => command === "set_session_credential")).toBe(false);
  });
});
