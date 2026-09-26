import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createDesktopBridge, type DesktopBridge } from "../../platform/desktop-bridge";
import type { SessionCredentialSummary } from "../../shared/contracts";
import { SessionCredentialManager } from "./SessionCredentialManager";

const primaryCredential: SessionCredentialSummary = {
  credentialId: "credential-primary",
  label: "Primary production",
  order: 0,
  isActive: true,
};

const backupCredential: SessionCredentialSummary = {
  credentialId: "credential-backup",
  label: "Backup quota",
  order: 1,
  isActive: false,
};

const emergencyCredential: SessionCredentialSummary = {
  credentialId: "credential-emergency",
  label: "Emergency only",
  order: 2,
  isActive: false,
};

function createBridge(overrides: Partial<DesktopBridge> = {}): DesktopBridge {
  return {
    ...createDesktopBridge(async () => { throw new Error("Unexpected IPC in test"); }),
    listWorkspaces: vi.fn(),
    createWorkspace: vi.fn(),
    openWorkspace: vi.fn(),
    updateWorkspace: vi.fn(),
    inspectContext: vi.fn(),
    previewContextTransition: vi.fn(),
    getContextTree: vi.fn(),
    setActiveContext: vi.fn(),
    renameBranch: vi.fn(),
    updateContextDraft: vi.fn(),
    createContextCheckpoint: vi.fn(),
    summarizeAndSetActiveContext: vi.fn(),
    cancelContextMaintenance: vi.fn(),
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

describe("SessionCredentialManager", () => {
  it("restores keyboard focus to the exact Delete trigger after cancellation", async () => {
    const user = userEvent.setup();
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
        emergencyCredential,
      ]),
    });

    render(
      <SessionCredentialManager
        bridge={bridge}
        providerProfileId="provider-openai"
      />,
    );

    const primaryRow = await screen.findByRole("listitem", { name: "Primary production" });
    const primaryDelete = within(primaryRow).getByRole("button", {
      name: "删除 Primary production",
    });
    const emergencyRow = screen.getByRole("listitem", { name: "Emergency only" });
    const emergencyDelete = within(emergencyRow).getByRole("button", {
      name: "删除 Emergency only",
    });

    emergencyDelete.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "确认删除 Emergency only" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "取消" })).toHaveFocus();
    await user.keyboard("{Enter}");

    await waitFor(() => expect(emergencyDelete).toHaveFocus());
    expect(primaryDelete).not.toHaveFocus();
    expect(bridge.removeSessionCredential).not.toHaveBeenCalled();
  });

  it("loads only safe summaries and explains when this session has no credentials", async () => {
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([]),
    });

    render(
      <SessionCredentialManager
        bridge={bridge}
        providerProfileId="provider-openai"
      />,
    );

    expect(await screen.findByText("本次会话还没有凭据。")).toBeVisible();
    expect(bridge.listSessionCredentials).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
    });
    expect(screen.getByText(/退出应用后会全部清除/)).toBeVisible();
  });

  it("adds a named credential, clears the password, and never echoes the secret", async () => {
    const user = userEvent.setup();
    const secret = "sk-this-must-never-render";
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([primaryCredential]),
      setSessionCredential: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
      ]),
    });

    render(
      <SessionCredentialManager
        bridge={bridge}
        providerProfileId="provider-openai"
      />,
    );

    await screen.findByText("Primary production");
    await user.type(screen.getByLabelText("凭据标签"), "Backup quota");
    const password = screen.getByLabelText("API Key（仅本次会话）");
    await user.type(password, secret);
    await user.click(screen.getByRole("button", { name: "添加会话凭据" }));

    expect(bridge.setSessionCredential).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
      credentialLabel: "Backup quota",
      credential: secret,
    });
    expect(password).toHaveValue("");
    expect(await screen.findByText("Backup quota")).toBeVisible();
    expect(document.body).not.toHaveTextContent(secret);
    expect(document.body).not.toHaveTextContent("must-never-render");
  });

  it("activates, reorders, and removes credentials only after confirmation", async () => {
    const user = userEvent.setup();
    const reordered = [backupCredential, primaryCredential, emergencyCredential].map(
      (credential, order) => ({
        ...credential,
        order,
        isActive: order === 0,
      }),
    );
    const afterRemoval = reordered
      .filter(({ credentialId }) => credentialId !== "credential-primary")
      .map((credential, order) => ({ ...credential, order, isActive: order === 0 }));
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
        emergencyCredential,
      ]),
      activateSessionCredential: vi.fn().mockResolvedValue(reordered),
      reorderSessionCredentials: vi.fn().mockResolvedValue(reordered),
      removeSessionCredential: vi.fn().mockResolvedValue(afterRemoval),
    });

    render(
      <SessionCredentialManager
        bridge={bridge}
        providerProfileId="provider-openai"
      />,
    );

    const initialPrimaryRow = await screen.findByRole("listitem", {
      name: "Primary production",
    });
    expect(
      within(initialPrimaryRow).getByRole("button", { name: "删除 Primary production" }),
    ).toBeDisabled();
    expect(
      within(initialPrimaryRow).getByRole("button", { name: "删除 Primary production" }),
    ).toHaveAttribute("title", "请先将另一凭据设为首选，再删除当前首选。");

    const backupRow = screen.getByRole("listitem", { name: "Backup quota" });
    await user.click(within(backupRow).getByRole("button", { name: "设为首选" }));
    expect(bridge.activateSessionCredential).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
      credentialId: "credential-backup",
    });
    expect(within(screen.getByRole("listitem", { name: "Backup quota" })).getByText("当前首选"))
      .toBeVisible();

    const activeRow = screen.getByRole("listitem", { name: "Backup quota" });
    expect(within(activeRow).getByRole("button", { name: "下移 Backup quota" })).toBeDisabled();
    const primaryRow = screen.getByRole("listitem", { name: "Primary production" });
    expect(within(primaryRow).getByRole("button", { name: "上移 Primary production" })).toBeDisabled();
    const emergencyRow = screen.getByRole("listitem", { name: "Emergency only" });
    await user.click(within(emergencyRow).getByRole("button", { name: "上移 Emergency only" }));
    expect(bridge.reorderSessionCredentials).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
      orderedCredentialIds: [
        "credential-backup",
        "credential-emergency",
        "credential-primary",
      ],
    });

    await user.click(
      within(screen.getByRole("listitem", { name: "Primary production" }))
        .getByRole("button", { name: "删除 Primary production" }),
    );
    expect(bridge.removeSessionCredential).not.toHaveBeenCalled();
    const confirm = screen.getByRole("button", { name: "确认删除 Primary production" });
    expect(confirm).toHaveFocus();
    await user.click(confirm);
    expect(bridge.removeSessionCredential).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
      credentialId: "credential-primary",
    });
    expect(screen.queryByText("Primary production")).not.toBeInTheDocument();
  });

  it("shows bridge failures, focuses the alert, and clears rejected passwords", async () => {
    const user = userEvent.setup();
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([]),
      setSessionCredential: vi.fn().mockRejectedValue(new Error("凭据标签已存在。")),
    });

    render(
      <SessionCredentialManager
        bridge={bridge}
        providerProfileId="provider-openai"
      />,
    );

    await screen.findByText("本次会话还没有凭据。");
    await user.type(screen.getByLabelText("凭据标签"), "Duplicate");
    const password = screen.getByLabelText("API Key（仅本次会话）");
    await user.type(password, "secret-rejected-by-core");
    await user.click(screen.getByRole("button", { name: "添加会话凭据" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("凭据标签已存在。");
    expect(alert).toHaveFocus();
    expect(password).toHaveValue("");
    expect(document.body).not.toHaveTextContent("secret-rejected-by-core");
  });
});
