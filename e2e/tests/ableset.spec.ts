import { randomBytes } from "node:crypto";
import http from "node:http";
import type { Locator, Page, Route } from "@playwright/test";
import { ABLESET_AUD, TUNNEL, accessJwt } from "./support/access";
import { expect, test } from "./support/fixtures";
import {
  BASE,
  HUB_SOCKET,
  RELOAD_KEY,
  countLoads,
  fakeAbleSet,
  frames,
  harness,
  hubEvents,
  openDeck,
  openSurface,
  pageEvents,
  pageHits,
  token,
  until,
} from "./support/live";
import { HTTPS } from "./support/pwa";

// AbleSet's tab (#90 PR C; the owner's variant A of
// docs/mockups/ableset-tab-v1.html): ABLESET is the last tab, right after
// STREAM DECK; its page puts the column at the right edge and AbleSet's own
// page, framed from AbleSet's name through the hub, in the rest. The e2e
// job's harness runs the fake AbleSet (e2e/harness/fake_ableset.py) behind
// the hub's `[ableset]` (`ableset.e2e.test`, a sibling of `foh.e2e.test`):
// its page keeps a `browserId` in localStorage, opens a Socket.IO-like
// socket (`data-ws="echo"`), has a long list and a button that POSTs
// `/api/press`. The frame needs fohmixer's own https page; the surface on
// the emergency address (the suite's plain http by address) shows a panel.

const ABLESET = new URL(process.env.E2E_ABLESET_URL || "https://ableset.e2e.test:8443");
/** The probe's address: AbleSet's name with the page's port, a number making each probe new. */
const PROBE_URL = new RegExp(`^${ABLESET.origin.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/\\.fohmixer/probe\\?\\d+$`);
const isProbe = (url: URL) => PROBE_URL.test(url.href);
/** The page's own requests a hub restart under it fails (#90): its layout fetch and its reports, on fohmixer's https page. */
const PAGE_API = new RegExp(
  `^${new URL(HTTPS).origin.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/api/(layout|client-report)$`,
);
/** The hub's page on AbleSet's name that stores AbleSet's lock, then opens AbleSet (#90, `ableset::LOCKED`). */
const LOCKED = "/.fohmixer/locked";
/** The lock page's script on AbleSet's name (`ableset::LOCK_SCRIPT`). */
const LOCK_SCRIPT_URL = new RegExp(`^${ABLESET.origin.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/\\.fohmixer/locked\\.js$`);
/** How long the probe may take (`behave::ableset::PROBE_MS`). */
const PROBE_MS = 8_000;
const SANDBOX =
  "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads";

/** `/api/status` `ableset` with a token taken once (a login hashes the PIN). */
async function ablesetStatus(bearer: string): Promise<any> {
  const response = await fetch(`${BASE}/api/status`, { headers: { authorization: `Bearer ${bearer}` } });
  if (!response.ok) throw new Error(`status: ${response.status}`);
  return (await response.json()).ableset;
}

/** AbleSet's tab tapped: its page, whose `data-state` says what it shows. */
async function openAbleSet(page: Page): Promise<Locator> {
  const tab = page.getByTestId("ableset-tab");
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
  return page.getByTestId("ableset");
}

const frameOf = (page: Page) => page.frameLocator('[data-testid="ableset-frame"]');

/** The frame's document on AbleSet's name. */
const inside = (page: Page) => page.frames().find((f) => f.url().startsWith(ABLESET.origin));

/** The hub's `ableset_lock` records since `since` (a lock the lock page could not store, #90), once there is one. */
const lockRecords = (since: number) =>
  until(
    async () => (await hubEvents()).filter((r: any) => r.ev === "ableset_lock" && r.ts >= since),
    (r) => r.length > 0,
    "the hub's record of the lock that could not be stored",
  );

/** AbleSet's page loaded in the frame, its socket echoing through the hub: its body. */
async function ablesetLoaded(page: Page): Promise<Locator> {
  const body = frameOf(page).locator("body");
  await expect(body).toHaveAttribute("data-app", "loaded");
  await expect(body).toHaveAttribute("data-ws", "echo");
  return body;
}

/** The page hidden or shown, as an iPad's lock or app switch does (deck.spec.ts's `setHidden`). */
async function setHidden(page: Page, hidden: boolean) {
  await page.evaluate((on) => {
    const doc = document as any;
    if (on) {
      Object.defineProperty(doc, "hidden", { configurable: true, get: () => true });
      Object.defineProperty(doc, "visibilityState", { configurable: true, get: () => "hidden" });
    } else {
      delete doc.hidden;
      delete doc.visibilityState;
    }
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  }, hidden);
}

/** A layout page's tab: the mixer again. */
async function leaveForTheMixer(page: Page) {
  await page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]').first().click();
  await expect(page.getByTestId("page")).toBeVisible();
}

