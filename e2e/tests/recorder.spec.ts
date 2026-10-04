import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  type PointerStep,
  dispatchPointer,
  hubEvents,
  impair,
  openSurface,
  pageEvents,
  ready,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

// The page's flight recorder on a slow link (#43, PR D; design note §5.2):
// the recorder never delays a fader's sets (its batches wait while sets go,
// and are small and rate-capped), and it never falls behind (the newest page
// event reaches the hub's event log within seconds). The link is the impair
// proxy's rate limit on the page's bytes to the hub; "the recorder off" is the
// same page with its `trace` frames dropped by a wrapper of
// `WebSocket.prototype.send` (as the link sees it, nothing of the recorder).

const HAND2 = track("Hand2 #");
const KEY = `band|${volume(HAND2)}|value`;
const START = 0.5;
/** The slow link: 24 KB/s from the page to the hub (one dragged fader's sets are ~9 KB/s). */
const LINK_BYTES_PER_S = 24 * 1024;

/**
 * One drag's length (px, a 1 px move every 20 ms: 2 s). Its page events must
 * fill 4 batches (the gate check below can fail only then); WebKit on the
 * runner draws ~14 frames a second and, since PR E records no acks, a 75 px
 * drag left it 3 881 bytes.
 */
const DRAG_PX = 100;

/** The page's clock (`performance.timeOrigin + performance.now()`, the `t` of its sets and events). */
const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

/** Whether the page's `trace` frames are dropped before they reach the socket. */
const dropTraces = (page: Page, on: boolean) =>
  page.evaluate((drop) => {
    (window as unknown as { dropTraces: boolean }).dropTraces = drop;
  }, on);

/** A finger dragging `fader` `px` px (up when positive), a 1 px move every 20 ms. */
function drag(fader: Locator, pointerId: number, px: number): Promise<number> {
  const steps: PointerStep[] = [{ type: "pointerdown" }];
  const sign = Math.sign(px);
  for (let i = 1; i <= Math.abs(px); i++) steps.push({ wait: 20 }, { type: "pointermove", dy: sign * i });
  steps.push({ type: "pointerup", dy: px });
  return dispatchPointer(fader, steps, pointerId);
}

/** The moves of a long drag (#43 PR E): 1 200 moves of 1 px every 20 ms, 24 s (WebKit on the runner draws ~14 frames a second here: its moves must still pass PR D's 48 KB). */
const LONG_MOVES = 1200;
/** How far a long drag goes either way (px): a triangle wave around the fader's start. */
const LONG_AMPLITUDE = 60;

/** A finger dragging `fader` `moves` 1 px moves every 20 ms, up and down in a triangle wave of ±`LONG_AMPLITUDE` px. */
function longDrag(fader: Locator, pointerId: number, moves: number): Promise<number> {
  const a = LONG_AMPLITUDE;
  const steps: PointerStep[] = [{ type: "pointerdown" }];
  let dy = 0;
  for (let i = 1; i <= moves; i++) {
    dy = a - Math.abs(((i + a) % (4 * a)) - 2 * a);
    steps.push({ wait: 20 }, { type: "pointermove", dy });
  }
  steps.push({ type: "pointerup", dy });
  return dispatchPointer(fader, steps, pointerId);
}

/** The page events of the hub's trace records, each with its record's `ts`. */
function traceEvents(events: any[]): { ts: number; e: any }[] {
  return events.filter((r) => r.ev === "trace").flatMap((r) => r.events.map((e: any) => ({ ts: r.ts, e })));
}

/** Two drags back to back (the second starts 50 ms after the first one's lift): the page clock before, between and after. */
async function twoDrags(page: Page, fader: Locator, pointer: number): Promise<{ from: number; between: number; to: number }> {
  const from = await pageNow(page);
  await drag(fader, pointer, DRAG_PX);
  const between = await pageNow(page);
  await page.waitForTimeout(50);
  await drag(fader, pointer + 1, -DRAG_PX);
  return { from, between, to: await pageNow(page) };
}

/** The fader's sets the page sent from `from` to `to` (a drag sends dozens): the first and last one's arrival (`hub_ms`) and page time (`t`). */
function dragSets(events: any[], from: number, to: number): { first: number; last: number; t0: number; t1: number } {
  const sets = events.filter((e) => e.ev === "set" && e.key === KEY && e.t >= from && e.t <= to);
  expect(sets.length, "the drag's sets").toBeGreaterThan(10);
  const arrived = sets.map((e) => e.hub_ms);
  const sent = sets.map((e) => e.t);
  return { first: Math.min(...arrived), last: Math.max(...arrived), t0: Math.min(...sent), t1: Math.max(...sent) };
}

/** The bytes of the page events (each once) that the hub's trace records hold with a page time from `from` to `to`. */
function eventBytes(events: any[], from: number, to: number): number {
  const texts = new Set<string>();
  for (const r of events.filter((e) => e.ev === "trace")) {
    for (const e of r.events) if (e.t >= from && e.t <= to) texts.add(JSON.stringify(e));
  }
  return [...texts].reduce((sum, text) => sum + text.length, 0);
}

