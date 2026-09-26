import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  desktopBinaryPath, executeDesktopCommand, planDesktopCommand, requireLiveAgentOptIn,
} from "./desktop-toolchain.ts";

test("build plans preserve paths and arguments on all supported desktop systems", () => {
  for (const platform of ["darwin", "linux", "win32"]) {
    const root = platform === "win32" ? "C:\\Projects\\Thinking Space" : "/Projects/Thinking Space";
    const nodePath = platform === "win32" ? "C:\\Program Files\\nodejs\\node.exe" : "/tools/node";
    const env = { PATH: "test path", CARGO_TARGET_DIR: "stale", KEEP: "value" };
    const plan = planDesktopCommand("webview-build", { platform, root, nodePath, env });
    assert.equal(plan.command, nodePath);
    assert.equal(plan.options.shell, false);
    assert.equal(plan.options.cwd, root);
    assert.equal(plan.options.env.KEEP, "value");
    assert.equal(plan.options.env.VITE_THOUGHSFLOW_WEBVIEW_E2E, "1");
    assert.equal(env.CARGO_TARGET_DIR, "stale", "planning must not mutate parent environment");
    assert.deepEqual(plan.args.slice(1), ["build", "--debug", "--no-bundle", "--features", "webview-e2e", "--config", "src-tauri/tauri.webview.conf.json"]);
    assert.ok(plan.args[0].endsWith(platform === "win32" ? "@tauri-apps\\cli\\tauri.js" : "@tauri-apps/cli/tauri.js"));
    assert.equal(desktopBinaryPath(root, platform), joinFor(platform, plan.options.env.CARGO_TARGET_DIR, "debug", platform === "win32" ? "thoughsflow.exe" : "thoughsflow"));
    const literal = "path with spaces; $(echo no-shell) & more";
    const wdio = planDesktopCommand("wdio", { platform, root, args: ["run", literal] });
    assert.equal(wdio.args.at(-1), literal);
    assert.ok(!wdio.args[0].includes(".bin"));
  }
});

function joinFor(platform, ...parts) {
  return parts.join(platform === "win32" ? "\\" : "/");
}

test("unknown OS and command fail before spawning; live agent requires both opt-ins", () => {
  assert.throws(() => planDesktopCommand("webview-build", { platform: "freebsd" }), /Unsupported desktop/);
  assert.throws(() => desktopBinaryPath("/tmp", "aix"), /Unsupported desktop/);
  assert.throws(() => planDesktopCommand("unknown"), /Unknown desktop task/);
  for (const env of [{}, { TF_AGENT_LIVE: "1" }, { TF_AGENT_LIVE_REPO_ID: "repo" }, { TF_AGENT_LIVE: "1", TF_AGENT_LIVE_REPO_ID: "  " }]) {
    assert.throws(() => requireLiveAgentOptIn(env), /Explicitly set/);
  }
  assert.doesNotThrow(() => requireLiveAgentOptIn({ TF_AGENT_LIVE: "1", TF_AGENT_LIVE_REPO_ID: "repo" }));
  assert.equal(planDesktopCommand("webview-smoke", { env: {} }).options.env.TF_WEBVIEW_LIVE_PROXY, undefined);
  assert.equal(planDesktopCommand("webview-live-proxy", { env: {} }).options.env.TF_WEBVIEW_LIVE_PROXY, "1");
});

test("execution keeps nonzero exit status and never evaluates shell syntax", () => {
  const root = mkdtempSync(join(tmpdir(), "thoughsflow runner space "));
  try {
    const script = join(root, "child with spaces.mjs");
    const literal = "$(echo must-remain-literal); &";
    writeFileSync(script, `import assert from "node:assert/strict"; assert.equal(process.argv[2], ${JSON.stringify(literal)}); process.exit(7);`);
    const plan = planDesktopCommand("wdio", { root, env: process.env });
    plan.args = [script, literal];
    assert.equal(executeDesktopCommand(plan), 7);
    assert.throws(() => executeDesktopCommand({ ...plan, command: join(root, "missing-node") }), /ENOENT/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
