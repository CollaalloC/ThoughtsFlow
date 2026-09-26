import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { Socket } from "node:net";

/**
 * An inspectable copy of a request that reached the fixture. Credentials are
 * deliberately represented only by their presence, never by their value.
 */
export type ProviderRequestSnapshot = {
  method: string;
  pathname: string;
  search: string;
  headers: Record<string, string>;
  body: Record<string, unknown>;
};

const retainedHeaderNames = new Set([
  "accept",
  "content-type",
  "anthropic-version",
]);
const redactedHeaderNames = new Set([
  "authorization",
  "proxy-authorization",
  "x-api-key",
  "x-goog-api-key",
]);

function capturedHeaders(request: IncomingMessage): Record<string, string> {
  return Object.fromEntries(
    Object.entries(request.headers).flatMap(([name, value]) => {
      const normalized = name.toLowerCase();
      if (redactedHeaderNames.has(normalized)) return [[normalized, "[redacted]"]];
      if (!retainedHeaderNames.has(normalized)) return [];
      return [[normalized, Array.isArray(value) ? value.join(",") : (value ?? "")]];
    }),
  );
}

function copyBody(body: Record<string, unknown>): Record<string, unknown> {
  // JSON bodies are the only protocol accepted by this fixture. A serialized
  // copy keeps later test mutation from altering the evidence it asserts on.
  return JSON.parse(JSON.stringify(body)) as Record<string, unknown>;
}

async function readJson(request: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.from(chunk));
  return JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>;
}

function lastPrompt(body: Record<string, unknown>): string {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const last = messages.at(-1);
  if (!last || typeof last !== "object") return "";
  const content = (last as Record<string, unknown>).content;
  return typeof content === "string" ? content : "";
}

function writeJson(response: ServerResponse, body: Record<string, unknown>) {
  response.writeHead(200, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(JSON.stringify(body));
}

function writeModelCatalog(pathname: string, response: ServerResponse): boolean {
  if (pathname === "/v1/models") {
    writeJson(response, {
      object: "list",
      data: [
        {
          id: "fixture-model",
          object: "model",
          created: 1_704_067_200,
          owned_by: "thoughtsflow-fixture",
        },
      ],
    });
    return true;
  }

  if (pathname === "/api/tags") {
    writeJson(response, {
      models: [
        {
          name: "fixture-model",
          model: "fixture-model",
          modified_at: "2024-01-01T00:00:00Z",
          size: 1_024,
          digest: "sha256:fixture-model",
          details: {
            format: "gguf",
            family: "fixture",
            parameter_size: "1B",
            quantization_level: "Q4_0",
          },
        },
      ],
    });
    return true;
  }

  if (pathname === "/v1beta/models") {
    writeJson(response, {
      models: [
        {
          name: "models/fixture-model",
          displayName: "fixture-model",
          supportedGenerationMethods: ["generateContent", "streamGenerateContent"],
        },
      ],
    });
    return true;
  }

  return false;
}

type Schedule = (callback: () => void, delay: number) => void;

function headerValue(request: IncomingMessage, name: string): string {
  const value = request.headers[name.toLowerCase()];
  return Array.isArray(value) ? value.join(",") : (value ?? "");
}

function writeHttpError(
  response: ServerResponse,
  status: number,
  message: string,
  code = "fixture_error",
) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(JSON.stringify({ error: { code, message } }));
}

function writeAnthropicApiError(response: ServerResponse, status: number, message: string) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(
    JSON.stringify({
      type: "error",
      error: { type: "rate_limit_error", message },
      request_id: "req_fixture",
    }),
  );
}

function writeGoogleApiError(response: ServerResponse, status: number, message: string) {
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
  });
  response.end(
    JSON.stringify({
      error: { code: status, message, status: "RESOURCE_EXHAUSTED" },
    }),
  );
}