/** The page's AbleSet events since `since` (its flight recorder, through the hub's `trace` records), once `ok` holds. */
function ablesetEvents(since: number, ok: (events: any[]) => boolean, what: string): Promise<any[]> {
  return until(
    async () =>
      pageEvents(await hubEvents()).filter((e: any) => (e.ev === "ableset" || e.ev === "ableset_view") && e.t >= since),
    ok,
    what,
    15_000,
  );
}

/**
 * The hub's `ableset` message to a socket of its own opened with `headers`
 * (a raw upgrade of the hub's /ws, as remote.spec.ts's `upgrade`): each page
 * is told whether its own socket came over the LAN (#90 PR C review M2). A
 * browser's socket cannot carry the tunnel's header (Playwright's extra
 * headers never reach a WebSocket handshake: the hub logged `source="lan"`
 * for the internet-path test's sockets), so the hub's side is read here.
 */
function toldOfAbleSet(headers: Record<string, string>): Promise<any> {
  return new Promise(async (resolve, reject) => {
    const url = new URL(BASE);
    const request = http.request({
      host: url.hostname,
      port: url.port,
      path: `/ws?token=${await token()}&proto=2`,
      headers: {
        Connection: "Upgrade",
        Upgrade: "websocket",
        "Sec-WebSocket-Version": "13",
        "Sec-WebSocket-Key": randomBytes(16).toString("base64"),
        ...headers,
      },
    });
    request.on("upgrade", (_response, socket, head) => {
      // The hub's frames are unmasked: a byte of FIN and opcode, the length
      // (7 bits, or 126 / 127 and 2 / 8 bytes), the payload.
      let data = Buffer.from(head);
      const timer = setTimeout(() => {
        socket.destroy();
        reject(new Error("no ableset message within 5 s"));
      }, 5_000);
      const read = () => {
        while (data.length >= 2) {
          let length = data[1] & 0x7f;
          let at = 2;
          if (length === 126) {
            if (data.length < 4) return;
            length = data.readUInt16BE(2);
            at = 4;
          } else if (length === 127) {
            if (data.length < 10) return;
            length = Number(data.readBigUInt64BE(2));
            at = 10;
          }
          if (data.length < at + length) return;
          const text = (data[0] & 0x0f) === 1 ? data.subarray(at, at + length).toString("utf8") : "";
          data = data.subarray(at + length);
          if (text.includes('"type":"ableset"')) {
            clearTimeout(timer);
            socket.destroy();
            resolve(JSON.parse(text));
            return;
          }
        }
      };
      socket.on("data", (chunk: Buffer) => {
        data = Buffer.concat([data, chunk]);
        read();
      });
      read();
    });
    request.on("response", (response) => {
      response.resume();
      reject(new Error(`no upgrade: ${response.statusCode}`));
    });
    request.on("error", reject);
    request.end();
  });
}

/**
 * The page's hub socket routed through, the hub's `ableset` message told as
 * through the tunnel (`lan` false). The internet-path test's own socket
 * cannot carry the tunnel's header (see `toldOfAbleSet`), so the hub tells
 * it the LAN; this stands in for the tunnel on that one field, which
 * `toldOfAbleSet` reads from the hub itself.
 */
async function asThroughTheTunnel(page: Page): Promise<() => number> {
  let told = 0;
  await page.routeWebSocket(HUB_SOCKET, (ws) => {
    const server = ws.connectToServer();
    server.onMessage((message) => {
      if (typeof message === "string" && message.includes('"type":"ableset"')) {
        told += 1;
        ws.send(JSON.stringify({ ...JSON.parse(message), lan: false }));
        return;
      }
      ws.send(message);
    });
  });
  return () => told;
}

