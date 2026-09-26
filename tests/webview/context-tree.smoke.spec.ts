import { $, $$, browser, expect } from "@wdio/globals";
import type { ChainablePromiseElement } from "webdriverio";
import { ProviderFixture } from "../fixtures/provider-server";
import { assertNativePlatform } from "./support";

function xpathLiteral(value: string) {
  if (!value.includes('"')) return `"${value}"`;
  if (!value.includes("'")) return `'${value}'`;
  return `concat(${value
    .split('"')
    .map((part, index) => `${index > 0 ? `, '"', ` : ""}"${part}"`)
    .join("")})`;
}

async function button(name: string) {
  const element = await $(
    `//button[@aria-label=${xpathLiteral(name)} or normalize-space(.)=${xpathLiteral(name)}]`,
  );
  try {
    await element.waitForClickable();
  } catch (reason) {
    const geometry = await browser.execute((target) => {
      const rect = target.getBoundingClientRect();
      return {
        disabled: target.hasAttribute("disabled"),
        rect: rect.toJSON(),
        viewport: { width: innerWidth, height: innerHeight },
        coveringElement: document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)?.outerHTML,
      };
    }, element);
    throw new Error(`${String(reason)}\nButton geometry: ${JSON.stringify(geometry)}\nNative WebView state:\n${await $("body").getText()}`);
  }
  return element;
}

async function controlByLabel(name: string, elementName: "input" | "select" = "input") {
  const direct = await $(`${elementName}[aria-label=${JSON.stringify(name)}]`);
  if (await direct.isExisting()) return direct;

  const element = await $(
    `//label[.//span[normalize-space(.)=${xpathLiteral(name)}]]//${elementName}`,
  );
  await element.waitForDisplayed();
  return element;
}

async function waitForBodyText(expected: string) {
  try {
    await browser.waitUntil(
      async () => (await $("body").getText()).includes(expected),
      {
        interval: 100,
        timeout: 20_000,
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

async function contextTreeItem(prompt: string): Promise<WebdriverIO.Element> {
  const items = await $$('[role="treeitem"]');
  for (const item of items) {
    const label = await item.getAttribute("aria-label");
    if (label?.startsWith(`${prompt} ·`)) return item;
  }
  throw new Error(`Context Tree item was not found for prompt: ${prompt}`);
}

async function selectOptionByText(
  select: WebdriverIO.Element | ChainablePromiseElement,
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
  // The embedded WKWebView driver updates the native select value but does not
  // consistently dispatch the React change event. Dispatching the ordinary DOM
  // events keeps the journey on the same public UI control.
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

describe("ThoughsFlow native desktop WebView", () => {
  const provider = new ProviderFixture();

  before(async () => {
    await provider.start();
  });

  after(async () => {
    await provider.stop();
  });

  it("selects an exact Model Run and keeps the cursor through a WKWebView reload", async () => {
    assertNativePlatform();
    // Pin the single embedded window so the service does not probe the optional
    // advanced WDIO plugin before every ordinary WebDriver command.
    await browser.tauri.switchWindow("main");

    const suffix = `${Date.now()}`;
    const providerName = `WebView Fixture ${suffix}`;
    const workspaceName = `WebView Context Tree ${suffix}`;
    const rootPrompt = `WKWebView 根问题 ${suffix}`;
    const leafPrompt = `WKWebView 叶问题 ${suffix}`;

    const workSurface = await $('[aria-label="工作面"]');
    await workSurface.waitForDisplayed();
    await (await button("Provider 设置")).click();

    const settings = await $('[aria-labelledby="provider-settings-title"]');
    await settings.waitForDisplayed();
    await (await button("新建 Provider")).click();
    await (await controlByLabel("名称")).setValue(providerName);
    await (await controlByLabel("Provider 模板", "select")).selectByAttribute(
      "value",
      "openai-compatible",
    );
    await (await controlByLabel("Base URL")).setValue(`${provider.baseUrl}/v1`);
    await (await controlByLabel("模型")).setValue("fixture-model");
    await (await button("保存 Provider")).click();
    await waitForBodyText("Provider 已保存");

    await (await button("返回 Focus")).click();
    await (await button("添加工作区")).click();
    await (await controlByLabel("工作区名称")).setValue(workspaceName);
    await (await controlByLabel("工作区目标")).setValue("验证原生 Context Tree 游标持久化");
    await (await button("确认创建")).click();
    await waitForBodyText(workspaceName);

    const providerSelect = await $('select[aria-label="Provider"]');
    await selectOptionByText(providerSelect, `${providerName} · fixture-model`);
    await waitForBodyText(new URL(provider.baseUrl).host);

    const composer = await $('textarea[aria-label="消息"]');
    await composer.setValue(rootPrompt);
    await (await button("发送")).click();
    await waitForBodyText(`Fixture 回答 #1：${rootPrompt}（完成）`);

    await composer.setValue(leafPrompt);
    await (await button("发送")).click();
    await waitForBodyText(`Fixture 回答 #1：${leafPrompt}（完成）`);

    await (await button("打开 Context Tree")).click();
    await (await contextTreeItem(rootPrompt)).click();
    try {
      await browser.waitUntil(
        async () =>
          (await (await contextTreeItem(rootPrompt)).getAttribute("aria-current")) === "true",
        {
          interval: 100,
          timeout: 20_000,
          timeoutMsg: "The selected ancestor Model Run never became CURRENT",
        },
      );
    } catch {
      const treeState = [];
      for (const item of await $$('[role="treeitem"]')) {
        treeState.push({
          current: await item.getAttribute("aria-current"),
          label: await item.getAttribute("aria-label"),
        });
      }
      throw new Error(
        `The selected ancestor Model Run never became CURRENT.\nTree state: ${JSON.stringify(treeState)}\n${await $("body").getText()}`,
      );
    }

    await browser.refresh();
    await (await $('[aria-label="工作面"]')).waitForDisplayed();
    const workspaceButton = await $(
      `//nav[@aria-label="工作区"]//button[normalize-space(.)=${xpathLiteral(workspaceName)}]`,
    );
    if (await workspaceButton.isExisting()) await workspaceButton.click();
    await (await button("打开 Context Tree")).click();
    await expect(await contextTreeItem(rootPrompt)).toHaveAttribute("aria-current", "true");

    const sentPrompts = provider
      .capturedRequests()
      .filter((request) => request.pathname === "/v1/chat/completions")
      .map((request) => {
        const messages = Array.isArray(request.body.messages) ? request.body.messages : [];
        const last = messages.at(-1);
        return last && typeof last === "object"
          ? (last as Record<string, unknown>).content
          : undefined;
      });
    expect(sentPrompts).toContain(rootPrompt);
    expect(sentPrompts).toContain(leafPrompt);
  });
});
