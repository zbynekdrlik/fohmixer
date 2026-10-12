import type { Browser, BrowserType } from "@playwright/test";

import { expect, test, watchCrash } from "../tests/support/fixtures";
import { jscEnv, jscOf, ownEnv, webProcesses } from "./processes";

// Do JavaScriptCore options given as environment variables reach the engine
// of Playwright's WebKit page? (#43) The stress run's legs set them on the
// test runner; Playwright hands its environment to the browser it launches
// (unless `launch({ env })` says otherwise), and the browser starts its page
// processes. JavaScriptCore reads `JSC_<option>=<value>` from its own
// environment at start (Options.cpp, Linux). Each test launches a WebKit of
// its own (`launchServer`, as crash.spec.ts: its process tree is known), on a
// `data:` page, no hub needed.

const BLANK = "data:text/html,<title>JSC probe</title>";

/** `typeof WebAssembly` in a new page of `browser`, or why the page failed. */
async function wasmType(browser: Browser): Promise<string> {
  const page = await browser.newPage();
  const crash = watchCrash(page, "webkit");
  try {
    await page.goto(BLANK);
    const type = await page.evaluate(() => typeof WebAssembly);
    crash.check();
    return type;
  } catch (error) {
    try {
      crash.check();
    } catch (crashed) {
      return (crashed as Error).message;
    }
    return `the page failed: ${(error as Error).message.split("\n")[0]}`;
  }
}

/** Runs `body` with a WebKit launched with `env`, and the root of its process tree. */
async function withWebKit(
  webkit: BrowserType,
  env: Record<string, string>,
  body: (browser: Browser, root: number) => Promise<void>,
) {
  const server = await webkit.launchServer({ env });
  try {
    const root = server.process().pid;
    expect(root, "the launched browser's process").toBeDefined();
    const browser = await webkit.connect(server.wsEndpoint());
    try {
      await body(browser, root!);
    } finally {
      await browser.close();
    }
  } finally {
    await server.close();
  }
}

/** Every page process under `root` started with the `JSC_` variables of `env`, each with its value. */
function expectPageProcessesHold(root: number, env: Record<string, string>) {
  const pages = webProcesses(root);
  expect(pages.length, "the WebKit page processes").toBeGreaterThan(0);
  for (const pid of pages) expect(jscEnv(pid), `the JSC_ variables of page process ${pid}`).toMatchObject(jscOf(env));
}

test.describe("JavaScriptCore options in the WebKit page process (#43)", () => {
  test("JSC_useWasm=0 takes WebAssembly out of the page, and the page process holds it", async ({ playwright }) => {
    // The option's effect is visible: without `useWasm` JavaScriptCore puts
    // no `WebAssembly` on the global object (JSGlobalObject.cpp).
    const env = { ...ownEnv(), JSC_useWasm: "0" };
    await withWebKit(playwright.webkit, env, async (browser, root) => {
      expect(await wasmType(browser), "typeof WebAssembly with JSC_useWasm=0").toBe("undefined");
      expectPageProcessesHold(root, env);
    });
  });

  test("the leg's own JSC options are options of this WebKit, and its page keeps WebAssembly", async ({ playwright }) => {
    // JSC_validateOptions=1: JavaScriptCore crashes at start on a JSC_
    // variable it does not know or cannot parse, so a mistyped leg fails
    // here instead of measuring the defaults.
    const leg = jscOf(process.env);
    test.info().annotations.push({ type: "JSC options of this leg", description: JSON.stringify(leg) });
    const env = { ...ownEnv(), JSC_validateOptions: "1" };
    await withWebKit(playwright.webkit, env, async (browser, root) => {
      expect(await wasmType(browser), `typeof WebAssembly with this leg's options and JSC_validateOptions=1`).toBe("object");
      expectPageProcessesHold(root, env);
    });
  });
});