test.describe("AbleSet's tab (the LAN path, fohmixer's own https page)", () => {
  test.use({ baseURL: HTTPS });

  test("is the last tab, frames AbleSet's page through the hub, and a page's tab takes it away", async ({ page }) => {
    const since = Date.now();
    const bearer = await token();
    const pressed = (await fakeAbleSet.state()).presses;
    await openSurface(page);
    // The tab: from the hub's message, last, right after the Stream Deck's,
    // both as wide as the bar (the mockup; since #148 every tab of the bar
    // is a row of its own, the page tabs' larger), no red dot.
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    const ids = await bar.locator(":scope > button").evaluateAll((tabs) => tabs.map((t) => t.getAttribute("data-testid")));
    expect(ids.slice(-2)).toEqual(["deck-tab", "ableset-tab"]);
    const tab = page.getByTestId("ableset-tab");
    await expect(tab).toHaveText("AbleSet");
    await expect(tab).toHaveAttribute("data-offline", "false");
    await expect(page.getByTestId("ableset-dot")).toHaveCount(0);
    const width = async (l: Locator) => (await l.boundingBox())!.width;
    expect(Math.abs((await width(tab)) - (await width(page.getByTestId("deck-tab"))))).toBeLessThan(1);
    expect(await width(tab)).toBeGreaterThan(0.9 * (await width(bar)));
    // The service worker controls the page when the probe runs (#90: its
    // own fetch of the probe failed under its connect-src 'self').
    await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);

    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "frame");
    const frame = page.getByTestId("ableset-frame");
    // The hub's page that stores AbleSet's lock first (#90).
    await expect(frame).toHaveAttribute("src", `${ABLESET.origin}${LOCKED}`);
    await expect(frame).toHaveAttribute("sandbox", SANDBOX);
    await expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    await expect(frame).toHaveAttribute("allow", "");
    await ablesetLoaded(page);
    // Variant A: the column at the right edge, AbleSet left of it.
    const viewport = page.viewportSize()!;
    const column = (await area.getByTestId("column").boundingBox())!;
    const framed = (await frame.boundingBox())!;
    expect(column.x + column.width).toBeGreaterThan(viewport.width - 12);
    expect(framed.x + framed.width).toBeLessThanOrEqual(column.x);
    expect(framed.width).toBeGreaterThan(viewport.width - column.width - 40);
    // AbleSet's socket goes through the hub, and a press inside the frame reaches AbleSet.
    await until(() => ablesetStatus(bearer), (s) => s.sockets >= 1, "AbleSet's socket counted");
    // The hub's record of that socket opening (design §6): the frame's own
    // page, over the LAN (#90 PR C review L8).
    const tunnels = async (what: string) =>
      (await hubEvents()).filter((r: any) => r.ev === "ableset_ws" && r.what === what && r.ts >= since);
    const [opened] = await until(
      async () => (await tunnels("open")).filter((r: any) => r.origin === ABLESET.host),
      (r) => r.length === 1,
      "the hub's record of AbleSet's socket opening",
    );
    expect(opened.source).toBe("lan");
    await frameOf(page).locator("#fake-button").click();
    await expect(frameOf(page).locator("body")).toHaveAttribute("data-pressed", "ok");
    expect((await fakeAbleSet.state()).presses).toBe(pressed + 1);
    // A page's tab beside the frame takes AbleSet away: the frame and its socket go.
    await leaveForTheMixer(page);
    await expect(page.getByTestId("ableset")).toHaveCount(0);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await expect(tab).toHaveAttribute("data-selected", "false");
    await until(() => ablesetStatus(bearer), (s) => s.sockets === 0, "AbleSet's socket closed");
    // And its close: the same tunnel, its time and its bytes both ways.
    const [closed] = await until(
      async () => (await tunnels("close")).filter((r: any) => r.client === opened.client),
      (r) => r.length === 1,
      "the hub's record of AbleSet's socket closing",
    );
    expect(closed.ms).toBeGreaterThan(0);
    expect(closed.up).toBeGreaterThan(0);
    expect(closed.down).toBeGreaterThan(0);
    // The page's flight recorder: the tab's opening, the brief check, the
    // probe's answer, the frame's load and the tab's closing.
    const events = await ablesetEvents(
      since,
      (e) => e.some((x) => x.ev === "ableset_view" && x.on === false),
      "the tab's events",
    );
    const steps = events.map((e) =>
      e.ev === "ableset_view" ? `view ${e.on}` : e.what === "probe" ? `probe ${e.ok}` : e.what === "panel" ? `panel ${e.panel}` : e.what,
    );
    expect(steps).toEqual(["view true", "panel checking", "probe true", "load", "view false"]);
    const timed = events.filter((e) => e.what === "probe" || e.what === "load");
    for (const e of timed) expect(e.ms, e.what).toBeGreaterThanOrEqual(0);
  });

  test("a finger reaches AbleSet's own page inside the frame", async ({ page, browserName }) => {
    await openSurface(page);
    await openAbleSet(page);
    await ablesetLoaded(page);
    if (browserName === "chromium") {
      // A touch drag (Chromium's touch emulation: the desktop project has no
      // touch) scrolls AbleSet's list: the page's own `touch-action: none`
      // does not reach into the frame.
      const box = (await page.getByTestId("ableset-frame").boundingBox())!;
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
      const x = box.x + box.width / 2;
      const y = box.y + box.height - 100;
      await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
      for (let i = 1; i <= 15; i++) {
        await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x, y: y - i * 20 }] });
      }
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
      const inside = page.frame({ url: /ableset\.e2e\.test/ })!;
      await expect.poll(() => inside.evaluate(() => document.scrollingElement!.scrollTop)).toBeGreaterThan(100);
    } else {
      // WebKit's Playwright has no touch drag (the scroll is the real iPad's
      // check, design §5.8 L1): a real finger's tap presses AbleSet's button.
      const pressed = (await fakeAbleSet.state()).presses;
      const button = (await frameOf(page).locator("#fake-button").boundingBox())!;
      await page.touchscreen.tap(button.x + button.width / 2, button.y + button.height / 2);
      await expect(frameOf(page).locator("body")).toHaveAttribute("data-pressed", "ok");
      expect((await fakeAbleSet.state()).presses).toBe(pressed + 1);
    }
  });

  test("AbleSet's storage stays across opens and a reload, so AbleSet sees one device", async ({ page }) => {
    await openSurface(page);
    const ids: string[] = [];
    const openAndRead = async () => {
      await openAbleSet(page);
      ids.push((await (await ablesetLoaded(page)).getAttribute("data-browser-id"))!);
    };
    await openAndRead();
    await leaveForTheMixer(page);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await openAndRead();
    // A reload opens the mixer, never the tab (it is not remembered).
    await page.reload();
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await expect(page.getByTestId("page")).toBeVisible();
    await expect(page.getByTestId("ableset-tab")).toHaveAttribute("data-selected", "false");
    await openAndRead();
    expect(ids[0]).toMatch(/^b[a-z0-9]+$/);
    expect(ids).toEqual([ids[0], ids[0], ids[0]]);
  });

  test("the Stream Deck's tab still works beside it", async ({ page }) => {
    await openSurface(page);
    const area = await openAbleSet(page);
    await ablesetLoaded(page);
    await openDeck(page);
    await expect(area).toHaveCount(0);
    await expect(page.getByTestId("ableset-tab")).toHaveAttribute("data-selected", "false");
    await openAbleSet(page);
    await expect(page.getByTestId("deck")).toHaveCount(0);
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "false");
    await expect(area).toHaveAttribute("data-state", "frame");
    await ablesetLoaded(page);
  });
});

