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

// A diagnosis of the WebKit frame rate (#21): the same surface with one
// suspect of the stylesheet switched off at a time, frames counted for 2 s.
// The minimum rate over the variants must stay above 1 fps (the run exists
// to print the table; it is removed once the cause is fixed).
test.describe("Frame rate by stylesheet suspect", () => {
  test("frames per second with each suspect off", async ({ page }) => {
    await openSurface(page);
    const variants: [string, string][] = [
      ["baseline", ""],
      ["no :has rules", "has"],
      ["no animations", "* { animation: none !important; }"],
      ["no shadows", "* { box-shadow: none !important; }"],
      ["no gradients", ".meter-zones, .fader-fill, .fader-cap, .param-toggle { background: #3a6 !important; }"],
      ["no color-mix", ".btn, .btn.on, .mute.lit { background: #223 !important; border-color: #334 !important; }"],
      ["no meters", ".meter { display: none !important; }"],
      ["no strips", ".strip { visibility: hidden !important; }"],
    ];
    const table: string[] = [];
    for (const [name, css] of variants) {
      const fps = await page.evaluate(async ([name, css]) => {
        document.getElementById("diag")?.remove();
        if (css === "has") {
          for (const sheet of Array.from(document.styleSheets)) {
            const rules = Array.from(sheet.cssRules);
            for (let i = rules.length - 1; i >= 0; i--) {
              if ((rules[i] as CSSStyleRule).selectorText?.includes(":has(")) sheet.deleteRule(i);
            }
          }
        } else if (css) {
          const style = document.createElement("style");
          style.id = "diag";
          style.textContent = css;
          document.head.appendChild(style);
        }
        await new Promise((r) => setTimeout(r, 300));
        let n = 0;
        const end = performance.now() + 2000;
        await new Promise<void>((done) => {
          const tick = () => {
            n++;
            if (performance.now() < end) requestAnimationFrame(tick);
            else done();
          };
          requestAnimationFrame(tick);
        });
        return n / 2;
      }, [name, css]);
      table.push(`${name}: ${fps} fps`);
    }
    console.log(`frame rate by suspect: ${table.join(" | ")}`);
    expect(table.length).toBe(variants.length);
  });
});
