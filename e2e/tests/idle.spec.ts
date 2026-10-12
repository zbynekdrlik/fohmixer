import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { HUB_SOCKET, LiveClient, hubEvents, impair, openSurface, pageEvents, track, until, volume } from "./support/live";

// The page's idle state (#51, PR 2a: the hidden case; touch-idle follows in
// its own PR). A hidden page (a locked iPad, the app in the background) is
// idle at once: its `visibilitychange` handler sends `idle {on: true,
// meter_ms: 0}`, and the hub holds every meter value for it while every
// other value goes at once. Shown again, it wakes (`idle {on: false}` and a
// ping) and the meters come back at once. While idle the link waits longer
// (pings a second apart, a dropout only after 2.5 s of silence), and the
// wake starts the silence over, so an idle ping is never a dropout under the
// finger. SimLive animates every meter at 30 Hz, so a page that gets meters
// gets many.

const HAND2 = track("Hand2 #");
/** The hub key of Hand2's volume, without its `display` flag. */
const VOLUME_KEY = `band|${volume(HAND2)}|value`;
/** Live's meter properties (`fohmixer_proto::client::METER_PROPS`). */
const METER_PROPS = ["output_meter_level", "output_meter_left", "output_meter_right"];

/** A message on the page's hub socket and when the test saw it (test clock). */
type Frame = { at: number; msg: any };
type Link = { sent: Frame[]; received: Frame[] };

/** The page's hub socket, recorded both ways (its text messages). */
async function recordLink(page: Page): Promise<Link> {
  const link: Link = { sent: [], received: [] };
  await page.routeWebSocket(HUB_SOCKET, (ws: WebSocketRoute) => {
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      if (typeof message === "string") link.sent.push({ at: Date.now(), msg: JSON.parse(message) });
      server.send(message);
    });
    server.onMessage((message) => {
      if (typeof message === "string") link.received.push({ at: Date.now(), msg: JSON.parse(message) });
      ws.send(message);
    });
  });
  return link;
}

/** The page's messages of one type, in order. */
const sentOf = (link: Link, type: string) => link.sent.filter((f) => f.msg.type === type).map((f) => f.msg);

/** The value items of a hub message (`values`, `subbed`). */
function items(msg: any): any[] {
  if (msg.type === "values") return msg.items;
  if (msg.type === "subbed") return [msg];
  return [];
}

/** Whether a hub key names a meter (its property is the second field from the right). */
function isMeter(sub: string): boolean {
  const fields = sub.split("|");
  return METER_PROPS.includes(fields[fields.length - 2]);
}

/** How many meter values reached the page from `from` to `to` (test clock). */
function meters(link: Link, from: number, to = Infinity): number {
  return link.received
    .filter((f) => f.at >= from && f.at < to)
    .reduce((n, f) => n + items(f.msg).filter((i) => isMeter(i.sub)).length, 0);
}

/** When the first meter value from `from` on reached the page. */
const meterAt = (link: Link, from: number) =>
  link.received.find((f) => f.at >= from && items(f.msg).some((i) => isMeter(i.sub)))?.at;

/** When the first value of Hand2's volume from `from` on reached the page. */
const volumeAt = (link: Link, from: number) =>
  link.received.find((f) => f.at >= from && items(f.msg).some((i) => i.sub.startsWith(`${VOLUME_KEY}|`)))?.at;

/**
 * The page hidden (`document.hidden`, `visibilityState`) or shown again, with
 * the `visibilitychange` a browser fires on the document (as deck.spec.ts).
 */
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

/**
 * Watches the dropout counter in the page every animation frame (a
 * Playwright poll can miss a red phase of a few hundred ms): `everRed`
 * says whether it was ever red since.
 */