test.describe("AbleSet opened from the tab is locked (#90, the owner's report of 2026-10-14)", () => {
  // AbleSet 3.4.5 reads its lock once at load from its own storage
  // (`AbleSetLocked`, "true": locked; anything else or nothing: unlocked in a
  // browser). The fake's page says what it read (`data-locked`, `none`:
  // nothing stored). The tab's frame opens the hub's own page on AbleSet's
  // name first, which stores "true" and then opens AbleSet's own page.
  test.use({ baseURL: HTTPS });

  test("every open of the tab loads AbleSet locked, also after AbleSet was unlocked", async ({ page }) => {
    await fakeAbleSet.clear();
    // AbleSet left unlocked on this device: its own page, top-level on its
    // name, stores "false" as AbleSet's own unlock does. The hub leaves
    // AbleSet's own page as it is: loaded again there, it is still unlocked.
    const own = page.locator("body");
    await page.goto(`${ABLESET.origin}/`);
    await expect(own).toHaveAttribute("data-locked", "none");
    await page.evaluate(() => localStorage.setItem("AbleSetLocked", "false"));
    await page.reload();
    await expect(own).toHaveAttribute("data-app", "loaded");
    await expect(own).toHaveAttribute("data-locked", "false");
    // The tab opens it locked, and the frame ends on AbleSet's own page.
    await openSurface(page);
    await openAbleSet(page);
    let body = await ablesetLoaded(page);
    await expect(body).toHaveAttribute("data-locked", "true");
    expect(inside(page)?.url()).toBe(`${ABLESET.origin}/`);
    // Unlocked inside AbleSet (its button stores "false"), the tab left and
    // opened again: a fresh frame, locked again.
    await body.evaluate(() => localStorage.setItem("AbleSetLocked", "false"));
    await leaveForTheMixer(page);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await openAbleSet(page);
    body = await ablesetLoaded(page);
    await expect(body).toHaveAttribute("data-locked", "true");
    expect(inside(page)?.url()).toBe(`${ABLESET.origin}/`);
    // The lock page and its script are the hub's own: AbleSet never sees them.
    const paths = pageHits((await fakeAbleSet.state()).hits).map((h) => h.path);
    expect(paths.filter((path) => path.startsWith("/.fohmixer/"))).toEqual([]);
  });

  test("storage refused: AbleSet's page all the same, and the hub records why", async ({ page }) => {
    const since = Date.now();
    // Storage refused on the lock page only, as a browser that blocks a
    // site's storage throws there (AbleSet's own page keeps its storage).
    await page.addInitScript((path) => {
      if (location.pathname === path) {
        Storage.prototype.setItem = function () {
          throw new DOMException("refused by the test", "SecurityError");
        };
      }
    }, LOCKED);
    await openSurface(page);
    await openAbleSet(page);
    const body = await ablesetLoaded(page);
    await expect(body).toHaveAttribute("data-locked", "none");
    expect(inside(page)?.url()).toBe(`${ABLESET.origin}/`);
    const records = await lockRecords(since);
    expect(records).toHaveLength(1);
    expect(records[0]).toMatchObject({ why: "SecurityError", source: "lan", forwarded: "-", skipped: 0 });
  });
});

