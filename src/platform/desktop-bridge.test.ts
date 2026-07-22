import { describe, expect, it } from "vitest";

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
});
