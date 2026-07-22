import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ContextInspector, type LockedSnapshot } from "./ContextInspector";

const snapshot: LockedSnapshot = {
  canonicalHash: "sha256:receipt",
  createdAt: "2026-07-22T09:11:00Z",
  providerMetadataStatus: "resolved",
  providerId: "anthropic",
  templateRevision: 1,
  providerName: "Anthropic",
  streamProtocol: "anthropic_sse",
  authPlacement: "api_key_header",
  authHeaderName: "x-api-key",
  additionalHeaders: {
    "anthropic-version": "2023-06-01",
    Authorization: "must-not-render",
  },
  model: "claude-sonnet",
  baseUrl: "https://api.anthropic.com",
  parameters: {
    temperature: 0.1,
    stop: ["END"],
  },
  items: [],
};

describe("ContextInspector locked receipt", () => {
  it("renders resolved protocol, auth, non-secret headers, and effective parameters", () => {
    render(
      <ContextInspector
        open
        items={[]}
        estimatedTokens={0}
        limitTokens={8192}
        warnings={[]}
        blocked={false}
        provider={{
          name: "Anthropic",
          model: "claude-sonnet",
          baseUrl: "https://api.anthropic.com",
          local: false,
        }}
        snapshot={snapshot}
        runs={[]}
        onClose={vi.fn()}
        onToggleIncluded={vi.fn()}
        onTogglePinned={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("tab", { name: "本次实际发送的内容" }));

    expect(screen.getByText("anthropic r1")).toBeVisible();
    expect(screen.getByText("Anthropic SSE")).toBeVisible();
    expect(screen.getByText("x-api-key: API Key …")).toBeVisible();
    expect(screen.getByText("anthropic-version")).toBeVisible();
    expect(screen.getByText("2023-06-01")).toBeVisible();
    expect(screen.queryByText("must-not-render")).not.toBeInTheDocument();
    expect(screen.getByText("temperature")).toBeVisible();
    expect(screen.getByText("0.1")).toBeVisible();
    expect(screen.getByText('["END"]')).toBeVisible();
  });
});
