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
  harness,
  openSurface,
  panning,
  ready,
  selectPage,
  shown,
  strip,
  groupOf,
  token,
  track,
  until,
  volume,
} from "./support/live";

// A strip (spec F2–F5, F8–F13, F20, F22, I8): Live's values, the fader, pan
// and mute writing Live, the meter, the status pill, and the controls waiting
// for Live's value before they take input.

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

const HAND2 = track("Hand2 #");

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
    // #63: a group whose strips share an instance names it in its title,
    // not on each strip; a return says RET on its name button.
    await expect(groupOf(page, strip(page, "Hand2 #")).getByTestId("group-instance")).toHaveText("band");
    await expect(strip(page, "Hand2 #").getByTestId("strip-instance")).toHaveCount(0);
    await expect(strip(page, "B-Main repro #").getByTestId("strip-instance")).toHaveCount(0);
    await expect(strip(page, "B-Main repro #").getByTestId("strip-ret")).toHaveText("RET");
    await expect(strip(page, "Hand2 #").getByTestId("strip-ret")).toHaveCount(0);
  });

  test("its texts fit their boxes: the dB text, the name and the instance", async ({ page }) => {
    // #9, findings 3-4: a narrow strip's 39 x 25 instance label showed "BANC"
    // for BAND, and the dB text must show Live's whole string.
    const VOCAL1 = track("Vocal 1 repro#");
    const before = await live.get("band", volume(VOCAL1), "value");
    try {
      await live.set("band", volume(VOCAL1), "value", 0.829725);
      await openSurface(page);
      const narrow = strip(page, "Vocal 1 repro#");
      await expect(narrow.getByTestId("db")).toHaveText(dbForm(await live.display("band", volume(VOCAL1), 0.829725)));
      const named = groupOf(page, narrow).getByTestId("group-instance");
      await expect(named).toHaveText(/band/i);
      expect(await clipped(named), "the group's instance").toEqual([]);
      for (const name of ["Vocal 1 repro#", "Hand2 #", "B-Main repro #"]) {
        for (const part of ["db", "strip-label"]) {
          const el = strip(page, name).getByTestId(part);
          await expect(el).not.toHaveText("");
          expect(await clipped(el), `${name} ${part}`).toEqual([]);
        }
      }
      const ret = strip(page, "B-Main repro #").getByTestId("strip-ret");
      expect(await clipped(ret), "the RET mark").toEqual([]);
    } finally {
      await live.set("band", volume(VOCAL1), "value", before);
    }
  });

  test("at the narrowest strip width a long name fits, every part stays in its strip, no half scrolls up or down", async ({ page }) => {
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
    // A master return first: the longest instance tag, "master · ret".
    strips.unshift({ kind: "strip", binding: { instance: "master", anchor: { kind: "return", name: "A-Reverb #" } }, strip_kind: "return" });
    foh.rows = [{ sections: [{ kind: "group", id: "narrow", title: "Narrow", controls: strips }] }];
    await page.setViewportSize({ width: 1236, height: 720 });
    await openSurface(page);
    const group = page.locator('[data-testid="group"][data-group="narrow"]');
    try {
      await harness("/hub/layout", { layout: changed });
      await expect(group.locator('[data-testid="strip"]')).toHaveCount(16, { timeout: 10_000 });
      const alert = group.locator('[data-testid="strip"][data-track="TechAlert #"]');
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
      await expect(group.locator('[data-testid="strip"][data-track="A-Reverb #"]').getByTestId("strip-instance")).toHaveText(/master · ret/i);
      for (const part of ["strip-label", "strip-instance"]) {
        for (const text of await group.getByTestId(part).all()) {
          expect(await clipped(text), `${part} ${await text.textContent()}`).toEqual([]);
        }
      }
      const scrolls = await page.getByTestId("half").evaluateAll((halves) =>
        halves.filter((r) => r.scrollHeight > r.clientHeight).map((r) => `${r.scrollHeight} > ${r.clientHeight}`),
      );
      expect(scrolls, "halves with a vertical scroll range").toEqual([]);
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

  test("a pan drag moves Live's panning and a double tap centres it", async ({ page }) => {
    await live.set("band", panning(HAND2), "value", 0.0);
    await openSurface(page);
    const pan = strip(page, "Hand2 #").getByTestId("pan");
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
    await page.waitForTimeout(700);
    expect(await live.get("master", hand1, "mute")).toBe(false);
    await expect(mute).not.toHaveClass(/\barmed\b/);
    await doubleTap(mute, 150);
    await until(() => live.get("master", hand1, "mute"), (v) => v === true, "muted after the confirming tap");
    // The band's track of the same name is another instance's (spec F2, X4).
    expect(await live.get("band", hand1, "mute")).toBe(false);
    await live.set("master", hand1, "mute", false);
  });

  test("the name button takes the track's colour from Live and follows it", async ({ page }) => {
    // #21: the strip's name button is its mute, lit in the track's Live
    // colour (SimLive's Hand2 #: 0xFF3636) with the text that reads on it.
    await live.set("band", HAND2, "mute", false);
    const before = await live.get("band", HAND2, "color");
    try {
      await openSurface(page);
      const mute = strip(page, "Hand2 #").getByTestId("mute");
      await ready(mute);
      await expect(mute).toHaveCSS("background-color", "rgb(255, 54, 54)");
      await expect(mute).toHaveCSS("color", "rgb(16, 16, 26)");
      await live.set("band", HAND2, "color", 0x1e3a8a);
      await expect(mute).toHaveCSS("background-color", "rgb(30, 58, 138)");
      await expect(mute).toHaveCSS("color", "rgb(244, 244, 250)");
      // Muted: dark whatever the colour, with its MUTE mark.
      await mute.click();
      await expect(mute).toHaveAttribute("data-muted", "true");
      await expect(mute).toHaveCSS("background-color", "rgb(31, 31, 46)");
      await expect(mute.locator(".mute-mark")).toBeVisible();
    } finally {
      await live.set("band", HAND2, "color", before);
      await live.set("band", HAND2, "mute", false);
    }
  });

  test("the dB scale sits beside the fader, its 0 at the fader's 0 dB", async ({ page }) => {
    // #21 (parity audit #12): TouchOSC's labels at the fader positions of
    // their levels.
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const ticks = s.getByTestId("scale").locator(".tick");
    // #63: Live's own fader law by default, its low end spread out as a
    // console's, so −30 and −60 are labelled too.
    await expect(ticks).toHaveText(["+6", "0", "−6", "−12", "−18", "−24", "−30", "−40", "−60"]);
    const fader = s.getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.85) < 0.001, "the fader at 0 dB");
    await frames(page);
    const cap = (await s.locator(".fader-cap").boundingBox())!;
    const zero = (await ticks.nth(1).boundingBox())!;
    expect(Math.abs(cap.y + cap.height / 2 - (zero.y + zero.height / 2))).toBeLessThan(2);
    // +6 dB is the top of the travel.
    const track = (await fader.boundingBox())!;
    const top = (await ticks.nth(0).boundingBox())!;
    expect(Math.abs(top.y + top.height / 2 - track.y)).toBeLessThan(2);
    // The position is Live's volume: 0 dB at 85 % of the travel.
    const down = (cap.y + cap.height / 2 - track.y) / track.height;
    expect(Math.abs(down - 0.15), `0 dB ${down} of the travel from its top`).toBeLessThan(0.01);
  });

  test("the meter moves with Live's meter", async ({ page }) => {
    await openSurface(page);
    const meter = strip(page, "Hand2 #").getByTestId("meter");
    const first = await until(async () => Number(await meter.getAttribute("data-level")), (v) => v > 0, "a level");
    await until(async () => Number(await meter.getAttribute("data-level")), (v) => v !== first, "the level to move");
  });

  test("the meter holds its peak and lights the clip light at 0 dB until it is tapped", async ({ page }) => {
    // #21: new with the redesign (TouchOSC had neither).
    await openSurface(page);
    const meter = strip(page, "Hand2 #").getByTestId("meter");
    const clip = meter.getByTestId("clip");
    await expect(clip).toHaveAttribute("data-on", "false");
    await until(async () => Number(await meter.getAttribute("data-peak")), (v) => v > 0, "a peak");
    try {
      expect(await hostLine("band", 'meter "Hand2 #" 1.0')).toBe("METER 1");
      await expect(clip).toHaveAttribute("data-on", "true");
      await until(async () => Number(await meter.getAttribute("data-peak")), (v) => v > 0.99, "the peak at the top");
      // The level falls; the light stays lit and the peak holds a while.
      expect(await hostLine("band", 'meter "Hand2 #" 0.3')).toBe("METER 1");
      await until(async () => Number(await meter.getAttribute("data-level")), (v) => v < 0.5, "the bar down");
      expect(Number(await meter.getAttribute("data-peak"))).toBeGreaterThan(0.9);
      await expect(clip).toHaveAttribute("data-on", "true");
      // Then the peak falls back to the bar.
      await until(async () => Number(await meter.getAttribute("data-peak")), (v) => v < 0.5, "the peak down", 5000);
      // A tap turns the light off.
      await clip.click();
      await expect(clip).toHaveAttribute("data-on", "false");
    } finally {
      await hostLine("band", 'meter "Hand2 #" off');
    }
  });

  test("with the stereo meter source a strip shows two bars", async ({ page }) => {
    const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
    await openSurface(page);
    const bars = strip(page, "Hand2 #").getByTestId("meter").locator(".meter-bar");
    await expect(bars).toHaveCount(1);
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    changed.config.meter_source = "lr";
    try {
      await harness("/hub/layout", { layout: changed });
      await expect(bars).toHaveCount(2, { timeout: 10_000 });
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(bars).toHaveCount(1, { timeout: 10_000 });
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
    await expect(groupOf(page, master).getByTestId("group-instance")).toHaveText("master");
    await expect(master.getByTestId("db")).toHaveText(dbForm(await live.display("master", volume(track("Hand1 #")), 0.6)));
  });
});

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
