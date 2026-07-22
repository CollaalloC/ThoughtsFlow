import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import { ProviderSettings } from "./ProviderSettings";

describe("ProviderSettings", () => {
  it("saves credentials only to the Rust session and explains the outbound boundary", async () => {
    const bridge = {
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      saveProviderProfile: vi.fn().mockResolvedValue({ id: "provider-1" }),
      setSessionCredential: vi.fn().mockResolvedValue(undefined),
      testProviderConnection: vi.fn().mockResolvedValue({
        ok: true,
        message: "连接成功",
      }),
    } as unknown as DesktopBridge;

    render(<ProviderSettings bridge={bridge} />);

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
      expect(bridge.setSessionCredential).toHaveBeenCalledWith({
        providerProfileId: "provider-1",
        credential: "secret-value",
      }),
    );
    expect(bridge.saveProviderProfile).toHaveBeenCalledWith(
      expect.not.objectContaining({ apiKey: expect.anything() }),
    );
    expect(screen.getByText(/工作区内容保存在本机/)).toBeVisible();
    expect(screen.getByText(/选中的 Context 会发送到 llm\.example\.com/)).toBeVisible();
  });

  it("rejects insecure non-loopback HTTP endpoints before saving", () => {
    const bridge = {
      listProviderProfiles: vi.fn().mockResolvedValue([]),
      saveProviderProfile: vi.fn(),
      setSessionCredential: vi.fn(),
      testProviderConnection: vi.fn(),
    } as unknown as DesktopBridge;
    render(<ProviderSettings bridge={bridge} />);

    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "http://llm.example.com/v1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存 Provider" }));

    expect(screen.getByRole("alert")).toHaveTextContent("远程端点必须使用 HTTPS");
    expect(bridge.saveProviderProfile).not.toHaveBeenCalled();
  });
});
