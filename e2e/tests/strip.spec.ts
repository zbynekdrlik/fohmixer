import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  clipped,
  dbForm,
  doubleTap,
  frames,
  hostLine,
  hubEvents,
  harness,
  openDetail,
  openSurface,
  panning,
  ready,
  seen,
  selectPage,
  shown,
  strip,
  groupLabelOf,
  token,
  track,
  until,
  volume,
} from "./support/live";

// A strip (spec F2–F5, F8–F13, F20, F22, I8): Live's values, the fader and
// mute writing Live (the pan in the strip's channel detail, #71), the meter,
// the status pill, and the controls waiting for Live's value before they take
// input.

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

const HAND2 = track("Hand2 #");
/**
 * Each meter bar as drawn (#151: no per-bar attribute is written each
 * frame): its level, 1 − its cover's `scaleY`, and its peak line's `--pk`.
 */
async function drawn(bars: import("@playwright/test").Locator): Promise<{ level: number; peak: number }[]> {
  return bars.evaluateAll((all) =>
    all.map((b) => {
      const cover = b.querySelector(".meter-cover") as HTMLElement;
      const scale = /scaleY\(([-\d.]+)\)/.exec(cover.style.transform);
      const peak = (b.querySelector(".meter-peak") as HTMLElement).style.getPropertyValue("--pk");
      return { level: scale ? 1 - Number(scale[1]) : 0, peak: Number(peak || 0) };
    }),
  );
}

/** A strip's calm name button (#151, mockup v8: `#15151f`). */
const CALM = "rgb(21, 21, 31)";

