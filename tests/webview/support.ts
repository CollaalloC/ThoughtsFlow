import { $, $$, browser } from "@wdio/globals";
import "@wdio/native-types";
import type { ChainablePromiseElement } from "webdriverio";

type WdioElement = WebdriverIO.Element | ChainablePromiseElement;

export function requiredEnvironment(name: string) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Missing required WebView journey environment: ${name}`);
  return value;
}

export function xpathLiteral(value: string) {
  if (!value.includes('"')) return `"${value}"`;
  if (!value.includes("'")) return `'${value}'`;
  return `concat(${value
    .split('"')
    .map((part, index) => `${index > 0 ? `, '"', ` : ""}"${part}"`)
    .join("")})`;
}

export async function pinMainWindow() {
  await browser.tauri.switchWindow("main");
  await (await $('[aria-label="工作面"]')).waitForDisplayed();
}

export async function button(name: string) {
  const element = await $(
    `//button[@aria-label=${xpathLiteral(name)} or normalize-space(.)=${xpathLiteral(name)}]`,
  );
  await element.waitForClickable();
  return element;
}

export async function controlByLabel(
  name: string,
  elementName: "input" | "select" | "textarea" = "input",
) {
  const direct = await $(`${elementName}[aria-label=${JSON.stringify(name)}]`);
  if (await direct.isExisting()) {
    await direct.waitForDisplayed();
    return direct;
  }

  const element = await $(
    `//label[.//span[normalize-space(.)=${xpathLiteral(name)}]]//${elementName}`,
  );
  await element.waitForDisplayed();
  return element;
}

export async function waitForBodyText(expected: string, timeout = 20_000) {
  try {
    await browser.waitUntil(
      async () => (await $("body").getText()).includes(expected),
      {
        interval: 100,
        timeout,
        timeoutMsg: `Expected the native WebView to contain: ${expected}`,
      },
    );
  } catch {
    const bodyText = await $("body").getText();
    throw new Error(
      `Expected the native WebView to contain: ${expected}\nCurrent native WebView text:\n${bodyText}`,
    );
  }
}

export async function contextTreeItem(prompt: string): Promise<WebdriverIO.Element> {
  const titlePrefix = prompt.trim().slice(0, 48);
  const items = await $$('[role="treeitem"]');
  for (const item of items) {
    const label = await item.getAttribute("aria-label");
    if (label?.startsWith(`${titlePrefix} ·`)) return item;
  }
  throw new Error(`Context Tree item was not found for prompt: ${prompt}`);
}

export async function selectOptionByText(
  select: WdioElement,
  label: string,
) {
  const resolvedSelect = (await select) as WebdriverIO.Element;
  const options = await resolvedSelect.$$("option");
  let value: string | null = null;
  for (const option of options) {
    if ((await option.getText()) === label) {
      value = await option.getAttribute("value");
      break;
    }
  }
  if (!value) throw new Error(`Select option was not found: ${label}`);

  await resolvedSelect.selectByAttribute("value", value);
  // WKWebView's embedded driver updates the native select value, but React
  // observes the ordinary input/change events. This remains a public UI action.
  await browser.execute(
    (element, nextValue) => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLSelectElement.prototype,
        "value",
      )?.set;
      setter?.call(element, nextValue);
      element.dispatchEvent(new Event("input", { bubbles: true }));
      element.dispatchEvent(new Event("change", { bubbles: true }));
    },
    resolvedSelect,
    value,
  );
}

export async function configureOpenAiProvider(input: {
  name: string;
  baseUrl: string;
  model: string;
}) {
  await (await button("Provider 设置")).click();
  await (await $('[aria-labelledby="provider-settings-title"]')).waitForDisplayed();
  await (await button("新建 Provider")).click();
  await (await controlByLabel("名称")).setValue(input.name);
  await selectOptionByText(
    await controlByLabel("Provider 模板", "select"),
    "Generic OpenAI-compatible",
  ).catch(async () => {
    await (await controlByLabel("Provider 模板", "select")).selectByAttribute(
      "value",
      "openai-compatible",
    );
  });
  await (await controlByLabel("Base URL")).setValue(input.baseUrl);
  await (await controlByLabel("模型")).setValue(input.model);
  await (await button("保存 Provider")).click();
  await waitForBodyText("Provider 已保存");
  await (await button("返回 Focus")).click();
}

export async function createWorkspace(input: { name: string; goal: string }) {
  await (await button("添加工作区")).click();
  await (await controlByLabel("工作区名称")).setValue(input.name);
  await (await controlByLabel("工作区目标")).setValue(input.goal);
  await (await button("确认创建")).click();
  await waitForBodyText(input.name);
}

export async function openWorkspace(name: string) {
  const workspaceButton = await $(
    `//nav[@aria-label="工作区"]//button[normalize-space(.)=${xpathLiteral(name)}]`,
  );
  await workspaceButton.waitForClickable();
  await workspaceButton.click();
  await waitForBodyText(name);
}

