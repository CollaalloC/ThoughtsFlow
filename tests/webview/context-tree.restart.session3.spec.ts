import { browser, expect } from "@wdio/globals";
import { readFile } from "node:fs/promises";
import {
  captureActiveReceipt,
  captureTreeState,
  closeContextTree,
  openContextTree,
  openWorkspace,
  pinMainWindow,
  requiredEnvironment,
  selectConversationProvider,
  sendAndWaitForCompletion,
  waitForBodyText,
  type ReceiptState,
  type TreeState,
} from "./support";

type CapturedMaintenanceReplay = {
  input: Record<string, unknown>;
  baselineTree: TreeState;
  baselineReceipt: ReceiptState;
};

type ReplayResult = {
  ok: boolean;
  response?: {
    apiVersion?: number;
    data?: {
      checkpoint?: { id?: string } | null;
    };
  };
  error?: string;
};

describe("ThoughsFlow real process restart journey — session 3", () => {
  it("replays the exact committed operation without a second Provider call", async () => {

    const journeyId = requiredEnvironment("TF_WEBVIEW_JOURNEY_ID");
    const fixtureRoot = requiredEnvironment("TF_WEBVIEW_FIXTURE_BASE_URL");
    const crashOperationId = requiredEnvironment(
      "TF_WEBVIEW_CRASH_OPERATION_ID",
    );
    const crashCapturePath = requiredEnvironment(
      "TF_WEBVIEW_CRASH_CAPTURE_PATH",
    );
    const crashBaselinePath = requiredEnvironment(
      "TF_WEBVIEW_CRASH_BASELINE_PATH",
    );
    const providerName = `Restart Fixture ${journeyId}`;
    const workspaceName = `Restart Context ${journeyId}`;
    const recoveryPrompt = `Restart recovered continuation ${journeyId}`;
    const successPrompt = `Restart audited summary ${journeyId}`;
    const postCheckpointPrompt = `Restart post-checkpoint ${journeyId}`;

    await pinMainWindow();
    await openWorkspace(workspaceName);
    await selectConversationProvider({
      name: providerName,
      model: "fixture-model",
      baseUrl: `${fixtureRoot}/v1`,
    });
    await waitForBodyText(`Fixture 回答 #1：${recoveryPrompt}（完成）`);

    const capturedInputBytes = await readFile(crashCapturePath);
    const capturedBaseline = JSON.parse(
      await readFile(crashBaselinePath, "utf8"),
    ) as Omit<CapturedMaintenanceReplay, "input">;
    const captured: CapturedMaintenanceReplay = {
      input: JSON.parse(capturedInputBytes.toString("utf8")) as Record<
        string,
        unknown
      >,
      ...capturedBaseline,
    };
    expect(captured.input.clientOperationId).toBe(crashOperationId);
    expect(captured.input.summaryPrompt).toBe(successPrompt);

    await openContextTree();
    const committedTree = await captureTreeState();
    expect(committedTree.currentLabel).toContain(recoveryPrompt.slice(0, 48));
    expect(committedTree.version).toBeGreaterThan(
      captured.baselineTree.version,
    );
    expect(committedTree.checkpoints.length).toBe(
      captured.baselineTree.checkpoints.length + 1,
    );
    expect(committedTree.checkpoints.join("\n")).toContain("压缩检查点");
    await closeContextTree();

    const committedReceipt = await captureActiveReceipt();
    expect(committedReceipt).toEqual(captured.baselineReceipt);

    const replay = await browser.executeAsync(
      (
        input: Record<string, unknown>,
        done: (result?: ReplayResult) => void,
      ) => {
        type Invoke = (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
        const invoke = (
          window as unknown as {
            __TAURI_INTERNALS__: { invoke: Invoke };
          }
        ).__TAURI_INTERNALS__.invoke;
        void invoke("summarize_and_set_active_context", { input }).then(
          (response) => done({
            ok: true,
            response: response as ReplayResult["response"],
          }),
          (reason) => done({
            ok: false,
            error: reason instanceof Error
              ? reason.message
              : JSON.stringify(reason),
          }),
        );
      },
      captured.input,
    ) as ReplayResult;
    expect(replay.ok).toBe(true);
    if (!replay.ok) throw new Error(replay.error ?? "Maintenance replay failed");
    expect(replay.response?.apiVersion).toBe(1);
    expect(replay.response?.data?.checkpoint?.id).toBe(crashOperationId);

    await browser.refresh();
    await waitForBodyText(workspaceName);
    await selectConversationProvider({
      name: providerName,
      model: "fixture-model",
      baseUrl: `${fixtureRoot}/v1`,
    });
    await openContextTree();
    const replayedTree = await captureTreeState();
    expect(replayedTree).toEqual(committedTree);
    await closeContextTree();

    const replayedReceipt = await captureActiveReceipt();
    expect(replayedReceipt).toEqual(captured.baselineReceipt);
    await sendAndWaitForCompletion(postCheckpointPrompt);
  });
});
