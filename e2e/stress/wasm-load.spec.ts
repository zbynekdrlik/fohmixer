import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { Browser, BrowserContextOptions } from "@playwright/test";

import { expect, test, watchCrash } from "../tests/support/fixtures";
import { harness, servedLayout, token, until } from "../tests/support/live";
import { jscEnv, jscOf, webProcesses } from "./processes";

// The WebKit wasm trap (#43), on demand. Playwright's Linux WebKit now and
// then throws a WebAssembly `RuntimeError` ("access to a null reference",
// "Unreachable code should not be executed" or "Out of bounds memory access")
// inside libcore's float formatting while the strips render the first time,
// about once in 40 CI runs. This loads the surface STRESS_LOADS times with a
// layout of 32 strips (the FOH page as two rows of 16; the screen shows the
// ones that fit, 28 on the iPad), each load in a fresh browser context as
// every normal test gets one (so a fresh page process), and counts the loads
// that trapped, with the load number, the time after `goto`, the message and
// the stack. Other page errors, console errors and warnings, crashed page
// processes (the test browser's own crash, .claude/rules/e2e.md) and loads
// that never showed their strips are counted apart. It keeps going to the
// end, so the count is a rate, and fails then if anything was recorded.
//
// The run's JavaScriptCore options are `JSC_*` environment variables (the
// legs of .github/workflows/wasm-stress.yml); the first load checks that
// its page processes hold them. The report (STRESS_REPORT, JSON) is
// written as the loads go, so a run cut short still has its numbers.

const LOADS_TEXT = process.env.STRESS_LOADS ?? "500";
if (!/^[0-9]+$/.test(LOADS_TEXT) || Number(LOADS_TEXT) < 1) throw new Error(`STRESS_LOADS must be a whole number from 1, not ${LOADS_TEXT}`);
const LOADS = Number(LOADS_TEXT);
const LEG = process.env.STRESS_LEG ?? "local";

/** A load's own bound: the stage, the connection and the strips. */
const READY_MS = 15_000;
/** How long a load stays open after its strips showed (a late trap still counts). */
const SETTLE_MS = 250;
/** The budget of one load in the test's timeout (a load takes ~1-2 s on the runner). */
const LOAD_BUDGET_MS = 4_000;
/**
 * The strips a load must show: on the iPad (1194 px) the column leaves two
 * sides of 519 px, 7 strips of 64 px each, on each of the two lines (28);
 * 24 leaves room for a wider column.
 */
const MIN_STRIPS = 24;
/** The report is rewritten every this many loads (and after every fault). */
const SAVE_EVERY = 25;
/** The longest list of each kind kept in the report (every one is counted). */
const KEEP = 100;
/** The stack lines kept of each error. */
const STACK_LINES = 60;
const STRIPS = '[data-testid="strip"]';
/** A WebAssembly trap: its error name, or one of the messages it has come with. */
const TRAP = /\bRuntimeError\b|access to a null reference|Unreachable code should not be executed|Out of bounds memory access/;

const FIXTURE = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
// The fixture's synthetic track names (SimLive's set, both instances).
const BAND = ["Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #", "Vocal 1 repro#", "Vocal 2 repro#", "Vocal 3 repro#", "Drums #", "Bass #"];
const MASTER = ["Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #", "Vocal 1 repro#", "Vocal 2 repro#", "Vocal 3 repro#"];

/** The fixture with its FOH page as two rows of 16 strips (band, then master). */
function stressLayout(): any {
  const layout = JSON.parse(readFileSync(FIXTURE, "utf-8"));
  const foh = layout.pages.find((p: any) => p.id === "foh");
  const strip = (instance: string, name: string) => ({
    kind: "strip",
    binding: { instance, anchor: { kind: "track", name } },
    strip_kind: "standard",
  });
  const row = (n: number) => ({
    sections: [
      { kind: "group", id: `stress-${n}-band`, title: `BAND ${n}`, controls: BAND.map((name) => strip("band", name)) },
      { kind: "group", id: `stress-${n}-master`, title: `MASTER ${n}`, controls: MASTER.map((name) => strip("master", name)) },
    ],
  });
  foh.rows = [row(1), row(2)];
  return layout;
}

