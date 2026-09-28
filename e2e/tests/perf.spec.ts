import { test, expect } from "./support/fixtures";
import { openSurface } from "./support/live";

// The surface keeps drawing while it loads, subscribes and refreshes (#21):
// a frame gap is a fader that does not follow the finger. The page logs
// every animation frame from its first script on, with the moments the
// surface appears and REFRESH ALL runs, and the test prints the timeline of
// any long gap, so a stall says where it happens.

test.describe("Frames while the surface loads", () => {
  test("no frame gap over 700 ms from the load through the automatic refresh", async ({ page }) => {
    await page.addInitScript(() => {
      const w = window as any;
      w.__frames = [];
      w.__marks = [];
      let refreshes: string | null = null;
      let stage = false;
      const tick = (t: number) => {
        w.__frames.push(t);
        if (!stage && document.querySelector('[data-testid="stage"]')) {
          stage = true;
          w.__marks.push([t, "stage"]);
        }
        const r = document.querySelector('[data-testid="surface"]')?.getAttribute("data-refreshes") ?? null;
        if (r !== refreshes) {
          refreshes = r;
          w.__marks.push([t, `refreshes=${r}`]);
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    await openSurface(page);
    await page.waitForTimeout(3000);
    const r = await page.evaluate(() => {
      const w = window as any;
      const frames: number[] = w.__frames;
      const gaps = frames.slice(1).map((t, i) => [frames[i], t - frames[i]]);
      const long = gaps.filter(([, g]) => g > 100).map(([at, g]) => `${Math.round(at)}+${Math.round(g)}`);
      const max = gaps.reduce((m, [, g]) => Math.max(m, g), 0);
      return { frames: frames.length, max: Math.round(max), long, marks: w.__marks.map(([t, m]: [number, string]) => `${Math.round(t)} ${m}`) };
    });
    console.log(`frames while loading: ${JSON.stringify(r)}`);
    expect(r.frames).toBeGreaterThan(20);
    expect(r.max, `long gaps (start+length ms): ${r.long.join(", ")}; marks: ${r.marks.join(", ")}`).toBeLessThan(700);
  });
});
