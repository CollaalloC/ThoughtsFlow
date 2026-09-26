import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { executeDesktopCommand, planDesktopCommand, requireLiveAgentOptIn } from "../../scripts/desktop-toolchain.ts";

requireLiveAgentOptIn();
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
process.exitCode = executeDesktopCommand(planDesktopCommand("wdio", {
  args: ["run", "wdio.webview.conf.ts", "--spec", "tests/webview/agents.live.spec.ts"], env,
}));
