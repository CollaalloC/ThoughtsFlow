import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { createDesktopBridge, type DesktopBridge } from "../../platform/desktop-bridge";
import type { ModelRun, SessionCredentialSummary } from "../../shared/contracts";
import { CredentialRecovery } from "./CredentialRecovery";

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

const failedRun: ModelRun = {
  id: "run-failed",
  turnId: "turn-1",
  status: "failed",
  output: "已保留的部分回答",
  providerProfileId: "provider-openai",
  providerName: "OpenAI",
  model: "gpt-test",
  baseUrl: "https://api.openai.com/v1",
  createdAt: "2026-07-23T00:00:00Z",
  error: {
    code: "quota_exhausted",
    message: "当前凭据额度已耗尽。",
    retryable: true,
    status: 429,
  },
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

describe("CredentialRecovery", () => {
  it.each(["quota_exhausted", "rate_limited"])(
    "offers an explicit recovery for a retryable %s failure",
    (code) => {
      const bridge = createBridge();
      const run = { ...failedRun, error: { ...failedRun.error!, code } };

      render(
        <CredentialRecovery
          bridge={bridge}
          run={run}
          onRetryWithCredential={vi.fn()}
        />,
      );

      expect(screen.getByRole("region", { name: "凭据恢复" })).toBeVisible();
      expect(screen.getByText(/旧的失败回答和部分输出会原样保留/)).toBeVisible();
      expect(screen.getByText(/会新增一个回答版本/)).toBeVisible();
      expect(screen.getByRole("button", { name: "选择备用凭据" })).toHaveAttribute(
        "aria-expanded",
        "false",
      );
      expect(bridge.listSessionCredentials).not.toHaveBeenCalled();
    },
  );

  it.each([
    ["not retryable", { ...failedRun, error: { ...failedRun.error!, retryable: false } }],
    ["other code", { ...failedRun, error: { ...failedRun.error!, code: "provider_error" } }],
    ["no error", { ...failedRun, error: null }],
  ])("does not render for an ineligible run: %s", (_label, run) => {
    const { container } = render(
      <CredentialRecovery
        bridge={createBridge()}
        run={run}
        onRetryWithCredential={vi.fn()}
      />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("loads on keyboard expansion and selects the next non-active safe label", async () => {
    const user = userEvent.setup();
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        emergencyCredential,
        primaryCredential,
        backupCredential,
      ]),
    });

    render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={vi.fn()}
      />,
    );

    screen.getByRole("button", { name: "选择备用凭据" }).focus();
    await user.keyboard("{Enter}");

    expect(await screen.findByRole("combobox", { name: "备用凭据" })).toHaveValue(
      "credential-backup",
    );
    expect(screen.getByRole("option", { name: "Backup quota" })).toBeVisible();
    expect(screen.getByRole("option", { name: "Emergency only" })).toBeVisible();
    expect(screen.queryByRole("option", { name: "Primary production" })).not.toBeInTheDocument();
    expect(bridge.listSessionCredentials).toHaveBeenCalledTimes(1);
    expect(bridge.listSessionCredentials).toHaveBeenCalledWith({
      providerProfileId: "provider-openai",
    });
  });

  it("submits the selected backup through one atomic recovery callback", async () => {
    const user = userEvent.setup();
    const recovery = deferred<void>();
    const onRetryWithCredential = vi.fn(() => recovery.promise);
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
      ]),
    });

    render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={onRetryWithCredential}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole("combobox", { name: "备用凭据" });
    await user.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    expect(onRetryWithCredential).toHaveBeenCalledWith(
      "provider-openai",
      "credential-backup",
    );
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();

    recovery.resolve();
    await vi.waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("已用所选凭据");
    });
  });

  it("shows an atomic recovery failure without invoking a separate activation", async () => {
    const user = userEvent.setup();
    const onRetryWithCredential = vi.fn().mockRejectedValue(
      new Error("备用凭据已从会话中移除。"),
    );
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
      ]),
    });

    render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={onRetryWithCredential}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole("combobox", { name: "备用凭据" });
    await user.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("备用凭据已从会话中移除。");
    expect(onRetryWithCredential).toHaveBeenCalledTimes(1);
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
  });

  it("does not automatically repeat a failed atomic recovery", async () => {
    const user = userEvent.setup();
    const onRetryWithCredential = vi.fn().mockRejectedValue(new Error("新回答启动失败。"));
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
      ]),
    });

    render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={onRetryWithCredential}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole("combobox", { name: "备用凭据" });
    await user.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("新回答启动失败。");
    expect(onRetryWithCredential).toHaveBeenCalledTimes(1);
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
  });

  it("does not write stale completion state when the selected run changes", async () => {
    const user = userEvent.setup();
    const recovery = deferred<void>();
    const onRetryWithCredential = vi.fn(() => recovery.promise);
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([
        primaryCredential,
        backupCredential,
      ]),
    });
    const { rerender } = render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={onRetryWithCredential}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择备用凭据" }));
    await screen.findByRole("combobox", { name: "备用凭据" });
    await user.click(screen.getByRole("button", { name: "切换并新增回答版本" }));

    rerender(
      <CredentialRecovery
        bridge={bridge}
        run={{ ...failedRun, id: "run-new-selection" }}
        onRetryWithCredential={onRetryWithCredential}
      />,
    );
    await act(async () => {
      recovery.resolve();
      await recovery.promise;
    });
    expect(onRetryWithCredential).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(/已用所选凭据/)).not.toBeInTheDocument();
  });

  it("opens settings for the failed run's profile when no backup exists", async () => {
    const user = userEvent.setup();
    const onOpenSettings = vi.fn();
    const onRetryWithCredential = vi.fn();
    const bridge = createBridge({
      listSessionCredentials: vi.fn().mockResolvedValue([primaryCredential]),
    });

    render(
      <CredentialRecovery
        bridge={bridge}
        run={failedRun}
        onRetryWithCredential={onRetryWithCredential}
        onOpenSettings={onOpenSettings}
      />,
    );

    await user.click(screen.getByRole("button", { name: "选择备用凭据" }));
    expect(await screen.findByText("当前会话没有可用的备用凭据。")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "打开此 Provider 设置" }));

    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    expect(onOpenSettings).toHaveBeenCalledWith("provider-openai");
    expect(bridge.activateSessionCredential).not.toHaveBeenCalled();
    expect(onRetryWithCredential).not.toHaveBeenCalled();
  });
});