/** Whether the hub serves the stress layout (its first group). */
const servesStress = (layout: any) =>
  layout.pages.find((p: any) => p.id === "foh")?.rows?.[0]?.sections?.[0]?.id === "stress-1-band";

type Fault = { load: number; ms: number; name: string; message: string; stack: string };

type Report = {
  leg: string;
  jsc: Record<string, string>;
  webkit: string;
  loads_requested: number;
  loads: number;
  finished: boolean;
  ms_per_load: number | null;
  strips: { min: number | null; max: number | null };
  counts: { traps: number; page_errors: number; console: number; crashes: number; stalls: number };
  traps: Fault[];
  page_errors: Fault[];
  console: { load: number; text: string }[];
  crashes: number[];
  stalls: { load: number; why: string }[];
  page_process_jsc: Record<string, string>[];
};

/** The report's path: STRESS_REPORT, else the test's output folder. */
const reportPath = () => process.env.STRESS_REPORT || test.info().outputPath("wasm-stress.json");

function save(report: Report) {
  const path = reportPath();
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(report, null, 1));
}

const firstLine = (text: string) => text.split("\n")[0];

/** One line of the leg's numbers. */
function summary(report: Report): string {
  const c = report.counts;
  const rate = c.traps === 0 ? `0 in ${report.loads}` : `1 in ${Math.round(report.loads / c.traps)}`;
  return (
    `wasm stress ${report.leg} ${JSON.stringify(report.jsc)}: ${c.traps} traps in ${report.loads} loads (${rate}), ` +
    `other page errors ${c.page_errors}, console ${c.console}, crashes ${c.crashes}, stalls ${c.stalls}, ` +
    `${report.ms_per_load} ms per load, strips ${report.strips.min}-${report.strips.max}`
  );
}

/**
 * What one load saw: the surface opened in a fresh context, until its
 * strips showed (and `SETTLE_MS` more) or a page error, a crash or
 * `READY_MS` came first. At the first load, the JSC options its page
 * process holds.
 */
async function load(
  browser: Browser,
  options: BrowserContextOptions,
  seeded: string,
  n: number,
  report: Report,
  checkProcess: boolean,
) {
  const context = await browser.newContext(options);
  try {
    await context.addInitScript((t) => {
      try {
        localStorage.setItem("fohmixer_token", t);
      } catch {
        // storage blocked: the load then fails on the login page
      }
    }, seeded);
    const page = await context.newPage();
    const crash = watchCrash(page, "webkit");
    const errors: Fault[] = [];
    let start = Date.now();
    let seen: () => void = () => {};
    const errored = new Promise<"error">((resolve) => (seen = () => resolve("error")));
    page.on("pageerror", (error) => {
      errors.push({
        load: n,
        ms: Date.now() - start,
        name: error.name,
        message: error.message,
        stack: (error.stack ?? "").split("\n").slice(0, STACK_LINES).join("\n"),
      });
      seen();
    });
    page.on("console", (msg) => {
      if (msg.type() !== "error" && msg.type() !== "warning") return;
      report.counts.console += 1;
      if (report.console.length < KEEP) report.console.push({ load: n, text: `[${msg.type()}] ${msg.text()}` });
    });
    start = Date.now();
    const ready = (async () => {
      await page.goto("/");
      await page.waitForFunction(
        (min) =>
          document.querySelector('[data-testid="stage"]') !== null &&
          document.querySelector('[data-testid="surface"]')?.getAttribute("data-connected") === "true" &&
          document.querySelectorAll('[data-testid="strip"]').length >= min,
        MIN_STRIPS,
        { polling: "raf", timeout: READY_MS },
      );
      return "ready" as const;
    })().catch((error: Error) => ({ stalled: firstLine(error.message) }));
    const outcome = await Promise.race([ready, errored, crash.crashed.then(() => "crash" as const)]);
    // A load that showed its strips, or trapped, stays a moment: what
    // follows at once belongs to it too.
    if (outcome === "ready" || outcome === "error") await new Promise((done) => setTimeout(done, SETTLE_MS));
    let crashed = false;
    try {
      crash.check();
    } catch {
      crashed = true;
    }
    if (checkProcess) {
      const pages = webProcesses(process.pid);
      report.page_process_jsc = pages.map(jscEnv);
      save(report);
      expect(pages.length, "the WebKit page processes of this test").toBeGreaterThan(0);
      // The leg's settings, each with its value (any JSC_ variable WebKit
      // adds itself would be there in the normal suite too; the report shows it).
      for (const pid of pages) {
        expect(jscEnv(pid), `the JSC_ variables of page process ${pid}: this leg's, or the run measures something else`).toMatchObject(report.jsc);
      }
    }
    // `counts.traps` counts loads (the rate's numerator); `traps` keeps every trap's error.
    if (errors.some((error) => TRAP.test(`${error.name}: ${error.message}`))) report.counts.traps += 1;
    for (const error of errors) {
      if (TRAP.test(`${error.name}: ${error.message}`)) {
        report.traps.push(error);
      } else {
        report.counts.page_errors += 1;
        if (report.page_errors.length < KEEP) report.page_errors.push(error);
      }
    }
    if (crashed) {
      report.counts.crashes += 1;
      report.crashes.push(n);
    } else if (outcome === "ready") {
      const strips = await page.locator(STRIPS).count();
      report.strips.min = Math.min(report.strips.min ?? strips, strips);
      report.strips.max = Math.max(report.strips.max ?? strips, strips);
    } else if (typeof outcome === "object" && errors.length === 0) {
      report.counts.stalls += 1;
      if (report.stalls.length < KEEP) report.stalls.push({ load: n, why: outcome.stalled });
    }
    return errors.length > 0 || crashed || typeof outcome === "object";
  } finally {
    await context.close();
  }
}

