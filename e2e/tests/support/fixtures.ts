import { test as base, expect, type Page } from "@playwright/test";

type ConsoleGuard = {
  /** Console messages this test deliberately provokes (e.g. a 401 it asks for). */
  allowedConsole: RegExp[];
  consoleGuard: void;
};

/**
 * What a test fails with when its page's browser process crashes (#9). The
 * test browser's own engine crashes now and then (Playwright's WebKit build:
 * about once in 3,500 loads, in JavaScriptCore); the page then shows nothing
 * and answers nothing, which reads like the app stopping. It is not.
 */
export function pageCrashedMessage(browserName: string): string {
  const engine = browserName === "webkit" ? "WebKit" : browserName === "chromium" ? "Chromium" : browserName;
  return `the ${engine} page process crashed (a test-browser crash, see .claude/rules/e2e.md), not an app stall`;
}

export type CrashWatch = {
  /** Settles with the named error when the page's process crashes. */
  crashed: Promise<Error>;
  /** Throws the named error once the page's process has crashed. */
  check(): void;
};

/**
 * Watches `page` for a crash of its browser process (`page.on('crash')`).
 * The console guard checks it after every test, so a crashed page fails its
 * test with `pageCrashedMessage` next to whatever page call the crash broke.
 *
 * The crashed page is closed at once: Playwright rejects a pending page call
 * on a crash only between its retries, and a call that waits for the page's
 * script context (a locator right after a navigation) would otherwise sit
 * until its own timeout. Closing the page ends every pending call on it.
 */
export function watchCrash(page: Page, browserName: string): CrashWatch {
  let crashedPage = false;
  let closeFailure: string | undefined;
  const named = () =>
    new Error(
      pageCrashedMessage(browserName) +
        (closeFailure === undefined ? "" : ` (closing the crashed page failed too: ${closeFailure})`),
    );
  const crashed = new Promise<Error>((resolve) => {
    page.once("crash", () => {
      crashedPage = true;
      // A failed close is reported with the crash (check), never dropped.
      page.close().catch((closing: Error) => {
        closeFailure = closing.message;
      });
      resolve(named());
    });
  });
  return {
    crashed,
    check() {
      if (crashedPage) throw named();
    },
  };
}

/**
 * Every test fails if the browser console shows an error, a warning or a page
 * error that it did not declare in `allowedConsole` (clean-console rule), and
 * with a named error if the browser process of its page, or of any other page
 * its context opens, crashed. Both are reported when both happened.
 */
export const test = base.extend<ConsoleGuard>({
  allowedConsole: [[], { option: true }],
  consoleGuard: [
    async ({ page, allowedConsole, browserName }, use) => {
      // Playwright reads an array whose second element is an object as a
      // `[value, options]` fixture tuple, so `test.use({ allowedConsole:
      // [/a/, /b/] })` would arrive here as the single RegExp /a/. Lists of
      // two or more patterns go in the tuple form
      // `[[/a/, /b/], { scope: "test" }]`.
      if (!Array.isArray(allowedConsole)) {
        throw new Error(
          "allowedConsole must be a RegExp[]: wrap two or more patterns as [[/a/, /b/], { scope: \"test\" }]",
        );
      }
      const crashes = [watchCrash(page, browserName)];
      const watchOther = (other: Page) => crashes.push(watchCrash(other, browserName));
      page.context().on("page", watchOther);
      const problems: string[] = [];
      page.on("console", (msg) => {
        if (msg.type() !== "error" && msg.type() !== "warning") return;
        const text = msg.text();
        if (allowedConsole.some((pattern) => pattern.test(text))) return;
        problems.push(`[${msg.type()}] ${text}`);
      });
      page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
      await use();
      page.context().off("page", watchOther);
      // Soft, so a crash below does not hide what the console showed before it.
      expect.soft(problems, "browser console must stay clean").toEqual([]);
      for (const crash of crashes) crash.check();
    },
    { auto: true },
  ],
});

export { expect };
export type { Page } from "@playwright/test";
