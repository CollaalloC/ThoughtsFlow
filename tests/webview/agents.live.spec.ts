import { randomUUID } from "node:crypto";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { $, browser, expect } from "@wdio/globals";
import { button, controlByLabel, pinMainWindow, selectOptionByText, waitForBodyText } from "./support";

describe("Explicit opt-in: real Orca and OMP", () => {
  it("runs a bounded file verification task through the real desktop workbench", async () => {
    if (process.env.TF_AGENT_LIVE !== "1" || !process.env.TF_AGENT_LIVE_DIRECTORY || !process.env.TF_AGENT_LIVE_REPO_ID) {
      throw new Error("Live Agent testing requires explicit opt-in, a retained evidence directory and an exact Orca repository ID");
    }
    if (process.env.TF_AGENT_FIXTURE_STATE) throw new Error("A live test cannot use fixture state");
    const directory = process.env.TF_AGENT_LIVE_DIRECTORY;
    const marker = `TF_REAL_OMP_OK_${randomUUID()}`;
    const objective = `ThoughtsFlow live verification ${marker}`;
    const title = "真实 OMP 文件验证";
    const evidence: Record<string, unknown> = { marker, objective, title, startedAt: new Date().toISOString(), status: "opening" };
    const save = () => writeFileSync(join(directory, "journey.json"), JSON.stringify(evidence, null, 2));
    save();
    try {
      await pinMainWindow();
      await (await button("添加工作区")).click();
      await (await controlByLabel("工作区名称")).setValue("真实 OMP 验证");
      await (await controlByLabel("工作区目标")).setValue("验证真实模型、工具执行与 Orca 生命周期");
      await (await button("确认创建")).click();
      await waitForBodyText("真实 OMP 验证");
      await (await button("Agent 协作")).click();
      await (await $('[aria-label="Agent 协作工作面"]')).waitForDisplayed();
      const repository = await controlByLabel("代码仓库", "select");
      const option = await repository.$(`option[value="${process.env.TF_AGENT_LIVE_REPO_ID}"]`);
      await option.waitForExist();
      await selectOptionByText(repository, await option.getText());
      await (await controlByLabel("协作目标", "textarea")).setValue(objective);
      await (await button("创建协作")).click();
      await waitForBodyText("已连接 · 每 5 秒更新进展");
      evidence.missionId = await (await $('[aria-label="历史协作"]')).getValue();
      evidence.status = "mission-created";
      save();
      await (await controlByLabel("任务标题")).setValue(title);
      await (await controlByLabel("任务说明", "textarea")).setValue([
        "Target: only .thoughtsflow-smoke-proof.json in your assigned isolated worktree.",
        `Use a tool to create that UTF-8 JSON file with marker=${JSON.stringify(marker)}, sum=42, and cwd equal to the actual absolute working directory returned by a tool.`,
        "Then use Python's standard library to load the file and assert the marker, 17 + 25 == sum, and cwd.",
        "Do not read project source, install dependencies, edit any other file, commit, push, publish, or start subagents. Keep this test short.",
        `Report the exact marker ${marker} and absolute artifact path in the final summary. Follow your live orchestration preamble to report worker_done with outcome succeeded only after validation.`,
      ].join("\n"));
      await (await button("启动 OMP 任务")).click();
      evidence.status = "dispatch-requested";
      save();
      await browser.waitUntil(async () => {
        const text = await $("body").getText();
        if (text.includes("操作失败：") || text.includes("操作结果尚未确认")) throw new Error(text);
        return text.includes("任务已提交。");
      }, { timeout: 90_000, interval: 1_000, timeoutMsg: "The real launch did not return a confirmed receipt; inspect the retained operation before retrying" });
      const task = await $(`[aria-label="任务 ${title}"]`);
      await task.waitForDisplayed();
      evidence.status = "waiting-for-worker";
      save();
      await browser.waitUntil(async () => {
        const text = await task.getText();
        if (text.includes("失败") || text.includes("已取消")) throw new Error(text);
        return text.includes("已完成") && text.includes("释放已结束的 Agent");
      }, { timeout: 420_000, interval: 3_000, timeoutMsg: "Worker is not confirmed settled; inspect its Dispatch, do not launch a replacement" });
      evidence.status = "worker-settled";
      save();
      await (await task.$("details.agent-workspace__output > summary")).click();
      await (await button("读取输出")).click();
      const output = await task.$(".agent-workspace__output pre");
      await expect(output).toHaveText(expect.stringContaining(marker));
      evidence.output = await output.getText();
      await task.scrollIntoView();
      await browser.saveScreenshot(join(directory, "real-agent.png"));
      await (await button("释放已结束的 Agent")).click();
      await browser.waitUntil(async () => (await task.getText()).includes("已释放"), { timeout: 90_000, interval: 2_000 });
      evidence.status = "ui-journey-passed";
      evidence.completedAt = new Date().toISOString();
      save();
    } catch (reason) {
      evidence.error = String(reason);
      evidence.page = await $("body").getText().catch(() => "unavailable");
      save();
      throw reason;
    }
  });
});
