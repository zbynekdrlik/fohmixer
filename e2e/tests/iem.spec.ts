import type { Frame, Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import {
  HUB_SOCKET,
  countLoads,
  extraOnAPhone,
  fakeIem,
  harness,
  hubEvents,
  openDeck,
  openSurface,
  pageEvents,
  resetLayout,
  resetSectors,
  rightHalfLive,
  setIdleAfter,
  setSectors,
  type SectorsDoc,
  until,
} from "./support/live";
import { HTTPS } from "./support/pwa";

// iemmixer's tab (#166; the owner's approved docs/mockups/iem-tab-v1.html): a
// third extra tab under ABLESET, a frame of iemmixer's own page over the left
// half. Unlike AbleSet's frame it lives on behind the other tabs, so listening
// goes on while the engineer works the faders: a dot on the tab says so, and
// only the ✕ beside it ends it. The e2e job's harness runs the fake iemmixer
// (e2e/harness/fake_iem.py) on an origin of its own over HTTPS, which the
// browser reaches directly, as the iPad will: the hub proxies nothing of it.

const IEM = new URL(process.env.E2E_IEM_URL || "https://iem.e2e.test:39194/");
const SANDBOX =
  "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-modals allow-downloads";
/** A sectors document that places faders (#148): the default page shows its sectors. */
const SECTORS: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "STAGE", strips: [{ instance: "band", label: "Vocal 1 repro#" }] }],
    ru: [{ id: "ru-1", title: "MAIN", strips: [{ instance: "band", label: "B-Main repro #" }] }],
  },
};

const frameOf = (page: Page) => page.frameLocator('[data-testid="iem-frame"]');

/** The frame's document on iemmixer's origin. */
function inside(page: Page): Frame {
  const frame = page.frames().find((f) => f.url().startsWith(IEM.origin));
  if (!frame) throw new Error("no document on iemmixer's origin");
  return frame;
}

/** A value set inside the frame's own document: a reload of the frame loses it. */
const mark = (page: Page) => inside(page).evaluate(() => ((window as any).e2eKept = 1));
const marked = (page: Page): Promise<number> => inside(page).evaluate(() => (window as any).e2eKept ?? 0);
const ticks = (page: Page): Promise<number> => inside(page).evaluate(() => (window as any).ticks);

/** The tab tapped, its page loaded in the frame. */
async function openIem(page: Page) {
  const tab = page.getByTestId("iem-tab");
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
  await expect(frameOf(page).locator("body")).toHaveAttribute("data-app", "loaded");
}

/** A layout page's tab: the mixer again. */
async function backToTheMixer(page: Page) {
  await page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]').first().click();
  await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");
}

/** The frame's box as the page lays it out, shown or hidden. */
const boxOf = (frame: Locator) =>
  frame.evaluate((f) => {
    const r = f.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  });