test.describe("A strip", () => {
  test("shows Live's volume in TouchOSC's form, white exactly at 0 dB", async ({ page }) => {
    // #21 (parity audit #13): Live's own value (X1), one decimal, no unit.
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const db = strip(page, "Hand2 #").getByTestId("db");
    expect(await live.display("band", volume(HAND2), 0.85)).toBe("0.00 dB");
    await expect(db).toHaveText("0.0");
    await expect(db).toHaveClass(/\bunity\b/);
    await expect(db).toHaveCSS("color", "rgb(255, 255, 255)");
    await live.set("band", volume(HAND2), "value", 0.7);
    await expect(db).toHaveText(dbForm(await live.display("band", volume(HAND2), 0.7)));
    await expect(db).not.toHaveClass(/\bunity\b/);
    await expect(db).toHaveCSS("color", "rgb(155, 231, 168)");
    await live.set("band", volume(HAND2), "value", 0.0);
    await expect(db).toHaveText("−∞");
    await live.set("band", volume(HAND2), "value", 0.85);
    await expect(strip(page, "Hand2 #").getByTestId("strip-label")).toHaveText("Hand2");
    await expect(strip(page, "B-Main repro #").getByTestId("strip-label")).toHaveText("Main");
    // #63, #127: a group whose strips share an instance names it once, on
    // the side of its run's first strip (Hand2's group is untitled: the name
    // is the set's), not on each strip; #151: no RET on any strip.
    const named = await groupLabelOf(page, strip(page, "Hand2 #"));
    await expect(named).toHaveText("BAND");
    await expect(named).toHaveAttribute("data-instance", "band");
    await expect(strip(page, "Hand2 #").getByTestId("strip-instance")).toHaveCount(0);
    await expect(strip(page, "B-Main repro #").getByTestId("strip-instance")).toHaveCount(0);
    await expect(page.locator('[data-testid="strip"] .strip-ret')).toHaveCount(0);
    for (const s of await page.getByTestId("strip").all()) {
      expect(await s.textContent(), `${await s.getAttribute("data-track")}: no RET`).not.toMatch(/\bRET\b/i);
    }
    expect(await strip(page, "B-Main repro #").textContent()).not.toMatch(/\bRET\b/i);
  });

  test("its texts fit their boxes: the dB text, the name and the group's name", async ({ page }) => {
    // #9, findings 3-4: a narrow strip's 39 x 25 instance label showed "BANC"
    // for BAND, and the dB text must show Live's whole string.
    const VOCAL1 = track("Vocal 1 repro#");
    const before = await live.get("band", volume(VOCAL1), "value");
    try {
      await live.set("band", volume(VOCAL1), "value", 0.829725);
      await openSurface(page);
      const narrow = strip(page, "Vocal 1 repro#");
      await expect(narrow.getByTestId("db")).toHaveText(dbForm(await live.display("band", volume(VOCAL1), 0.829725)));
      // #127: the group's name on the strip's side (#151: in its info column).
      const named = narrow.getByTestId("group-side");
      await expect(named).toHaveText(/stage/i);
      await expect(named).toHaveAttribute("data-instance", "band");
      expect(
        await named.evaluate((e) => [e.scrollWidth - e.clientWidth, e.scrollHeight - e.clientHeight]),
        "the group's name shown whole",
      ).toEqual([0, 0]);
      for (const name of ["Vocal 1 repro#", "Hand2 #", "B-Main repro #"]) {
        for (const part of ["db", "strip-label"]) {
          const el = strip(page, name).getByTestId(part);
          await expect(el).not.toHaveText("");
          expect(await clipped(el), `${name} ${part}`).toEqual([]);
        }
      }
    } finally {
      await live.set("band", volume(VOCAL1), "value", before);
    }
  });

  test("at the narrowest strip width a long name fits, every part stays in its strip, no line scrolls up or down", async ({ page }) => {
    // #21 second review: without a clip, a long name widened the strip's
    // column over the next strip (a 9-letter name at 64 px: 17 px in
    // WebKit), the fader's rail gave a scrolling row a vertical scroll
    // range, and a name longer than about 6 letters ended in an ellipsis.
    // #63: the page's one row of 19 strips does not fit, so the line shows a
    // window of as many as fit; at 1236 px a side (540 px) holds 8 strips
    // at exactly 64 px.
    const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
    const BAND = ["TechAlert #", "Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #", "Vocals Repro grp#", "Vocal 1 repro#", "Vocal 2 repro#", "Vocal 3 repro#", "Keys 1", "Stems grp#", "Drums #", "Bass #", "Mics Stage #"];
    const MASTER = ["Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #"];
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    const foh = changed.pages.find((p: any) => p.id === "foh");
    const strips = [...BAND.map((name) => ["band", name]), ...MASTER.map((name) => ["master", name])].map(([instance, name]) => ({
      kind: "strip",
      binding: { instance, anchor: { kind: "track", name } },
      strip_kind: "standard",
    }));
    // A master return first: its instance tag says the instance alone (#151:
    // no RET on a strip).
    strips.unshift({ kind: "strip", binding: { instance: "master", anchor: { kind: "return", name: "A-Reverb #" } }, strip_kind: "return" });
    foh.rows = [{ sections: [{ kind: "group", id: "narrow", title: "Narrow", controls: strips }] }];
    await page.setViewportSize({ width: 1236, height: 720 });
    await openSurface(page);
    const group = page.locator('[data-testid="group"][data-group="narrow"]');
    const cells = page.locator('.slot[data-group="narrow"]');
    try {
      await harness("/hub/layout", { layout: changed });
      await expect(cells.locator('[data-testid="strip"]')).toHaveCount(16, { timeout: 10_000 });
      const alert = cells.locator('[data-testid="strip"][data-track="TechAlert #"]');
      await expect(alert.getByTestId("strip-label")).toHaveText("TechAlert");
      await frames(page);
      const width = await page.getByTestId("page").evaluate((el) => parseFloat(getComputedStyle(el).getPropertyValue("--strip-w")));
      expect(width, "the premise: the strips at their narrowest").toBe(64);
      // Every part of every strip on the page inside its strip (the meter's
      // moving layers are clipped by their bar, the cap's rail is a
      // transparent layer that takes no touch: the bar and the cap count).
      const outside = await page.locator('[data-testid="strip"]').evaluateAll((all) =>
        all.flatMap((node) => {
          const s = node.getBoundingClientRect();
          const name = node.getAttribute("data-track");
          return Array.from(node.querySelectorAll("*"))
            .filter((el) => !el.closest(".meter-bar") || el.classList.contains("meter-bar"))
            .filter((el) => !el.classList.contains("fader-rail"))
            .flatMap((el) => {
              const r = el.getBoundingClientRect();
              if (r.width === 0 && r.height === 0) return [];
              const slack = 0.5;
              const out = r.left < s.left - slack || r.right > s.right + slack || r.top < s.top - slack || r.bottom > s.bottom + slack;
              return out ? [`${name} .${el.className}: (${r.left.toFixed(1)}..${r.right.toFixed(1)}, ${r.top.toFixed(1)}..${r.bottom.toFixed(1)}) outside (${s.left.toFixed(1)}..${s.right.toFixed(1)}, ${s.top.toFixed(1)}..${s.bottom.toFixed(1)})`] : [];
            });
        }),
      );
      expect(outside).toEqual([]);
      await expect(cells.locator('[data-testid="strip"][data-track="A-Reverb #"]').getByTestId("strip-instance")).toHaveText(/^master$/i);
      for (const part of ["strip-label", "strip-instance"]) {
        for (const text of await cells.getByTestId(part).all()) {
          expect(await clipped(text), `${part} ${await text.textContent()}`).toEqual([]);
        }
      }
      const scrolls = await page.getByTestId("line").evaluateAll((lines) =>
        lines.filter((r) => r.scrollHeight > r.clientHeight).map((r) => `${r.scrollHeight} > ${r.clientHeight}`),
      );
      expect(scrolls, "lines with a vertical scroll range").toEqual([]);
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(group).toHaveCount(0, { timeout: 10_000 });
  });

  test("a fader drag moves Live's volume the way of the finger", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.5);
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    const { x, y } = await centre(fader);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(x, y - 8 * i);
    await page.mouse.up();
    const up = await until(() => live.get("band", volume(HAND2), "value"), (v) => v > 0.55, "the volume to rise");
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(x, y + 8 * i);
    await page.mouse.up();
    await until(() => live.get("band", volume(HAND2), "value"), (v) => v < up - 0.05, "the volume to fall");
  });

  test("a drag that starts on the scale or the meter moves the fader", async ({ page }) => {
    // #21 review: the fader takes touches over the whole fader zone, so a
    // narrow strip's fader is as easy to catch as a wide one's.
    await live.set("band", volume(HAND2), "value", 0.5);
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const fader = s.getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    const scale = (await s.getByTestId("scale").boundingBox())!;
    const zone = await centre(fader);
    const x = Math.round(scale.x + scale.width / 2);
    await page.mouse.move(x, zone.y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(x, zone.y - 8 * i);
    await page.mouse.up();
    await until(() => live.get("band", volume(HAND2), "value"), (v) => v > 0.55, "the volume to rise");
  });

  test("a drag that starts on the cap where it stands out of the fader moves it", async ({ page }) => {
    // #21 second review: the cap's rail takes no touch, and the cap had
    // inherited that, so its half below the track at −∞ took nothing.
    await live.set("band", volume(HAND2), "value", 0);
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const fader = s.getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => v === 0, "the fader at −∞");
    await frames(page);
    const track = (await fader.boundingBox())!;
    const cap = (await s.locator(".fader-cap").boundingBox())!;
    const x = Math.round(cap.x + cap.width / 2);
    const y = Math.round(cap.y + cap.height - 3);
    expect(y, "the point is below the fader's box").toBeGreaterThan(track.y + track.height);
    const hit = await page.evaluate(([px, py]) => document.elementFromPoint(px, py)?.closest('[data-testid="fader"]') !== null, [x, y]);
    expect(hit, "the cap's lower edge belongs to the fader").toBe(true);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(x, y - 8 * i);
    await page.mouse.up();
    await until(() => live.get("band", volume(HAND2), "value"), (v) => v > 0.05, "the volume to rise");
  });

  test("a double tap glides the fader to 0 dB", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.5);
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    await doubleTap(fader);
    await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - 0.85) < 1e-6, "0 dB", 6000);
    await expect(strip(page, "Hand2 #").getByTestId("db")).toHaveText("0.0");
  });

  test("its detail's pan drag moves Live's panning and a double tap centres it", async ({ page }) => {
    await live.set("band", panning(HAND2), "value", 0.0);
    await openSurface(page);
    // The pan is the channel detail's (#71).
    const pan = (await openDetail(page, "Hand2 #")).getByTestId("pan");
    await ready(pan);
    const { x, y } = await centre(pan);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 5; i++) await page.mouse.move(x + 6 * i, y);
    await page.mouse.up();
    await until(() => live.get("band", panning(HAND2), "value"), (v) => v > 0.05, "panning right");
    await expect(pan.locator(".pan-dot")).toHaveCSS("background-color", "rgb(52, 193, 220)");
    await page.waitForTimeout(400);
    await doubleTap(pan);
    await until(() => live.get("band", panning(HAND2), "value"), (v) => v === 0, "centred");
    await expect(pan.locator(".pan-dot")).toHaveCSS("background-color", "rgb(100, 100, 100)");
  });

  test("a mute tap toggles Live's mute; the button is lit while the track is audible", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    await openSurface(page);
    const mute = strip(page, "Hand2 #").getByTestId("mute");
    await ready(mute);
    await expect(mute).toHaveAttribute("data-muted", "false");
    await expect(mute).toHaveClass(/\blit\b/);
    await mute.click();
    await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted");
    await expect(mute).toHaveAttribute("data-muted", "true");
    await expect(mute).not.toHaveClass(/\blit\b/);
    await mute.click();
    await until(() => live.get("band", HAND2, "mute"), (v) => v === false, "audible again");
  });

  test("a guarded mute needs a second tap within 500 ms", async ({ page }) => {
    const hand1 = track("Hand1 #");
    await live.set("master", hand1, "mute", false);
    await openSurface(page);
    await selectPage(page, "others");
    const mute = strip(page, "Hand1 #", "master").getByTestId("mute");
    await ready(mute);
    await mute.click();
    await expect(mute).toHaveClass(/\barmed\b/);
    // #151 (PR #153's review M5): the armed outline is drawn inside the name
    // button, the strip's top edge, so no line's top can cut it.
    const reach = await mute.evaluate((el) => {
      const cs = getComputedStyle(el);
      return { style: cs.outlineStyle, reach: parseFloat(cs.outlineOffset) + parseFloat(cs.outlineWidth) };
    });
    expect(reach.style).not.toBe("none");
    expect(reach.reach, "the armed outline inside the button").toBeLessThanOrEqual(0.01);
    await page.waitForTimeout(700);
    expect(await live.get("master", hand1, "mute")).toBe(false);
    await expect(mute).not.toHaveClass(/\barmed\b/);
    await doubleTap(mute, 150);
    await until(() => live.get("master", hand1, "mute"), (v) => v === true, "muted after the confirming tap");
    // The band's track of the same name is another instance's (spec F2, X4).
    expect(await live.get("band", hand1, "mute")).toBe(false);
    await live.set("master", hand1, "mute", false);
  });

  test("the name button is calm, the track's colour only as its 3 px edge, and follows Live's colour", async ({ page }) => {
    // #151, mockup v8: one colour per strip, the Live track's own, as a thin
    // edge at the name button's left (SimLive's Hand2 #: 0xFF3636); the
    // button itself is the strip's dark ground with the name in light.
    await live.set("band", HAND2, "mute", false);
    const before = await live.get("band", HAND2, "color");
    try {
      await openSurface(page);
      const mute = strip(page, "Hand2 #").getByTestId("mute");
      await ready(mute);
      const edge = (el: Element) => {
        const after = getComputedStyle(el, "::after");
        return { color: after.backgroundColor, width: after.width };
      };
      await expect(mute).toHaveCSS("background-color", CALM);
      await expect(mute).toHaveCSS("color", "rgb(234, 234, 240)");
      await expect.poll(() => mute.evaluate(edge)).toEqual({ color: "rgb(255, 54, 54)", width: "3px" });
      // Another strip of another colour: the same button, its own edge.
      const other = strip(page, "B-Main repro #").getByTestId("mute");
      await ready(other);
      await expect(other).toHaveCSS("background-color", CALM);
      expect((await other.evaluate(edge)).color).not.toBe("rgb(255, 54, 54)");
      // A colour change in Live changes the edge only.
      await live.set("band", HAND2, "color", 0x1e3a8a);
      await expect.poll(() => mute.evaluate(edge)).toEqual({ color: "rgb(30, 58, 138)", width: "3px" });
      await expect(mute).toHaveCSS("background-color", CALM);
      await expect(mute).toHaveCSS("color", "rgb(234, 234, 240)");
      // No group colour on a strip: no top edge.
      expect(await strip(page, "Hand2 #").evaluate((el) => getComputedStyle(el, "::before").borderTopWidth)).toBe("0px");
    } finally {
      await live.set("band", HAND2, "color", before);
    }
  });

  test("a muted strip recedes: darker, its name of lower contrast, MUTED under the name, its cap, meter and readout dimmed", async ({ page }) => {
    // #151, mockup v6-v8: what is switched off is never the most striking
    // thing on the mix.
    await live.set("band", HAND2, "mute", false);
    try {
      await openSurface(page);
      const s = strip(page, "Hand2 #");
      const mute = s.getByTestId("mute");
      await ready(mute);
      await mute.click();
      await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted");
      await expect(mute).toHaveAttribute("data-muted", "true");
      await expect(mute).toHaveCSS("background-color", "rgb(16, 16, 24)");
      const looks = await page.getByTestId("mute").evaluateAll((all) => {
        const lum = (c: string) => {
          const m = (c.match(/[\d.]+/g) ?? []).map(Number);
          const lin = (v: number) => {
            v /= 255;
            return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
          };
          return 0.2126 * lin(m[0]) + 0.7152 * lin(m[1]) + 0.0722 * lin(m[2]);
        };
        const contrast = (a: string, b: string) => {
          const x = lum(a);
          const y = lum(b);
          return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
        };
        return all
          // The strips' buttons with Live's values (an unresolved one is red, I5).
          .filter((b) => b.closest('[data-testid="strip"]') && b.getAttribute("data-binding") !== "unresolved")
          .map((b) => {
            const cs = getComputedStyle(b);
            const label = b.querySelector(".strip-label")!;
            const ls = getComputedStyle(label);
            return { muted: b.getAttribute("data-muted") === "true", bg: lum(cs.backgroundColor), c: contrast(ls.color, cs.backgroundColor), deco: ls.textDecorationLine };
          });
      });
      const on = looks.filter((l) => !l.muted);
      const off = looks.filter((l) => l.muted);
      expect(on.length, "strips that play").toBeGreaterThan(0);
      expect(off.length, "muted strips").toBeGreaterThan(0);
      for (const m of off) {
        expect(m.deco).toBe("none");
        for (const u of on) {
          expect(m.bg, "darker than every strip that plays").toBeLessThan(u.bg);
          expect(m.c, "a name of lower contrast").toBeLessThan(u.c);
        }
      }
      // MUTED under the name, quiet red.
      const word = s.getByTestId("mute-mark");
      expect(await seen(word)).toBe(true);
      await expect(word).toHaveText("MUTED");
      await expect(word).toHaveCSS("color", "rgb(168, 71, 79)");
      const name = (await s.getByTestId("strip-label").boundingBox())!;
      expect((await word.boundingBox())!.y).toBeGreaterThanOrEqual(name.y + name.height - 0.5);
      for (const part of [s.getByTestId("fader-cap"), s.getByTestId("meter"), s.getByTestId("strip-readout"), s.getByTestId("strip-col")]) {
        await expect(part).toHaveCSS("opacity", "0.4");
      }
      // A strip that plays has no mute word.
      expect(await seen(strip(page, "B-Main repro #").getByTestId("mute-mark"))).toBe(false);
    } finally {
      await live.set("band", HAND2, "mute", false);
    }
  });

  test("the dB scale sits right of the track, its 0 at the fader's 0 dB and the unity mark", async ({ page }) => {
    // #21 (parity audit #12): TouchOSC's labels at the fader positions of
    // their levels; #151 (mockup v8, the LV1 way): 17 px at the strip's
    // right, 2 px right of the fader's 12 px track, labels left-aligned.
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const scale = s.getByTestId("scale");
    const ticks = scale.locator(".tick");
    // #63: Live's own fader law by default, its low end spread out as a
    // console's, so −30 and −60 are labelled too.
    await expect(ticks).toHaveText(["+6", "0", "−6", "−12", "−18", "−24", "−30", "−40", "−60"]);
    const fader = s.getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.85) < 0.001, "the fader at 0 dB");
    await frames(page);
    const box = (await scale.boundingBox())!;
    const track = (await s.getByTestId("meter").boundingBox())!;
    expect(Math.abs(track.width - 12), `the track ${track.width} px wide`).toBeLessThan(0.5);
    expect(Math.abs(box.width - 17), `the scale ${box.width} px wide`).toBeLessThan(0.5);
    expect(Math.abs(box.x - (track.x + track.width) - 2), "the scale 2 px right of the track").toBeLessThan(0.5);
    const left = await ticks.evaluateAll((all) => all.map((t) => t.getBoundingClientRect().left));
    for (const x of left) expect(Math.abs(x - box.x), "every label at the scale's left").toBeLessThan(0.5);
    const cap = (await s.getByTestId("fader-cap").boundingBox())!;
    const zero = (await ticks.nth(1).boundingBox())!;
    expect(Math.abs(cap.y + cap.height / 2 - (zero.y + zero.height / 2))).toBeLessThan(2);
    // The unity mark (the fader's ::after, 1 px high, its top at the 0 dB
    // position) under the 0 label's centre, which translateY(-50%) puts at
    // that same position: compare the mark's top edge.
    const unity = await fader.evaluate((el) => el.getBoundingClientRect().top + parseFloat(getComputedStyle(el, "::after").top));
    expect(Math.abs(unity - (zero.y + zero.height / 2)), "the 0 label on the unity mark").toBeLessThan(0.5);
    // +6 dB is the top of the travel.
    const travel = (await fader.boundingBox())!;
    const top = (await ticks.nth(0).boundingBox())!;
    expect(Math.abs(top.y + top.height / 2 - travel.y)).toBeLessThan(2);
    // The position is Live's volume: 0 dB at 85 % of the travel.
    const down = (cap.y + cap.height / 2 - travel.y) / travel.height;
    expect(Math.abs(down - 0.15), `0 dB ${down} of the travel from its top`).toBeLessThan(0.01);
  });

  test("the strip's meter is two bars in the fader's track, under the cap, with no clip light", async ({ page }) => {
    // #151 (mockup v8): the stereo pair in the fader's track replaces the
    // meter beside the scale and the fader's fill; no clip light anywhere
    // (Live mixes in 32-bit float). The fixture's source is `level`: both
    // bars read the one value.
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const track = s.getByTestId("meter");
    const bars = track.getByTestId("meter-bar");
    await expect(bars).toHaveCount(2);
    try {
      expect(await hostLine("band", 'meter "Hand2 #" 0.6')).toBe("METER 1");
      const pair = async () => (await drawn(bars)).map((b) => b.level);
      const both = await until(pair, (v) => v[0] > 0.3 && Math.abs(v[0] - v[1]) < 1e-4, "both bars at the held level");
      expect(both[1]).toBeCloseTo(both[0], 4);
      const fader = s.getByTestId("fader");
      await ready(fader);
      await until(() => shown(fader), (v) => Math.abs(v - 0.85) < 0.001, "the fader at 0 dB");
      await frames(page);
      const t = (await track.boundingBox())!;
      const cap = (await s.getByTestId("fader-cap").boundingBox())!;
      expect(Math.abs(t.width - 12)).toBeLessThan(0.5);
      expect(Math.abs(t.x + t.width / 2 - (cap.x + cap.width / 2)), "the track centred on the cap").toBeLessThan(0.5);
      const hit = await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.getAttribute("data-testid"), [cap.x + cap.width / 2, cap.y + cap.height / 2]);
      expect(hit, "the cap over the bars").toBe("fader-cap");
      await expect(s.locator(".fader-fill")).toHaveCount(0);
      await expect(s.locator(".fader-groove")).toHaveCount(0);
      // At 0 dB of Live's meter: no clip light, no CLIP or PEAK text, on the
      // strip or in its detail.
      expect(await hostLine("band", 'meter "Hand2 #" 1.0')).toBe("METER 1");
      await until(pair, (v) => v[0] > 0.9, "the bars at the top");
      // The peak line holds a while after the level falls (each bar's own).
      expect(await hostLine("band", 'meter "Hand2 #" 0.3')).toBe("METER 1");
      await until(pair, (v) => v[0] < 0.5 && v[1] < 0.5, "the bars down");
      const peaks = (await drawn(bars)).map((b) => b.peak);
      for (const p of peaks) expect(p, "the peak held").toBeGreaterThan(0.9);
      await expect(page.getByTestId("clip")).toHaveCount(0);
      expect(await s.textContent()).not.toMatch(/CLIP|PEAK/);
      const detail = await openDetail(page, "Hand2 #");
      await expect(detail.getByTestId("meter")).toHaveCount(1);
      await expect(page.getByTestId("clip")).toHaveCount(0);
      await expect(page.locator(".meter-clip")).toHaveCount(0);
    } finally {
      await hostLine("band", 'meter "Hand2 #" off');
    }
  });

  test("the info column holds the empty LUFS slot and the group's name, clear of the cap", async ({ page }) => {
    // #151 (mockup v8): left of the cap's widest reach, the LUFS row (empty
    // and hidden until the LUFS source) and the group's name.
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const lufs = page.locator('[data-testid="strip"] [data-testid="lufs"]');
    expect(await lufs.count()).toBe(await page.getByTestId("strip").count());
    for (const l of await lufs.all()) {
      await expect(l).toHaveText("");
      await expect(l).toHaveCSS("visibility", "hidden");
    }
    const s = strip(page, "Hand2 #");
    expect((await s.getByTestId("lufs").boundingBox())!.height).toBeCloseTo(16, 1);
    const fader = s.getByTestId("fader");
    await ready(fader);
    const col = s.getByTestId("strip-col");
    const { x, y } = await centre(fader);
    try {
      // The cap at the top, then at the bottom: the column stays 3 px clear.
      for (const dy of [-400, 800]) {
        await page.mouse.move(x, y);
        await page.mouse.down();
        for (let i = 1; i <= 10; i++) await page.mouse.move(x, y + (dy * i) / 10);
        await page.mouse.up();
        await frames(page);
        const c = (await col.boundingBox())!;
        const cap = (await s.getByTestId("fader-cap").boundingBox())!;
        expect(cap.x - (c.x + c.width), "the column 3 px clear of the cap").toBeGreaterThanOrEqual(3 - 0.01);
      }
    } finally {
      await live.set("band", volume(HAND2), "value", 0.85);
    }
  });

  test("a layout switching the meter source rebuilds the bars: lr one channel each, level the one value on both", async ({ page }) => {
    // #151 (Review Focus 4): SimLive's moving meters give L and R apart.
    const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
    await openSurface(page);
    const bars = () => strip(page, "Hand2 #").getByTestId("meter").getByTestId("meter-bar");
    const pair = async () => (await drawn(bars())).map((b) => b.level);
    await expect(bars()).toHaveCount(2);
    const lr = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    lr.config.meter_source = "lr";
    const level = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    level.config.meter_source = "level";
    try {
      await harness("/hub/layout", { layout: lr });
      await until(pair, (v) => v.length === 2 && Math.abs(v[0] - v[1]) > 0.01, "the bars apart (L and R)", 10_000);
      await harness("/hub/layout", { layout: level });
      // Equal on every frame from the rebuild on.
      await until(pair, (v) => v.length === 2 && v[0] > 0 && Math.abs(v[0] - v[1]) < 1e-4, "the bars together (one value)", 10_000);
      for (let i = 0; i < 5; i++) {
        await frames(page);
        const v = await pair();
        expect(Math.abs(v[0] - v[1]), `frame ${i}: ${v}`).toBeLessThan(1e-4);
      }
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(bars()).toHaveCount(2, { timeout: 10_000 });
  });

  test("the fader's travel on the iPad is at least 322 px", async ({ page }) => {
    // #151: the v8 head (37 + 2 + 20 px) costs the fader nothing (the FOH
    // fixture at the iPad's 1194 × 834; a tagged strip loses the tag's
    // ~16 px, as before).
    await page.setViewportSize({ width: 1194, height: 834 });
    await openSurface(page);
    await expect(page.getByTestId("strip").first()).toBeVisible();
    await frames(page);
    const travels = await page.getByTestId("strip").evaluateAll((all) =>
      all
        .filter((s) => !s.querySelector('[data-testid="strip-instance"]') && s.querySelector('[data-testid="fader"]'))
        .map((s) => ({ name: s.getAttribute("data-track"), h: s.querySelector('[data-testid="fader"]')!.getBoundingClientRect().height })),
    );
    expect(travels.length).toBeGreaterThan(0);
    console.log(`v8 travel (${test.info().project.name}): ${[...new Set(travels.map((t) => t.h.toFixed(2)))].join(", ")} px`);
    // 322 px within the rows' sub-pixel rounding (reverb.spec.ts pins it within 1 px).
    for (const t of travels) expect(t.h, `${t.name}`).toBeGreaterThanOrEqual(322 - 0.5);
  });

  test("the status pill is red when unbound and not red when bound", async ({ page }) => {
    await openSurface(page);
    const bound = strip(page, "Hand2 #").getByTestId("status");
    await expect(bound).toHaveAttribute("data-state", "bound");
    await expect(bound).not.toHaveCSS("background-color", "rgb(255, 0, 0)");
    // Two tracks are called "Keys 1": ambiguous, never guessed (spec I5).
    const keys = strip(page, "Keys 1");
    await expect(keys.getByTestId("status")).toHaveAttribute("data-state", "unbound");
    await expect(keys.getByTestId("status")).toHaveCSS("background-color", "rgb(255, 0, 0)");
    await expect(keys.getByTestId("fader")).toHaveAttribute("aria-disabled", "true");
    await expect(keys.getByTestId("mute")).toHaveAttribute("aria-disabled", "true");
  });

  test("the master's strip binds to the master instance", async ({ page }) => {
    await live.set("master", volume(track("Hand1 #")), "value", 0.6);
    await live.set("band", volume(track("Hand1 #")), "value", 0.85);
    await openSurface(page);
    await selectPage(page, "others");
    const master = strip(page, "Hand1 #", "master");
    // Its group names its set (#127: untitled, so the name is the set's).
    const named = await groupLabelOf(page, master);
    await expect(named).toHaveText("MASTER");
    await expect(named).toHaveAttribute("data-instance", "master");
    await expect(master.getByTestId("strip-instance")).toHaveCount(0);
    await expect(master.getByTestId("db")).toHaveText(dbForm(await live.display("master", volume(track("Hand1 #")), 0.6)));
  });
});

