import { execFileSync } from "node:child_process";
import fs from "node:fs";

import type { Browser } from "@playwright/test";

import { expect, pageCrashedMessage, test, watchCrash } from "./support/fixtures";
import { token } from "./support/live";

// A crash of the test browser's page process (#9) must fail its test at once
// and name itself. Before, a WebKit page that crashed while loading the
// surface looked like an app that stopped: `openSurface` waited 5 s and
// reported "stage not found" while the crashed process sat in its core dump
// for 15–45 s. The CI job turns core dumps off for the test browsers; this
// spec crashes a page in the middle of its load and checks, on the runner
// itself, that the page call waiting on it and the console guard's crash
// watch both answer within 5 s, the watch with the guard's named message.
//
// The page lives in a browser of its own (launched here), so the crash
// cannot reach the page of any other test, nor this test's own `page`
// fixture: the console guard keeps its full check for this test too.

/** How long a crash may take to reach the test. */
const CRASH_REPORTED_MS = 5000;

/** The processes under `root` (itself excluded), read from /proc. */
function descendants(root: number): number[] {
  const children = new Map<number, number[]>();
  for (const name of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(name)) continue;
    let stat: string;
    try {
      stat = fs.readFileSync(`/proc/${name}/stat`, "utf8");
    } catch {
      continue; // the process ended while the list was read
    }
    const parent = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
    const list = children.get(parent) ?? [];
    list.push(Number(name));
    children.set(parent, list);
  }
  const found: number[] = [];
  const queue = [...(children.get(root) ?? [])];
  while (queue.length > 0) {
    const pid = queue.shift()!;
    found.push(pid);
    queue.push(...(children.get(pid) ?? []));
  }
  return found;
}

/**
 * The page-content processes of the browser rooted at `root`: WebKit's web
 * processes, Chromium's renderers. Chromium rewrites its processes' titles
 * (`/proc/<pid>/cmdline` then holds one space-separated line), so the flag is
 * looked for in the whole text.
 */
function pageProcesses(root: number): number[] {
  return descendants(root).filter((pid) => {
    let cmdline: string;
    try {
      cmdline = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8");
    } catch {
      return false;
    }
    return cmdline.split("\0")[0].endsWith("/WPEWebProcess") || /(^|[\s\0])--type=renderer([\s\0]|$)/.test(cmdline);
  });
}

/** `value` after `ms`: the loser of a race that bounds a wait. */
function after<T>(ms: number, value: T): Promise<T> {
  return new Promise((resolve) => setTimeout(() => resolve(value), ms));
}

/**
 * Opens the surface in `browser` (its processes under `root`), crashes the
 * page processes right after the navigation commits, and checks what a test
 * sees: the crash watch's named error and the end of the page call waiting
 * on the page, both within `CRASH_REPORTED_MS`.
 */
async function crashMidLoad(browser: Browser, root: number, browserName: string, baseURL: string | undefined) {
  const page = await (await browser.newContext({ baseURL })).newPage();
  const crash = watchCrash(page, browserName);
  const seeded = await token();
  await page.addInitScript((t) => localStorage.setItem("fohmixer_token", t), seeded);
  await page.goto("/", { waitUntil: "commit" });
  expect(() => crash.check(), "no crash before the signal").not.toThrow();
  // What openSurface waits on, with room to show whether the crash ends it early.
  let brokenAt = 0;
  const waiting = expect(page.getByTestId("stage"))
    .toBeVisible({ timeout: 30_000 })
    .then(
      () => "the stage showed",
      (error: Error) => {
        brokenAt = Date.now();
        return error.message;
      },
    );

  const victims = pageProcesses(root);
  expect(victims.length, "the page processes of the launched browser").toBeGreaterThan(0);
  // The crash itself: SIGABRT, a core-dumping signal like the engine's own
  // SIGSEGV. A SIGSEGV sent from outside would not do: JavaScriptCore's fault
  // handler (it catches WASM out-of-bounds accesses) takes it and the page
  // lives on. Sent with the `kill` command, as the hub's tests send their
  // signals (`graceful_stop.rs`); it ends nothing but this test's own
  // browser's page.
  const signalled = Date.now();
  execFileSync("kill", ["-s", "ABRT", ...victims.map(String)]);

  const reported = await Promise.race([crash.crashed.then(() => Date.now() - signalled), after(6000, null)]);
  test.info().annotations.push({ type: "crash reported after (ms)", description: String(reported) });
  expect(reported, "the crash reached the test (null: not within 6 s)").not.toBeNull();
  expect(reported!).toBeLessThan(CRASH_REPORTED_MS);
  expect(() => crash.check()).toThrow(pageCrashedMessage(browserName));

  const outcome = await Promise.race([waiting, after(CRASH_REPORTED_MS, "still waiting")]);
  expect(outcome, "the page call the crash broke").not.toBe("still waiting");
  expect(outcome).not.toBe("the stage showed");
  test.info().annotations.push({
    type: "page call failed after (ms)",
    description: `${brokenAt - signalled}: ${outcome.replace(/\x1b\[[0-9;]*m/g, "").split("\n")[0]}`,
  });
  expect(brokenAt - signalled).toBeLessThan(CRASH_REPORTED_MS);
}

test.describe("A crashed page process (#9)", () => {
  test("fails its test within 5 s with the named message, not as a stalled app", async ({
    playwright,
    browserName,
    baseURL,
  }) => {
    // A browser whose page process still writes a core dump (the job's
    // setting not in force) closes only after the dump: room for that, so
    // the test fails on its own assertion, not on its timeout.
    test.setTimeout(90_000);
    const browserType = playwright[browserName];
    const server = await browserType.launchServer();
    try {
      const root = server.process().pid;
      expect(root, "the launched browser's process").toBeDefined();
      const browser = await browserType.connect(server.wsEndpoint());
      try {
        await crashMidLoad(browser, root!, browserName, baseURL);
      } finally {
        await browser.close();
      }
    } finally {
      await server.close();
    }
  });
});
