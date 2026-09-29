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
 *
 * By default every request is granted. With `{ activation: true }`
 * (`page.addInitScript(stubWakeLock, { activation: true })`) the stub grants
 * a request only with user activation, as iPadOS does (WebKit's
 * `WakeLock::request`, #5 K4), and rejects any other with a
 * `NotAllowedError`. Activation is the HTML standard's: a trusted mouse
 * `pointerdown`, a trusted `pointerup` of a finger or pen, or a trusted
 * `touchend` — so a finger's `pointerdown` is none — and it lasts 5 s (both
 * engines' transient activation). The stub models it with its own listeners
 * because Playwright activates the page itself (`page.evaluate`, locator
 * actions), whatever a real device would do.
 */
export function stubWakeLock(options?: { activation?: boolean }) {
  const requests: string[] = [];
  const held: (EventTarget & { release(): Promise<void> })[] = [];
  let activatedAt = Number.NEGATIVE_INFINITY;
  if (options?.activation) {
    const activates = (event: Event) => {
      if (!event.isTrusted) return false;
      if (event.type === "touchend") return true;
      const pointer = (event as PointerEvent).pointerType;
      return event.type === "pointerdown" ? pointer === "mouse" : pointer !== "mouse";
    };
    // The window's capture listeners run before any of the page's own.
    for (const type of ["pointerdown", "pointerup", "touchend"]) {
      window.addEventListener(
        type,
        (event) => {
          if (activates(event)) activatedAt = performance.now();
        },
        { capture: true, passive: true },
      );
    }
  }
  const active = () => !options?.activation || performance.now() - activatedAt < 5000;
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
          if (!active()) {
            return Promise.reject(new DOMException("Permission was denied", "NotAllowedError"));
          }
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

/** The wake lock requests the page made (`stubWakeLock`), granted or refused. */
export async function wakeLockRequests(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).__wakeLock.requests);
}

/** Releases every wake lock the page holds, as the system does when the page is hidden. */
export async function releaseWakeLocks(page: Page): Promise<void> {
  await page.evaluate(() => (window as any).__wakeLock.releaseAll());
}