test.describe("AbleSet's lock page without its script (#90 review M1)", () => {
  // The lock page's script refused, as a Wi-Fi drop or a hub restart
  // between the page and its script would: Chromium's failed load of that
  // address only (WebKit logs a refused route at info). The service worker
  // is kept out so the route sees every request (Playwright's own warning
  // says so).
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [
      [{ text: /^Failed to load resource: /, url: LOCK_SCRIPT_URL }, /^Service Worker registration blocked by Playwright$/],
      { scope: "test" },
    ],
  });

  test("goes on to AbleSet by its refresh, and the hub records it", async ({ page, context }) => {
    const since = Date.now();
    let refused = 0;
    await context.route(LOCK_SCRIPT_URL, (route) => {
      refused += 1;
      return route.abort();
    });
    await openSurface(page);
    await openAbleSet(page);
    // The page's refresh comes 3 s after it loaded without its script; the
    // hub records it and answers with AbleSet's own page.
    await expect(frameOf(page).locator("body")).toHaveAttribute("data-app", "loaded", { timeout: 15_000 });
    const body = await ablesetLoaded(page);
    await expect(body).toHaveAttribute("data-locked", "none");
    expect(inside(page)?.url()).toBe(`${ABLESET.origin}/`);
    expect(refused).toBe(1);
    const records = await lockRecords(since);
    expect(records).toHaveLength(1);
    expect(records[0]).toMatchObject({ why: "script", source: "lan", forwarded: "-" });
  });
});

test.describe("A probe that fails on the LAN", () => {
  // The probe's image refused here, as a broken certificate or a missing LAN
  // name would fail it: a failed image load in the console, of the probe's
  // address only (an abort has no status). The service worker is kept out so
  // the route sees the page's own request (Playwright's own warning says so).
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [
      [{ text: /^Failed to load resource: /, url: PROBE_URL }, /^Service Worker registration blocked by Playwright$/],
      { scope: "test" },
    ],
  });

  test("shows why, never the internet sign-in, and the mixer stays; a page's tab and back tries again", async ({
    page,
  }) => {
    const since = Date.now();
    const probe = isProbe;
    let refused = 0;
    await page.route(probe, (route) => {
      refused += 1;
      return route.abort();
    });
    await openSurface(page);
    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "unreachable");
    expect(refused).toBe(1);
    const panel = page.getByTestId("ableset-panel");
    await expect(panel).toHaveAttribute("data-panel", "unreachable");
    await expect(panel.locator("h3")).toHaveText("AbleSet can't load");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "AbleSet did not load over the church network: its name or certificate does not work. The mixer works as usual; fohmixer tries again when you come back here.",
    );
    // Nothing to tap that would take the mixer away, and it stays.
    await expect(panel.locator("button")).toHaveCount(0);
    await expect(page.getByTestId("ableset-signin")).toHaveCount(0);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await expect(page.getByTestId("ableset-tab")).toHaveAttribute("data-offline", "false");
    expect(page.url()).toBe(`${HTTPS}/`);
    // Back from a page's tab, the probe passes and AbleSet loads.
    await page.unroute(probe);
    await leaveForTheMixer(page);
    await openAbleSet(page);
    await expect(page.getByTestId("ableset")).toHaveAttribute("data-state", "frame");
    await ablesetLoaded(page);
    const events = await ablesetEvents(
      since,
      (e) => e.filter((x) => x.what === "probe").length >= 2,
      "both probes' answers",
    );
    const steps = events.map((e) =>
      e.ev === "ableset_view" ? `view ${e.on}` : e.what === "probe" ? `probe ${e.ok}` : e.what === "panel" ? `panel ${e.panel}` : e.what,
    );
    expect(steps.slice(0, 7)).toEqual([
      "view true",
      "panel checking",
      "probe false",
      "panel unreachable",
      "view false",
      "view true",
      "panel checking",
    ]);
  });
});

