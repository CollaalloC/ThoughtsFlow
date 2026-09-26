import { spawnSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

if (process.env.TF_AGENT_LIVE !== "1" || !process.env.TF_AGENT_LIVE_REPO_ID) {
  throw new Error("Explicitly set TF_AGENT_LIVE=1 and TF_AGENT_LIVE_REPO_ID for this real model test");
}
const directory = mkdtempSync(join(tmpdir(), "thoughsflow-real-agent-"));
const env = {
  ...process.env,
  THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: join(directory, "app-data"),
  TF_AGENT_LIVE_DIRECTORY: directory,
};
delete env.TF_AGENT_FIXTURE_STATE;
delete env.THOUGHSFLOW_ORCA_BIN;
delete env.THOUGHSFLOW_OMP_BIN;
console.log(`Live Agent evidence directory: ${directory}`);
// Keep the database and receipts even after failure so an unknown Dispatch is never blindly retried.
const result = spawnSync(process.execPath, ["node_modules/@wdio/cli/bin/wdio.js", "run", "wdio.webview.conf.ts", "--spec", "tests/webview/agents.live.spec.ts"], { stdio: "inherit", env });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