test.describe("The WebKit wasm trap at page load, on demand (#43)", () => {
  test(`${LOADS} loads of a 32-strip surface without a page error`, async ({
    browser,
    baseURL,
    viewport,
    userAgent,
    deviceScaleFactor,
    isMobile,
    hasTouch,
  }) => {
    test.setTimeout(LOADS * LOAD_BUDGET_MS + 300_000);
    const options: BrowserContextOptions = { baseURL, viewport, userAgent, deviceScaleFactor, isMobile, hasTouch };
    const report: Report = {
      leg: LEG,
      jsc: jscOf(process.env),
      webkit: browser.version(),
      loads_requested: LOADS,
      loads: 0,
      finished: false,
      ms_per_load: null,
      strips: { min: null, max: null },
      counts: { traps: 0, page_errors: 0, console: 0, crashes: 0, stalls: 0 },
      traps: [],
      page_errors: [],
      console: [],
      crashes: [],
      stalls: [],
      page_process_jsc: [],
    };
    save(report);
    const seeded = await token();
    await harness("/hub/layout", { layout: stressLayout() });
    try {
      await until(servedLayout, servesStress, "the hub serving the 32-strip layout");
      const began = Date.now();
      for (let n = 1; n <= LOADS; n++) {
        const fault = await load(browser, options, seeded, n, report, n === 1);
        report.loads = n;
        report.ms_per_load = Math.round((Date.now() - began) / n);
        if (fault || n % SAVE_EVERY === 0) save(report);
      }
      report.finished = true;
      save(report);
    } finally {
      await harness("/hub/layout/reset");
      await until(servedLayout, (layout) => !servesStress(layout), "the hub serving the fixture layout again");
    }
    const line = summary(report);
    console.log(line);
    test.info().annotations.push({ type: "wasm stress", description: line });
    await test.info().attach("wasm-stress.json", { path: reportPath(), contentType: "application/json" });
    for (const trap of report.traps) console.log(`trap at load ${trap.load}, ${trap.ms} ms after goto:\n${trap.stack || trap.message}`);
    expect.soft(report.counts.page_errors, `other page errors: ${JSON.stringify(report.page_errors.slice(0, 3))}`).toBe(0);
    expect.soft(report.counts.console, `console errors and warnings: ${JSON.stringify(report.console.slice(0, 3))}`).toBe(0);
    expect.soft(report.counts.crashes, `crashed page processes at loads ${report.crashes.join(", ")} (a test-browser crash)`).toBe(0);
    expect.soft(report.counts.stalls, `loads that never showed ${MIN_STRIPS} strips: ${JSON.stringify(report.stalls.slice(0, 3))}`).toBe(0);
    expect(report.counts.traps, `WebAssembly traps in ${report.loads} loads (${line})`).toBe(0);
  });
});