test.describe("A probe with no answer within its bound", () => {
  // The probe is held by the test's route, so the service worker is kept out
  // (Playwright's own warning says so); nothing fails to load.
  test.use({ baseURL: HTTPS, serviceWorkers: "block", allowedConsole: [/^Service Worker registration blocked by Playwright$/] });

  test("says so after 8 s, its late answer changes nothing, and the page shown again probes anew", async ({ page }) => {
    const since = Date.now();
    const held: Route[] = [];
    await page.route(isProbe, (route) => {
      held.push(route);
    });
    // The page's clock is the test's (#90 PR C review L6): the 8 s bound is
    // jumped over, never waited out. The clock runs on by itself meanwhile.
    await page.clock.install();
    await openSurface(page);
    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "checking");
    await expect.poll(() => held.length).toBe(1);
    await page.clock.fastForward(PROBE_MS);
    await expect(area).toHaveAttribute("data-state", "unreachable");
    const panel = page.getByTestId("ableset-panel");
    await expect(panel.locator("h3")).toHaveText("AbleSet can't load");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "AbleSet did not answer within 8 seconds. The mixer works as usual; fohmixer tries again when you come back here.",
    );
    // The held probe answers late: only a probe on its way takes an answer,
    // and the page takes this one without a fault.
    const answered = page.waitForEvent("requestfinished", (request) => request === held[0].request());
    await held[0].continue();
    await answered;
    await expect(area).toHaveAttribute("data-state", "unreachable");
    // Hidden, nothing probes; shown again, a fresh probe passes and AbleSet loads.
    await page.unroute(isProbe);
    await setHidden(page, true);
    await expect(area).toHaveAttribute("data-state", "unreachable");
    await setHidden(page, false);
    await expect(area).toHaveAttribute("data-state", "frame");
    await ablesetLoaded(page);
    expect(held).toHaveLength(1);
    const events = await ablesetEvents(
      since,
      (e) => e.filter((x) => x.what === "probe").length >= 2 && e.some((x) => x.what === "load"),
      "both probes' answers and the frame's load",
    );
    const steps = events.map((e) =>
      e.ev === "ableset_view" ? `view ${e.on}` : e.what === "probe" ? `probe ${e.ok}` : e.what === "panel" ? `panel ${e.panel}` : e.what,
    );
    expect(steps).toEqual([
      "view true",
      "panel checking",
      "probe false",
      "panel unreachable",
      "panel checking",
      "probe true",
      "load",
    ]);
    expect(events.find((e) => e.what === "probe").ms).toBeGreaterThanOrEqual(PROBE_MS);
  });
});

test.describe("AbleSet down while its tab is open", () => {
  // AbleSet stopped closes its socket in the frame under the page (the test
  // stops it on purpose): WebKit reports the reset of that socket.
  test.use({ baseURL: HTTPS, allowedConsole: [/^WebSocket connection to 'wss:\/\/ableset\.e2e\.test(:\d+)?\/socket\.io\//] });

  test("shows the red dot and the down panel, and AbleSet back mounts a fresh frame", async ({ page }) => {
    // The hub reads AbleSet every 5 s: down after two failed reads (up to
    // ~10 s), up at the next good one, then the probe and the frame's load.
    test.setTimeout(60_000);
    const bearer = await token();
    await openSurface(page);
    const area = await openAbleSet(page);
    await ablesetLoaded(page);
    await page.getByTestId("ableset-frame").evaluate((f) => f.setAttribute("data-e2e-kept", "true"));
    await fakeAbleSet.down();
    try {
      await expect(area).toHaveAttribute("data-state", "down", { timeout: 15_000 });
      await expect(page.getByTestId("ableset-dot")).toBeVisible();
      await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
      const panel = page.getByTestId("ableset-panel");
      await expect(panel.locator("h3")).toHaveText("AbleSet is not running");
      await expect(panel.locator(".ableset-text")).toHaveText(
        "AbleSet on the PC is not answering right now. Fohmixer keeps retrying; the mixer works as usual.",
      );
    } finally {
      await fakeAbleSet.up();
      // The link is up again before the next test, also after a failure above.
      await until(() => ablesetStatus(bearer), (s) => s.online === true, "AbleSet's link up", 10_000);
    }
    await expect(area).toHaveAttribute("data-state", "frame", { timeout: 10_000 });
    await expect(page.getByTestId("ableset-dot")).toHaveCount(0);
    await expect(page.getByTestId("ableset-frame")).not.toHaveAttribute("data-e2e-kept", "true");
    await ablesetLoaded(page);
  });
});

test.describe("AbleSet's tab on the internet path", () => {
  // The probe refused (no AbleSet sign-in yet) is a failed image load in the
  // console, the probe's address with Access's 403: the test provokes it.
  // The service worker is kept out
  // (Playwright's own warning says so): fohmixer's own requests then leave
  // the page itself with the test's tunnel header, which the worker's fetch
  // of them may not carry. The probe never passes through the worker (sw.js
  // leaves another origin's requests to the page); the LAN tests above run
  // with the worker.
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [
      [
        { text: /^Failed to load resource: the server responded with a status of 403 \(/, url: PROBE_URL },
        /^Service Worker registration blocked by Playwright$/,
      ],
      { scope: "test" },
    ],
  });

  test("the hub tells a page through the tunnel it is not on the LAN, and a LAN page it is", async () => {
    expect((await toldOfAbleSet({})).lan).toBe(true);
    const tunnel = await toldOfAbleSet({ ...TUNNEL, Cookie: `CF_Authorization=${accessJwt()}` });
    expect(tunnel).toMatchObject({ type: "ableset", lan: false, name: ABLESET.hostname });
  });

  test("needs AbleSet's own sign-in once, which comes back to the open tab", async ({ page, context }) => {
    const told = await asThroughTheTunnel(page);
    await context.setExtraHTTPHeaders(TUNNEL);
    await context.addCookies([{ name: "CF_Authorization", value: accessJwt(), url: HTTPS }]);
    await openSurface(page);
    await expect.poll(told).toBeGreaterThan(0);
    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "signin");
    const panel = page.getByTestId("ableset-panel");
    await expect(panel.locator("h3")).toHaveText("AbleSet sign-in");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "You are away from the church. AbleSet wants you to confirm the sign-in once, like fohmixer.",
    );
    const signIn = page.getByTestId("ableset-signin");
    await expect(signIn).toHaveText("SIGN IN");
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    // Cloudflare's sign-in for AbleSet's name sets its own cookie: here, at once.
    await context.addCookies([
      { name: "CF_Authorization", value: accessJwt({ aud: [ABLESET_AUD] }), url: ABLESET.origin },
    ]);
    await page.evaluate(() => ((window as any).e2eBefore = true));
    const signedIn = page.waitForRequest(`${ABLESET.origin}/.fohmixer/signed-in`);
    await signIn.click();
    const answer = await (await signedIn).response();
    expect(answer?.status()).toBe(303);
    expect(answer?.headers()["location"]).toBe(`${HTTPS}/#ableset`);
    // A fresh page (the hub sent it back) opened the tab by itself and
    // cleared the hash; the probe now passes and AbleSet loads.
    await expect(page.getByTestId("ableset")).toHaveAttribute("data-state", "frame");
    expect(await page.evaluate(() => (window as any).e2eBefore)).toBeUndefined();
    await expect(page.getByTestId("ableset-tab")).toHaveAttribute("data-selected", "true");
    await expect(page).toHaveURL(`${HTTPS}/`);
    await ablesetLoaded(page);
  });
});