/** The one-way delay (ms) of each of the fader's sets the page sent from `from` to `to` (page clock), on the runner's one clock. */
function delays(events: any[], from: number, to: number): number[] {
  return events
    .filter((e) => e.ev === "set" && e.key === KEY && e.t >= from && e.t <= to)
    .map((e) => e.hub_ms - e.t)
    .sort((a, b) => a - b);
}

/** The nearest-rank percentile of sorted `values`. */
function percentile(values: number[], fraction: number): number {
  return values[Math.max(0, Math.ceil(fraction * values.length) - 1)];
}

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", volume(HAND2), "value", START);
});
test.afterEach(async () => {
  await impair.rate(0);
  await live.set("band", volume(HAND2), "value", START);
  live.close();
});

test("on a slow link the recorder never delays a drag's sets and its newest event reaches the hub within seconds", async ({ page }) => {
  test.setTimeout(90_000);
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
      const drop = (window as unknown as { dropTraces?: boolean }).dropTraces === true;
      if (drop && typeof data === "string" && data.startsWith('{"type":"trace"')) return;
      return send.call(this, data);
    };
  });
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < 1e-3, "the fader at 0.5");
  await impair.rate(LINK_BYTES_PER_S);
  // The load's own events go first.
  await page.waitForTimeout(3000);

  // The recorder off (its frames never reach the link): two drags.
  await dropTraces(page, true);
  const off = await twoDrags(page, fader, 71);
  await page.waitForTimeout(3000);

  // The recorder on: the same two drags; the second starts while the first
  // one's events wait to go up.
  await dropTraces(page, false);
  const on = await twoDrags(page, fader, 81);

  // The newest page event, the second drag's lift, reaches the hub's event
  // log within seconds of the lift.
  const lift = await until(
    async () =>
      (await hubEvents())
        .filter((r) => r.ev === "trace")
        .flatMap((r) => r.events.map((e: any) => ({ ts: r.ts, e })))
        .find(({ e }) => e.ev === "touch" && e.what === "up" && e.pointer === 82 && e.t >= on.from),
    (found) => found !== undefined,
    "the second drag's lift in the event log",
    20_000,
  );
  const lag = lift!.ts - lift!.e.t;
  test.info().annotations.push({ type: "lag-ms", description: String(Math.round(lag)) });
  // The bound (design note §5.2): the two drags' events (~33 KB, at one
  // batch of at most 1 000 bytes per 100 ms tick once the fingers rest about
  // 3.5 s, and the last, partial batch waits its 2 s) are about what PR D's
  // two drags left (~32 KB); the page's timers run late on a loaded runner (a
  // 6.1 s lag for ~32 KB, #43), so 10 s. PR C's gate starved it for minutes.
  expect(lag, "the recorder's lag behind the page (ms)").toBeLessThan(10_000);

  // With the recorder on, no batch goes while a drag's sets do: between a
  // drag's first and last set at most 2 `trace` records arrive (the socket's
  // one task stamps both in arrival order; `ts` is whole ms, `hub_ms` finer:
  // 5 ms either side). A tick lets one through only after a page frame over
  // 100 ms without a set (WebKit on the runner draws 14 to 22 frames a second).
  // The check can fail: the page events recorded between the drag's first
  // and last set fill at least 4 batches of 1 000 bytes, so without the
  // gate at least 3 would go inside it.
  const events = await hubEvents();
  for (const [a, b] of [
    [on.from, on.between],
    [on.between, on.to],
  ]) {
    const { first, last, t0, t1 } = dragSets(events, a, b);
    const bytes = eventBytes(events, t0, t1);
    test.info().annotations.push({ type: "drag-bytes", description: String(bytes) });
    expect(bytes, "the page events between the drag's first and last set (bytes)").toBeGreaterThanOrEqual(4000);
    const inside = events.filter((e) => e.ev === "trace" && e.ts > first + 5 && e.ts < last - 5);
    expect(inside.length, `trace records inside a drag of ${Math.round(last - first)} ms: ${JSON.stringify(inside.map((e) => e.ts - first))}`).toBeLessThanOrEqual(2);
  }

  // It drains without a stall once the fingers rest: from the second drag's
  // last set to the lift's batch the hub logs a trace record at least every
  // 500 ms (a full batch per tick, five ticks of slack for late timers).
  // The drain's last batch holds what no longer fills one, and that waits
  // its 2 s since the batch before (design note §5.2: smaller amounts every
  // 2 s; a 2 005 ms last gap failed the 500 ms bound once, #43 PR E).
  const lastSet = dragSets(events, on.between, on.to).last;
  const drained = events
    .filter((e) => e.ev === "trace" && e.ts >= lastSet && e.ts <= lift!.ts)
    .map((e) => e.ts)
    .sort((a, b) => a - b);
  const stalls = [lastSet, ...drained].slice(1).map((ts, i) => ts - [lastSet, ...drained][i]);
  const gaps = `the gaps between trace records while it drains (ms): ${JSON.stringify(stalls)}`;
  expect(Math.max(...stalls.slice(0, -1)), gaps).toBeLessThanOrEqual(500);
  expect(stalls[stalls.length - 1], gaps).toBeLessThanOrEqual(2500);

  // The recorder on delays the sets no more than the link itself does with
  // the recorder off (both phases' sets as the hub logged them).
  const offDelays = delays(events, off.from, off.to);
  const onDelays = delays(events, on.from, on.to);
  expect(offDelays.length, "the drags' sets with the recorder off").toBeGreaterThan(20);
  expect(onDelays.length, "the drags' sets with the recorder on").toBeGreaterThan(20);
  const summary = `off: ${JSON.stringify(offDelays.map(Math.round))}; on: ${JSON.stringify(onDelays.map(Math.round))}`;
  expect(percentile(onDelays, 0.5), `median, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.5) + 20);
  expect(percentile(onDelays, 0.9), `p90, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.9) + 30);
});

