import { test, expect } from "./support/fixtures";
import { LiveClient, type PointerStep, dispatchPointer, frames, openSurface, ready, shown, strip, track, until, volume } from "./support/live";

// A touch's start (#43 PR F, design comment 5990698988): on the FOH iPad the
// first pointer event of a touch came a median 83 ms after the down and 6 px
// away (up to 15 px), later ones 2 px every 17 ms. Applied at once, that
// first event moved the cap by several px in one frame (the "jump at the
// first touch"). The touch's first pointer move now only anchors the drag:
// the cap stays on that frame, and the drag follows the finger from there.

const HAND2 = track("Hand2 #");
const START = 0.5;
/** How far the finger goes after its first event (px), 2 px a move. */
const FOLLOW_PX = 30;

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", volume(HAND2), "value", START);
});
test.afterEach(async () => {
  await live.set("band", volume(HAND2), "value", START);
  live.close();
});

test("a touch's first move only anchors the drag: no cap jump on its frame, then the cap follows the finger", async ({ page }) => {
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < 1e-3, "the fader at 0.5");

  // Every animation frame: the cap's top on the page (px); every pointer
  // move the fader gets: its time.
  await fader.evaluate((el) => {
    const w = window as any;
    const cap = el.querySelector(".fader-cap") as Element;
    w.__caps = [];
    w.__moves = [];
    w.__sampling = true;
    el.addEventListener("pointermove", () => w.__moves.push(performance.now()), { capture: true });
    const step = () => {
      w.__caps.push([performance.now(), cap.getBoundingClientRect().top]);
      if (w.__sampling) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
  await frames(page, 3);

  // A finger as the iPad gives it: its first event 10 px up 80 ms after the
  // down, then nothing for 300 ms (frames to see a jump in: WebKit on the CI
  // runner draws ~14 a second), then 2 px every 17 ms.
  const steps: PointerStep[] = [{ type: "pointerdown" }, { wait: 80 }, { type: "pointermove", dy: 10 }, { wait: 300 }];
  for (let i = 1; i <= FOLLOW_PX / 2; i++) steps.push({ type: "pointermove", dy: 10 + 2 * i }, { wait: 17 });
  steps.push({ type: "pointerup", dy: 10 + FOLLOW_PX });
  await dispatchPointer(fader, steps, 71);
  await frames(page, 3);
  const { caps, moves } = await page.evaluate(() => {
    const w = window as any;
    w.__sampling = false;
    return { caps: w.__caps as [number, number][], moves: w.__moves as number[] };
  });

  expect(moves.length, "the finger's moves").toBe(1 + FOLLOW_PX / 2);
  const [first, second] = moves;
  const resting = caps.filter(([at]) => at < first);
  expect(resting.length, "frames before the touch's first move").toBeGreaterThan(0);
  const rest = resting[resting.length - 1][1];
  const anchored = caps.filter(([at]) => at > first && at < second);
  expect(anchored.length, "frames drawn between the first move and the next").toBeGreaterThan(1);
  const jumps = anchored.map(([, top]) => Math.abs(top - rest));
  expect(Math.max(...jumps), `the cap on the first move's frames (px off its rest): ${JSON.stringify(jumps)}`).toBeLessThanOrEqual(3);

  // From the anchor the cap follows the finger: up by about its 30 px (the
  // touch shaping scales the first moves by 0.9 to 1.0; 0.85 near 0 dB).
  const end = caps[caps.length - 1][1];
  const rose = rest - end;
  expect(rose, "the cap rose with the finger after the anchor (px)").toBeGreaterThan(FOLLOW_PX * 0.75);
  expect(rose, "never by the first move's 10 px too").toBeLessThanOrEqual(FOLLOW_PX + 1);
  // And Live follows the fader.
  const shownAfter = await shown(fader);
  expect(shownAfter).toBeGreaterThan(START);
  await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - shownAfter) < 1e-3, "Live at the fader's value");
});
