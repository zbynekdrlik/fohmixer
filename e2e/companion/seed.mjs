// Seeds the real Companion of the `companion` CI job (#52) with the
// synthetic test configuration, through Companion 5.0.7's own Import /
// Export page (Companion's HTTP API cannot create actions):
//
//     cd e2e && COMPANION_URL=http://127.0.0.1:8000 node companion/seed.mjs companion/test.companionconfig
//
// The page uploads the file over its tRPC socket, so the file is set only
// once the socket is up (the sidebar's version comes over it). A fresh
// Companion shows its welcome wizard and What's New over the page, which
// leave the page behind them aria-hidden: CSS locators, and the visible
// closers clicked. "Import Preserving Unselected" keeps Companion's settings
// (the Satellite API stays on). Exit 0 once the test variables exist.
// With SEED_TRACE=<file.zip> a failed seed leaves its Playwright trace there
// (the job's failure evidence).
import { chromium } from "@playwright/test";

const base = process.env.COMPANION_URL ?? "http://127.0.0.1:8000";
const trace = process.env.SEED_TRACE;
const file = process.argv[2];
if (!file) {
  throw new Error("usage: node companion/seed.mjs <export file>");
}
// The most modals a fresh Companion opens over the page (the welcome wizard,
// What's New): more rounds than that means a closer that does not close.
const CLOSE_ROUNDS = 10;
const launch = process.env.CHROME ? { executablePath: process.env.CHROME } : {};
const browser = await chromium.launch(launch);
let seeded = false;
try {
  const context = await browser.newContext();
  if (trace) await context.tracing.start({ screenshots: true, snapshots: true });
  try {
    const page = await context.newPage();
    await page.goto(`${base}/import-export`);
    await page.getByText("v5.0.7", { exact: true }).waitFor({ timeout: 60_000 });
    await page.locator('input[type=file][accept=".companionconfig,.yaml"]').setInputFiles(file);
    const importButton = page.locator("button", { hasText: "Import Preserving Unselected" });
    await importButton.waitFor({ timeout: 30_000 });
    const closers = page.locator('[aria-label="Close modal"]').filter({ visible: true });
    let closed = 0;
    while ((await closers.count()) > 0) {
      if (closed === CLOSE_ROUNDS) throw new Error(`a modal still open after ${CLOSE_ROUNDS} closes`);
      await closers.first().click();
      closed += 1;
      await page.waitForTimeout(500);
    }
    console.log(`modals closed: ${closed}`);
    await importButton.click();
    await importButton.waitFor({ state: "detached", timeout: 30_000 });
    const values = {};
    for (const name of ["light_a", "scene_1", "hold_c"]) {
      const response = await page.request.get(`${base}/api/custom-variable/${name}/value`);
      values[name] = (await response.text()).trim();
    }
    console.log(`seeded: ${JSON.stringify(values)}`);
    if (values.light_a !== "off" || values.scene_1 !== "idle" || values.hold_c !== "up") {
      throw new Error(`the import did not create the test variables: ${JSON.stringify(values)}`);
    }
    seeded = true;
  } finally {
    if (trace) await context.tracing.stop(seeded ? {} : { path: trace });
  }
} finally {
  await browser.close();
}
