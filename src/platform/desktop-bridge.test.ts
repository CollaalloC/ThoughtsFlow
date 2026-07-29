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

  it("routes Context Tree reads and cursor CAS updates through versioned commands", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      if (command === "get_context_tree") {
        return {
          apiVersion: 1,
          data: {
            workspaceId: "workspace-1",
            rootId: "workspace-root:workspace-1",
            draftVersion: 6,
            cursor: {
              workspaceId: "workspace-1",
              activeRunId: "run-1",
              branchId: "branch-1",
              version: 2,
              updatedAt: "2026-07-28T00:00:00.000Z",
            },
            nodes: [],
            edges: [],
            branches: [],
            checkpoints: [],
          },
        } as T;
      }
      return {
        apiVersion: 1,
        data: {
          workspaceId: "workspace-1",
          activeRunId: "run-2",
          branchId: "branch-1",
          version: 3,
          updatedAt: "2026-07-28T00:01:00.000Z",
        },
      } as T;
    };
    const bridge = createDesktopBridge(invokeCommand);

    const tree = await bridge.getContextTree({ workspaceId: "workspace-1" });
    const cursor = await bridge.setActiveContext({
      workspaceId: "workspace-1",
      runId: "run-2",
      branchId: "branch-1",
      expectedCursorVersion: tree.cursor.version,
      expectedDraftVersion: tree.draftVersion,
    });

    expect(cursor).toMatchObject({ activeRunId: "run-2", version: 3 });
    expect(calls).toEqual([
      {
        command: "get_context_tree",
        args: { input: { workspaceId: "workspace-1" } },
      },
      {
        command: "set_active_context",
        args: {
          input: {
            workspaceId: "workspace-1",
            runId: "run-2",
            branchId: "branch-1",
            expectedCursorVersion: 2,
            expectedDraftVersion: 6,
          },
        },
      },
    ]);
  });

  it("persists next-send Context Draft identities and returns its new version", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return { apiVersion: 1, data: { draftVersion: 8 } } as T;
    };

    const result = await createDesktopBridge(invokeCommand).updateContextDraft({
      workspaceId: "workspace-1",
      parentRunId: "run-7",
      expectedDraftVersion: 7,
      items: [
        {
          sourceRef: { kind: "model-run", id: "run-2" },
          contentBlockId: "block-2",
          included: true,
          pinned: true,
        },
      ],
    });

    expect(result).toEqual({ draftVersion: 8 });
    expect(calls).toEqual([
      {
        command: "update_context_draft",
        args: {
          input: {
            workspaceId: "workspace-1",
            parentRunId: "run-7",
            expectedDraftVersion: 7,
            items: [
              {
                sourceRef: { kind: "model-run", id: "run-2" },
                contentBlockId: "block-2",
                included: true,
                pinned: true,
              },
            ],
          },
        },
      },
    ]);
  });

  it("keeps checkpoint preview, creation, and summarize-and-activate as distinct IPC actions", async () => {
    const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
    const invokeCommand = async <T,>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      calls.push({ command, args });
      return {
        apiVersion: 1,
        data:
          command === "preview_context_transition"
            ? {
                hash: "sha256:preview",
                estimatedTokens: 42,
                limitTokens: 1000,
                blocked: false,
                warnings: [],
                providerProfileId: "provider-1",
                providerName: "Provider",
                model: "model-1",
                baseUrl: "https://provider.example/v1",
                items: [],
                rawItems: [],
                draftVersion: 4,
                appliedCheckpoint: null,
              }
            : command === "create_context_checkpoint"
              ? {
                  id: "checkpoint-1",
                  workspaceId: "workspace-1",
                  branchId: "branch-1",
                  branchVersion: 1,
                  kind: "compaction",
                  anchorRunId: "run-2",
                  sourceRunIds: ["run-1"],
                  sourceHash: "sha256:sources",
                  firstKeptRunId: "run-2",
                  summary: "Stable summary",
                  provider: null,
                  status: "completed",
                  createdAt: "2026-07-28T00:00:00.000Z",
                }
              : {
                  cursor: {
                    workspaceId: "workspace-1",
                    activeRunId: "run-2",
                    branchId: "branch-1",
                    version: 5,
                    updatedAt: "2026-07-28T00:00:00.000Z",
                  },
                  checkpoint: null,
                },
      } as T;
    };
    const bridge = createDesktopBridge(invokeCommand);

    await bridge.previewContextTransition({
      workspaceId: "workspace-1",
      parentRunId: "run-2",
      prompt: "Next",
      providerProfileId: "provider-1",
      draftVersion: 4,
    });
    await bridge.createContextCheckpoint({
      clientOperationId: "11111111-1111-4111-8111-111111111111",
      workspaceId: "workspace-1",
      branchId: "branch-1",
      kind: "compaction",
      sourceRunIds: ["run-1"],
      firstKeptRunId: "run-2",
      summary: "Stable summary",
      expectedCursorVersion: 4,
      expectedBranchVersion: 1,
    });
    await bridge.summarizeAndSetActiveContext({
      clientOperationId: "22222222-2222-4222-8222-222222222222",
      workspaceId: "workspace-1",
      targetRunId: "run-2",
      branchId: "branch-1",
      sourceRunIds: ["run-1"],
      firstKeptRunId: "run-2",
      summaryPrompt: "Summarize decisions",
      providerProfileId: "provider-1",
      expectedCursorVersion: 4,
      expectedBranchVersion: 1,
      expectedDraftVersion: 4,
    });
    await bridge.cancelContextMaintenance("22222222-2222-4222-8222-222222222222");

    expect(calls.map(({ command }) => command)).toEqual([
      "preview_context_transition",
      "create_context_checkpoint",
      "summarize_and_set_active_context",
      "cancel_context_maintenance",
    ]);
    expect(calls[1]?.args).toStrictEqual({
      input: {
        clientOperationId: "11111111-1111-4111-8111-111111111111",
        workspaceId: "workspace-1",
        branchId: "branch-1",
        kind: "compaction",
        sourceRunIds: ["run-1"],
        firstKeptRunId: "run-2",
        summary: "Stable summary",
        expectedCursorVersion: 4,
        expectedBranchVersion: 1,
      },
    });
    expect(calls.at(-1)?.args).toEqual({
      clientOperationId: "22222222-2222-4222-8222-222222222222",
    });
  });
});
