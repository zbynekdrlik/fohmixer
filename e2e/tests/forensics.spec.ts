import type { Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  type PointerStep,
  dispatchPointer,
  harness,
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

// A fader dragged through a stall, read back from the logs alone (#43, PR C;
// design note §5, §6.2 test 7): the hub's event log holds the move's every
// hop (`set` → `batch` → `applied` → `ack`) and the page's own flight
// recorder (the touch, its sends, the dropout), and the forensics timeline
// (`tools/forensics/timeline.py`, run by the harness over the hub's logs)
// renders that window with the stall marked.

const HAND2 = track("Hand2 #");
const KEY = `band|${volume(HAND2)}|value`;
const START = 0.5;

/** The page's clock (`performance.timeOrigin + performance.now()`, the `t` of its events). */
const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

/** The data attributes of every `<rect>` of `cls` in a report. */
function rects(html: string, cls: string): Record<string, string>[] {
  return [...html.matchAll(/<rect\b[^>]*>/g)]
    .map((m) => m[0])
    .filter((tag) => new RegExp(`class="${cls}"`).test(tag))
    .map((tag) => Object.fromEntries([...tag.matchAll(/data-([a-z-]+)="([^"]*)"/g)].map((a) => [a[1], a[2]])));
}

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", volume(HAND2), "value", START);
});
test.afterEach(async () => {
  await live.set("band", volume(HAND2), "value", START);
  live.close();
});

test("a fader dragged through a stall: every hop and the page's record in the event log, and the timeline marks the stall", async ({ page }) => {
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < 1e-3, "the fader at 0.5");

  // A finger drags the fader up for 1.6 s (a 1 px move every 20 ms); 400 ms
  // in, the link stalls 800 ms in both directions.
  const steps: PointerStep[] = [{ type: "pointerdown" }];
  for (let i = 1; i <= 80; i++) steps.push({ wait: 20 }, { type: "pointermove", dy: i });
  steps.push({ type: "pointerup", dy: 80 });
  const dragFrom = await pageNow(page);
  const drag = dispatchPointer(fader, steps, 61);
  await page.waitForTimeout(400);
  const stallFrom = await pageNow(page);
  await impair.stall(800);
  await drag;
  const stallTo = stallFrom + 800;
  const dragTo = await pageNow(page);

  // The page's record: its touch (down and up), its sends of the fader, the
  // dropout of the stall.
  const pageEvents = async () =>
    (await hubEvents())
      .filter((r) => r.ev === "trace")
      .flatMap((r) => r.events)
      .filter((e: any) => e.t >= dragFrom - 1000);
  const record = await until(
    pageEvents,
    (all) => all.some((e: any) => e.ev === "touch" && e.what === "up" && e.keys.includes(KEY)),
    "the page's touch in the event log",
    10_000,
  );
  expect(record.some((e: any) => e.ev === "touch" && e.what === "down" && e.keys.includes(KEY) && e.pointer === 61)).toBe(true);
  const sends = record.filter((e: any) => e.ev === "send" && e.key === KEY);
  expect(sends.length, "the page's sends of the fader").toBeGreaterThan(5);
  const dropout = record.find((e: any) => e.ev === "dropout" && e.t < stallTo && e.t + e.ms > stallFrom);
  expect(dropout, `the stall's dropout among ${JSON.stringify(record.filter((e: any) => e.ev === "dropout"))}`).toBeTruthy();

  // Every hop of the last send: set → batch → applied → ack.
  const last = sends.reduce((a: any, b: any) => (b.seq > a.seq ? b : a));
  const mine = (e: any) => e.ev === "set" && e.key === KEY && e.seq === last.seq && e.t === last.t;
  const events = await until(
    hubEvents,
    (ev) => {
      const set = ev.find(mine);
      return !!set && ev.some((e) => e.ev === "ack" && e.client === set.client && e.key === KEY && e.seq === last.seq);
    },
    "the last send's set and ack in the event log",
    10_000,
  );
  const set = events.find(mine);
  const batch = events.find(
    (e) => e.ev === "batch" && e.instance === "band" && e.sent.some((s: any) => s.key === KEY && s.client === set.client && s.seq === last.seq),
  );
  expect(batch, "its batch").toBeTruthy();
  const applied = events.find((e) => e.ev === "applied" && e.instance === "band" && e.batch === batch.batch);
  expect(applied, "Live's result of its batch").toBeTruthy();
  expect(applied.errors).toBe(0);
  await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - last.value) < 1e-6, "Live at the last send");

  // The forensics timeline of that window marks the stall: the dropout, and
  // the gap in the hub's arrivals of the fader's sets.
  const report = await harness("/forensics/timeline", { from_ms: dragFrom - 2000, to_ms: dragTo + 3000 });
  expect(report.exit, report.stderr).toBe(0);
  const html: string = report.html;
  const marked = rects(html, "dropout").filter((r) => Number(r.start) < stallTo && Number(r.end) > stallFrom);
  expect(marked.length, `a dropout over the stall among ${JSON.stringify(rects(html, "dropout"))}`).toBeGreaterThan(0);
  expect(Number(marked[0].ms)).toBeGreaterThanOrEqual(300);
  const gaps = rects(html, "gap").filter(
    (r) => r.line === "arrival" && Number(r.ms) >= 400 && Number(r.start) < stallTo && Number(r.end) > stallFrom,
  );
  expect(gaps.length, `the arrivals' gap over the stall among ${JSON.stringify(rects(html, "gap"))}`).toBeGreaterThan(0);
  // The summary names numbers only (never a key or a track name).
  expect(report.stdout).toMatch(/^dropouts=[1-9]\d*$/m);
  expect(report.stdout).toMatch(/^confirmation_n=[1-9]\d*$/m);
  expect(report.stdout).toMatch(/^longest_dropout_ms=\d/m);
  expect(report.stdout).not.toContain("Hand2");
});
