import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { Socket } from "node:net";

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
          owned_by: "thoughsflow-fixture",
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

export class ProviderFixture {
  private server: Server | null = null;
  private sockets = new Set<Socket>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  private promptCounts = new Map<string, number>();
  baseUrl = "";

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
        const pathname = new URL(request.url ?? "/", "http://provider.fixture").pathname;
        if (request.method === "GET") {
          if (writeModelCatalog(pathname, response)) return;
          response.writeHead(404).end();
          return;
        }

        if (
          request.method !== "POST" ||
          (pathname !== "/v1/chat/completions" && pathname !== "/api/chat")
        ) {
          response.writeHead(404).end();
          return;
        }

        const body = await readJson(request);
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
        response.writeHead(400, { "content-type": "application/json" });
        response.end(JSON.stringify({ error: error instanceof Error ? error.message : "bad request" }));
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
