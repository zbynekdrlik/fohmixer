import type { Page } from "@playwright/test";

// The PWA on the https origin (#17, #26): a recording Screen Wake Lock for
// the specs that load the page there (remote.spec.ts, client-report.spec.ts).

/** The e2e job's public name over HTTPS (a per-run test CA both browsers trust). */
export const HTTPS = process.env.E2E_HTTPS_URL || "https://foh.e2e.test:8443";

/**
 * Replaces the browser's Screen Wake Lock with a recording one (an init
 * script: `page.addInitScript(stubWakeLock)`): headless browsers hold no real
 * wake lock, and what is tested is the page's logic (take it on the https
 * origin, take it again when the page comes back).
 */
export function stubWakeLock() {
  const requests: string[] = [];
  const held: (EventTarget & { release(): Promise<void> })[] = [];
  (window as any).__wakeLock = {
    requests,
    releaseAll() {
      for (const sentinel of held.splice(0)) sentinel.release();
    },
  };
  Object.defineProperty(Navigator.prototype, "wakeLock", {
    configurable: true,
    get() {
      return {
        request(type: string) {
          requests.push(type);
          const sentinel = Object.assign(new EventTarget(), {
            released: false,
            type,
            release() {
              if (!sentinel.released) {
                sentinel.released = true;
                sentinel.dispatchEvent(new Event("release"));
              }
              return Promise.resolve();
            },
          });
          held.push(sentinel);
          return Promise.resolve(sentinel);
        },
      };
    },
  });
}

/** The wake lock requests the page made (`stubWakeLock`). */
export async function wakeLockRequests(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).__wakeLock.requests);
}

/** Releases every wake lock the page holds, as the system does when the page is hidden. */
export async function releaseWakeLocks(page: Page): Promise<void> {
  await page.evaluate(() => (window as any).__wakeLock.releaseAll());
}
