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

/** Two drags back to back (the second starts 50 ms after the first one's lift): the page clock before and after. */
async function twoDrags(page: Page, fader: Locator, pointer: number): Promise<{ from: number; to: number }> {
  const from = await pageNow(page);
  await drag(fader, pointer, 75);
  await page.waitForTimeout(50);
  await drag(fader, pointer + 1, -75);
  return { from, to: await pageNow(page) };
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

  // Every set of both phases reached the hub; the recorder on delays none of
  // them beyond what the link itself does with the recorder off.
  const events = await hubEvents();
  const offDelays = delays(events, off.from, off.to);
  const onDelays = delays(events, on.from, on.to);
  expect(offDelays.length, "the drags' sets with the recorder off").toBeGreaterThan(20);
  expect(onDelays.length, "the drags' sets with the recorder on").toBeGreaterThan(20);
  const summary = `off: ${JSON.stringify(offDelays.map(Math.round))}; on: ${JSON.stringify(onDelays.map(Math.round))}`;
  expect(percentile(onDelays, 0.5), `median, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.5) + 20);
  expect(percentile(onDelays, 0.9), `p90, ${summary}`).toBeLessThanOrEqual(percentile(offDelays, 0.9) + 30);
});