function validateStreamingHeaders(
  request: IncomingMessage,
  response: ServerResponse,
  credentialHeader: "x-api-key" | "x-goog-api-key",
): boolean {
  if (!headerValue(request, credentialHeader).trim()) {
    writeHttpError(response, 401, `Missing ${credentialHeader}`, "missing_credential");
    return false;
  }
  if (!headerValue(request, "content-type").toLowerCase().includes("application/json")) {
    writeHttpError(response, 400, "Content-Type must be application/json", "invalid_headers");
    return false;
  }
  if (!headerValue(request, "accept").toLowerCase().includes("text/event-stream")) {
    writeHttpError(response, 400, "Accept must include text/event-stream", "invalid_headers");
    return false;
  }
  return true;
}

function googleLastPrompt(body: Record<string, unknown>): string {
  const contents = Array.isArray(body.contents) ? body.contents : [];
  const last = contents.at(-1);
  if (!last || typeof last !== "object") return "";
  const parts = Array.isArray((last as Record<string, unknown>).parts)
    ? ((last as Record<string, unknown>).parts as unknown[])
    : [];
  return parts
    .filter((part): part is Record<string, unknown> => Boolean(part) && typeof part === "object")
    .map((part) => part.text)
    .filter((text): text is string => typeof text === "string")
    .join("");
}

function isOptionalNumber(value: unknown): boolean {
  return value === undefined || typeof value === "number";
}

function isOptionalStringArray(value: unknown): boolean {
  return (
    value === undefined ||
    (Array.isArray(value) && value.every((item) => typeof item === "string"))
  );
}

function hasTextParts(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  const parts = Array.isArray((value as Record<string, unknown>).parts)
    ? ((value as Record<string, unknown>).parts as unknown[])
    : [];
  return (
    parts.length > 0 &&
    parts.every(
      (part) =>
        Boolean(part) &&
        typeof part === "object" &&
        typeof (part as Record<string, unknown>).text === "string",
    )
  );
}

function hasValidAnthropicSystem(value: unknown): boolean {
  if (value === undefined || typeof value === "string") return true;
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every(
      (block) =>
        Boolean(block) &&
        typeof block === "object" &&
        (block as Record<string, unknown>).type === "text" &&
        typeof (block as Record<string, unknown>).text === "string",
    )
  );
}

function hasValidAnthropicBody(body: Record<string, unknown>): boolean {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  return (
    typeof body.model === "string" &&
    body.model.length > 0 &&
    typeof body.max_tokens === "number" &&
    body.max_tokens > 0 &&
    body.stream === true &&
    hasValidAnthropicSystem(body.system) &&
    isOptionalNumber(body.temperature) &&
    isOptionalNumber(body.top_p) &&
    isOptionalStringArray(body.stop_sequences) &&
    messages.length > 0 &&
    messages.every((message) => {
      if (!message || typeof message !== "object") return false;
      const record = message as Record<string, unknown>;
      return (
        (record.role === "user" || record.role === "assistant") &&
        typeof record.content === "string"
      );
    })
  );
}

function hasValidGoogleBody(body: Record<string, unknown>): boolean {
  const contents = Array.isArray(body.contents) ? body.contents : [];
  const generationConfig = body.generationConfig;
  const hasValidGenerationConfig =
    generationConfig === undefined ||
    (Boolean(generationConfig) &&
      typeof generationConfig === "object" &&
      isOptionalNumber((generationConfig as Record<string, unknown>).temperature) &&
      isOptionalNumber((generationConfig as Record<string, unknown>).topP) &&
      isOptionalNumber((generationConfig as Record<string, unknown>).maxOutputTokens) &&
      isOptionalStringArray((generationConfig as Record<string, unknown>).stopSequences));
  return (
    (body.systemInstruction === undefined || hasTextParts(body.systemInstruction)) &&
    hasValidGenerationConfig &&
    contents.length > 0 &&
    contents.every((content) => {
      if (!content || typeof content !== "object") return false;
      const record = content as Record<string, unknown>;
      return (
        (record.role === "user" || record.role === "model") &&
        hasTextParts(record)
      );
    })
  );
}

function sseEvent(event: string | null, data: Record<string, unknown>, crlf = false): string {
  const newline = crlf ? "\r\n" : "\n";
  const eventLine = event ? `event: ${event}${newline}` : "";
  return `${eventLine}data: ${JSON.stringify(data)}${newline}${newline}`;
}

