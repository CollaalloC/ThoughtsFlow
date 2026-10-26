import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { executeDesktopCommand, planDesktopCommand, projectRoot } from "../../scripts/desktop-toolchain.ts";

const directory = mkdtempSync(join(tmpdir(), "thoughtsflow-agent-journey-"));
const fixturePath = resolve(projectRoot, "tests/fixtures/orca-cli.mjs");
const wrapperPath = join(directory, "orca-cli.mjs");
// The final reload deliberately loses the runtime while retaining the native
// SQLite database. This wrapper never invokes an installed Agent or model.
writeFileSync(wrapperPath, `#!/usr/bin/env node
import { appendFileSync, existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
const state = process.env.TF_AGENT_FIXTURE_STATE;
const argv = process.argv.slice(2);
if (state && existsSync(state + ".offline")) {
  appendFileSync(state + ".offline-calls.jsonl", JSON.stringify({ argv, at: new Date().toISOString() }) + "\\n");
  if (argv.length === 1 && argv[0] === "--version") {
    process.stdout.write("1.4.237-fixture\\n");
  } else if (argv[0] === "status") {
    process.stdout.write(JSON.stringify({ ok: true, result: { runtime: { state: "not_running", reachable: false, runtimeId: null }, graph: { state: "not_running" } }, _meta: { runtimeId: "none" } }) + "\\n");
  } else {
    process.stdout.write(JSON.stringify({ ok: false, error: { code: "runtime_unavailable", message: "Fixture runtime intentionally offline" }, _meta: { runtimeId: "none" } }) + "\\n");
    process.exitCode = 1;
  }
} else {
  const child = spawnSync(process.execPath, [${JSON.stringify(fixturePath)}, ...argv], { env: process.env, stdio: "inherit" });
  if (child.error) throw child.error;
  process.exitCode = child.status ?? 1;
}
`, { mode: 0o700 });
try {
  process.exitCode = executeDesktopCommand(planDesktopCommand("wdio", {
    args: ["run", "wdio.webview.conf.ts", "--spec", "tests/webview/agents.smoke.spec.ts"],
    env: {
      ...process.env,
      THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: join(directory, "app-data"),
      THOUGHSFLOW_ORCA_BIN: wrapperPath,
      THOUGHSFLOW_OMP_BIN: wrapperPath,
      TF_AGENT_FIXTURE_STATE: join(directory, "orca-state.json"),
    },
  }));
} finally {
  rmSync(directory, { recursive: true, force: true });
}
