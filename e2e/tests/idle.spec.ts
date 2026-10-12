import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  HUB_SOCKET,
  LiveClient,
  type PointerStep,
  dispatchPointer,
  hubEvents,
  impair,
  openSurface,
  pageEvents,
  ready,
  resetLayout,
  servedLayout,
  setIdleAfter,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

// The page's idle state (#51). A hidden page (a locked iPad, the app in the
// background) is idle at once (PR 2a): its `visibilitychange` handler sends
// `idle {on: true, meter_ms: 0}`, and the hub holds every meter value for it
// while every other value goes at once. Shown again, it wakes (`idle {on:
// false}` and a ping) and the meters come back at once. While idle the link
// waits longer (pings a second apart, a dropout only after 2.5 s of
// silence), and the wake starts the silence over, so an idle ping is never a
// dropout under the finger. SimLive animates every meter at 30 Hz, so a
// page that gets meters gets many.
//
// Touch-idle (PR 2b): a page left untouched for the layout's
// `idle_after_s` (absent or 0, the fixture's default: never) is idle with
// its meters paced (250 ms); any pointer down wakes it, in a capture-phase
// listener on the window, before the control's own handler. The tests set
// a short `idle_after_s` through `/hub/layout` (never Playwright's fake
// clock: it fakes the page's timers and `performance.now`, and the pings'
// `t` would leave the hub's clock pairing). While idle its frame loop waits
// 100 ms after each frame (PR 2c), and the wake brings back every frame.

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

/** Touch-idle's seconds in the tests' layout. */
const IDLE_AFTER_S = 2;

/** How many values of each meter key reached the page from `from` to `to` (test clock). */
function meterCounts(link: Link, from: number, to: number): Map<string, number> {
  const counts = new Map<string, number>();
  for (const f of link.received.filter((f) => f.at >= from && f.at < to)) {
    for (const i of items(f.msg).filter((i) => isMeter(i.sub))) counts.set(i.sub, (counts.get(i.sub) ?? 0) + 1);
  }
  return counts;
}

/** The page's pings sent from `from` to `to` (test clock). */
const pingsBetween = (link: Link, from: number, to: number) => link.sent.filter((f) => f.msg.type === "ping" && f.at >= from && f.at < to);

/** A frame seen in the page: `performance.now()` when its callback ran, and the frame's own time. */
type Seen = [number, number];
type Frames = { app: Seen[]; browser: Seen[] };

/**
 * Counts the page's frames from its first script on (PR 2c): every callback
 * the app hands `requestAnimationFrame` (web-sys calls it through the
 * window's property, and `raf.rs` is the page's only caller), and every
 * frame the browser draws (a loop on the original function). Callbacks of
 * one frame get the same frame time, so the two lists match frame by frame.
 */
async function countFrames(page: Page) {
  await page.addInitScript(() => {
    const w = window as any;
    const raw = window.requestAnimationFrame.bind(window);
    w.__appFrames = [];
    w.__browserFrames = [];
    window.requestAnimationFrame = (callback: FrameRequestCallback) =>
      raw((time: number) => {
        w.__appFrames.push([performance.now(), time]);
        callback(time);
      });
    const loop = (time: number) => {
      w.__browserFrames.push([performance.now(), time]);
      raw(loop);
    };
    raw(loop);
  });
}

const readFrames = (page: Page): Promise<Frames> =>
  page.evaluate(() => {
    const w = window as any;
    return { app: w.__appFrames, browser: w.__browserFrames };
  });

/** The page clock now (`performance.now()`). */
const pageNow = (page: Page) => page.evaluate(() => performance.now());