function writeFragmented(
  response: ServerResponse,
  payload: string,
  schedule: Schedule,
) {
  const bytes = Buffer.from(payload, "utf8");
  const fragmentSizes = [1, 2, 7, 13, 29, 5, 61, 127];
  let offset = 0;
  let fragment = 0;

  const writeNext = () => {
    if (response.destroyed || response.writableEnded) return;
    if (offset >= bytes.length) {
      response.end();
      return;
    }
    const size = fragmentSizes[fragment % fragmentSizes.length] ?? 1;
    const end = Math.min(bytes.length, offset + size);
    response.write(bytes.subarray(offset, end));
    offset = end;
    fragment += 1;
    schedule(writeNext, 2);
  };

  writeNext();
}

function writeOpenAiStream(
  response: ServerResponse,
  answer: string,
  prompt: string,
  schedule: (callback: () => void, delay: number) => void,
) {
  response.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });
  response.write(
    `data: ${JSON.stringify({ choices: [{ delta: { content: answer } }] })}\n\n`,
  );

  if (prompt.includes("[disconnect]")) {
    schedule(() => response.destroy(), 250);
    return;
  }

  if (prompt.includes("[hang]")) {
    return;
  }

  const complete = () => {
    if (response.destroyed || response.writableEnded) return;
    response.write(`data: ${JSON.stringify({ choices: [{ delta: { content: "（完成）" } }] })}\n\n`);
    response.end("data: [DONE]\n\n");
  };

  if (prompt.includes("[slow]")) schedule(complete, 5_000);
  else complete();
}

function writeOllamaStream(
  response: ServerResponse,
  answer: string,
  prompt: string,
  schedule: (callback: () => void, delay: number) => void,
) {
  response.writeHead(200, { "content-type": "application/x-ndjson" });
  response.write(`${JSON.stringify({ message: { content: answer }, done: false })}\n`);

  if (prompt.includes("[disconnect]")) {
    schedule(() => response.destroy(), 250);
    return;
  }

  if (prompt.includes("[hang]")) {
    return;
  }

  const complete = () => {
    if (response.destroyed || response.writableEnded) return;
    response.write(`${JSON.stringify({ message: { content: "（完成）" }, done: false })}\n`);
    response.end(`${JSON.stringify({ done: true, done_reason: "stop" })}\n`);
  };

  if (prompt.includes("[slow]")) schedule(complete, 5_000);
  else complete();
}

function writeAnthropicStream(
  response: ServerResponse,
  answer: string,
  prompt: string,
  schedule: Schedule,
) {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });

  if (prompt.includes("[protocol-error]")) {
    response.end("event: content_block_delta\ndata: {not-json\n\n");
    return;
  }
  if (prompt.includes("[stream-error]")) {
    response.end(
      sseEvent("error", {
        type: "error",
        error: { type: "overloaded_error", message: "Overloaded" },
      }),
    );
    return;
  }

  const crlf = prompt.includes("[chunks]");
  const opening = [
    sseEvent(
      "message_start",
      {
        type: "message_start",
        message: {
          id: "msg_fixture",
          type: "message",
          role: "assistant",
          content: [],
          model: "claude-fixture",
          stop_reason: null,
          stop_sequence: null,
          usage: { input_tokens: 7, output_tokens: 1 },
        },
      },
      crlf,
    ),
    sseEvent("ping", { type: "ping" }, crlf),
    sseEvent(
      "content_block_start",
      {
        type: "content_block_start",
        index: 0,
        content_block: { type: "thinking", thinking: "", signature: "" },
      },
      crlf,
    ),
    sseEvent(
      "content_block_delta",
      {
        type: "content_block_delta",
        index: 0,
        delta: { type: "thinking_delta", thinking: "Fixture 思考" },
      },
      crlf,
    ),
    sseEvent(
      "content_block_delta",
      {
        type: "content_block_delta",
        index: 0,
        delta: { type: "signature_delta", signature: "fixture-signature" },
      },
      crlf,
    ),
    sseEvent("content_block_stop", { type: "content_block_stop", index: 0 }, crlf),
    sseEvent(
      "content_block_start",
      {
        type: "content_block_start",
        index: 1,
        content_block: { type: "text", text: "" },
      },
      crlf,
    ),
    sseEvent(
      "content_block_delta",
      {
        type: "content_block_delta",
        index: 1,
        delta: { type: "text_delta", text: answer },
      },
      crlf,
    ),
  ].join("");
  const completion = [
    sseEvent("content_block_stop", { type: "content_block_stop", index: 1 }, crlf),
    sseEvent(
      "message_delta",
      {
        type: "message_delta",
        delta: { stop_reason: "end_turn", stop_sequence: null },
        usage: { output_tokens: 3 },
      },
      crlf,
    ),
    sseEvent("message_stop", { type: "message_stop" }, crlf),
  ].join("");

  if (crlf) {
    writeFragmented(response, opening + completion, schedule);
    return;
  }

  response.write(opening);
  if (prompt.includes("[disconnect]")) {
    schedule(() => response.destroy(), 250);
    return;
  }
  if (prompt.includes("[hang]") || prompt.includes("[cancel]")) return;

  const complete = () => {
    if (response.destroyed || response.writableEnded) return;
    response.end(completion);
  };
  if (prompt.includes("[slow]")) schedule(complete, 5_000);
  else complete();
}

