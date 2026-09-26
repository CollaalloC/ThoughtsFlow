import { spawnSync } from "node:child_process";
import { posix, resolve, win32 } from "node:path";

export const projectRoot = resolve(import.meta.dirname, "..");

interface CommandOptions {
  root?: string;
  platform?: NodeJS.Platform;
  nodePath?: string;
  env?: NodeJS.ProcessEnv;
  args?: string[];
}

function desktopPaths(platform: NodeJS.Platform) {
  if (!["darwin", "linux", "win32"].includes(platform)) {
    throw new Error(`Unsupported desktop platform: ${platform}`);
  }
  return platform === "win32" ? win32 : posix;
}

export function desktopBinaryPath(root = projectRoot, platform = process.platform) {
  return desktopPaths(platform).join(
    root, "src-tauri", "target", "webview-e2e", "debug",
    platform === "win32" ? "thoughsflow.exe" : "thoughsflow",
  );
}

/** Build argv directly; npm's .cmd shims and shell parsing are never involved. */
export function planDesktopCommand(task: string, options: CommandOptions = {}) {
  const {
    root = projectRoot, platform = process.platform, nodePath = process.execPath,
    env = process.env, args = [],
  } = options;
  const paths = desktopPaths(platform);
  const childEnv = { ...env };
  let cli: string;
  let cliArgs: string[];
  switch (task) {
    case "tauri":
      cli = "@tauri-apps/cli/tauri.js";
      cliArgs = args;
      break;
    case "webview-build":
      cli = "@tauri-apps/cli/tauri.js";
      cliArgs = ["build", "--debug", "--no-bundle", "--features", "webview-e2e", "--config", "src-tauri/tauri.webview.conf.json", ...args];
      childEnv.VITE_THOUGHSFLOW_WEBVIEW_E2E = "1";
      childEnv.CARGO_TARGET_DIR = paths.join(root, "src-tauri", "target", "webview-e2e");
      break;
    case "wdio":
      cli = "@wdio/cli/bin/wdio.js";
      cliArgs = args;
      break;
    case "webview-smoke":
    case "webview-live-proxy":
      cli = "@wdio/cli/bin/wdio.js";
      cliArgs = ["run", "wdio.webview.conf.ts"];
      if (task === "webview-live-proxy") {
        childEnv.TF_WEBVIEW_LIVE_PROXY = "1";
        cliArgs.push("--spec", "tests/webview/live-proxy.spec.ts");
      }
      cliArgs.push(...args);
      break;
    default:
      throw new Error(`Unknown desktop task: ${task}`);
  }
  return {
    command: nodePath,
    args: [paths.join(root, "node_modules", cli), ...cliArgs],
    options: { cwd: root, env: childEnv, shell: false as const, stdio: "inherit" as const },
  };
}

export function executeDesktopCommand(plan: ReturnType<typeof planDesktopCommand>) {
  const result = spawnSync(plan.command, plan.args, plan.options);
  if (result.error) throw result.error;
  return result.status ?? 1;
}

export function requireLiveAgentOptIn(env: NodeJS.ProcessEnv = process.env) {
  if (env.TF_AGENT_LIVE !== "1" || !env.TF_AGENT_LIVE_REPO_ID?.trim()) {
    throw new Error("Explicitly set TF_AGENT_LIVE=1 and TF_AGENT_LIVE_REPO_ID for this real model test");
  }
}
