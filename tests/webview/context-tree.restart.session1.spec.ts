import { browser, expect } from "@wdio/globals";
import {
  closeContextTree,
  configureOpenAiProvider,
  createWorkspace,
  openContextTree,
  pinMainWindow,
  requiredEnvironment,
  selectConversationProvider,
  selectTreeRun,
  selectVirtualRoot,
  sendAndWaitForCompletion,
  sendPrompt,
  showAllTreeNodes,
  waitForBodyText,
} from "./support";

describe("ThoughsFlow real process restart journey — session 1", () => {
  it("persists a partial streaming Run before the native app process exits", async () => {

    const journeyId = requiredEnvironment("TF_WEBVIEW_JOURNEY_ID");
    const fixtureRoot = requiredEnvironment("TF_WEBVIEW_FIXTURE_BASE_URL");
    const providerName = `Restart Fixture ${journeyId}`;
    const workspaceName = `Restart Context ${journeyId}`;
    const model = "fixture-model";
    const rootPrompt = `Restart root ${journeyId}`;
    const leafPrompt = `Restart leaf ${journeyId}`;
    const hangPrompt = `Restart partial [hang] ${journeyId}`;

    await pinMainWindow();
    await configureOpenAiProvider({
      name: providerName,
      baseUrl: `${fixtureRoot}/v1`,
      model,
    });
    await createWorkspace({
      name: workspaceName,
      goal: `验证真实双进程恢复 ${journeyId}`,
    });
    await selectConversationProvider({
      name: providerName,
      model,
      baseUrl: `${fixtureRoot}/v1`,
    });

    await sendAndWaitForCompletion(rootPrompt);
    await sendAndWaitForCompletion(leafPrompt);

    await openContextTree();
    await selectVirtualRoot();
    await showAllTreeNodes();
    await selectTreeRun(leafPrompt);
    await closeContextTree();

    await sendPrompt(hangPrompt);
    const partial = `Fixture 回答 #1：${hangPrompt}`;
    await waitForBodyText(partial);
    expect(await browser.$("body").getText()).not.toContain(`${partial}（完成）`);

    // Returning from this spec lets @wdio/tauri-service terminate the real app.
    // The outer orchestrator keeps the ProviderFixture alive for session 2.
  });
});
