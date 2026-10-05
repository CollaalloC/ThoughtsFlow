import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { button, controlByLabel, createAgentWorkspace, pinMainWindow, selectOptionByText, waitForBodyText } from "./support";

const fixtureOutput = "OMP fixture output; no model was called";

function recordedCommands(path: string): string[][] {
  if (!existsSync(path)) return [];
  return readFileSync(path, "utf8").trim().split("\n").filter(Boolean)
    .map((line) => (JSON.parse(line) as { argv: string[] }).argv);
}

describe("Agent workbench through native IPC and an isolated fixture CLI", () => {
  it("needs no Provider, reuses saved Agent evidence, and reloads immutable history offline", async () => {
    const statePath = process.env.TF_AGENT_FIXTURE_STATE;
    if (!statePath) throw new Error("This journey requires the isolated fixture runner");
    await pinMainWindow();
    await expect(await $('[aria-label="Agent 工作台"]')).toBeDisplayed();
    await expect(await $('select[aria-label="Provider"]')).not.toExist();
    await createAgentWorkspace({ name: "Agent 原生验证", goal: "管理任务上下文与证据；无需配置 Provider，不调用模型" });
    await waitForBodyText("1.4.206-fixture");
    const repository = await controlByLabel("代码仓库", "select");
    const option = await repository.$('option[value="fixture-repo"]');
    await selectOptionByText(repository, await option.getText());
    await (await controlByLabel("协作目标", "textarea")).setValue("原生工作台上下文闭环");
    await (await button("创建协作")).click();
    await waitForBodyText("已连接 · 每 5 秒更新进展");
    const missionId = await (await $('[aria-label="历史协作"]')).getValue();
    expect(missionId).not.toBe("");

    await (await controlByLabel("任务标题")).setValue("采集输出证据");
    await (await controlByLabel("任务说明", "textarea")).setValue("仅执行 fixture，用固定文本验证输出；不得调用模型或修改项目文件。");
    await expect(await $('//button[normalize-space(.)="启动 OMP 任务"]')).toBeDisabled();
    await (await button("预览交接内容")).click();
    const preview = await $('[aria-label="交接预览"]');
    await preview.waitForDisplayed();
    await expect(preview).toHaveText(expect.stringContaining("输出观察 0"));
    const firstHandoffHash = await (await preview.$("code")).getText();
    expect(firstHandoffHash).toMatch(/^[a-f0-9]{64}$/);
    await (await button("启动 OMP 任务")).click();
    await waitForBodyText("任务已提交");
    const firstTask = await $('[aria-label="任务 采集输出证据"]');
    await firstTask.waitForDisplayed();
    const reply = await $('textarea[aria-label^="回复问题 "]');
    await reply.waitForDisplayed();
    await reply.setValue("确认，继续 fixture 验证");
    await (await button("发送回复")).click();
    await browser.waitUntil(async () => (await firstTask.getText()).includes("已完成"));
    await (await firstTask.$("details.agent-workspace__output > summary")).click();
    await (await button("读取输出")).click();
    await expect(await firstTask.$(".agent-workspace__output pre")).toHaveText(expect.stringContaining(fixtureOutput));
    await (await button("保存当前页观察（重新读取）")).click();
    await waitForBodyText("输出观察已保存，可在下一任务中选择。");
    const evidenceSection = await $('[aria-label="已保存的 Agent 证据"]');
    const savedDetails = await evidenceSection.$("details");
    await savedDetails.waitForExist();
    await (await savedDetails.$("summary")).click();
    await expect(await savedDetails.$("pre")).toHaveText(expect.stringContaining(fixtureOutput));
    const evidenceHash = await (await savedDetails.$("code")).getText();
    expect(evidenceHash).toMatch(/^[a-f0-9]{64}$/);
    const evidenceChoice = await $('input[aria-label^="使用证据 "]');
    const evidenceId = await evidenceChoice.getAttribute("data-evidence-id");
    if (!evidenceId) throw new Error("Saved evidence is missing its stable identity");
    await (await firstTask.$('.//button[normalize-space(.)="释放已结束的 Agent"]')).click();
    await browser.waitUntil(async () => (await firstTask.getText()).includes("已释放"));

    await (await controlByLabel("任务标题")).setValue("复用已保存证据");
    await (await controlByLabel("任务说明", "textarea")).setValue("仅使用明确勾选的 fixture 观察继续验证；不调用模型，不修改代码。");
    await evidenceChoice.click();
    await (await button("预览交接内容")).click();
    await preview.waitForDisplayed();
    await expect(preview).toHaveText(expect.stringContaining("输出观察 1"));
    await expect(await preview.$("pre")).toHaveText(expect.stringContaining(fixtureOutput));
    const reusedHandoffText = await (await preview.$("pre")).getProperty("textContent");
    const reusedHandoffHash = await (await preview.$("code")).getText();
    expect(reusedHandoffHash).not.toBe(firstHandoffHash);
    await (await button("启动 OMP 任务")).click();
    const secondTask = await $('[aria-label="任务 复用已保存证据"]');
    await secondTask.waitForDisplayed();
    const secondReply = await $('textarea[aria-label^="回复问题 "]');
    await secondReply.waitForDisplayed();
    await secondReply.setValue("确认本次输入包含保存的证据");
    await (await button("发送回复")).click();
    await browser.waitUntil(async () => (await secondTask.getText()).includes("已完成"));
    await (await secondTask.$('.//button[normalize-space(.)="释放已结束的 Agent"]')).click();
    await browser.waitUntil(async () => (await secondTask.getText()).includes("已释放"));
    await expect(await $('[aria-label="人工编排来源图"]')).toBeDisplayed();

    const starts = recordedCommands(`${statePath}.calls.jsonl`).filter((argv) => argv.slice(0, 2).join(" ") === "orchestration worker-start");
    expect(starts).toHaveLength(2);
    for (const argv of starts) {
      expect(argv[argv.indexOf("--agent") + 1]).toBe("omp");
      expect(argv).not.toContain("--model");
      expect(argv).not.toContain("--effort");
    }
    expect(starts[1][starts[1].indexOf("--spec") + 1]).toBe(reusedHandoffText);
    writeFileSync(`${statePath}.offline`, "fixture runtime intentionally offline\n");
    // Reload destroys all React state. The runtime wrapper now refuses reads
    // and writes, so persisted handoffs/evidence must come from SQLite.
    await browser.refresh();
    await pinMainWindow();
    await waitForBodyText("原生工作台上下文闭环");
    await waitForBodyText("连接已断开");
    await expect(await $('[aria-label="历史协作"]')).toHaveValue(missionId);
    const offlineEvidence = await $('[aria-label="已保存的 Agent 证据"] details');
    await offlineEvidence.waitForExist();
    await (await offlineEvidence.$("summary")).click();
    await expect(await offlineEvidence.$("pre")).toHaveText(expect.stringContaining(fixtureOutput));
    await expect(await offlineEvidence.$("code")).toHaveText(evidenceHash);
    const histories = await $$('[aria-label="历史任务交接"] details');
    expect(histories).toHaveLength(2);
    const hashes: string[] = [];
    for (const history of histories) {
      await (await history.$("summary")).click();
      hashes.push(await (await history.$("code")).getText());
    }
    expect(hashes.sort()).toEqual([firstHandoffHash, reusedHandoffHash].sort());
    await expect(await $('[aria-label="历史任务交接"]')).toHaveText(expect.stringContaining(evidenceId));

    await (await controlByLabel("任务标题")).setValue("离线准备下一任务");
    await (await controlByLabel("任务说明", "textarea")).setValue("只预览来源，离线时不得启动或重派。");
    await (await $('input[aria-label^="使用证据 "]')).click();
    await (await button("预览交接内容")).click();
    await expect(await $('[aria-label="交接预览"] pre')).toHaveText(expect.stringContaining(fixtureOutput));
    await expect(await $('//button[normalize-space(.)="启动 OMP 任务"]')).toBeDisabled();
    const offlineCommands = recordedCommands(`${statePath}.offline-calls.jsonl`);
    expect(offlineCommands.some((argv) => argv[0] === "status")).toBe(true);
    expect(offlineCommands.some((argv) => argv.slice(0, 2).join(" ") === "orchestration worker-start")).toBe(false);
    expect(recordedCommands(`${statePath}.calls.jsonl`).filter((argv) => argv.slice(0, 2).join(" ") === "orchestration worker-start")).toHaveLength(2);
    await (await $('[aria-label="已保存的 Agent 证据"]')).scrollIntoView();
    await browser.saveScreenshot(join(tmpdir(), "thoughtsflow-workbench-native-evidence.png"));
    await (await $('[aria-label="人工编排来源图"]')).scrollIntoView();
    await browser.saveScreenshot(join(tmpdir(), "thoughtsflow-workbench-native-graph.png"));
    console.log(`TF_AGENT_WORKBENCH_EVIDENCE ${JSON.stringify({ missionId, evidenceId, evidenceHash, handoffHashes: hashes, dispatchCount: starts.length, offlineCommands, providerConfigured: false, modelCalled: false })}`);
  });
});
