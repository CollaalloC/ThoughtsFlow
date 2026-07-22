import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, test } from "node:test";

import { ProviderFixture } from "./provider-server.ts";

type NewProviderDialect = "anthropic" | "google";

function streamingRequest(
  dialect: NewProviderDialect,
  prompt: string,
  signal?: AbortSignal,
): { url: string; init: RequestInit } {
  if (dialect === "anthropic") {
    return {
      url: "/v1/messages",
      init: {
        method: "POST",
        signal,
        headers: {
          accept: "text/event-stream",
          "content-type": "application/json",
          "x-api-key": "fixture-anthropic-key",
          "anthropic-version": "2023-06-01",
        },
        body: JSON.stringify({
          model: "claude-fixture",
          max_tokens: 256,
          stream: true,
          system: [{ type: "text", text: "fixture system" }],
          messages: [{ role: "user", content: prompt }],
          temperature: 0.2,
          top_p: 0.9,
          stop_sequences: ["fixture-stop"],
        }),
      },
    };
  }

  return {
    url: "/v1beta/models/fixture-model:streamGenerateContent?alt=sse",
    init: {
      method: "POST",
      signal,
      headers: {
        accept: "text/event-stream",
        "content-type": "application/json",
        "x-goog-api-key": "fixture-google-key",
      },
      body: JSON.stringify({
        systemInstruction: { parts: [{ text: "fixture system" }] },
        contents: [{ role: "user", parts: [{ text: prompt }] }],
        generationConfig: {
          temperature: 0.2,
          topP: 0.9,
          maxOutputTokens: 256,
          stopSequences: ["fixture-stop"],
        },
      }),
    },
  };
}

async function postStream(
  provider: ProviderFixture,
  dialect: NewProviderDialect,
  prompt: string,
  signal?: AbortSignal,
) {
  const request = streamingRequest(dialect, prompt, signal);
  return fetch(`${provider.baseUrl}${request.url}`, request.init);
}

