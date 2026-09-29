import type { Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { BASE, HUB_SOCKET, RELOAD_KEY, countLoads, hubStatus, openSurface } from "./support/live";
import { HTTPS, releaseWakeLocks, stubWakeLock } from "./support/pwa";

// The pages' diagnostic reports (#26): every page tells the hub what it is
// and what happens to it (`POST /api/client-report`), and the hub keeps the
// newest in `/api/status` `client_reports`; its `perf` reports (#5, K4) say
// how fast it draws and how many fingers it saw at once. And a hub of
// another build reloads an open page onto its bundle (the update after a
// deploy).
//
// Every spec shares one hub, so a test finds its own page's reports by the
// page's user agent and screen (each describe below gives the page a screen
// size no other test uses) and by the time the test started.

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
async function reportOf(w: Who, since: number, match: (report: any) => boolean, timeout = 10_000): Promise<any> {
  let found: any;
  await expect
    .poll(
      async () => {
        const reports: any[] = (await hubStatus()).client_reports;
        found = reports.find((r) => r.ua === w.ua && r.screen === w.screen && r.at >= since && match(r));
        return found !== undefined;
      },
      { timeout },
    )
    .toBe(true);
  return found;
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

test.describe("A dropped connection", () => {
  test.use({ contextOptions: { screen: { width: 1121, height: 621 } } });

  test("is reported, and so is the reconnect with its count", async ({ page }) => {
    const sockets: WebSocketRoute[] = [];
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      sockets.push(ws);
      ws.connectToServer();
    });
    const since = nowSecs();
    await openSurface(page);
    const w = await who(page);
    // The socket drops (a Wi-Fi blip); the page reconnects.
    await sockets[0].close({ code: 4000, reason: "gone" });
    await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    const lost = await reportOf(w, since, (r) => r.kind === "disconnected");
    expect(lost.reconnects).toBe("0");
    const back = await reportOf(w, since, (r) => r.kind === "reconnect");
    expect(back.reconnects).toBe("1");
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
    // The page comes back and takes the lock again within 5 s: the kind's
    // trailing report carries the new state, so the hub does not keep
    // "released".
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await expect(page.locator("html")).toHaveAttribute("data-wake-lock", "held");
    const held = await reportOf(w, since, (r) => r.kind === "wake-lock" && r.wake_lock === "held");
    // It went when the kind's 5 s window ended, not at once (the hub
    // stamps whole seconds).
    expect(held.at - released.at).toBeGreaterThanOrEqual(4);
  });
});

test.describe("The frame rate", () => {
  test.use({ contextOptions: { screen: { width: 1123, height: 623 } } });

  test("is reported with the longest frame once ten seconds of frames are counted", async ({ page }) => {
    // The first window closes after 10 s of the surface's frames; the
    // report goes then (later ones at most once a minute).
    test.setTimeout(60_000);
    const since = nowSecs();
    await openSurface(page);
    const w = await who(page);
    const perf = await reportOf(w, since, (r) => r.kind === "perf" && r.fps !== null, 30_000);
    // One decimal, whole milliseconds: numbers the hub log shows as words.
    expect(perf.fps).toMatch(/^\d+\.\d$/);
    expect(perf.long_frame_ms).toMatch(/^\d+$/);
    const fps = Number(perf.fps);
    expect(fps).toBeGreaterThan(0);
    expect(fps).toBeLessThan(1000);
    // The longest gap is at least the window's mean gap.
    expect(Number(perf.long_frame_ms)).toBeGreaterThanOrEqual(Math.floor(1000 / fps));
    // Nothing touched this page.
    expect(perf.touches_max).toBe("0");
    expect(perf.pointer).toBeNull();
    expect(perf.visibility).toBe("visible");
  });
});

test.describe("Fingers at once", () => {
  test.use({ contextOptions: { screen: { width: 1125, height: 625 } } });

  test("are reported at once when the page sees more than ever, lifted and cancelled ones not counted", async ({
    page,
  }) => {
    // The first periodic report (about 10 s of frames), then two trailing
    // touch reports (at most 5 s each).
    test.setTimeout(90_000);
    const since = nowSecs();
    await openSurface(page);
    const w = await who(page);
    // The first periodic report: the next periodic one is a minute away,
    // so a report within the next seconds comes from the fingers alone.
    await reportOf(w, since, (r) => r.kind === "perf" && r.fps !== null, 30_000);
    // Pointer events with distinct ids, as a multi-finger touch delivers
    // them on the iPad (multitouch.spec.ts). The page counts them in the
    // capture phase, so the element they land on does not matter.
    const touch = (steps: [string, number][]) =>
      page.evaluate((list) => {
        for (const [type, id] of list) {
          document.body.dispatchEvent(
            new PointerEvent(type, {
              pointerId: id,
              pointerType: "touch",
              isPrimary: id === 21 || id === 31,
              bubbles: true,
              cancelable: true,
            }),
          );
        }
      }, steps);
    // Shown again: the frame window starts over, so a touch report now has
    // no frame rate yet.
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    // Never more than three at once: one cancelled, one lifted while the
    // others stay. A cancel or an up the page missed would count four.
    await touch([
      ["pointerdown", 21],
      ["pointerdown", 22],
      ["pointercancel", 22],
      ["pointerdown", 23],
      ["pointerdown", 24],
      ["pointerup", 21],
      ["pointerdown", 25],
      ["pointerup", 23],
      ["pointerup", 24],
      ["pointerup", 25],
    ]);
    const three = await reportOf(
      w,
      since,
      (r) => r.kind === "perf" && r.fps === null && (r.touches_max === "3" || r.touches_max === "4"),
    );
    expect(three.touches_max).toBe("3");
    expect(three.pointer).toBe("touch");
    // Four at once: more than ever, reported with the kind's next report.
    await touch([
      ["pointerdown", 31],
      ["pointerdown", 32],
      ["pointerdown", 33],
      ["pointerdown", 34],
      ["pointerup", 31],
      ["pointerup", 32],
      ["pointerup", 33],
      ["pointerup", 34],
    ]);
    const four = await reportOf(w, since, (r) => r.kind === "perf" && r.touches_max === "4");
    expect(four.pointer).toBe("touch");
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
