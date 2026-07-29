import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { ProviderFixture } from "../fixtures/provider-server.ts";

const projectRoot = resolve(import.meta.dirname, "../..");
const wdioBinary = resolve(projectRoot, "node_modules/.bin/wdio");
const configPath = resolve(projectRoot, "wdio.webview.conf.ts");
const journeyId = `${Date.now()}-${process.pid}`;
const fixture = new ProviderFixture();
const launcherPids: number[] = [];
const sessionExitCodes: number[] = [];
const crashOperationId = randomUUID();
const appDataDir = await mkdtemp(
  join(tmpdir(), "thoughsflow-webview-restart-"),
);
const crashCapturePath = join(
  appDataDir,
  "maintenance-replay-input.json",
);
const crashBaselinePath = join(
  appDataDir,
  "maintenance-replay-baseline.json",
);

function lastPrompt(body: Record<string, unknown>) {
  const messages = Array.isArray(body.messages) ? body.messages : [];
  const last = messages.at(-1);
  if (!last || typeof last !== "object") return "";
  const content = (last as Record<string, unknown>).content;
  return typeof content === "string" ? content : "";
}

async function runSession(
  label: string,
  spec: string,
  extraEnvironment: Record<string, string> = {},
  expectedAppAbort = false,
) {
  await new Promise<void>((resolveRun, rejectRun) => {
    const child = spawn(
      wdioBinary,
      ["run", configPath, "--spec", resolve(projectRoot, spec)],
      {
        cwd: projectRoot,
        env: {
          ...process.env,
          TF_WEBVIEW_FIXTURE_BASE_URL: fixture.baseUrl,
          TF_WEBVIEW_JOURNEY_ID: journeyId,
          TF_WEBVIEW_CRASH_OPERATION_ID: crashOperationId,
          TF_WEBVIEW_CRASH_CAPTURE_PATH: crashCapturePath,
          TF_WEBVIEW_CRASH_BASELINE_PATH: crashBaselinePath,
          THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: appDataDir,
          ...extraEnvironment,
        },
        stdio: "inherit",
      },
    );
    if (child.pid) launcherPids.push(child.pid);
    child.once("error", rejectRun);
    child.once("exit", (code, signal) => {
      sessionExitCodes.push(code ?? -1);
      if (code === 0) {
        resolveRun();
        return;
      }
      if (expectedAppAbort && code === 1 && signal === null) {
        resolveRun();
        return;
      }
      rejectRun(
        new Error(
          `${label} WDIO launcher failed with code ${String(code)} signal ${String(signal)}`,
        ),
      );
    });
  });
}

try {
  await fixture.start();
  await runSession(
    "restart-session-1",
    "tests/webview/context-tree.restart.session1.spec.ts",
  );
  await runSession(
    "restart-session-2",
    "tests/webview/context-tree.restart.session2.spec.ts",
    {
      THOUGHSFLOW_WEBVIEW_E2E_CRASH_AFTER_MAINTENANCE_COMMIT:
        crashOperationId,
    },
    true,
  );
  const capturedMaintenance = JSON.parse(
    await readFile(crashCapturePath, "utf8"),
  ) as { clientOperationId?: string };
  assert.equal(
    capturedMaintenance.clientOperationId,
    crashOperationId,
    "Session 2 did not persist the exact maintenance IPC input before abort",
  );
  const crashSummaryPrompt = `Restart audited summary ${journeyId}`;
  const crashSummaryRequests = fixture
    .capturedRequests()
    .filter(
      (request) =>
        request.pathname === "/v1/chat/completions"
        && lastPrompt(request.body) === crashSummaryPrompt,
    );
  assert.equal(
    crashSummaryRequests.length,
    1,
    "Session 2 must reach the Provider exactly once before its expected abort",
  );
  await runSession(
    "restart-session-3",
    "tests/webview/context-tree.restart.session3.spec.ts",
  );

  assert.equal(launcherPids.length, 3);
  assert.deepEqual(
    sessionExitCodes,
    [0, 1, 0],
    "Only the commit-after-response-window session may exit non-zero",
  );
  assert.equal(
    new Set(launcherPids).size,
    3,
    "The restart journey must use three distinct WDIO launcher processes",
  );

  const rootPrompt = `Restart root ${journeyId}`;
  const leafPrompt = `Restart leaf ${journeyId}`;
  const hangPrompt = `Restart partial [hang] ${journeyId}`;
  const recoveryPrompt = `Restart recovered continuation ${journeyId}`;
  const failurePrompt = `Restart summary [disconnect] ${journeyId}`;
  const cancellationPrompt = `Restart summary [hang] ${journeyId}`;
  const successPrompt = `Restart audited summary ${journeyId}`;
  const postCheckpointPrompt = `Restart post-checkpoint ${journeyId}`;
  const requests = fixture
    .capturedRequests()
    .filter((request) => request.pathname === "/v1/chat/completions");
  const prompts = requests.map((request) => lastPrompt(request.body));

  for (const expected of [
    rootPrompt,
    leafPrompt,
    hangPrompt,
    recoveryPrompt,
    failurePrompt,
    cancellationPrompt,
    successPrompt,
    postCheckpointPrompt,
  ]) {
    assert(
      prompts.includes(expected),
      `ProviderFixture did not receive expected prompt: ${expected}`,
    );
  }
  assert.equal(
    prompts.filter((prompt) => prompt === successPrompt).length,
    1,
    "Idempotent replay must not issue a second Provider summary request",
  );

  const successfulSummaryRequest = requests.find(
    (request) => lastPrompt(request.body) === successPrompt,
  );
  assert(successfulSummaryRequest, "Successful summary Provider request was not captured");
  const summarySourcePayload = JSON.stringify(successfulSummaryRequest.body.messages);
  assert(summarySourcePayload.includes(rootPrompt));
  assert(summarySourcePayload.includes(leafPrompt));

  const postCheckpointRequest = requests.find(
    (request) => lastPrompt(request.body) === postCheckpointPrompt,
  );
  assert(postCheckpointRequest, "Post-checkpoint Provider request was not captured");
  const compiledPayload = JSON.stringify(postCheckpointRequest.body.messages);
  assert(
    compiledPayload.includes(`Fixture 回答 #1：${successPrompt}`),
    "Post-checkpoint payload did not include the activated summary",
  );
  assert(
    compiledPayload.includes(recoveryPrompt),
    "Post-checkpoint payload did not retain the requested raw tail",
  );
  assert(
    compiledPayload.includes(postCheckpointPrompt),
    "Post-checkpoint payload did not include the next current prompt",
  );
  assert(
    !compiledPayload.includes(rootPrompt),
    "Post-checkpoint payload leaked a compacted source prompt",
  );
  assert(
    !compiledPayload.includes(leafPrompt),
    "Post-checkpoint payload leaked a compacted source prompt",
  );

  console.log(
    JSON.stringify(
      {
        journeyId,
        crashOperationId,
        launcherPids,
        sessionExitCodes,
        providerRequests: requests.length,
        result: "three-process WKWebView crash-window journey passed",
      },
      null,
      2,
    ),
  );
} finally {
  await fixture.stop();
  await rm(appDataDir, { force: true, recursive: true });
}
