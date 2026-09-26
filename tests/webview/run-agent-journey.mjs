import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { executeDesktopCommand, planDesktopCommand, projectRoot } from "../../scripts/desktop-toolchain.ts";

const directory = mkdtempSync(join(tmpdir(), "thoughsflow-agent-journey-"));
try {
  process.exitCode = executeDesktopCommand(planDesktopCommand("wdio", {
    args: ["run", "wdio.webview.conf.ts", "--spec", "tests/webview/agents.smoke.spec.ts"],
    env: {
      ...process.env,
      THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: join(directory, "app-data"),
      THOUGHSFLOW_ORCA_BIN: resolve(projectRoot, "tests/fixtures/orca-cli.mjs"),
      THOUGHSFLOW_OMP_BIN: resolve(projectRoot, "tests/fixtures/orca-cli.mjs"),
      TF_AGENT_FIXTURE_STATE: join(directory, "orca-state.json"),
    },
  }));
} finally {
  rmSync(directory, { recursive: true, force: true });
}