export async function selectConversationProvider(input: {
  name: string;
  model: string;
  baseUrl: string;
}) {
  const select = await $('select[aria-label="Provider"]');
  await selectOptionByText(select, `${input.name} · ${input.model}`);
  await waitForBodyText(new URL(input.baseUrl).host);
}

export async function sendPrompt(prompt: string) {
  const composer = await $('textarea[aria-label="消息"]');
  await composer.waitForEnabled();
  await composer.setValue(prompt);
  await (await button("发送")).click();
}

export async function sendAndWaitForCompletion(prompt: string) {
  await sendPrompt(prompt);
  await waitForBodyText(`Fixture 回答 #1：${prompt}（完成）`);
}

export async function openContextTree() {
  await (await button("打开 Context Tree")).click();
  await (await $('[aria-label="Context Tree"]')).waitForDisplayed();
}

export async function closeContextTree() {
  await (await button("关闭 Context Tree")).click();
}

export async function selectTreeRun(prompt: string) {
  await (await contextTreeItem(prompt)).click();
  await browser.waitUntil(
    async () =>
      (await (await contextTreeItem(prompt)).getAttribute("aria-current")) === "true",
    {
      interval: 100,
      timeout: 20_000,
      timeoutMsg: `Context Tree did not activate run for: ${prompt}`,
    },
  );
}

export async function selectVirtualRoot() {
  const root = await $('[role="treeitem"][aria-label^="工作区起点"]');
  await root.waitForClickable();
  await root.click();
  await browser.waitUntil(
    async () =>
      (await (await $('[role="treeitem"][aria-label^="工作区起点"]'))
        .getAttribute("aria-current")) === "true",
    {
      interval: 100,
      timeout: 20_000,
      timeoutMsg: "Context Tree did not activate its virtual workspace root",
    },
  );
}

export async function showAllTreeNodes() {
  const all = await $(
    '//div[@role="group" and @aria-label="Context Tree 范围"]//button[normalize-space(.)="全部节点"]',
  );
  await all.waitForClickable();
  await all.click();
}

export type TreeState = {
  version: number;
  currentLabel: string;
  checkpoints: string[];
};

export async function captureTreeState(): Promise<TreeState> {
  const versionText = await $(".context-tree__version").getText();
  const current = await $('[role="treeitem"][aria-current="true"]');
  const checkpoints: string[] = [];
  for (const checkpoint of await $$(".context-tree__checkpoint")) {
    checkpoints.push(await checkpoint.getText());
  }
  return {
    version: Number(versionText.replace(/^v/, "")),
    currentLabel: (await current.getAttribute("aria-label")) ?? "",
    checkpoints,
  };
}

export type ReceiptState = {
  hash: string;
  items: string[];
};

export async function captureActiveReceipt(): Promise<ReceiptState> {
  await (await button("打开上下文检查器")).click();
  const inspector = await $('[aria-label="Context Inspector"]');
  await inspector.waitForDisplayed();
  await (await button("本次实际发送的内容")).click();
  const hash = await inspector.$(".context-inspector__snapshot-heading code");
  await hash.waitForDisplayed();
  await browser.waitUntil(async () => (await hash.getText()).trim().length > 0, {
    interval: 100,
    timeout: 20_000,
    timeoutMsg: "The immutable Context Receipt hash was not displayed",
  });
  const items: string[] = [];
  for (const item of await inspector.$$(".context-inspector__items .context-item")) {
    items.push(await item.getText());
  }
  const receipt = { hash: await hash.getText(), items };
  await (await button("关闭上下文检查器")).click();
  return receipt;
}

export async function openProviderMaintenance(providerLabel: string) {
  await (await button("准备 Context 压缩")).click();
  const maintenance = await $('[aria-label="Context 压缩预览"]');
  await maintenance.waitForDisplayed();
  await (await button("Provider 生成摘要")).click();
  await selectOptionByText(
    await controlByLabel("摘要 Provider", "select"),
    providerLabel,
  );
  await browser.waitUntil(
    async () =>
      (await $('[aria-label="压缩影响预览"]').getText()).includes(providerLabel),
    {
      interval: 100,
      timeout: 20_000,
      timeoutMsg: `Maintenance preview did not select Provider: ${providerLabel}`,
    },
  );
}

export async function submitProviderSummary(prompt: string) {
  const field = await controlByLabel("摘要请求", "textarea");
  await field.setValue(prompt);
  await (await button("确认生成并切换 Context")).click();
}

export async function closeMaintenance() {
  const close = await $('button[aria-label="关闭 Context 压缩预览"]');
  if (await close.isExisting()) {
    await close.waitForClickable();
    await close.click();
  }
}

export async function closeVisibleError() {
  const notice = await $(".focus-notice.is-error");
  if (!(await notice.isExisting())) return;
  const close = await notice.$('.//button[normalize-space(.)="关闭"]');
  if (await close.isExisting()) await close.click();
}
