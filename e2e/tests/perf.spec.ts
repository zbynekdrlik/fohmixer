import { test, expect } from "./support/fixtures";
import { openSurface } from "./support/live";

// The surface keeps drawing while it loads and subscribes (#21; #58 removed
// REFRESH ALL and its resubscription a second after load): a frame gap is a
// fader that does not follow the finger. The page logs every animation frame
// from its first script on, with the moments the surface appears and
// connects, and the test prints the timeline of any long gap, so a stall says
// where it happens.
//
// Three bounds, measured on the CI runner (WebKit draws in software there;
// Chromium stays near 60 fps throughout):
// - the surface's first paint, one frame nobody can touch yet: ~870 ms in
//   WebKit, bounded at 1500 ms so a doubling fails;
// - every gap after it, through the first values: at most ~170 ms,
//   bounded at 500 ms (the TechAlert wash blinks every 500 ms);
// - the steady rate after it with every meter moving: ~22 fps in
//   WebKit, bounded at 10 fps (endless animations, blurred shadows and
//   rounded clips had dropped it to 2.5 fps).

const FIRST_PAINT_MS = 1500;
const GAP_MS = 500;
const STEADY_FPS = 10;

test.describe("Frames while the surface loads", () => {
  test("the surface paints once, then keeps drawing through its first values", async ({ page }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__frames = [];
      w.__marks = [];
      let connected: string | null = null;
      let stage = false;
      const tick = (t: number) => {
        w.__frames.push(t);
        if (!stage && document.querySelector('[data-testid="stage"]')) {
          stage = true;
          w.__marks.push([t, "stage"]);
        }
        const c = document.querySelector('[data-testid="surface"]')?.getAttribute("data-connected") ?? null;
        if (c !== connected) {
          connected = c;
          w.__marks.push([t, `connected=${c}`]);
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await openSurface(page);
    await page.waitForTimeout(3000);
    // The moment of the read counts as the end of the last gap, so a stall
    // at the end of the window is not lost.
    const { frames, marks, end } = await page.evaluate(() => {
      const w = window as any;
      return { frames: w.__frames as number[], marks: w.__marks as [number, string][], end: performance.now() };
    });
    const timeline = marks.map(([t, m]) => `${Math.round(t)} ${m}`).join(", ");
    const at = (name: string) => {
      const mark = marks.find(([, m]) => m === name);
      expect(mark, `no "${name}" mark; marks: ${timeline}`).toBeTruthy();
      return mark![0];
    };
    const stage = frames.indexOf(at("stage"));
    const connectedAt = at("connected=true");
    expect(stage, `the stage frame is not in the frame log; marks: ${timeline}`).toBeGreaterThanOrEqual(0);
    expect(frames.length, `frames after the stage appeared; marks: ${timeline}`).toBeGreaterThan(stage + 1);

    const firstPaint = frames[stage + 1] - frames[stage];
    const after = [...frames.slice(stage + 1), end];
    const gaps = after.slice(1).map((t, i) => [after[i], t - after[i]]);
    const long = gaps.filter(([, g]) => g > 100).map(([t, g]) => `${Math.round(t)}+${Math.round(g)}`);
    const maxGap = gaps.reduce((m, [, g]) => Math.max(m, g), 0);
    const settled = frames.filter((t) => t >= connectedAt + 500);
    const span = settled[settled.length - 1] - settled[0];
    const fps = span > 0 ? ((settled.length - 1) * 1000) / span : 0;
    console.log(
      `frames while loading: first paint ${Math.round(firstPaint)} ms, max gap after it ${Math.round(maxGap)} ms, ` +
        `steady ${fps.toFixed(1)} fps over ${Math.round(span)} ms; long gaps: ${long.join(", ")}; marks: ${timeline}`,
    );

    expect(firstPaint, `the surface's first paint (marks: ${timeline})`).toBeLessThan(FIRST_PAINT_MS);
    expect(maxGap, `long gaps (start+length ms): ${long.join(", ")}; marks: ${timeline}`).toBeLessThan(GAP_MS);
    expect(span, "the steady window after it").toBeGreaterThan(1500);
    expect(fps, `frames per second after it (long gaps: ${long.join(", ")})`).toBeGreaterThan(STEADY_FPS);
  });
});
