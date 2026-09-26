import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { desktopBinaryPath } from "./scripts/desktop-toolchain.ts";
import "@wdio/native-types";
import type { TauriCapabilities } from "@wdio/tauri-service";

const appBinaryPath = desktopBinaryPath();
const suppliedDataDir =
  process.env.THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR?.trim();
const appDataDir =
  suppliedDataDir || join(tmpdir(), `thoughsflow-webview-${process.pid}`);

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./tests/webview/context-tree.smoke.spec.ts"],
  maxInstances: 1,
  maxInstancesPerCapability: 1,
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: appBinaryPath,
      },
    } as TauriCapabilities,
  ],
  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath,
        captureBackendLogs:
          process.env.TF_WEBVIEW_CAPTURE_BACKEND_LOGS === "1",
        commandTimeout: 60_000,
        driverProvider: "embedded",
        embeddedPort: 4_445,
        env: {
          THOUGHSFLOW_WEBVIEW_E2E_DATA_DIR: appDataDir,
          THOUGHSFLOW_NODE_BIN: process.execPath,
          ...Object.fromEntries(
            ["THOUGHSFLOW_ORCA_BIN", "THOUGHSFLOW_OMP_BIN", "TF_AGENT_FIXTURE_STATE"]
              .flatMap((key) => process.env[key] ? [[key, process.env[key]!]] : []),
          ),
        },
        startTimeout: 90_000,
      },
    ],
  ],
  framework: "mocha",
  reporters: ["spec"],
  logLevel: "warn",
  waitforTimeout: 20_000,
  connectionRetryTimeout: 90_000,
  connectionRetryCount: 1,
  mochaOpts: {
    timeout: process.env.TF_AGENT_LIVE === "1" ? 600_000 : 120_000,
  },
  onComplete: () => {
    if (!suppliedDataDir) {
      rmSync(appDataDir, { force: true, recursive: true });
    }
  },
};