test("a long drag on a slow link keeps every move, and the recorder still never delays its sets", async ({ page }) => {
  // #43 PR E: a drag longer than the old 48 KB backlog (about 4 s of one
  // fader at 60 Hz) lost its oldest moves; the page records no `send` or
  // `ack` any more, and its backlog holds a 30 s drag of two faders.
  test.setTimeout(300_000);
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
      const drop = (window as unknown as { dropTraces?: boolean }).dropTraces === true;
      if (drop && typeof data === "string" && data.startsWith('{"type":"trace"')) return;
      return send.call(this, data);
    };
  });
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < 1e-3, "the fader at 0.5");
  await impair.rate(LINK_BYTES_PER_S);
  await page.waitForTimeout(3000);

  // The recorder off: a long drag; its events drain (dropped on the way)
  // before the next one.
  await dropTraces(page, true);
  const offFrom = await pageNow(page);
  await longDrag(fader, 91, LONG_MOVES);
  const offTo = await pageNow(page);
  await page.waitForTimeout(30_000);

  // The recorder on: the same long drag.
  await dropTraces(page, false);
  const onFrom = await pageNow(page);
  await longDrag(fader, 92, LONG_MOVES);
  const onTo = await pageNow(page);

  // Its lift reaches the event log once the backlog before it drained
  // (Chromium's ~200 KB at the 10 KB/s cap: about 20 s).
  await until(
    async () => traceEvents(await hubEvents()).find(({ e }) => e.ev === "touch" && e.what === "up" && e.pointer === 92 && e.t >= onFrom),
    (found) => found !== undefined,
    "the long drag's lift in the event log",
    90_000,
  );
  const events = await hubEvents();
  const logged = pageEvents(events);

  // Every frame that sent a set recorded its move: each of the drag's sets
  // (the hub's records) has the `mv` that names its seq.
  const sets = events.filter((e) => e.ev === "set" && e.key === KEY && e.t >= onFrom && e.t <= onTo);
  expect(sets.length, "the long drag's sets").toBeGreaterThan(200);
  const moves = logged.filter((e) => e.ev === "mv" && e.p === 92 && e.t >= onFrom && e.t <= onTo);
  const named = new Set(moves.map((e) => e.q));
  const missing = sets.filter((e) => !named.has(e.seq)).map((e) => e.seq);
  expect(missing, `sets whose move record is missing (of ${sets.length})`).toEqual([]);
  // Nothing was dropped, and the drag's moves reaching the hub are more than
  // the old 48 KB bound held (the check can fail).
  const notes = logged.filter((e) => e.ev === "overflow" && e.t >= offFrom);
  expect(notes, "the recorder's drop notes").toEqual([]);
  const bytes = eventBytes(events, onFrom, onTo);
  test.info().annotations.push({ type: "drag-bytes", description: String(bytes) });
  expect(bytes, "the long drag's page events (bytes)").toBeGreaterThan(48 * 1024);
  // No `send` (every set was taken) and no `ack` record: the hub has both.
  const hubs = logged.filter((e) => (e.ev === "send" || e.ev === "ack") && e.t >= offFrom);
  expect(hubs.length, "send and ack records").toBe(0);

  // The recorder on delays the long drag's sets no more than with it off.
  const offDelays = delays(events, offFrom, offTo);
  const onDelays = delays(events, onFrom, onTo);
  expect(offDelays.length, "the long drag's sets with the recorder off").toBeGreaterThan(200);
  const summary = `off: p50 ${percentile(offDelays, 0.5)}, p90 ${percentile(offDelays, 0.9)}; on: p50 ${percentile(onDelays, 0.5)}, p90 ${percentile(onDelays, 0.9)}`;
  expect(percentile(onDelays, 0.5), `median, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.5) + 20);
  expect(percentile(onDelays, 0.9), `p90, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.9) + 30);
});