// SEL never takes the fader's touch (#151, Review Focus 3, the main
// session's ruling on #151): with the cap at −∞ the key's touch box ends
// 3 px left of the cap, a finger down on SEL and dragged writes no volume,
// and a finger down on the cap's centre grabs the fader. On the iPad and on
// both phones. The key is 44 px high on a narrow strip, 36 px else, by the
// strip's own measured width (arrange::is_narrow: under 78 px; a wide strip
// is measured at its own 1.1x), never by the viewport: upright, a phone's
// strips are stretched to about 79 px, not narrow.
for (const size of [
  { width: 1194, height: 834 },
  { width: 844, height: 390 },
  { width: 390, height: 844 },
]) {
  test.describe(`SEL and the cap at −∞, ${size.width} × ${size.height}`, () => {
    test.use({ viewport: { width: size.width, height: size.height } });

    test("SEL ends 3 px left of the cap; a drag from SEL writes no volume, a drag from the cap moves the fader", async ({ page }) => {
      await openSurface(page);
      await expect(page.getByTestId("strip").first()).toBeVisible();
      await frames(page);
      // A bound, unmarked track strip whole on the screen.
      const pick = await page.getByTestId("strip").evaluateAll((all) => {
        for (const s of all) {
          const r = s.getBoundingClientRect();
          const fader = s.querySelector('[data-testid="fader"]');
          const on = r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight;
          if (on && fader && fader.getAttribute("aria-disabled") === "false" && !s.hasAttribute("data-mark") && s.getAttribute("data-kind") === "standard") {
            return { track: s.getAttribute("data-track")!, instance: s.getAttribute("data-instance")!, keys: fader.getAttribute("data-keys")! };
          }
        }
        return null;
      });
      expect(pick, "a strip whole on the screen").not.toBeNull();
      const key: string = JSON.parse(pick!.keys)[0];
      const [instance, target, prop] = key.split("|");
      const s = strip(page, pick!.track, pick!.instance);
      const fader = s.getByTestId("fader");
      const before = await live.get(instance, target, prop);
      try {
        await live.set(instance, target, prop, 0);
        await until(() => shown(fader), (v) => v === 0, "the fader at −∞");
        await frames(page);
        const sel = (await s.getByTestId("detail-key").boundingBox())!;
        const cap = (await s.getByTestId("fader-cap").boundingBox())!;
        const own = (await s.boundingBox())!.width;
        const high = own < 78 ? 44 : 36;
        expect(Math.abs(sel.height - high), `SEL ${sel.width} × ${sel.height} on a ${own} px strip`).toBeLessThan(0.5);
        expect(sel.width).toBeGreaterThanOrEqual(15);
        expect(cap.x - (sel.x + sel.width), "SEL 3 px left of the cap at −∞").toBeGreaterThanOrEqual(3 - 0.01);
        // A finger down on SEL, dragged up over the fader, then lifted:
        // no volume write, Live still at −∞.
        const since = Date.now();
        const k = { x: sel.x + sel.width / 2, y: sel.y + sel.height / 2 };
        await page.mouse.move(k.x, k.y);
        await page.mouse.down();
        for (let i = 1; i <= 8; i++) await page.mouse.move(k.x, k.y - 10 * i);
        await page.mouse.up();
        await page.waitForTimeout(500);
        expect(await live.get(instance, target, prop), "Live untouched by SEL").toBe(0);
        const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since && r.key === key);
        expect(sets, "no volume write from SEL").toEqual([]);
        await expect(page.getByTestId("detail")).toHaveCount(0);
        // A finger down on the cap's centre grabs the fader.
        const c = { x: cap.x + cap.width / 2, y: cap.y + cap.height / 2 };
        const hit = await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.getAttribute("data-testid"), [c.x, c.y]);
        expect(hit, "the cap's centre is the cap's").toBe("fader-cap");
        await page.mouse.move(c.x, c.y);
        await page.mouse.down();
        for (let i = 1; i <= 10; i++) await page.mouse.move(c.x, c.y - 8 * i);
        await page.mouse.up();
        await until(() => live.get(instance, target, prop), (v) => v > 0.05, "the volume to rise from the cap");
      } finally {
        await live.set(instance, target, prop, before);
      }
    });
  });
}

