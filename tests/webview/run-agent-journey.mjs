import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const directory = mkdtempSync(join(tmpdir(), "thoughsflow-agent-journey-"));
try {
  const result = spawnSync(
    process.execPath,
    ["node_modules/@wdio/cli/bin/wdio.js", "run", "wdio.webview.conf.ts", "--spec", "tests/webview/agents.smoke.spec.ts"],
    {
      stdio: "inherit",
      env: {
        ...process.env,
        THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: join(directory, "app-data"),
        THOUGHSFLOW_ORCA_BIN: resolve("tests/fixtures/orca-cli.mjs"),
        THOUGHSFLOW_OMP_BIN: resolve("tests/fixtures/orca-cli.mjs"),
        TF_AGENT_FIXTURE_STATE: join(directory, "orca-state.json"),
      },
    },
  );
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
} finally {
  rmSync(directory, { recursive: true, force: true });
}