test.describe("The IEM tab (fohmixer's own https page)", () => {
  test.use({ baseURL: HTTPS });

  test("is the row under ABLESET, opens over the left half, and the column and the right half stay", async ({ page }) => {
    await fakeIem.clear();
    await openSurface(page);
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    const rows = await bar.evaluate((b) => [...b.children].map((c) => c.getAttribute("data-testid")));
    expect(rows.slice(-3)).toEqual(["deck-tab", "ableset-tab", "iem-row"]);
    const tab = page.getByTestId("iem-tab");
    await expect(tab).toHaveText("IEM MIXER");
    await expect(tab).toHaveAttribute("data-alive", "false");
    // Nothing of it before the first open: no frame, no dot, no ✕, no load.
    await expect(page.getByTestId("iem-frame")).toHaveCount(0);
    await expect(page.getByTestId("iem-dot")).toHaveCount(0);
    await expect(page.getByTestId("iem-close")).toHaveCount(0);
    expect((await fakeIem.state()).loads).toBe(0);
    // As wide as AbleSet's tab until the ✕ takes its place beside it.
    const width = async (l: Locator) => (await l.boundingBox())!.width;
    expect(Math.abs((await width(tab)) - (await width(page.getByTestId("ableset-tab"))))).toBeLessThan(1);
    const column = (await page.getByTestId("column").boundingBox())!;

    await openIem(page);
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "iem");
    const frame = page.getByTestId("iem-frame");
    await expect(frame).toHaveAttribute("src", IEM.href);
    await expect(frame).toHaveAttribute("title", "IEM MIXER");
    await expect(frame).toHaveAttribute("allow", "microphone; autoplay");
    await expect(frame).toHaveAttribute("sandbox", SANDBOX);
    await expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    await expect(frameOf(page).locator("#fake-iem")).toHaveText("ENGINEER");
    // While it shows: the ✕, no dot.
    await expect(tab).toHaveAttribute("data-alive", "true");
    await expect(page.getByTestId("iem-close")).toBeVisible();
    await expect(page.getByTestId("iem-dot")).toHaveCount(0);
    // The column has not moved, and the frame fills the left half up to it.
    expect(await page.getByTestId("column").boundingBox()).toEqual(column);
    const viewport = page.viewportSize()!;
    const framed = (await frame.boundingBox())!;
    expect(framed.x).toBeGreaterThanOrEqual(0);
    expect(framed.x + framed.width).toBeLessThanOrEqual(column.x + 0.5);
    expect(framed.width).toBeGreaterThan((viewport.width - column.width) / 2 - 30);
    // The left half's cells hide, the right half is live beside the frame.
    await expect(page.locator('.slot[data-side="left"] [data-testid="strip"]').first()).toBeHidden();
    await rightHalfLive(page, page.locator('.slot[data-side="right"] [data-testid="strip"]'));
    expect((await fakeIem.state()).loads).toBe(1);
  });

  test("stays the same running document behind FOH and back, with the dot, and the strips under it work", async ({ page }) => {
    await fakeIem.clear();
    await openSurface(page);
    await openIem(page);
    const frame = page.getByTestId("iem-frame");
    const pane = page.getByTestId("iem");
    const tab = page.getByTestId("iem-tab");
    await mark(page);
    await expect(pane).toHaveAttribute("data-shown", "true");
    const shown = await boxOf(frame);

    await backToTheMixer(page);
    // Hidden, alive: the dot on the tab, the ✕ beside it.
    await expect(tab).toHaveAttribute("data-selected", "false");
    await expect(page.getByTestId("iem-dot")).toBeVisible();
    await expect(page.getByTestId("iem-close")).toBeVisible();
    await expect(pane).toHaveAttribute("data-shown", "false");
    await expect(frame).toBeAttached();
    await expect(frame).toBeHidden();
    // `visibility`, never `display: none`, and the frame keeps its box and
    // its place (iem.css says why).
    const style = await frame.evaluate((f) => {
      const s = getComputedStyle(f);
      return { visibility: s.visibility, display: s.display };
    });
    expect(style).toEqual({ visibility: "hidden", display: "block" });
    expect(await boxOf(frame)).toEqual(shown);
    // Its page runs on: the counter its own timer keeps goes up, in the same
    // document.
    const before = await ticks(page);
    await expect.poll(() => ticks(page)).toBeGreaterThan(before);
    expect(await marked(page)).toBe(1);
    // The hidden frame takes no touch: a strip under it is hit and works.
    await rightHalfLive(page, page.locator('.slot[data-side="left"] [data-testid="strip"]'));

    await tab.click();
    await expect(pane).toHaveAttribute("data-shown", "true");
    await expect(frame).toBeVisible();
    await expect(page.getByTestId("iem-dot")).toHaveCount(0);
    expect(await marked(page), "the same document after FOH and back").toBe(1);
    // Another extra tab hides it the same way.
    await openDeck(page);
    await expect(page.getByTestId("iem-dot")).toBeVisible();
    await expect(pane).toHaveAttribute("data-shown", "false");
    expect(await marked(page)).toBe(1);
    expect((await fakeIem.state()).loads, "the frame's page loaded once").toBe(1);
  });

  test("lives through a new layout and the sectors switching under it", async ({ page }) => {
    // A Tuner edit in Live is a new layout: the shell, the extra pane and the
    // deck mount again. The frame lies outside it, or the audio would stop.
    await fakeIem.clear();
    await openSurface(page);
    await openIem(page);
    const frame = page.getByTestId("iem-frame");
    await frame.evaluate((f) => f.setAttribute("data-e2e-kept", "1"));
    await mark(page);
    const kept = async () => {
      await expect(frame).toHaveAttribute("data-e2e-kept", "1");
      expect(await marked(page), "iemmixer's page not loaded again").toBe(1);
      await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "iem");
      await expect(page.getByTestId("iem-tab")).toHaveAttribute("data-selected", "true");
      expect((await fakeIem.state()).loads).toBe(1);
    };
    await page.getByTestId("stage").evaluate((s) => s.setAttribute("data-e2e-shell", "1"));
    try {
      await setIdleAfter(3600);
      // The old shell is gone: the page has mounted the new layout.
      await expect(page.locator('[data-testid="stage"][data-e2e-shell="1"]')).toHaveCount(0);
      await expect(page.getByTestId("stage")).toBeVisible();
      await kept();
      await resetLayout();
      await setSectors(SECTORS);
      await expect(page.locator('[data-testid="page"][data-sectors="true"]')).toBeVisible();
      await kept();
      await resetSectors();
      await expect(page.locator('[data-testid="page"][data-sectors="true"]')).toHaveCount(0);
      await kept();
    } finally {
      await resetSectors();
      await resetLayout();
    }
  });

  test("the ✕ on its own tab ends the frame and gives way to the page, and the next open is a new document", async ({ page }) => {
    await fakeIem.clear();
    const since = Date.now();
    await openSurface(page);
    await openIem(page);
    await mark(page);
    const close = page.getByTestId("iem-close");
    await close.click();
    await expect(page.getByTestId("iem-frame")).toHaveCount(0);
    await expect(close).toHaveCount(0);
    await expect(page.getByTestId("iem-dot")).toHaveCount(0);
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");
    await expect(page.getByTestId("iem-tab")).toHaveAttribute("data-selected", "false");
    await expect(page.getByTestId("iem-tab")).toHaveAttribute("data-alive", "false");
    // Its strips again, under nothing.
    await expect(page.locator('.slot[data-side="left"] [data-testid="strip"]').first()).toBeVisible();
    expect(page.frames().some((f) => f.url().startsWith(IEM.origin))).toBe(false);

    await openIem(page);
    expect(await marked(page), "a new document").toBe(0);
    expect((await fakeIem.state()).loads).toBe(2);
    // The flight recorder has the frame's life.
    const events = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "iem" && e.t >= since),
      (found) => found.filter((e: any) => e.what === "open").length >= 2,
      "the page's iem events",
      15_000,
    );
    expect(events.map((e: any) => e.what).filter((what: string) => what !== "load")).toEqual(["open", "close", "open"]);
    expect(events.find((e: any) => e.what === "load").ms).toBeGreaterThanOrEqual(0);
  });

  test("the ✕ behind another tab ends the frame and moves nothing else", async ({ page }) => {
    await openSurface(page);
    await openIem(page);
    await backToTheMixer(page);
    await page.getByTestId("iem-close").click();
    await expect(page.getByTestId("iem-frame")).toHaveCount(0);
    await expect(page.getByTestId("iem-dot")).toHaveCount(0);
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");

    await openIem(page);
    await openDeck(page);
    await page.getByTestId("iem-close").click();
    await expect(page.getByTestId("iem-frame")).toHaveCount(0);
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "deck");
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "true");
  });

  for (const size of [
    { width: 844, height: 390 },
    { width: 390, height: 844 },
  ]) {
    test.describe(`on a phone, ${size.width}×${size.height}`, () => {
      test.use({ viewport: size });

      test("takes the screen but the thin ☰ strip, ☰ opens the column in its middle, and it lives on behind a page", async ({ page }) => {
        await openSurface(page);
        await extraOnAPhone(page, page.getByTestId("iem-tab"), page.getByTestId("iem"), page.getByTestId("iem-frame"), async () => {
          await expect(frameOf(page).locator("body")).toHaveAttribute("data-app", "loaded");
          await mark(page);
        });
        await expect(page.getByTestId("iem-frame")).toBeAttached();
        expect(await marked(page)).toBe(1);
      });
    });
  }
});

