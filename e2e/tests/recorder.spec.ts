import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  type PointerStep,
  dispatchPointer,
  hubEvents,
  impair,
  openSurface,
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

/** Two drags back to back (the second starts 50 ms after the first one's lift): the page clock before, between and after. */
async function twoDrags(page: Page, fader: Locator, pointer: number): Promise<{ from: number; between: number; to: number }> {
  const from = await pageNow(page);
  await drag(fader, pointer, 75);
  const between = await pageNow(page);
  await page.waitForTimeout(50);
  await drag(fader, pointer + 1, -75);
  return { from, between, to: await pageNow(page) };
}

/** The hub's arrival (`hub_ms`) of the first and last of the fader's sets the page sent from `from` to `to` (a drag sends dozens). */
function arrivals(events: any[], from: number, to: number): [number, number] {
  const sets = events.filter((e) => e.ev === "set" && e.key === KEY && e.t >= from && e.t <= to).map((e) => e.hub_ms);
  expect(sets.length, "the drag's sets").toBeGreaterThan(10);
  return [Math.min(...sets), Math.max(...sets)];
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
  expect(lift!.ts - lift!.e.t, "the recorder's lag behind the page (ms)").toBeLessThan(6000);

  // With the recorder on, no batch goes while a drag's sets do: between a
  // drag's first and last set at most 2 `trace` records arrive (the socket's
  // one task stamps both in arrival order; `ts` is whole ms, `hub_ms` finer:
  // 5 ms either side). A tick lets one through only after a page frame over
  // 100 ms without a set (WebKit on the runner draws ~22 frames a second).
  // The check can fail: the drag's own events fill at least 4 batches of
  // 1 000 bytes, so without the gate at least 3 would go inside it.
  const events = await hubEvents();
  for (const [a, b] of [
    [on.from, on.between],
    [on.between, on.to],
  ]) {
    const [first, last] = arrivals(events, a, b);
    expect(eventBytes(events, a, b), "the drag's own page events (bytes)").toBeGreaterThanOrEqual(4000);
    const inside = events.filter((e) => e.ev === "trace" && e.ts > first + 5 && e.ts < last - 5);
    expect(inside.length, `trace records inside a drag of ${Math.round(last - first)} ms: ${JSON.stringify(inside.map((e) => e.ts - first))}`).toBeLessThanOrEqual(2);
  }

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