/** Waits until the page clock reaches `at`. */
const pageTimeReaches = (page: Page, at: number, what: string) => until(() => pageNow(page), (now) => now >= at, what, 10_000);

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

    // A 1.5 s stall while idle: the pings, a second apart, wait for it
    // under the 2.5 s an idle page allows. One goes out within the stall
    // and its pong waits for its end, so an active page's 300 ms rule would
    // count it. The page is shown about 800 ms after the stall ended (PR 2a's
    // review: a wake 1.9 s into a 2 s stall left a slow runner 100-200 ms),
    // so the wake comes on a quiet link. The wake's own rule (the silence
    // starts over at the wake, so an idle ping never becomes a dropout under
    // the finger) is pinned by the native tests in behave/link/tests.rs.
    const stalledAt = Date.now();
    await impair.stall(1500);
    await page.waitForTimeout(Math.max(0, stalledAt + 2300 - Date.now()));
    await setHidden(page, false);
    await expect(surface).toHaveAttribute("data-idle", "false");
    await page.waitForTimeout(1500);
    expect(await everRed(page), "never red").toBe(false);
    await expect(counter).toHaveText("0");
  });
});

test.describe("Touch-idle", () => {
  test("an untouched page goes idle after the layout's seconds with its meters paced; a fader's touch wakes it before the fader sees it, and its sets reach Live at once", async ({
    page,
  }) => {
    const start = await live.get("band", volume(HAND2), "value");
    await live.set("band", volume(HAND2), "value", 0.5);
    const link = await recordLink(page);
    try {
      await setIdleAfter(IDLE_AFTER_S);
      const opened = Date.now();
      await openSurface(page);
      const surface = page.getByTestId("surface");
      const fader = strip(page, "Hand2 #").getByTestId("fader");
      await ready(fader);
      await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");

      // Untouched for the layout's 2 s: idle, its meters paced at 250 ms.
      await expect(surface).toHaveAttribute("data-idle", "true", { timeout: 10_000 });
      await expect.poll(() => sentOf(link, "idle")).toEqual([{ type: "idle", on: true, meter_ms: 250 }]);
      const slept = link.sent.find((f) => f.msg.type === "idle")!;
      expect(slept.at - opened, "not before the layout's seconds").toBeGreaterThanOrEqual(IDLE_AFTER_S * 1000 - 100);
      const records = await until(() => idleRecords(opened), (r) => r.length >= 1, "the hub's idle record");
      expect(records).toMatchObject([{ on: true, meter_ms: 250 }]);
      // While idle: a ping a second, every meter key at most 4 values a
      // second (SimLive's go at 30), none paused. The bounds follow the
      // window as measured (a slow runner makes it longer than 2.2 s): one
      // ping per second and one meter release per 250 ms of it, +2 for one
      // at each edge of the window. At full rate (pings every 100 ms,
      // SimLive's meters at 30 a second) both fail many times over.
      await page.waitForTimeout(2500);
      const quiet = { from: slept.at + 300, to: Date.now() };
      const span = quiet.to - quiet.from;
      const pings = pingsBetween(link, quiet.from, quiet.to).length;
      expect(pings, `pings a second apart in ${span} ms`).toBeLessThanOrEqual(Math.floor(span / 1000) + 2);
      const counts = meterCounts(link, quiet.from, quiet.to);
      expect(counts.size, "the meters still come").toBeGreaterThan(0);
      const most = Math.max(...counts.values());
      expect(most, `the most values of one meter key in ${span} ms`).toBeLessThanOrEqual(Math.floor(span / 250) + 2);

      // A finger on Hand2's fader. A listener on the fader itself notes the
      // page clock at the down: the wake's ping (sent right after its
      // `idle` off) already carries an earlier time, so the window's
      // capture-phase wake ran before anything on the fader.
      await fader.evaluate((el) => {
        el.addEventListener(
          "pointerdown",
          () => {
            (window as any).__faderDownT = performance.timeOrigin + performance.now();
          },
          { once: true },
        );
      });
      const touchedAt = Date.now();
      const drag: PointerStep[] = [{ type: "pointerdown" }, { wait: 30 }, { type: "pointermove", dy: 0 }];
      for (let i = 1; i <= 6; i++) drag.push({ wait: 30 }, { type: "pointermove", dy: 15 * i });
      drag.push({ wait: 30 }, { type: "pointerup", dy: 90 });
      await dispatchPointer(fader, drag);
      const liftedAt = Date.now();
      await expect(surface).toHaveAttribute("data-idle", "false");
      const isSet = (f: Frame) => f.msg.type === "set" && `${f.msg.instance}|${f.msg.target}|${f.msg.prop}` === VOLUME_KEY;
      const after = await until(async () => link.sent.filter((f) => f.at >= touchedAt), (a) => a.some(isSet), "the drag's sets");
      const off = after.findIndex((f) => f.msg.type === "idle");
      expect(after[off]?.msg).toEqual({ type: "idle", on: false });
      const wakePing = after[off + 1]?.msg;
      expect(wakePing?.type, "a ping right after it").toBe("ping");
      const downT = await page.evaluate(() => (window as any).__faderDownT as number);
      expect(wakePing.t, "the wake ran before the fader's own listener").toBeLessThanOrEqual(downT);
      expect(after.findIndex(isSet), "the first set after the wake's idle and ping").toBeGreaterThan(off + 1);
      // All of the drag's sets (the last goes at the lift).
      await page.waitForTimeout(300);
      const sets = link.sent.filter((f) => f.at >= touchedAt && isSet(f));

      // The sets reach Live at once: the first is acked well inside the 1 s
      // that would make it unconfirmed, and Live ends at the last one.
      const ackOf = (f: Frame) => f.msg.type === "ack" && f.msg.items.some((i: any) => i.key === VOLUME_KEY && i.seq >= sets[0].msg.seq);
      const acked = await until(async () => link.received.find(ackOf), (f) => f !== undefined, "the first set's ack");
      expect(acked!.at - sets[0].at, "the first set's ack (ms)").toBeLessThan(500);
      const last = sets[sets.length - 1].msg;
      expect(last.value).toBeGreaterThan(0.52);
      await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - last.value) < 1e-9, "Live at the last set's value", 2000);
      await expect(fader).toHaveAttribute("data-intent", "confirmed");
      // Awake: pings every 100 ms again.
      await page.waitForTimeout(Math.max(0, liftedAt + 1000 - Date.now()));
      expect(pingsBetween(link, liftedAt, liftedAt + 1000).length, "pings in the second after the lift").toBeGreaterThanOrEqual(5);

      // Untouched again, it goes idle the layout's seconds after the lift
      // (the finger held it awake until then).
      await expect(surface).toHaveAttribute("data-idle", "true", { timeout: 10_000 });
      const again = await until(
        async () => link.sent.filter((f) => f.msg.type === "idle" && f.at >= liftedAt),
        (a) => a.length >= 1,
        "the second idle",
      );
      expect(again.map((f) => f.msg)).toEqual([{ type: "idle", on: true, meter_ms: 250 }]);
      expect(again[0].at - liftedAt, "counted from the lift").toBeGreaterThanOrEqual(IDLE_AFTER_S * 1000 - 300);
      expect(again[0].at - liftedAt, "and not much later").toBeLessThan(IDLE_AFTER_S * 1000 + 1500);

      // The hub's records and the page's own events of the sleep and the wake.
      const all = await until(() => idleRecords(opened), (r) => r.length >= 3, "the hub's idle records");
      expect(all).toMatchObject([
        { on: true, meter_ms: 250 },
        { on: false, meter_ms: null },
        { on: true, meter_ms: 250 },
      ]);
      const events = await until(
        async () => pageEvents(await hubEvents()).filter((e) => e.ev === "idle" && e.t >= opened),
        (e) => e.length >= 2,
        "the page's idle events",
        10_000,
      );
      expect(events.slice(0, 2)).toMatchObject([
        { on: true, why: "touch" },
        { on: false, why: "touch" },
      ]);
      expect(events[1].ms, "how long it was idle").toBeGreaterThan(2000);
    } finally {
      await resetLayout();
      await live.set("band", volume(HAND2), "value", start);
    }
  });

  test("an idle page draws about ten frames a second, and a touch brings back every frame at once", async ({ page }) => {
    // The band, honest for both projects: idle, the app's frames are at
    // least 95 ms apart (raf.rs waits 100 ms after a frame's work before it
    // asks for the next one; 5 ms for the clocks' grain), at most 10.5 and at
    // least 3 a second (the next display frame after the wait: about 9 in
    // Chromium at 60 Hz, 5 to 7 in the runner's software WebKit, which draws
    // 14 to 22 frames a second). Awake again, no absolute rate (WebKit's is
    // the runner's): the app draws in every frame the browser draws from the
    // touch on, the first included.
    await countFrames(page);
    try {
      await setIdleAfter(IDLE_AFTER_S);
      await openSurface(page);
      const surface = page.getByTestId("surface");
      await expect(surface).toHaveAttribute("data-idle", "true", { timeout: 10_000 });

      // Idle: from a moment after the turn (the frame the loop asked for
      // before it still comes at once), 2 s of the app's frames.
      const idleFrom = (await pageNow(page)) + 300;
      const idleTo = idleFrom + 2000;
      await pageTimeReaches(page, idleTo, "2 s of the idle page's frames");
      await expect(surface).toHaveAttribute("data-idle", "true");
      const idle = (await readFrames(page)).app.map(([at]) => at).filter((at) => at >= idleFrom && at < idleTo);
      const gaps = idle.slice(1).map((at, i) => at - idle[i]);
      const listed = gaps.map(Math.round).join(", ");
      expect(idle.length, `the app's frames in the idle 2 s (gaps: ${listed})`).toBeGreaterThanOrEqual(7);
      expect(Math.min(...gaps), `the app's frame gaps while idle: ${listed}`).toBeGreaterThanOrEqual(95);
      const rate = ((idle.length - 1) * 1000) / (idle[idle.length - 1] - idle[0]);
      expect(rate, `the app's frames a second while idle (gaps: ${listed})`).toBeLessThanOrEqual(10.5);
      expect(rate, `the app's frames a second while idle (gaps: ${listed})`).toBeGreaterThanOrEqual(3);

      // A finger anywhere on the page wakes it (the window's capture-phase
      // listener), lifted at once so nothing holds the page awake: the app's
      // frames come back in every frame the browser draws, from the first.
      // The page goes idle again 2 s after the touch, so the check reads the
      // 1.5 s after it.
      const woke = await page.evaluate(() => {
        const at = performance.now();
        for (const type of ["pointerdown", "pointerup"]) {
          document.body.dispatchEvent(
            new PointerEvent(type, { pointerId: 41, pointerType: "touch", isPrimary: true, bubbles: true, cancelable: true }),
          );
        }
        return at;
      });
      await expect(surface).toHaveAttribute("data-idle", "false");
      const wakeTo = woke + 1500;
      await pageTimeReaches(page, wakeTo, "1.5 s after the touch");
      const after = await readFrames(page);
      const drawn = after.browser.filter(([at]) => at > woke && at < wakeTo);
      const appTimes = new Set(after.app.filter(([at]) => at > woke).map(([, time]) => time));
      const missed = drawn.filter(([, time]) => !appTimes.has(time)).map(([at]) => Math.round(at - woke));
      expect(drawn.length, "the browser's frames in the 1.5 s after the touch").toBeGreaterThanOrEqual(5);
      expect(missed, `browser frames the app skipped after the touch (ms after it; ${drawn.length} drawn)`).toEqual([]);
    } finally {
      await resetLayout();
    }
  });

  test("the fixture's layout, with no idle_after_s, never makes an untouched page idle", async ({ page }) => {
    expect((await servedLayout()).config.idle_after_s, "the fixture sets no touch-idle").toBeUndefined();
    const link = await recordLink(page);
    const since = Date.now();
    await openSurface(page);
    // Twice the tests' touch-idle seconds, untouched.
    await page.waitForTimeout(2 * IDLE_AFTER_S * 1000);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-idle", "false");
    expect(sentOf(link, "idle"), "no idle sent").toEqual([]);
    expect(await idleRecords(since), "no idle record at the hub").toEqual([]);
    const now = Date.now();
    expect(pingsBetween(link, now - 1000, now).length, "pings every 100 ms").toBeGreaterThanOrEqual(6);
  });
});
