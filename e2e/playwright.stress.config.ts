import { defineConfig, devices } from "@playwright/test";

// The on-demand WebKit wasm stress run (#43, .github/workflows/wasm-stress.yml):
// thousands of surface loads in the iPad project's WebKit, under one set of
// JavaScriptCore options per run (JSC_* environment variables). Its own test
// folder: the e2e job's config (./tests) never runs these, this one runs only
// these. The iPad project only (the trap is WebKit's), one worker, no
// retries, the same base URL and harness as the e2e job. No trace: one test
// makes every load, and the spec records each fault's message and stack in
// its own report.
export default defineConfig({
  testDir: "./stress",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: [["html", { open: "never" }], ["list"]],
  use: {
    baseURL: process.env.E2E_LINK_URL || process.env.E2E_BASE_URL || "http://127.0.0.1:8480",
    trace: "off",
  },
  projects: [
    {
      name: "ipad",
      use: { ...devices["iPad Pro 11 landscape"], browserName: "webkit", hasTouch: true },
    },
  ],
});
