import { defineConfig, devices } from "@playwright/test";

// The real Companion 5.0.7 job (#52, ci.yml `companion`): the Stream Deck tab
// against Companion's official container, seeded with
// e2e/companion/test.companionconfig. Its own test folder: the e2e job's
// config (./tests) never runs these, this one runs only these. Both projects,
// one worker, no retries, as the main config.
export default defineConfig({
  testDir: "./companion",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: [["html", { open: "never" }], ["list"]],
  use: {
    baseURL: process.env.E2E_BASE_URL || "http://127.0.0.1:8480",
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "ipad",
      use: { ...devices["iPad Pro 11 landscape"], browserName: "webkit", hasTouch: true },
    },
  ],
});