test.describe("Controls wait for Live's value (I8)", () => {
  test("right after load the controls are disabled until their first value and take no input", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    const level = await live.get("band", volume(HAND2), "value");
    // The band host's main thread stops: its values are held back.
    await hostLine("band", "stall 4000");
    await openSurfaceDuringStall(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    const mute = strip(page, "Hand2 #").getByTestId("mute");
    await expect(fader).toHaveAttribute("aria-disabled", "true");
    await expect(mute).toHaveAttribute("aria-disabled", "true");
    // A tap and a drag on the disabled controls write nothing (a write would
    // wait in the stalled host and land after the stall).
    const m = await centre(mute);
    await page.mouse.click(m.x, m.y);
    const f = await centre(fader);
    await page.mouse.move(f.x, f.y);
    await page.mouse.down();
    for (let i = 1; i <= 5; i++) await page.mouse.move(f.x, f.y - 10 * i);
    await page.mouse.up();
    expect(await fader.getAttribute("aria-disabled"), "still disabled after the input").toBe("true");
    await expect(fader).toHaveAttribute("aria-disabled", "false", { timeout: 10_000 });
    await ready(mute);
    await page.waitForTimeout(300);
    expect(await live.get("band", HAND2, "mute")).toBe(false);
    expect(await live.get("band", volume(HAND2), "value")).toBe(level);
  });

  test("a host restart disables the controls until Live is back with its values", async ({ page }) => {
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    const restart = harness("/host/band/restart");
    await expect(fader).toHaveAttribute("aria-disabled", "true");
    await restart;
    await expect(fader).toHaveAttribute("aria-disabled", "false", { timeout: 10_000 });
    // The restarted host has its fixture's values again (Hand2 # at 0.8).
    await expect(strip(page, "Hand2 #").getByTestId("db")).toHaveText(dbForm(await live.display("band", volume(HAND2), 0.8)));
    await until(() => shown(fader), (v) => Math.abs(v - 0.8) < 0.001, "the fader at Live's value");
  });

  test("a stalled Live shows busy on its badge", async ({ page }) => {
    await openSurface(page);
    const badge = page.locator('[data-testid="badge"][data-instance="band"]');
    await expect(badge).toHaveAttribute("data-state", "online");
    await hostLine("band", "stall 1500");
    await expect(badge).toHaveAttribute("data-state", "busy", { timeout: 1200 });
    await expect(badge).toHaveAttribute("data-state", "online", { timeout: 5000 });
    await expect(page.locator('[data-testid="badge"][data-instance="master"]')).toHaveAttribute("data-state", "online");
  });
});

test.describe("Many clients (F20)", () => {
  test("a change on one tablet shows on another", async ({ page, context }) => {
    await live.set("band", HAND2, "mute", false);
    await openSurface(page);
    const other = await context.newPage();
    await openSurface(other);
    const here = strip(page, "Hand2 #").getByTestId("mute");
    const there = strip(other, "Hand2 #").getByTestId("mute");
    await ready(here);
    await ready(there);
    await here.click();
    await expect(there).toHaveAttribute("data-muted", "true");
    await there.click();
    await expect(here).toHaveAttribute("data-muted", "false");
    await other.close();
  });
});

/** Opens the surface while the band host is stalled (its values held back). */
async function openSurfaceDuringStall(page: import("@playwright/test").Page) {
  const seeded = await token();
  await page.addInitScript((t) => {
    try {
      localStorage.setItem("fohmixer_token", t);
    } catch {
      // storage blocked: the test then fails on the login page
    }
  }, seeded);
  await page.goto("/");
  await expect(page.getByTestId("stage")).toBeVisible();
}