async function watchCounter(counter: Locator) {
  await counter.evaluate((el: Element) => {
    const w = window as unknown as { counterRed: boolean };
    w.counterRed = false;
    const step = () => {
      if (el.getAttribute("data-active") === "true") w.counterRed = true;
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
}

const everRed = (page: Page) => page.evaluate(() => (window as unknown as { counterRed: boolean }).counterRed);

/** The hub's `idle` records since `since` (test clock), oldest first. */
const idleRecords = async (since: number) => (await hubEvents()).filter((r) => r.ev === "idle" && r.ts >= since);

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("A hidden page", () => {
  test("is idle at once: no meter reaches it, every other value does, and shown again its meters come back at once", async ({ page }) => {
    const start = await live.get("band", volume(HAND2), "value");
    const link = await recordLink(page);
    const since = Date.now();
    await openSurface(page);
    const surface = page.getByTestId("surface");
    await expect(surface).toHaveAttribute("data-idle", "false");
    await expect.poll(() => meters(link, since), { message: "meters while visible" }).toBeGreaterThan(20);
    expect(sentOf(link, "idle"), "no idle while visible").toEqual([]);

    // Hidden: the page says so from its handler, its meters paused.
    const hiddenAt = Date.now();
    await setHidden(page, true);
    await expect(surface).toHaveAttribute("data-idle", "true");
    await expect.poll(() => sentOf(link, "idle")).toEqual([{ type: "idle", on: true, meter_ms: 0 }]);
    const slept = await until(() => idleRecords(hiddenAt), (r) => r.length >= 1, "the hub's idle record");
    expect(slept, "the hub pauses the page's meters").toMatchObject([{ on: true, meter_ms: 0 }]);

    const target = start > 0.5 ? 0.3 : 0.7;
    try {
      // From a moment after the hub took it (meters already on their way
      // still arrive), no meter reaches the page while SimLive's go on; a
      // volume set meanwhile arrives at once.
      await page.waitForTimeout(300);
      const quietFrom = Date.now();
      await live.set("band", volume(HAND2), "value", target);
      const arrived = await until(async () => volumeAt(link, quietFrom), (at) => at !== undefined, "the volume on the hidden page", 2000);
      expect(arrived! - quietFrom, "every other value at once").toBeLessThan(1000);
      await page.waitForTimeout(1500);
      const quietTo = Date.now();
      expect(meters(link, quietFrom, quietTo), "no meter while hidden").toBe(0);

      // Shown again: the page wakes (idle off, then a ping at once) and its
      // meters come back at once; the hub's record of the wake follows.
      const shownAt = Date.now();
      await setHidden(page, false);
      await expect(surface).toHaveAttribute("data-idle", "false");
      const back = await until(async () => meterAt(link, shownAt), (at) => at !== undefined, "a meter after the wake", 3000);
      expect(back! - shownAt, "the meters at once").toBeLessThan(1000);
      const sent = link.sent.map((f) => f.msg);
      const off = sent.findIndex((m) => m.type === "idle" && m.on === false);
      expect(sent[off]).toEqual({ type: "idle", on: false });
      expect(sent[off + 1]?.type, "a ping right after it").toBe("ping");
      expect(sentOf(link, "idle"), "one idle each way").toHaveLength(2);
      const records = await until(() => idleRecords(hiddenAt), (r) => r.length >= 2, "the hub's wake record");
      expect(records).toMatchObject([
        { on: true, meter_ms: 0 },
        { on: false, meter_ms: null },
      ]);

      // The page's own idle events reach the event log (essential).
      const events = await until(
        async () => pageEvents(await hubEvents()).filter((e) => e.ev === "idle" && e.t >= since),
        (e) => e.length >= 2,
        "the page's idle events",
        10_000,
      );
      expect(events).toMatchObject([
        { on: true, why: "hidden" },
        { on: false, why: "visible" },
      ]);
      expect(events[1].ms, "how long it was idle").toBeGreaterThan(1500);
      expect(events[1].ms).toBeLessThan(shownAt - hiddenAt + 1000);
    } finally {
      await live.set("band", volume(HAND2), "value", start);
    }
  });

  test("an idle period's slow pings and its wake count no dropout", async ({ page }) => {
    await openSurface(page);
    const surface = page.getByTestId("surface");
    const counter = page.getByTestId("dropouts");
    await expect(counter).toHaveText("0");
    await watchCounter(counter);
    await setHidden(page, true);
    await expect(surface).toHaveAttribute("data-idle", "true");

    // A 2 s stall: the pings, a second apart, wait for it under the 2.5 s an
    // idle page allows (300 ms while active, so the old rule counted it).
    // The page is shown 1.9 s into it with a ping unanswered: the silence
    // starts over at the wake, and the stall ends inside the 300 ms an
    // active page allows (without the restart, the wake counted at once).
    await impair.stall(2000);
    await page.waitForTimeout(1900);
    await setHidden(page, false);
    await expect(surface).toHaveAttribute("data-idle", "false");
    await page.waitForTimeout(1500);
    expect(await everRed(page), "never red").toBe(false);
    await expect(counter).toHaveText("0");
  });
});