test.describe("A hub that does not tell of iemmixer", () => {
  test("gets no IEM tab: the tab comes only from the hub's message", async ({ page }) => {
    // Without [iem] the hub sends no `iem` (its Rust tests pin that); here
    // the page's socket holds the message back.
    let held = 0;
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (typeof message === "string" && message.includes('"type":"iem"')) {
          held += 1;
          return;
        }
        ws.send(message);
      });
    });
    await openSurface(page);
    await expect.poll(() => held).toBeGreaterThan(0);
    await expect(page.getByTestId("ableset-tab")).toBeVisible();
    await expect(page.getByTestId("iem-tab")).toHaveCount(0);
    await expect(page.getByTestId("iem-row")).toHaveCount(0);
    await expect(page.getByTestId("iem-frame")).toHaveCount(0);
  });
});

test.describe("A hub without [iem] and then with it", () => {
  // The hub restarts under an open page on fohmixer's https page, as in
  // ableset.spec.ts: while it is down the page's reconnect fails its layout
  // fetch and its `disconnected` report. The service worker is kept out, so
  // those failures are the page's own fetches.
  test.use({
    baseURL: HTTPS,
    serviceWorkers: "block",
    allowedConsole: [
      [
        { text: /^Failed to load resource: /, url: /\/api\/(layout|client-report)$/ },
        /^WebSocket connection to 'wss:\/\/[^']+\/ws\?[^']*' failed/,
        /^Service Worker registration blocked by Playwright$/,
      ],
      { scope: "test" },
    ],
  });

  test("shows no tab and may frame nothing there; gaining it reloads the page once, and the tab frames the page", async ({ page }) => {
    let on = true;
    try {
      on = (await harness("/hub/iem", { on: false })).iem;
      expect(on).toBe(false);
      const answer = await page.goto("/");
      expect(answer!.headers()["content-security-policy"]).not.toContain(IEM.hostname);
      await page.goto("about:blank");
      await openSurface(page);
      await expect(page.getByTestId("ableset-tab")).toBeVisible();
      await expect(page.getByTestId("iem-tab")).toHaveCount(0);
      const loads = countLoads(page);
      on = (await harness("/hub/iem", { on: true })).iem;
      expect(on).toBe(true);
      // The restarted hub's page policy names iemmixer's origin: the page
      // reloads by itself, or its old policy would refuse the frame.
      await expect.poll(loads, { message: "the page's reloads", timeout: 20_000, intervals: [50] }).toBe(1);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 15_000 });
      await openIem(page);
      expect(loads()).toBe(1);
    } finally {
      if (!on) {
        await page.goto("about:blank");
        await harness("/hub/iem", { on: true });
      }
    }
  });
});