async function readChunks(response: Response): Promise<{ chunks: Uint8Array[]; text: string }> {
  const reader = response.body?.getReader();
  assert.ok(reader);
  const chunks: Uint8Array[] = [];
  const decoder = new TextDecoder();
  let text = "";
  while (true) {
    const result = await reader.read();
    if (result.done) break;
    chunks.push(result.value);
    text += decoder.decode(result.value, { stream: true });
  }
  text += decoder.decode();
  return { chunks, text };
}

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

  test("validates Anthropic headers/body and serves a complete Messages SSE stream", async () => {
    const prompt = "anthropic fixture contract";
    const response = await postStream(provider, "anthropic", prompt);

    assert.equal(response.status, 200);
    assert.match(response.headers.get("content-type") ?? "", /text\/event-stream/);
    const stream = await response.text();
    assert.match(stream, /event: message_start/);
    assert.match(stream, /event: content_block_start/);
    assert.match(stream, /"type":"thinking_delta"/);
    assert.match(stream, /Fixture 思考/);
    assert.match(stream, new RegExp(`Fixture 回答 #1：${prompt}`));
    assert.match(stream, /"type":"message_delta"/);
    assert.match(stream, /"input_tokens":7/);
    assert.match(stream, /"output_tokens":3/);
    assert.match(stream, /event: message_stop/);

    const missingCredential = streamingRequest("anthropic", "invalid headers");
    delete (missingCredential.init.headers as Record<string, string>)["x-api-key"];
    const missingCredentialResponse = await fetch(
      `${provider.baseUrl}${missingCredential.url}`,
      missingCredential.init,
    );
    assert.equal(missingCredentialResponse.status, 401);

    const wrongVersion = streamingRequest("anthropic", "invalid version");
    (wrongVersion.init.headers as Record<string, string>)["anthropic-version"] = "2024-01-01";
    const wrongVersionResponse = await fetch(
      `${provider.baseUrl}${wrongVersion.url}`,
      wrongVersion.init,
    );
    assert.equal(wrongVersionResponse.status, 400);

    const invalidBody = streamingRequest("anthropic", "invalid body");
    invalidBody.init.body = JSON.stringify({ model: "claude-fixture", stream: false, messages: [] });
    const invalidBodyResponse = await fetch(
      `${provider.baseUrl}${invalidBody.url}`,
      invalidBody.init,
    );
    assert.equal(invalidBodyResponse.status, 400);
  });

  test("validates Google headers/body and serves a complete GenerateContent SSE stream", async () => {
    const prompt = "google fixture contract";
    const response = await postStream(provider, "google", prompt);

    assert.equal(response.status, 200);
    assert.match(response.headers.get("content-type") ?? "", /text\/event-stream/);
    const stream = await response.text();
    assert.match(stream, /"responseId":"resp-fixture"/);
    assert.match(stream, /"thought":true/);
    assert.match(stream, /Fixture 思考/);
    assert.match(stream, new RegExp(`Fixture 回答 #1：${prompt}`));
    assert.match(stream, /"finishReason":"STOP"/);
    assert.match(stream, /"promptTokenCount":7/);
    assert.match(stream, /"candidatesTokenCount":2/);
    assert.match(stream, /"totalTokenCount":9/);

    const missingCredential = streamingRequest("google", "invalid headers");
    delete (missingCredential.init.headers as Record<string, string>)["x-goog-api-key"];
    const missingCredentialResponse = await fetch(
      `${provider.baseUrl}${missingCredential.url}`,
      missingCredential.init,
    );
    assert.equal(missingCredentialResponse.status, 401);

    const missingSseQuery = streamingRequest("google", "invalid query");
    missingSseQuery.url = "/v1beta/models/fixture-model:streamGenerateContent";
    const missingSseQueryResponse = await fetch(
      `${provider.baseUrl}${missingSseQuery.url}`,
      missingSseQuery.init,
    );
    assert.equal(missingSseQueryResponse.status, 400);

    const invalidBody = streamingRequest("google", "invalid body");
    invalidBody.init.body = JSON.stringify({ contents: [] });
    const invalidBodyResponse = await fetch(
      `${provider.baseUrl}${invalidBody.url}`,
      invalidBody.init,
    );
    assert.equal(invalidBodyResponse.status, 400);
  });

  test("emits deterministic CRLF streams over arbitrary transport fragments", async () => {
    for (const dialect of ["anthropic", "google"] satisfies NewProviderDialect[]) {
      const response = await postStream(provider, dialect, `[chunks] ${dialect}`);
      assert.equal(response.status, 200);
      const streamed = await readChunks(response);
      assert.ok(streamed.chunks.length >= 3, `${dialect} fixture should force multiple reads`);
      assert.match(streamed.text, /\r\n/);
      if (dialect === "anthropic") assert.match(streamed.text, /event: message_stop/);
      else assert.match(streamed.text, /"finishReason":"STOP"/);
    }
  });

  test("serves deterministic protocol and HTTP failures for both new dialects", async () => {
    for (const dialect of ["anthropic", "google"] satisfies NewProviderDialect[]) {
      const protocolError = await postStream(provider, dialect, `[protocol-error] ${dialect}`);
      assert.equal(protocolError.status, 200);
      assert.match(protocolError.headers.get("content-type") ?? "", /text\/event-stream/);
      assert.match(await protocolError.text(), /data: \{not-json/);

      const streamError = await postStream(provider, dialect, `[stream-error] ${dialect}`);
      assert.equal(streamError.status, 200);
      const streamErrorBody = await streamError.text();
      if (dialect === "anthropic") assert.match(streamErrorBody, /overloaded_error/);
      else assert.match(streamErrorBody, /RESOURCE_EXHAUSTED/);

      const httpError = await postStream(provider, dialect, `[http-error] ${dialect}`);
      assert.equal(httpError.status, 429);
      assert.match(httpError.headers.get("content-type") ?? "", /application\/json/);
      const body = (await httpError.json()) as {
        type?: string;
        error?: { code?: number; message?: string; status?: string; type?: string };
      };
      assert.match(body.error?.message ?? "", /Fixture rate limit/);
      if (dialect === "anthropic") {
        assert.equal(body.type, "error");
        assert.equal(body.error?.type, "rate_limit_error");
      } else {
        assert.equal(body.error?.code, 429);
        assert.equal(body.error?.status, "RESOURCE_EXHAUSTED");
      }
    }
  });

  test("supports disconnect, hang, and client cancellation for both new dialects", async () => {
    for (const dialect of ["anthropic", "google"] satisfies NewProviderDialect[]) {
      const disconnected = await postStream(provider, dialect, `[disconnect] ${dialect}`);
      await assert.rejects(disconnected.text());

      const hanging = await postStream(provider, dialect, `[hang] ${dialect}`);
      const hangingReader = hanging.body?.getReader();
      assert.ok(hangingReader);
      assert.equal((await hangingReader.read()).done, false);
      const pendingRead = hangingReader.read();
      const pendingState = await Promise.race([
        pendingRead.then(() => "settled", () => "settled"),
        new Promise<"pending">((resolve) => setTimeout(() => resolve("pending"), 40)),
      ]);
      assert.equal(pendingState, "pending");
      await hangingReader.cancel();

      const controller = new AbortController();
      const cancelled = await postStream(provider, dialect, `[cancel] ${dialect}`, controller.signal);
      const cancelledReader = cancelled.body?.getReader();
      assert.ok(cancelledReader);
      assert.equal((await cancelledReader.read()).done, false);
      const pendingCancelledRead = cancelledReader.read();
      controller.abort();
      const cancellationOutcome = await settleWithin(
        pendingCancelledRead.then(
          () => ({ status: "resolved" as const, error: undefined }),
          (error: unknown) => ({ status: "rejected" as const, error }),
        ),
        500,
      );
      assert.equal(cancellationOutcome.status, "rejected");
      const cancellationError = cancellationOutcome.error as { name?: unknown } | undefined;
      assert.equal(cancellationError?.name, "AbortError");
    }

    const healthy = await fetch(`${provider.baseUrl}/v1/models`);
    assert.equal(healthy.status, 200);
  });
});