function writeGoogleStream(
  response: ServerResponse,
  answer: string,
  prompt: string,
  schedule: Schedule,
) {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });

  if (prompt.includes("[protocol-error]")) {
    response.end("data: {not-json\n\n");
    return;
  }
  if (prompt.includes("[stream-error]")) {
    response.end(
      sseEvent(null, {
        error: { code: 429, message: "fixture quota", status: "RESOURCE_EXHAUSTED" },
      }),
    );
    return;
  }

  const crlf = prompt.includes("[chunks]");
  const opening = sseEvent(
    null,
    {
      responseId: "resp-fixture",
      modelVersion: "gemini-fixture",
      candidates: [
        {
          index: 0,
          content: {
            role: "model",
            parts: [
              { text: "Fixture 思考", thought: true },
              { text: answer },
            ],
          },
        },
      ],
    },
    crlf,
  );
  const completion = sseEvent(
    null,
    {
      responseId: "resp-fixture",
      modelVersion: "gemini-fixture",
      candidates: [{ index: 0, finishReason: "STOP" }],
      usageMetadata: {
        promptTokenCount: 7,
        candidatesTokenCount: 2,
        totalTokenCount: 9,
      },
    },
    crlf,
  );

  if (crlf) {
    writeFragmented(response, opening + completion, schedule);
    return;
  }

  response.write(opening);
  if (prompt.includes("[disconnect]")) {
    schedule(() => response.destroy(), 250);
    return;
  }
  if (prompt.includes("[hang]") || prompt.includes("[cancel]")) return;

  const complete = () => {
    if (response.destroyed || response.writableEnded) return;
    response.end(completion);
  };
  if (prompt.includes("[slow]")) schedule(complete, 5_000);
  else complete();
}

export class ProviderFixture {
  private server: Server | null = null;
  private sockets = new Set<Socket>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private promptCounts = new Map<string, number>();
  private capturedRequestLog: ProviderRequestSnapshot[] = [];
  baseUrl = "";

  capturedRequests(): ProviderRequestSnapshot[] {
    return this.capturedRequestLog.map((request) => ({
      ...request,
      headers: { ...request.headers },
      body: copyBody(request.body),
    }));
  }

  private recordRequest(
    request: IncomingMessage,
    requestUrl: URL,
    body: Record<string, unknown>,
  ) {
    this.capturedRequestLog.push({
      method: request.method ?? "POST",
      pathname: requestUrl.pathname,
      search: requestUrl.search,
      headers: capturedHeaders(request),
      body: copyBody(body),
    });
  }

  private schedule(callback: () => void, delay: number) {
    const timer = setTimeout(() => {
      this.timers.delete(timer);
      callback();
    }, delay);
    this.timers.add(timer);
  }

