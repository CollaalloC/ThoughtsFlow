import { $, browser, expect } from "@wdio/globals";
import {
  captureActiveReceipt,
  closeContextTree,
  configureOpenAiProvider,
  contextTreeItem,
  createWorkspace,
  openContextTree,
  pinMainWindow,
  selectConversationProvider,
  sendPrompt,
  waitForBodyText,
} from "./support";

const baseUrl = "http://localhost:8317/v1";
const model = "gpt-5.6-sol";
const prompt = "Reply with exactly TF_APP_OK and nothing else.";

describe("ThoughtsFlow opt-in live OpenAI-compatible proxy probe", () => {
  it("records an exact Run and immutable Receipt through the native WKWebView", async () => {
    if (process.env.TF_WEBVIEW_LIVE_PROXY !== "1") {
      throw new Error(
        "Live proxy probe is opt-in; run npm run test:webview:live-proxy",
      );
    }

    const suffix = `${Date.now()}-${process.pid}`;
    const providerName = `Live Local Proxy ${suffix}`;
    const workspaceName = `Live Proxy Probe ${suffix}`;
    await pinMainWindow();
    await configureOpenAiProvider({
      name: providerName,
      baseUrl,
      model,
    });
    await createWorkspace({
      name: workspaceName,
      goal: "只验证本地 OpenAI-compatible 端点；不包含任何项目内容。",
    });
    await selectConversationProvider({
      name: providerName,
      model,
      baseUrl,
    });

    await sendPrompt(prompt);
    await waitForBodyText(prompt);
    const turn = await $(
      `//article[contains(@class,"focus-turn") and .//section[contains(@class,"focus-turn__prompt")]//*[normalize-space(.)=${JSON.stringify(prompt)}]]`,
    );
    await turn.waitForDisplayed();
    const answer = await turn.$(".focus-turn__answer > .tf-markdown");
    await answer.waitForDisplayed();
    await browser.waitUntil(
      async () => (await answer.getText()).trim() === "TF_APP_OK",
      {
        interval: 250,
        timeout: 120_000,
        timeoutMsg: "Live proxy did not return the exact TF_APP_OK response",
      },
    );
    expect((await answer.getText()).trim()).toBe("TF_APP_OK");
    const branch = await turn.$('button[aria-label="从此回答创建分支"]');
    await branch.waitForEnabled({ timeout: 120_000 });

    await openContextTree();
    const runItem = await contextTreeItem(prompt);
    const runLabel = (await runItem.getAttribute("aria-label")) ?? "";
    expect(runLabel).toContain("已完成");
    expect(await runItem.getAttribute("aria-current")).toBe("true");
    const runId = runLabel.match(
      / · ([0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}) · /i,
    )?.[1];
    expect(runId).toBeTruthy();
    await closeContextTree();

    const receipt = await captureActiveReceipt();
    expect(receipt.hash).toMatch(/^[a-f0-9]{64}$/);
    expect(receipt.items.some((item) => item.includes(prompt))).toBe(true);

    console.log(
      `TF_LIVE_PROXY_EVIDENCE ${JSON.stringify({
        baseUrl,
        model,
        prompt,
        response: "TF_APP_OK",
        runId,
        receiptHash: receipt.hash,
      })}`,
    );
  });
});
