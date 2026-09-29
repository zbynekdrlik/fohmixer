import type { Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { BASE, hubStatus, openSurface } from "./support/live";
import { HTTPS, releaseWakeLocks, stubWakeLock } from "./support/pwa";

// The pages' diagnostic reports (#26): every page tells the hub what it is
// and what happens to it (`POST /api/client-report`), and the hub keeps the
// newest in `/api/status` `client_reports`. And a hub of another build
// reloads an open page onto its bundle (the update after a deploy).
//
// Every spec shares one hub, so a test finds its own page's reports by the
// page's user agent and screen (each describe below gives the page a screen
// size no other test uses) and by the time the test started.

/** The page's hub socket. */
const HUB_SOCKET = /\/ws\?/;

/** Where the page keeps its last handshake reload (wall clock ms). */
const RELOAD_KEY = "fohmixer_proto_reload_at";

/** Unix seconds now (the hub stamps its reports in the same clock). */
const nowSecs = () => Math.floor(Date.now() / 1000);

/** How a report names its page: its user agent and screen. */
type Who = { ua: string; screen: string };

async function who(page: Page): Promise<Who> {
  return page.evaluate(() => ({
    ua: navigator.userAgent,
    screen: `${screen.width}x${screen.height}@${devicePixelRatio}`,
  }));
}

/** Waits until the hub keeps a report of page `w` since `since` that `match` accepts. */
async function reportOf(w: Who, since: number, match: (report: any) => boolean): Promise<any> {
  let found: any;
  await expect
    .poll(
      async () => {
        const reports: any[] = (await hubStatus()).client_reports;
        found = reports.find((r) => r.ua === w.ua && r.screen === w.screen && r.at >= since && match(r));
        return found !== undefined;
      },
      { timeout: 10_000 },
    )
    .toBe(true);
  return found;
}

/** Every page load of `page`, counted. */
function countLoads(page: Page): () => number {
  let loads = 0;
  page.on("load", () => (loads += 1));
  return () => loads;
}

test.describe("A browser tab", () => {
  test.use({ contextOptions: { screen: { width: 1111, height: 611 } } });

  test("reports its load, its connection and its visibility to the hub", async ({ page }) => {
    const since = nowSecs();
    const build = (await (await fetch(`${BASE}/api/version`)).json()).version;
    await openSurface(page);
    const w = await who(page);
    const host = await page.evaluate(() => location.host);
    const load = await reportOf(w, since, (r) => r.kind === "load");
    expect(load).toMatchObject({
      source: "lan",
      display: "browser",
      build,
      host,
      visibility: "visible",
      reconnects: "0",
      error: null,
    });
    // The plain-http path: no service worker, no wake lock.
    expect(load.sw).toBeNull();
    expect(load.wake_lock).toBeNull();
    const connected = await reportOf(w, since, (r) => r.kind === "connected");
    expect(connected.reconnects).toBe("0");
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    const visibility = await reportOf(w, since, (r) => r.kind === "visibility");
    expect(visibility.visibility).toBe("visible");
  });
});

test.describe("A Home Screen app", () => {
  test.use({ contextOptions: { screen: { width: 1113, height: 613 } } });

  test("reports itself standalone", async ({ page }) => {
    // iOS marks a Home Screen app with navigator.standalone.
    await page.addInitScript(() =>
      Object.defineProperty(navigator, "standalone", { value: true, configurable: true }),
    );
    const since = nowSecs();
    await openSurface(page);
    const load = await reportOf(await who(page), since, (r) => r.kind === "load");
    expect(load.display).toBe("standalone");
  });
});

test.describe("A JavaScript error", () => {
  test.use({ contextOptions: { screen: { width: 1115, height: 615 } } });

  test("is reported with where it happened", async ({ page }) => {
    const since = nowSecs();
    await openSurface(page);
    // A dispatched event reaches the page's listener without being an
    // uncaught error (which the console guard would fail).
    await page.evaluate(() =>
      window.dispatchEvent(
        new ErrorEvent("error", {
          message: "e2e synthetic error",
          filename: "https://example.org/e2e.js",
          lineno: 3,
          colno: 9,
        }),
      ),
    );
    const report = await reportOf(await who(page), since, (r) => r.kind === "error");
    expect(report.error).toBe("e2e synthetic error at https://example.org/e2e.js:3:9");
  });
});

test.describe("An unhandled promise rejection", () => {
  test.use({ contextOptions: { screen: { width: 1117, height: 617 } } });

  test("is reported with its reason", async ({ page }) => {
    const since = nowSecs();
    await openSurface(page);
    await page.evaluate(() =>
      window.dispatchEvent(
        new PromiseRejectionEvent("unhandledrejection", {
          promise: Promise.resolve(),
          reason: new Error("e2e synthetic rejection"),
        }),
      ),
    );
    const report = await reportOf(await who(page), since, (r) => r.kind === "error");
    expect(report.error).toBe("unhandled rejection: e2e synthetic rejection");
  });
});

test.describe("The PWA on the https origin", () => {
  test.use({ baseURL: HTTPS, contextOptions: { screen: { width: 1119, height: 619 } } });

  test("reports its service worker and its wake lock", async ({ page }) => {
    await page.addInitScript(stubWakeLock);
    const since = nowSecs();
    await openSurface(page);
    const w = await who(page);
    await expect(page.locator("html")).toHaveAttribute("data-sw", "registered");
    // The registration lands before or after the app starts: the load
    // report or the service worker's own report names it.
    await reportOf(w, since, (r) => r.sw === "registered");
    // The stub grants the lock before the app starts.
    const load = await reportOf(w, since, (r) => r.kind === "load");
    expect(load.wake_lock).toBe("held");
    // The system releases it (the page is hidden): reported.
    await releaseWakeLocks(page);
    const released = await reportOf(w, since, (r) => r.kind === "wake-lock");
    expect(released.wake_lock).toBe("released");
  });
});

test.describe("A hub of another build", () => {
  test("reloads the open page onto its bundle once, then the page keeps itself", async ({ page }) => {
    // The hub's hello names another build, as after a deploy with the same
    // protocol.
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        const msg = JSON.parse(String(message));
        if (msg.type === "hello") {
          ws.send(JSON.stringify({ ...msg, build: "0.0.0-e2e-other" }));
        } else {
          ws.send(message);
        }
      });
    });
    const loads = countLoads(page);
    await openSurface(page);
    // The first hello reloaded the page; the reloaded page, told the same
    // within the minute, keeps itself and is connected.
    await expect.poll(loads, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await page.waitForTimeout(3000);
    expect(loads()).toBe(2);
    expect(Number(await page.evaluate((key) => localStorage.getItem(key), RELOAD_KEY))).toBeGreaterThan(
      Date.now() - 60_000,
    );
  });
});
