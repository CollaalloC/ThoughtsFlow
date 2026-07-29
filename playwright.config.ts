import { defineConfig, devices } from "@playwright/test";

const externalBaseUrl = process.env.THOUGHSFLOW_E2E_BASE_URL;
const nativeModeRequested = process.env.THOUGHSFLOW_E2E_NATIVE === "1";

// Normal `npm run test:e2e` is allowed to discover the desktop journeys and
// report them as skipped.  Once native mode is explicitly requested, silently
// skipping because the harness URL was omitted would be a false green result.
if (nativeModeRequested && !externalBaseUrl) {
  throw new Error(
    "THOUGHSFLOW_E2E_NATIVE=1 requires THOUGHSFLOW_E2E_BASE_URL for the native Tauri WebDriver harness.",
  );
}

const nativeHarnessEnabled = nativeModeRequested;

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  // A native Tauri process owns one SQLite database and one WebView session;
  // running journeys concurrently makes restart/recovery evidence invalid.
  workers: nativeHarnessEnabled ? 1 : undefined,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["line"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: externalBaseUrl ?? "http://127.0.0.1:4173",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
  webServer: undefined,
});
