import { $, browser, expect } from "@wdio/globals";
import { writeFile } from "node:fs/promises";
import {
  button,
  captureActiveReceipt,
  captureTreeState,
  closeContextTree,
  closeMaintenance,
  closeVisibleError,
  controlByLabel,
  contextTreeItem,
  openContextTree,
  openProviderMaintenance,
  openWorkspace,
  pinMainWindow,
  requiredEnvironment,
  selectConversationProvider,
  selectTreeRun,
  sendAndWaitForCompletion,
  showAllTreeNodes,
  submitProviderSummary,
  waitForBodyText,
} from "./support";

describe("ThoughsFlow real process restart journey — session 2", () => {
  it("recovers interrupted output, rejects unsafe maintenance, then crashes after commit", async () => {
    expect(String(browser.capabilities.browserName).toLowerCase()).toBe("webkit");
    expect(String(browser.capabilities.platformName).toLowerCase()).toContain("mac");

    const journeyId = requiredEnvironment("TF_WEBVIEW_JOURNEY_ID");
    const fixtureRoot = requiredEnvironment("TF_WEBVIEW_FIXTURE_BASE_URL");
    const providerName = `Restart Fixture ${journeyId}`;
    const providerLabel = `${providerName} · fixture-model`;
    const workspaceName = `Restart Context ${journeyId}`;
    const rootPrompt = `Restart root ${journeyId}`;
    const leafPrompt = `Restart leaf ${journeyId}`;
    const hangPrompt = `Restart partial [hang] ${journeyId}`;
    const recoveryPrompt = `Restart recovered continuation ${journeyId}`;
    const failurePrompt = `Restart summary [disconnect] ${journeyId}`;
    const cancellationPrompt = `Restart summary [hang] ${journeyId}`;
    const successPrompt = `Restart audited summary ${journeyId}`;
    const postCheckpointPrompt = `Restart post-checkpoint ${journeyId}`;
    const recoveryTitle = recoveryPrompt.slice(0, 48);
    const crashOperationId = requiredEnvironment(
      "TF_WEBVIEW_CRASH_OPERATION_ID",
    );
    const crashBaselinePath = requiredEnvironment(
      "TF_WEBVIEW_CRASH_BASELINE_PATH",
    );

    await pinMainWindow();
    await openWorkspace(workspaceName);
    await selectConversationProvider({
      name: providerName,
      model: "fixture-model",
      baseUrl: `${fixtureRoot}/v1`,
    });

    await waitForBodyText(`Fixture 回答 #1：${hangPrompt}`);
    await openContextTree();
    await showAllTreeNodes();
    const interrupted = await contextTreeItem(hangPrompt);
    expect(await interrupted.getAttribute("aria-label")).toContain("已中断");
    expect(await interrupted.getAttribute("aria-current")).toBe("true");

    await selectTreeRun(leafPrompt);
    await closeContextTree();
    await sendAndWaitForCompletion(recoveryPrompt);

    const receiptBeforeCheckpoint = await captureActiveReceipt();
    expect(receiptBeforeCheckpoint.hash).toMatch(/^[a-f0-9]{64}$/);
    expect(receiptBeforeCheckpoint.items.length).toBeGreaterThan(0);

    await openContextTree();
    const stableTree = await captureTreeState();
    expect(stableTree.currentLabel).toContain(recoveryTitle);
    await closeContextTree();

    await openProviderMaintenance(providerLabel);
    await submitProviderSummary(failurePrompt);
    const failureNotice = await $(".focus-notice.is-error");
    await failureNotice.waitForDisplayed({ timeout: 20_000 });
    expect((await failureNotice.getText()).trim().length).toBeGreaterThan(0);
    await closeMaintenance();
    await closeVisibleError();

    await openContextTree();
    const afterFailure = await captureTreeState();
    expect(afterFailure).toEqual(stableTree);
    await closeContextTree();

    await openProviderMaintenance(providerLabel);
    await submitProviderSummary(cancellationPrompt);
    const cancel = await $(
      'button[aria-label="取消正在生成的 Context 摘要"]',
    );
    await cancel.waitForClickable({ timeout: 20_000 });
    await browser.pause(350);
    await cancel.click();
    await waitForBodyText("已请求取消摘要；检查点和当前 Context 不会移动。");
    await browser.waitUntil(
      async () =>
        await $('button[aria-label="关闭 Context 压缩预览"]').isExisting(),
      {
        interval: 100,
        timeout: 20_000,
        timeoutMsg: "Maintenance did not settle after cancellation",
      },
    );
    await closeMaintenance();
    await closeVisibleError();

    await openContextTree();
    const afterCancellation = await captureTreeState();
    expect(afterCancellation).toEqual(stableTree);
    await closeContextTree();

    await writeFile(
      crashBaselinePath,
      JSON.stringify({
        baselineTree: stableTree,
        baselineReceipt: receiptBeforeCheckpoint,
      }),
      "utf8",
    );
    await browser.execute(
      (operationId) => {
        document.documentElement.dataset.thoughtsflowWebviewE2eOperationId =
          operationId;
      },
      crashOperationId,
    );

    await openProviderMaintenance(providerLabel);
    const crashSummaryField = await controlByLabel("摘要请求", "textarea");
    await crashSummaryField.setValue(successPrompt);
    const crashSubmit = await button("确认生成并切换 Context");
    try {
      await crashSubmit.click();
    } catch (reason) {
      const message = reason instanceof Error
        ? `${reason.name}: ${reason.message}`
        : String(reason);
      if (!/session|connection|fetch|socket|refused|terminated|disconnect/i.test(message)) {
        throw reason;
      }
    }
    // The webview-e2e-only backend hook aborts the app after SQLite commits
    // but before the invoke response reaches this page. The next independent
    // session replays the exact captured IPC input and proves idempotency.
    await new Promise((resolve) => setTimeout(resolve, 2_000));

    // Keep the exact source prompts in the test process for the outer
    // ProviderFixture payload assertions.
    expect(rootPrompt).not.toBe(leafPrompt);
    expect(postCheckpointPrompt).not.toBe(successPrompt);
    expect(recoveryTitle.length).toBeGreaterThan(0);
  });
});