  async start() {
    if (this.server) return;
    this.server = createServer(async (request, response) => {
      try {
        const requestUrl = new URL(request.url ?? "/", "http://provider.fixture");
        const pathname = requestUrl.pathname;
        if (request.method === "GET") {
          if (writeModelCatalog(pathname, response)) return;
          response.writeHead(404).end();
          return;
        }

        if (request.method !== "POST") {
          response.writeHead(404).end();
          return;
        }

        if (pathname === "/v1/messages") {
          if (!validateStreamingHeaders(request, response, "x-api-key")) return;
          if (headerValue(request, "anthropic-version") !== "2023-06-01") {
            writeHttpError(
              response,
              400,
              "anthropic-version must be 2023-06-01",
              "invalid_headers",
            );
            return;
          }
          const body = await readJson(request);
          this.recordRequest(request, requestUrl, body);
          if (!hasValidAnthropicBody(body)) {
            writeHttpError(response, 400, "Invalid Anthropic Messages request", "invalid_request");
            return;
          }
          const prompt = lastPrompt(body);
          const count = (this.promptCounts.get(prompt) ?? 0) + 1;
          this.promptCounts.set(prompt, count);
          if (prompt.includes("[http-error]")) {
            writeAnthropicApiError(response, 429, "Fixture rate limit");
            return;
          }
          writeAnthropicStream(
            response,
            `Fixture 回答 #${count}：${prompt}`,
            prompt,
            (callback, delay) => this.schedule(callback, delay),
          );
          return;
        }

        const googleMatch = pathname.match(
          /^\/v1beta\/models\/([^/]+):streamGenerateContent$/,
        );
        if (googleMatch) {
          if (!validateStreamingHeaders(request, response, "x-goog-api-key")) return;
          if (requestUrl.searchParams.get("alt") !== "sse") {
            writeHttpError(response, 400, "Google streaming requires alt=sse", "invalid_query");
            return;
          }
          const body = await readJson(request);
          this.recordRequest(request, requestUrl, body);
          if (!hasValidGoogleBody(body)) {
            writeHttpError(response, 400, "Invalid Google GenerateContent request", "invalid_request");
            return;
          }
          const prompt = googleLastPrompt(body);
          const count = (this.promptCounts.get(prompt) ?? 0) + 1;
          this.promptCounts.set(prompt, count);
          if (prompt.includes("[http-error]")) {
            writeGoogleApiError(response, 429, "Fixture rate limit");
            return;
          }
          writeGoogleStream(
            response,
            `Fixture 回答 #${count}：${prompt}`,
            prompt,
            (callback, delay) => this.schedule(callback, delay),
          );
          return;
        }

        if (pathname !== "/v1/chat/completions" && pathname !== "/api/chat") {
          response.writeHead(404).end();
          return;
        }

        const body = await readJson(request);
        this.recordRequest(request, requestUrl, body);
        const prompt = lastPrompt(body);
        const count = (this.promptCounts.get(prompt) ?? 0) + 1;
        this.promptCounts.set(prompt, count);
        const answer = `Fixture 回答 #${count}：${prompt}`;
        if (pathname === "/v1/chat/completions") {
          writeOpenAiStream(response, answer, prompt, (callback, delay) => this.schedule(callback, delay));
          return;
        }
        if (pathname === "/api/chat") {
          writeOllamaStream(response, answer, prompt, (callback, delay) => this.schedule(callback, delay));
          return;
        }
      } catch (error) {
        if (response.headersSent) {
          response.destroy();
          return;
        }
        writeHttpError(
          response,
          400,
          error instanceof Error ? error.message : "bad request",
          "bad_request",
        );
      }
    });
    this.server.on("connection", (socket) => {
      this.sockets.add(socket);
      socket.once("close", () => this.sockets.delete(socket));
    });

    await new Promise<void>((resolve, reject) => {
      this.server?.once("error", reject);
      this.server?.listen(0, "127.0.0.1", resolve);
    });
    const address = this.server.address();
    if (!address || typeof address === "string") throw new Error("Provider fixture has no TCP address");
    this.baseUrl = `http://127.0.0.1:${address.port}`;
  }

  async stop() {
    if (!this.server) return;
    const server = this.server;
    this.server = null;
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
    for (const socket of this.sockets) socket.destroy();
    this.sockets.clear();
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
}
