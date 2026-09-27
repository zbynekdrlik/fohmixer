import { defineConfig, devices } from "@playwright/test";

// Copied from iemmixer @ 22372bc and adapted: the engineer's iPad (WebKit)
// joins Chromium as a second project, so a WebKit-only console warning fails
// CI from S0 on. Every spec runs in both projects.
export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0, // No retries - tests must pass first time
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
      // The engineer's tablet: iPad Pro 11 in landscape (the surface is
      // landscape-only), WebKit with touch.
      name: "ipad",
      use: { ...devices["iPad Pro 11 landscape"], browserName: "webkit", hasTouch: true },
    },
  ],
});
