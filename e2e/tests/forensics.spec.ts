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
  pageEvents,
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
// recorder (the touch with where it started, the moves of each frame, the
// dropout; PR D), the touch's first set carries Live's value before it
// (`live_before`), and the forensics timeline (`tools/forensics/timeline.py`,
// run by the harness over the hub's logs) renders that window with the stall
// marked. A touch that starts from a value Live no longer holds is a
// first-touch jump in the timeline.

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

/** The page events in the hub's event log (every `trace` record's, each once), from page time `from` on. */
async function pageEventsFrom(from: number): Promise<any[]> {
  return pageEvents(await hubEvents()).filter((e: any) => e.t >= from - 1000);
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

  // The page's record: its touch (down with where it started, and up), the
  // moves of each frame that sent, the dropout of the stall.
  const record = await until(
    () => pageEventsFrom(dragFrom),
    (all) => all.some((e: any) => e.ev === "touch" && e.what === "up" && e.keys.includes(KEY)),
    "the page's touch in the event log",
    10_000,
  );
  const down = record.find((e: any) => e.ev === "touch" && e.what === "down" && e.keys.includes(KEY) && e.pointer === 61);
  expect(down, "the down").toBeTruthy();
  for (const field of ["dt", "c", "travel", "pos", "live", "from"]) expect(typeof down[field], field).toBe("number");
  expect(down.local, "a fader at rest shows Live's value").toBe(false);
  expect(down.from, "the touch starts from Live's value").toBe(down.live);
  expect(down.dt, "the pointer event came before its handler").toBeLessThanOrEqual(0);
  const moves = record.filter((e: any) => e.ev === "mv" && e.key === KEY && e.p === 61);
  expect(moves.length, "the frames that sent from the finger").toBeGreaterThan(5);
  for (const m of moves) {
    expect(m.e.length, "a frame's moves").toBeGreaterThan(0);
    for (const [dt, c] of m.e) expect([typeof dt, typeof c]).toEqual(["number", "number"]);
    expect([typeof m.r, typeof m.s, typeof m.q]).toEqual(["number", "number", "number"]);
  }
  // The finger went up 1 px every 20 ms: its moves' coordinates fall.
  const ys = moves.flatMap((m: any) => m.e.map((e: number[]) => e[1]));
  expect(ys[ys.length - 1], "the finger's last coordinate").toBeLessThan(ys[0]);
  expect(record.some((e: any) => e.ev === "send"), "a set the socket took is the hub's record").toBe(false);
  const dropout = record.find((e: any) => e.ev === "dropout" && e.t < stallTo && e.t + e.ms > stallFrom);
  expect(dropout, `the stall's dropout among ${JSON.stringify(record.filter((e: any) => e.ev === "dropout"))}`).toBeTruthy();

  // Every hop of the last frame's set: set → batch → applied → ack.
  const lastSeq = Math.max(...moves.map((m: any) => m.q));
  const mine = (e: any) => e.ev === "set" && e.key === KEY && e.seq === lastSeq && e.t >= dragFrom;
  const events = await until(
    hubEvents,
    (ev) => {
      const set = ev.find(mine);
      return !!set && ev.some((e) => e.ev === "ack" && e.client === set.client && e.key === KEY && e.seq === lastSeq);
    },
    "the last frame's set and ack in the event log",
    10_000,
  );
  const set = events.find(mine);
  const batch = events.find(
    (e) => e.ev === "batch" && e.instance === "band" && e.sent.some((s: any) => s.key === KEY && s.client === set.client && s.seq === lastSeq),
  );
  expect(batch, "its batch").toBeTruthy();
  const applied = events.find((e) => e.ev === "applied" && e.instance === "band" && e.batch === batch.batch);
  expect(applied, "Live's result of its batch").toBeTruthy();
  expect(applied.errors).toBe(0);
  await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - set.value) < 1e-6, "Live at the last send");
  // The touch's first set carries Live's value before it; no other does.
  const sets = events.filter((e) => e.ev === "set" && e.key === KEY && e.client === set.client && e.t >= dragFrom);
  const first = sets.reduce((a: any, b: any) => (b.seq < a.seq ? b : a));
  expect(first.live_before, "Live's value before the touch").toBeCloseTo(START, 6);
  expect(sets.filter((e) => "live_before" in e), "only the touch's first set").toEqual([first]);

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
  // The touch started at Live's value: no first-touch jump.
  expect(report.stdout).toMatch(/^touches=1$/m);
  expect(report.stdout).toMatch(/^first_touch_jumps=0$/m);
  expect(report.stdout).toMatch(/^first_move_max_ms=\d/m);
  expect(report.stdout).not.toContain("Hand2");
});

test("a touch that starts from a value Live no longer holds is a first-touch jump in the timeline", async ({ page }) => {
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < 1e-3, "the fader at 0.5");

  // The page's link stalls; meanwhile Live goes to 0 dB (0.85), which the
  // page cannot hear, and a finger moves the fader 5 px from what it shows.
  const from = await pageNow(page);
  await impair.stall(2500);
  await live.set("band", volume(HAND2), "value", 0.85);
  await page.waitForTimeout(300);
  const steps: PointerStep[] = [{ type: "pointerdown" }];
  for (let i = 1; i <= 5; i++) steps.push({ wait: 20 }, { type: "pointermove", dy: i });
  steps.push({ type: "pointerup", dy: 5 });
  await dispatchPointer(fader, steps, 62);

  // After the stall the touch's first set reaches the hub, which held 0.85.
  const events = await until(
    hubEvents,
    (ev) => ev.some((e) => e.ev === "set" && e.key === KEY && e.t >= from && "live_before" in e),
    "the touch's first set",
    10_000,
  );
  const first = events.find((e) => e.ev === "set" && e.key === KEY && e.t >= from && "live_before" in e);
  expect(first.live_before).toBeCloseTo(0.85, 6);
  await until(
    () => pageEventsFrom(from),
    (all) => all.some((e: any) => e.ev === "touch" && e.what === "up" && e.pointer === 62),
    "the page's touch in the event log",
    10_000,
  );
  const to = (await pageNow(page)) + 1000;
  const report = await harness("/forensics/timeline", { from_ms: from - 1000, to_ms: to });
  expect(report.exit, report.stderr).toBe(0);
  expect(report.stdout).toMatch(/^first_touch_jumps=1$/m);
  const row = [...report.html.matchAll(/<tr\b[^>]*class="touch"[^>]*>/g)].map((m) => m[0]);
  expect(row.length, "one touch").toBe(1);
  expect(row[0]).toContain('data-first-jump="true"');
  expect(row[0]).toContain('data-why="stale"');
});