test.describe("AbleSet's tab on the emergency address", () => {
  test("shows the elsewhere panel with AbleSet's direct address and loads nothing", async ({ page, context }) => {
    const { port } = await fakeAbleSet.state();
    await fakeAbleSet.clear();
    const requested: string[] = [];
    page.on("request", (request) => requested.push(request.url()));
    // The suite's own base: plain http by address (the impair proxy's).
    await openSurface(page);
    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "elsewhere");
    const panel = page.getByTestId("ableset-panel");
    await expect(panel.locator("h3")).toHaveText("Fallback address");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "Fohmixer is open by IP without a certificate, so AbleSet can't be embedded. Open fohmixer by its name, or open AbleSet directly:",
    );
    const direct = page.getByTestId("ableset-direct");
    await expect(direct).toHaveText(`http://127.0.0.1:${port}/`);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await frames(page);
    expect(requested.filter((url) => url.includes(ABLESET.hostname))).toEqual([]);
    expect(pageHits((await fakeAbleSet.state()).hits)).toEqual([]);
    // The direct address opens AbleSet itself on the PC, in a new tab.
    const opened = context.waitForEvent("page");
    await direct.click();
    const own = await opened;
    await expect(own.locator("body")).toHaveAttribute("data-app", "loaded");
    expect(own.url()).toBe(`http://127.0.0.1:${port}/`);
    // Without an opener (#90 PR C review L4): AbleSet's page cannot reach
    // the mixer's tab.
    expect(await own.evaluate(() => window.opener === null)).toBe(true);
    await own.close();
  });
});

test.describe("A hub that does not tell of AbleSet", () => {
  test("gets no AbleSet tab: the tab comes only from the hub's message", async ({ page }) => {
    // Without [ableset] the hub sends no `ableset` (its Rust tests pin that);
    // here the page's socket holds the message back.
    let held = 0;
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (typeof message === "string" && message.includes('"type":"ableset"')) {
          held += 1;
          return;
        }
        ws.send(message);
      });
    });
    await openSurface(page);
    await expect.poll(() => held).toBeGreaterThan(0);
    await expect(page.getByTestId("deck-tab")).toBeVisible();
    await expect(page.getByTestId("ableset-tab")).toHaveCount(0);
    // Without it the deck's tab is the bar's last row (#148, the owner's
    // ruling of 29.10.2026: every tab a row of its own, the page tabs first
    // and larger; #90 PR C review L7's one cell beside a page's tab went
    // with the page tabs' own rows), as wide as a page's tab.
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    const deck = (await page.getByTestId("deck-tab").boundingBox())!;
    const first = (await bar.getByTestId("tab").first().boundingBox())!;
    expect(Math.abs(deck.width - first.width)).toBeLessThan(1);
    const boxes = await bar.locator(":scope > button").evaluateAll((tabs) =>
      tabs.map((t) => Math.round(t.getBoundingClientRect().top)),
    );
    expect(new Set(boxes).size, "one tab a row").toBe(boxes.length);
  });
});

/** The page policy's revision the hub wrote into the page's `index.html` (#90), or null. */
const loadedPolicy = (page: Page): Promise<string | null> =>
  page.evaluate(() => document.querySelector('meta[name="fohmixer-policy"]')?.getAttribute("content") ?? null);

/**
 * The `policy` of every hub hello the page receives from now on (#90), on
 * each of its hub sockets, across its reloads (null for a hello without one).
 */
