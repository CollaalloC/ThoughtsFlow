import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { $, $$, browser, expect } from "@wdio/globals";
import { projectRoot } from "../../scripts/desktop-toolchain";
import { button, controlByLabel, createAgentWorkspace, focusMainWindow, pinMainWindow, selectOptionByText, waitForBodyText } from "./support";
const output = "Herdr fixture output; no model was called";
describe("Herdr workbench through native IPC and generation-1 RPC fixture", () => {
  it("selects native templates, preserves handoffs and evidence, and reads history offline", async () => {
    const state = process.env.TF_AGENT_FIXTURE_STATE;
    if (!state) throw new Error("Isolated fixture runner required");
    await pinMainWindow();
    await createAgentWorkspace({ name: "Herdr 原生验证", goal: "选择上下文并复用原生 Agent；无需 Provider" });
    await (await button("连接 herdr-core")).click();
    await waitForBodyText("0.1.17-fixture");
    await (await controlByLabel("工作目录")).setValue(projectRoot);
    await (await controlByLabel("协作目标", "textarea")).setValue("Herdr 上下文证据闭环");
    await (await button("创建协作")).click();
    await waitForBodyText("已连接 · 每 5 秒更新进展");
    const missionId = await (await $('[aria-label="历史协作"]')).getValue();
    const hashes: string[] = [];
    for (const [index, template] of ["reviewer", "writer"].entries()) {
      const title = index ? "复用观察" : "采集观察";
      await (await controlByLabel("任务标题")).setValue(title);
      await (await controlByLabel("任务说明", "textarea")).setValue("仅执行 fixture，保留 $(echo literal) 文本，不调用模型。");
      await selectOptionByText(await controlByLabel("Agent 模板", "select"), template === "reviewer" ? "reviewer · Fixture review" : "writer · Fixture implementation");
      if (index) await (await $('input[aria-label^="使用证据 "]')).click();
      await (await button("预览交接内容")).click();
      const preview = await $('[aria-label="交接预览"]');
      await preview.waitForDisplayed();
      await expect(preview).toHaveText(expect.stringContaining(`请求模板：${template}`));
      if (index) await expect(await preview.$("pre")).toHaveText(expect.stringContaining(output));
      hashes.push(await (await preview.$("code")).getText());
      await (await button("启动 Agent 任务")).click();
      await waitForBodyText("任务已提交"); await focusMainWindow();
      const task = await $(`[aria-label="任务 ${title}"]`); await task.waitForDisplayed();
      const reply = await $('textarea[aria-label^="回复问题 "]'); await reply.waitForDisplayed();
      await reply.setValue("确认 fixture，结束程序"); await (await button("发送回复")).click();
      await focusMainWindow();
      await browser.waitUntil(async () => (await task.getText()).includes("执行已结束，结果待核实"));
      if (!index) {
        await (await task.$("details.agent-workspace__output > summary")).click();
        await (await button("读取输出")).click();
        await expect(await task.$("pre")).toHaveText(expect.stringContaining(output));
        await (await button("保存当前页观察（重新读取）")).click();
        await waitForBodyText("输出观察已保存，可在下一任务中选择。");
      }
      await (await task.$('.//button[normalize-space(.)="释放已结束的 Agent"]')).click();
      await browser.waitUntil(async () => (await task.getText()).includes("已释放"));
    }
    const calls = readFileSync(`${state}.calls.jsonl`, "utf8").trim().split("\n").map((line) => JSON.parse(line) as { method: string; params: Record<string, unknown> });
    const runs = calls.filter((call) => call.method === "terminal.run");
    expect(runs).toHaveLength(2);
    expect(runs.map((r) => (r.params.argv as string[])[0])).toEqual(["reviewer", "writer"]);
    expect((runs[1].params.argv as string[])[1]).toContain(output);
    for (const run of runs) { expect(run.params.keep).toBe(false); expect(run.params.no_parent).toBe(true); expect(run.params.line).toBeUndefined(); }
    expect(calls.filter((r) => r.method === "terminal.read").every((r) => r.params.lines === 0)).toBe(true);
    writeFileSync(`${state}.offline`, "offline\n");
    await browser.refresh(); await pinMainWindow();
    await waitForBodyText("Herdr 上下文证据闭环"); await waitForBodyText("连接已断开");
    await expect(await $('[aria-label="历史协作"]')).toHaveValue(missionId);
    const records = await $$('[aria-label="历史任务交接"] details'); expect(records).toHaveLength(2);
    const saved: string[]=[];
    for (const record of records) { await (await record.$("summary")).click(); saved.push(await (await record.$("code")).getText()); }
    expect(saved.sort()).toEqual(hashes.sort());
    await expect(await $('[aria-label="历史任务交接"]')).toHaveText(expect.stringContaining("请求模板：writer"));
    const evidence=await $('[aria-label="已保存的 Agent 证据"] details'); await (await evidence.$("summary")).click();
    await expect(await evidence.$("pre")).toHaveText(expect.stringContaining(output));
    await focusMainWindow(); await evidence.scrollIntoView({ block: "center" });
    await browser.saveScreenshot(join(tmpdir(), "thoughtsflow-herdr-native.png"));
    console.log("TF_HERDR_EVIDENCE", JSON.stringify({missionId, hashes, dispatches:runs.length, providerConfigured:false, modelCalled:false}));
  });
});
