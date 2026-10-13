import type { Locator, Page } from "@playwright/test";
import { ABLESET_AUD, TUNNEL, accessJwt } from "./support/access";
import { expect, test } from "./support/fixtures";
import {
  BASE,
  HUB_SOCKET,
  fakeAbleSet,
  frames,
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

/** AbleSet's page loaded in the frame, its socket echoing through the hub: its body. */
async function ablesetLoaded(page: Page): Promise<Locator> {
  const body = frameOf(page).locator("body");
  await expect(body).toHaveAttribute("data-app", "loaded");
  await expect(body).toHaveAttribute("data-ws", "echo");
  return body;
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

test.describe("AbleSet's tab (the LAN path, fohmixer's own https page)", () => {
  test.use({ baseURL: HTTPS });

  test("is the last tab, frames AbleSet's page through the hub, and a page's tab takes it away", async ({ page }) => {
    const since = Date.now();
    const bearer = await token();
    const pressed = (await fakeAbleSet.state()).presses;
    await openSurface(page);
    // The tab: from the hub's message, last, right after the Stream Deck's,
    // both as wide as the bar (the mockup), no red dot.
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    const ids = await bar.locator(":scope > button").evaluateAll((tabs) => tabs.map((t) => t.getAttribute("data-testid")));
    expect(ids.slice(-2)).toEqual(["deck-tab", "ableset-tab"]);
    const tab = page.getByTestId("ableset-tab");
    await expect(tab).toHaveText("AbleSet");
    await expect(tab).toHaveAttribute("data-offline", "false");
    await expect(page.getByTestId("ableset-dot")).toHaveCount(0);
    const width = async (l: Locator) => (await l.boundingBox())!.width;
    expect(Math.abs((await width(tab)) - (await width(page.getByTestId("deck-tab"))))).toBeLessThan(1);
    expect(await width(tab)).toBeGreaterThan(1.5 * (await width(bar.getByTestId("tab").first())));

    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "frame");
    const frame = page.getByTestId("ableset-frame");
    await expect(frame).toHaveAttribute("src", `${ABLESET.origin}/`);
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
    await frameOf(page).locator("#fake-button").click();
    await expect(frameOf(page).locator("body")).toHaveAttribute("data-pressed", "ok");
    expect((await fakeAbleSet.state()).presses).toBe(pressed + 1);
    // A page's tab beside the frame takes AbleSet away: the frame and its socket go.
    await leaveForTheMixer(page);
    await expect(page.getByTestId("ableset")).toHaveCount(0);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await expect(tab).toHaveAttribute("data-selected", "false");
    await until(() => ablesetStatus(bearer), (s) => s.sockets === 0, "AbleSet's socket closed");
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
      await expect(panel.locator("h3")).toHaveText("AbleSet nebeží");
      await expect(panel.locator(".ableset-text")).toHaveText(
        "AbleSet na PC teraz neodpovedá. Fohmixer ho skúša znova sám; mixér funguje ďalej.",
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
  // console: the test provokes it. The service worker is kept out
  // (Playwright's own warning says so): fohmixer's own requests then leave
  // the page itself with the test's tunnel header, which the worker's fetch
  // of them may not carry. The probe never passes through the worker (sw.js
  // leaves another origin's requests to the page); the LAN tests above run
  // with the worker.
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [[/^Failed to load resource: /, /^Service Worker registration blocked by Playwright$/], { scope: "test" }],
  });

  test("needs AbleSet's own sign-in once, which comes back to the open tab", async ({ page, context }) => {
    await context.setExtraHTTPHeaders(TUNNEL);
    await context.addCookies([{ name: "CF_Authorization", value: accessJwt(), url: HTTPS }]);
    await openSurface(page);
    const area = await openAbleSet(page);
    await expect(area).toHaveAttribute("data-state", "signin");
    const panel = page.getByTestId("ableset-panel");
    await expect(panel.locator("h3")).toHaveText("Prihlásenie do AbleSetu");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "Si mimo kostola. AbleSet chce raz potvrdiť prihlásenie, rovnako ako fohmixer.",
    );
    const signIn = page.getByTestId("ableset-signin");
    await expect(signIn).toHaveText("PRIHLÁSIŤ SA");
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
    await expect(panel.locator("h3")).toHaveText("Núdzová adresa");
    await expect(panel.locator(".ableset-text")).toHaveText(
      "Fohmixer je otvorený cez IP bez certifikátu, tak sa AbleSet nedá vložiť. Otvor fohmixer cez jeho meno, alebo AbleSet priamo:",
    );
    const direct = page.getByTestId("ableset-direct");
    await expect(direct).toHaveText(`http://127.0.0.1:${port}/`);
    await expect(page.getByTestId("ableset-frame")).toHaveCount(0);
    await expect(page.locator(".ableset-probe")).toHaveCount(0);
    await frames(page);
    expect(requested.filter((url) => url.includes(ABLESET.hostname))).toEqual([]);
    expect(pageHits((await fakeAbleSet.state()).hits)).toEqual([]);
    // The direct address opens AbleSet itself on the PC, in a new tab.
    const opened = context.waitForEvent("page");
    await direct.click();
    const own = await opened;
    await expect(own.locator("body")).toHaveAttribute("data-app", "loaded");
    expect(own.url()).toBe(`http://127.0.0.1:${port}/`);
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
  });
});
