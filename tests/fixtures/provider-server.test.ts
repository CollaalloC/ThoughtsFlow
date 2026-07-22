import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, test } from "node:test";

import { ProviderFixture } from "./provider-server.ts";

async function settleWithin<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error("Provider fixture stop timed out")), timeoutMs);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

describe("ProviderFixture", () => {
  let provider: ProviderFixture;

  beforeEach(async () => {
    provider = new ProviderFixture();
    await provider.start();
  });

  afterEach(async () => {
    await provider.stop();
  });

  test("serves deterministic model catalogs for empty-body GET requests", async () => {
    const credential = "fixture-secret-must-not-be-reflected";
    const headers = {
      authorization: `Bearer ${credential}`,
      "x-api-key": credential,
    };

    const [openAiResponse, ollamaResponse, googleResponse] = await Promise.all([
      fetch(`${provider.baseUrl}/v1/models`, { headers }),
      fetch(`${provider.baseUrl}/api/tags`, { headers }),
      fetch(`${provider.baseUrl}/v1beta/models`, { headers }),
    ]);

    assert.equal(openAiResponse.status, 200);
    assert.equal(ollamaResponse.status, 200);
    assert.equal(googleResponse.status, 200);

    const openAiBody = await openAiResponse.text();
    const ollamaBody = await ollamaResponse.text();
    const googleBody = await googleResponse.text();

    assert.equal(JSON.parse(openAiBody).data[0].id, "fixture-model");
    assert.equal(JSON.parse(ollamaBody).models[0].model, "fixture-model");
    assert.deepEqual(JSON.parse(googleBody).models[0], {
      name: "models/fixture-model",
      displayName: "fixture-model",
      supportedGenerationMethods: ["generateContent", "streamGenerateContent"],
    });
    assert.doesNotMatch(`${openAiBody}${ollamaBody}${googleBody}`, new RegExp(credential));
  });

  test("preserves OpenAI-compatible and Ollama streaming POST behavior", async () => {
    const prompt = "fixture contract prompt";
    const body = JSON.stringify({ messages: [{ role: "user", content: prompt }] });

    const openAiResponse = await fetch(`${provider.baseUrl}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
    });
    const ollamaResponse = await fetch(`${provider.baseUrl}/api/chat`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
    });

    assert.equal(openAiResponse.status, 200);
    assert.match(openAiResponse.headers.get("content-type") ?? "", /text\/event-stream/);
    assert.match(await openAiResponse.text(), new RegExp(`Fixture 回答 #1：${prompt}`));

    assert.equal(ollamaResponse.status, 200);
    assert.match(ollamaResponse.headers.get("content-type") ?? "", /application\/x-ndjson/);
    assert.match(await ollamaResponse.text(), new RegExp(`Fixture 回答 #2：${prompt}`));
  });

  test("stops active hanging streams and can restart without leaked fixture state", async () => {
    const response = await fetch(`${provider.baseUrl}/v1/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ messages: [{ role: "user", content: "[hang]" }] }),
    });
    const reader = response.body?.getReader();
    assert.ok(reader);
    assert.match(new TextDecoder().decode((await reader.read()).value), /Fixture 回答 #1：\[hang\]/);

    await settleWithin(provider.stop(), 1_000);
    await reader.cancel().catch(() => undefined);

    await provider.start();
    const restarted = await fetch(`${provider.baseUrl}/v1/models`);
    assert.equal(restarted.status, 200);
  });
});
