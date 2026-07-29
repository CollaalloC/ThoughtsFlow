import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { ProviderFixture } from "../fixtures/provider-server";

const nativeHarnessEnabled =
  process.env.THOUGHSFLOW_E2E_NATIVE === "1" && Boolean(process.env.THOUGHSFLOW_E2E_BASE_URL);
const restartHookUrl = process.env.THOUGHSFLOW_E2E_RESTART_URL;

function fixtureAnswer(prompt: string, sequence = 1) {
  return `Fixture 回答 #${sequence}：${prompt}`;
}

function uniqueName(testInfo: TestInfo, label: string) {
  const stamp = `${Date.now()}-${testInfo.workerIndex}`;
  return `${label}-${testInfo.project.name}-${stamp}`;
}

test.describe("ThoughsFlow Context Tree core journeys", () => {
  test.skip(
    !nativeHarnessEnabled,
    "Requires THOUGHSFLOW_E2E_NATIVE=1 and THOUGHSFLOW_E2E_BASE_URL for a Tauri WebDriver harness.",
  );

  const provider = new ProviderFixture();

  test.beforeAll(async () => provider.start());
  test.afterAll(async () => provider.stop());

  async function openApp(page: Page) {
    await page.goto("/");
    await expect(page.getByRole("navigation", { name: "工作面" })).toBeVisible();
  }

  async function configureFixtureProvider(page: Page, providerName: string) {
    const workSurface = page.getByRole("navigation", { name: "工作面" });
    await workSurface.getByRole("button", { name: "Provider 设置", exact: true }).click();

    const settings = page.getByRole("region", { name: "Providers" });
    await expect(settings).toBeVisible();
    await settings.getByRole("button", { name: "新建 Provider" }).click();
    await settings.getByLabel("名称", { exact: true }).fill(providerName);
    await settings
      .getByLabel("Provider 模板", { exact: true })
      .selectOption("openai-compatible");
    await settings.getByLabel("Base URL", { exact: true }).fill(`${provider.baseUrl}/v1`);
    await settings.getByLabel("模型", { exact: true }).fill("fixture-model");
    await settings.getByRole("button", { name: "保存 Provider" }).click();
    await expect(settings.getByRole("status")).toContainText("Provider 已保存");

    await page.getByRole("button", { name: "返回 Focus" }).click();
    await expect(page.getByLabel("消息")).toBeVisible();
  }

  async function createWorkspace(page: Page, workspaceName: string, providerName: string) {
    await page.getByRole("button", { name: "添加工作区" }).click();
    const form = page.getByRole("form", { name: "创建工作区" });
    await form.getByLabel("工作区名称").fill(workspaceName);
    await form.getByLabel("工作区目标").fill("验证 Context Tree 的精确路径与恢复语义");
    await form.getByRole("button", { name: "确认创建" }).click();
    await expect(page.getByRole("heading", { name: workspaceName, exact: true })).toBeVisible();
    await page.getByLabel("Provider").selectOption({ label: `${providerName} · fixture-model` });
  }

  async function prepareScenario(page: Page, testInfo: TestInfo, label: string) {
    const providerName = uniqueName(testInfo, `${label}-Provider`);
    const workspaceName = uniqueName(testInfo, label);
    await openApp(page);
    await configureFixtureProvider(page, providerName);
    await createWorkspace(page, workspaceName, providerName);
    return { providerName, workspaceName };
  }

  async function sendAndComplete(page: Page, prompt: string, sequence = 1) {
    await page.getByLabel("消息").fill(prompt);
    await page.getByRole("button", { name: "发送", exact: true }).click();
    await expect(page.getByText(`${fixtureAnswer(prompt, sequence)}（完成）`, { exact: true })).toBeVisible();
  }

  async function createBranch(page: Page, prompt: string, branchButtonIndex = 0) {
    const branchButtons = page.getByRole("button", { name: "从此回答创建分支" });
    await branchButtons.nth(branchButtonIndex).click();
    const form = page.getByRole("form", { name: "创建精确回答分支" });
    await form.getByPlaceholder("沿另一个方向追问…").fill(prompt);
    await form.getByRole("button", { name: "创建分支" }).click();
    await expect(page.getByText(`${fixtureAnswer(prompt)}（完成）`, { exact: true })).toBeVisible();
  }

  async function openRunSnapshot(page: Page) {
    await page.getByRole("tab", { name: "本次实际发送的内容" }).click();
    const receipt = page.getByRole("tabpanel", { name: "本次实际发送的内容" });
    await expect(receipt.getByText(/锁定/)).toBeVisible();
    return receipt;
  }

  test("01 creates a workspace and completes a real fixture HTTP stream", async ({ page }, testInfo) => {
    const { providerName } = await prepareScenario(page, testInfo, "E2E-创建与流式回答");
    const prompt = "建立真实流式请求基线";

    await sendAndComplete(page, prompt);

    const receipt = await openRunSnapshot(page);
    await expect(receipt).toContainText(`本机 · ${providerName} · ${new URL(provider.baseUrl).host}`);
    await expect(receipt.locator("code")).toHaveText(/^[a-f0-9]{64}$/);
    await expect(receipt.getByText(prompt, { exact: true })).toBeVisible();
  });

  test("02 retries by adding a second immutable Run", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-重试新增Run");
    const prompt = "同一问题保留两个回答版本";
    await sendAndComplete(page, prompt, 1);

    await page.getByRole("button", { name: "重试回答" }).click();
    const switcher = page.getByLabel(`${prompt}的回答版本`);
    const answerB = switcher.getByRole("button", { name: "回答 B · fixture-model" });
    await expect(answerB).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByText(`${fixtureAnswer(prompt, 2)}（完成）`, { exact: true })).toBeVisible();

    await switcher.getByRole("button", { name: "回答 A · fixture-model" }).click();
    await expect(page.getByText(`${fixtureAnswer(prompt, 1)}（完成）`, { exact: true })).toBeVisible();
    await expect(switcher.getByRole("button")).toHaveCount(2);
  });

  test("03 creates A and B branches from the same precise parent answer", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-同父回答分支");
    const rootPrompt = "选择迁移策略";
    const routeA = "路线 A：渐进迁移";
    const routeB = "路线 B：一次替换";
    await sendAndComplete(page, rootPrompt);

    await createBranch(page, routeA);
    // The first action remains the root answer in the ordered current lineage.
    await createBranch(page, routeB, 0);

    await page.locator(".focus-header").getByRole("button", { name: "路线图", exact: true }).click();
    const routeMap = page.getByRole("region", { name: "对话路线图" });
    await expect(routeMap).toBeVisible();
    await expect(routeMap.getByRole("article", { name: rootPrompt })).toBeVisible();
    await expect(routeMap.getByRole("article", { name: routeA })).toBeVisible();
    await expect(routeMap.getByRole("article", { name: routeB })).toBeVisible();
    await expect(routeMap.locator(".route-turn-node")).toHaveCount(3);
  });

  test("04 keeps sibling branch Context isolated in the locked receipt", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-分支Context隔离");
    const rootPrompt = "确定数据库迁移路线";
    const routeA = "旁支 A：双写过渡";
    const routeB = "旁支 B：停机切换";
    await sendAndComplete(page, rootPrompt);
    await createBranch(page, routeA);
    await createBranch(page, routeB, 0);

    const receipt = await openRunSnapshot(page);
    await expect(receipt.getByText(rootPrompt, { exact: true })).toBeVisible();
    await expect(receipt.getByText(routeB, { exact: true })).toBeVisible();
    await expect(receipt.getByText(routeA, { exact: true })).toHaveCount(0);
  });

  test("05 preserves pin/exclude preview order in the actual receipt", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-Context覆盖与凭证");
    const rootPrompt = "形成可复用的事实基线";
    const nextPrompt = "只携带确认后的 Context 继续";
    await sendAndComplete(page, rootPrompt);

    await page.getByLabel("消息").fill(nextPrompt);
    const inspector = page.getByRole("complementary", { name: "Context Inspector" });
    await expect(inspector.getByRole("button", { name: "固定 Ancestor prompt" })).toBeVisible();
    await inspector.getByRole("button", { name: "固定 Ancestor prompt" }).click();
    await inspector.getByRole("button", { name: "排除 Exact ancestor answer" }).click();
    await expect(inspector.getByRole("region", { name: "上下文用量" })).toContainText("1 固定");
    await expect(inspector.getByRole("region", { name: "上下文用量" })).toContainText("1 排除");

    const previewItems = inspector.locator(".context-item.is-included .context-item__copy > p");
    await expect(previewItems).not.toHaveCount(0);
    const previewOrder = await previewItems.allTextContents();
    expect(previewOrder).toContain(nextPrompt);
    expect(previewOrder).not.toContain(`${fixtureAnswer(rootPrompt)}（完成）`);

    await page.getByRole("button", { name: "发送", exact: true }).click();
    await expect(page.getByText(`${fixtureAnswer(nextPrompt)}（完成）`, { exact: true })).toBeVisible();
    const receipt = await openRunSnapshot(page);
    const receiptOrder = await receipt.locator(".context-item.is-included .context-item__copy > p").allTextContents();
    expect(receiptOrder).toEqual(previewOrder);
  });

  test("06 keeps partial output for cancellation and provider disconnect", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-取消与断流");
    const cancelPrompt = "[slow] 用户主动取消";
    const disconnectPrompt = "[disconnect] Provider 中途断流";

    await page.getByLabel("消息").fill(cancelPrompt);
    await page.getByRole("button", { name: "发送", exact: true }).click();
    await expect(page.getByText(fixtureAnswer(cancelPrompt), { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "停止生成" }).click();

    await page.getByRole("button", { name: "打开 Context Tree" }).click();
    const virtualRoot = page.getByRole("treeitem", { name: /^工作区起点/ });
    await virtualRoot.click();
    await expect(virtualRoot).toHaveAttribute("aria-current", "true");
    await page.getByRole("button", { name: "关闭 Context Tree" }).click();

    await page.getByLabel("消息").fill(disconnectPrompt);
    await page.getByRole("button", { name: "发送", exact: true }).click();
    await expect(page.getByText(fixtureAnswer(disconnectPrompt), { exact: true })).toBeVisible();
    await expect(page.getByRole("alert").filter({ hasText: /stream|connection|Provider|EOF/i })).toBeVisible();

    await page.getByRole("tab", { name: "运行记录" }).click();
    const runLog = page.getByRole("tabpanel", { name: "运行记录" });
    await expect(runLog).toContainText("回答 A · cancelled");
    await expect(runLog).toContainText("回答 A · failed");
    await expect(page.getByText(fixtureAnswer(cancelPrompt), { exact: true })).toBeVisible();
    await expect(page.getByText(fixtureAnswer(disconnectPrompt), { exact: true })).toBeVisible();
  });

  test("07 marks a checkpointed in-flight Run interrupted after a native restart", async ({ page }, testInfo) => {
    test.skip(
      !restartHookUrl,
      "Requires THOUGHSFLOW_E2E_RESTART_URL: a POST endpoint that hard-restarts the native app and returns when restart begins.",
    );

    const { workspaceName } = await prepareScenario(page, testInfo, "E2E-重启恢复");
    const prompt = "[hang] 保留 checkpoint 后模拟进程重启";
    await page.getByLabel("消息").fill(prompt);
    await page.getByRole("button", { name: "发送", exact: true }).click();
    await expect(page.getByText(fixtureAnswer(prompt), { exact: true })).toBeVisible();
    await page.waitForTimeout(650);

    const restart = await fetch(restartHookUrl!, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ reason: "playwright-interrupted-recovery" }),
    });
    expect(restart.ok, `native restart hook returned ${restart.status}`).toBe(true);

    await expect
      .poll(async () => {
        try {
          await page.goto("/", { timeout: 3_000, waitUntil: "domcontentloaded" });
          return await page.getByRole("navigation", { name: "工作区" }).isVisible();
        } catch {
          return false;
        }
      }, { timeout: 30_000 })
      .toBe(true);
    await page.getByRole("navigation", { name: "工作区" }).getByRole("button", { name: workspaceName }).click();
    await expect(page.getByText(fixtureAnswer(prompt), { exact: true })).toBeVisible();
    await page.getByRole("tab", { name: "运行记录" }).click();
    await expect(page.getByRole("tabpanel", { name: "运行记录" })).toContainText("回答 A · interrupted");
  });

  test("08 compares Runs, records adopted/rejected reasons, and exports a Decision Packet", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-比较标记与导出");
    const prompt = "比较两个可审查的技术结论";
    await sendAndComplete(page, prompt, 1);
    await page.getByRole("button", { name: "重试回答" }).click();
    await expect(page.getByText(`${fixtureAnswer(prompt, 2)}（完成）`, { exact: true })).toBeVisible();

    await page.locator(".focus-header").getByRole("button", { name: "决策", exact: true }).click();
    await expect(page.getByRole("region", { name: "路线比较与决策" })).toBeVisible();
    await page.getByLabel("路线 A").selectOption({ label: `${prompt} · 回答 A · fixture-model` });
    await page.getByLabel("路线 B").selectOption({ label: `${prompt} · 回答 B · fixture-model` });
    await page.getByRole("button", { name: "比较两条路线" }).click();
    await expect(page.getByRole("region", { name: "回答差异" })).toContainText(fixtureAnswer(prompt, 1));
    await expect(page.getByRole("region", { name: "回答差异" })).toContainText(fixtureAnswer(prompt, 2));
    await expect(page.getByRole("region", { name: "Context Diff" })).toBeVisible();

    const accepted = page.getByRole("article", { name: `${prompt} · 回答 A的决策` });
    await accepted.getByRole("button", { name: "采纳" }).click();
    await accepted.getByLabel(`${prompt} · 回答 A的判断理由`).fill("保留可回滚窗口，风险更可控。");
    await accepted.getByRole("button", { name: "保存决策标记" }).click();
    await expect(accepted).toContainText("已保存");

    const rejected = page.getByRole("article", { name: `${prompt} · 回答 B的决策` });
    await rejected.getByRole("button", { name: "否决" }).click();
    await rejected.getByLabel(`${prompt} · 回答 B的判断理由`).fill("失败半径过大，缺少回退路径。");
    await rejected.getByRole("button", { name: "保存决策标记" }).click();
    await expect(rejected).toContainText("已保存");

    await page.getByRole("button", { name: "导出 Decision Packet" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Decision Packet 已导出" })).toBeVisible();
  });

  test("09 sends the effective checkpoint Context and exposes its exact source evidence", async ({ page }, testInfo) => {
    await prepareScenario(page, testInfo, "E2E-检查点来源与真实Payload");
    const sourcePrompt = "来源范围：不可变事实基线";
    const tailPrompt = "保留尾部：继续审查";
    const nextPrompt = "检查 Provider 实际收到的压缩 Context";
    const summary = "检查点摘要：事实基线已经确认，保留尾部继续审查。";

    await sendAndComplete(page, sourcePrompt);
    await sendAndComplete(page, tailPrompt);

    await page.getByRole("button", { name: "准备 Context 压缩" }).click();
    const maintenance = page.getByRole("complementary", { name: "Context 压缩预览" });
    await expect(maintenance.getByText("1 个来源 Run")).toBeVisible();
    await maintenance.getByRole("textbox", { name: "人工摘要" }).fill(summary);
    await maintenance.getByRole("button", { name: "保存人工压缩检查点" }).click();
    await expect(page.getByText("Context 检查点已保存并激活。")).toBeVisible();

    await page.getByRole("button", { name: "打开上下文检查器" }).click();
    const inspector = page.getByRole("complementary", { name: "Context Inspector" });
    const checkpoint = inspector.getByRole("region", { name: "已应用 Context 检查点" });
    await expect(checkpoint).toContainText("1 个来源 Run");
    const sourceHash = await checkpoint
      .locator("dl > div")
      .filter({ hasText: "来源 Hash" })
      .locator("dd")
      .textContent();
    expect(sourceHash ?? "").toMatch(/^[a-f0-9]{64}$/);

    await page.getByRole("button", { name: "关闭上下文检查器" }).click();
    await sendAndComplete(page, nextPrompt);

    const lastRequest = provider
      .capturedRequests()
      .filter((request) => request.pathname === "/v1/chat/completions")
      .at(-1);
    expect(lastRequest).toBeDefined();
    const messages = Array.isArray(lastRequest?.body.messages)
      ? lastRequest.body.messages
      : [];
    const actualContent = messages
      .filter((message): message is Record<string, unknown> => Boolean(message) && typeof message === "object")
      .map((message) => message.content)
      .filter((content): content is string => typeof content === "string");
    expect(actualContent).toContain(summary);
    expect(actualContent).toContain(tailPrompt);
    expect(actualContent).toContain(nextPrompt);
    expect(actualContent).not.toContain(sourcePrompt);
  });
});
