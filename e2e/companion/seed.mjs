// Seeds the real Companion of the `companion` CI job (#52) with the
// synthetic test configuration, through Companion 5.0.7's own Import /
// Export page (Companion's HTTP API cannot create actions):
//
//     cd e2e && COMPANION_URL=http://127.0.0.1:8000 node companion/seed.mjs companion/test.companionconfig
//
// The page uploads the file over its tRPC socket, so the file is set only
// once the socket is up (the sidebar's version comes over it). A fresh
// Companion shows its welcome wizard and What's New over the page, which
// leave the page behind them aria-hidden: CSS locators. They can open late
// on a slow runner, so the import is clicked in a bounded loop: a click that
// a modal blocks gives up after a moment, the visible closers are clicked,
// and the click is tried again. "Import Preserving Unselected" keeps
// Companion's settings (the Satellite API stays on). Exit 0 once the test
// variables exist. With SEED_TRACE=<file.zip> a failed seed leaves its
// Playwright trace there (the job's failure evidence); a failure to write it
// is logged and never hides the seed's own error.
import { chromium } from "@playwright/test";

const base = process.env.COMPANION_URL ?? "http://127.0.0.1:8000";
const trace = process.env.SEED_TRACE;
const file = process.argv[2];
if (!file) {
  throw new Error("usage: node companion/seed.mjs <export file>");
}
// How long one try of the import click waits for a modal to go, and how
// long the tries may take in all.
const CLICK_TRY_MS = 2_000;
const CLICK_ALL_MS = 60_000;

/** Runs `step` (closing the trace or the browser); its failure is logged, never thrown. */
async function quietly(what, step) {
  try {
    await step();
  } catch (e) {
    console.error(`seed: ${what} failed: ${e}`);
  }
}

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
    const deadline = Date.now() + CLICK_ALL_MS;
    let closed = 0;
    let blocked = "no try yet";
    for (;;) {
      if (Date.now() > deadline) throw new Error(`the Import button took no click within ${CLICK_ALL_MS} ms: ${blocked}`);
      if ((await importButton.count()) === 0) throw new Error("the import dialog closed before its Import button took a click");
      if ((await closers.count()) > 0) {
        // A modal over the page (the welcome wizard, What's New): close it
        // and let it go before looking again.
        try {
          await closers.first().click({ timeout: CLICK_TRY_MS });
          closed += 1;
        } catch (e) {
          blocked = `a modal's closer took no click: ${e}`;
        }
        await page.waitForTimeout(500);
        continue;
      }
      try {
        await importButton.click({ timeout: CLICK_TRY_MS });
        break;
      } catch (e) {
        // A modal that opened meanwhile takes the pointer: the next round closes it.
        blocked = `${e}`;
      }
    }
    console.log(`modals closed: ${closed}`);
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
    if (trace) await quietly("writing the trace", () => context.tracing.stop(seeded ? {} : { path: trace }));
  }
} finally {
  await quietly("closing the browser", () => browser.close());
}