function hubPolicies(page: Page): () => (string | null)[] {
  const seen: (string | null)[] = [];
  page.on("websocket", (socket) => {
    if (!HUB_SOCKET.test(socket.url())) return;
    socket.on("framereceived", ({ payload }) => {
      if (typeof payload === "string" && payload.includes('"type":"hello"')) seen.push(JSON.parse(payload).policy ?? null);
    });
  });
  return () => seen;
}

test.describe("A page open while the hub restarts (#90: its page policy)", () => {
  // The hub restarts under an open page on fohmixer's https page (the hub's
  // own listener: no impair proxy holds the page's requests meanwhile), so
  // while it is down the page's reconnect fails its layout fetch and its
  // `disconnected` report: failed loads of those addresses only (and a
  // socket, should one attempt land in the gap). The service worker is kept
  // out (Playwright's own warning says so), so those failures are the page's
  // own fetches; it is network-only, so the page policy comes with the hub's
  // answer either way.
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [
      [
        { text: /^Failed to load resource: /, url: PAGE_API },
        /^WebSocket connection to 'wss:\/\/[^']+\/ws\?[^']*' failed/,
        /^Service Worker registration blocked by Playwright$/,
      ],
      { scope: "test" },
    ],
  });

  test("that gains [ableset] reloads once by itself, and its ABLESET tab then frames AbleSet", async ({ page }) => {
    // The church iPad (#90 comment 6163657943): a Home Screen app open
    // before the hub had [ableset] kept the policy it loaded with, whose
    // img-src and frame-src lack AbleSet's name, and its tab could not load.
    let on = true;
    try {
      // No page is open across this restart (deck.spec.ts's [companion] test).
      on = (await harness("/hub/ableset", { on: false })).ableset;
      expect(on).toBe(false);
      await openSurface(page);
      await expect(page.getByTestId("deck-tab")).toBeVisible();
      await expect(page.getByTestId("ableset-tab")).toHaveCount(0);
      const before = await loadedPolicy(page);
      const loads = countLoads(page);
      const policies = hubPolicies(page);
      on = (await harness("/hub/ableset", { on: true })).ableset;
      expect(on).toBe(true);
      // The restarted hub's hello names another page policy: the page
      // reloads by itself, once (the handshake's once-a-minute guard).
      await expect.poll(loads, { message: "the page's reloads", timeout: 20_000, intervals: [50] }).toBe(1);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 15_000 });
      expect(Number(await page.evaluate((key) => localStorage.getItem(key), RELOAD_KEY))).toBeGreaterThan(0);
      // The hello that reloaded it and the reloaded page's own name one
      // policy, which the reloaded page now carries; the page before had
      // another.
      await expect.poll(() => policies().length, { intervals: [50] }).toBeGreaterThanOrEqual(2);
      const told = policies();
      expect(told[0]).toMatch(/^[0-9a-f]{16}$/);
      expect(new Set(told)).toEqual(new Set([told[0]]));
      expect(before).toMatch(/^[0-9a-f]{16}$/);
      expect(before).not.toBe(told[0]);
      expect(await loadedPolicy(page)).toBe(told[0]);
      // With it the tab frames AbleSet.
      const area = await openAbleSet(page);
      await expect(area).toHaveAttribute("data-state", "frame", { timeout: 15_000 });
      await ablesetLoaded(page);
      expect(loads()).toBe(1);
    } finally {
      if (!on) {
        await page.goto("about:blank");
        await harness("/hub/ableset", { on: true });
      }
    }
  });

  test("with the same config keeps the page: it only reconnects", async ({ page }) => {
    await openSurface(page);
    await expect(page.getByTestId("ableset-tab")).toBeVisible();
    const surface = page.getByTestId("surface");
    // A reloaded page would not carry the mark.
    await surface.evaluate((el) => el.setAttribute("data-e2e-kept", "1"));
    const loads = countLoads(page);
    const policies = hubPolicies(page);
    // [ableset] stays on: the same config, and the hub restarted all the same.
    expect((await harness("/hub/ableset", { on: true })).ableset).toBe(true);
    // The page hears the restarted hub's hello (its socket closed at the
    // stop, so `data-connected` went false before it) and the same marked
    // page is connected again: the hello's decision, made before that, kept
    // it (a reload would have dropped the mark).
    await expect.poll(() => policies().length, { timeout: 20_000, intervals: [50] }).toBeGreaterThanOrEqual(1);
    await expect(page.locator('[data-testid="surface"][data-e2e-kept="1"]')).toHaveAttribute("data-connected", "true", {
      timeout: 15_000,
    });
    expect(loads()).toBe(0);
    expect(await page.evaluate((key) => localStorage.getItem(key), RELOAD_KEY)).toBeNull();
    // It knew the policy it loaded with, and the hub named the same.
    const loaded = await loadedPolicy(page);
    expect(loaded).toMatch(/^[0-9a-f]{16}$/);
    expect(new Set(policies())).toEqual(new Set([loaded]));
  });
});
