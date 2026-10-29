import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, centre, clipped, frames, harness, hostLine, hubStatus, openSurface, ready, seen, until, value2db } from "./support/live";

// The strip's loudness (#151, spec §6.1, §6.2, §9's E2E bullet; mockup
// docs/mockups/lufs-v9.html; the owner's ruling of 27.10 evening: like
// Sonible trueLevel; the owner's ruling of 29.10: mockup lufs-v10.html
// variant A, bigger and more prominent): SimLive's FohMixer LUFS devices
// send their readings to the hub, which serves them to the `+LUFS` strips.
// Every point shows its short-term `S` (the device's first number; `V` is
// no longer shown) with one decimal, bold and nearly white at the info
// column's whole width, under the smaller caption `LUFS` over `IN` (a mic,
// green within ±1 LU of its target) or `OUT` (an output, never green);
// silence shows `−∞` with its caption; the row keeps its height; under
// 78 px the caption leaves and the number stands upright, one character
// under another. A strip whose device is missing shows `!` and no number.
// strip-head.spec.ts sweeps the look at every width in both engines.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fixture with a LUFS group (the `+G:LUFS` marker strips) in the FOH page's second row. */
function withLufs(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "lufs", title: "LUFS", tags: "LUFS" });
  return frame;
}

/** SimLive's band set: the input point (Hand2 #), the output point (Hand3 #) and one without a device (Hand4 #). */
const IN_TRACK = "live_set tracks 1";
const OUT_TRACK = "live_set tracks 2";
const NONE_TRACK = "live_set tracks 3";
const TRACKS = [IN_TRACK, OUT_TRACK, NONE_TRACK];

/** The colours (v10 A): nearly white, and trueLevel's green inside an input's ±1 LU band. */
const WHITE = "rgb(226, 226, 236)";
const GREEN = "rgb(47, 199, 154)";

/** A `+LUFS` marker strip by its label. */
function lufsStrip(page: Page, label: string): Locator {
  return page.locator(`.slot[data-group="lufs"] [data-testid="strip"][data-label="${label}"]`);
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the Tuner markers gone", 10_000);
}

type Saved = { mute: boolean; volume: number }[];

/**
 * The three tagged tracks with their Tuners (pinned, so every screen shows
 * them), a device in a rack on the input and the output track, the output
 * at 0 dB and every track unmuted; Live's mutes and volumes as they were.
 */
async function setUp(page: Page, live: LiveClient): Promise<Saved> {
  const saved: Saved = [];
  for (const t of TRACKS) {
    saved.push({ mute: await live.get("band", t, "mute"), volume: await live.get("band", `${t} mixer_device volume`, "value") });
    await live.set("band", t, "mute", false);
  }
  // The output's reading is its short-term after its fader: 0 dB adds nothing.
  await live.set("band", `${OUT_TRACK} mixer_device volume`, "value", 0.85);
  await openSurface(page);
  // The fixture's default page is FOH (no tab to tap: a phone hides them).
  await harness("/hub/layout", { layout: withLufs() });
  expect(await hostLine("band", "maxdev lufs track 1 add rack")).toBe("MAXDEV 1");
  expect(await hostLine("band", "maxdev lufs track 2 add rack")).toBe("MAXDEV 1");
  expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:LUFS +LUFS:-25 +PIN'`)).toBe("TUNER 1");
  expect(await hostLine("band", `tuner track 2 set '"Vocals" +G:LUFS +LUFS +PIN'`)).toBe("TUNER 1");
  expect(await hostLine("band", `tuner track 3 set '"Vox 3" +G:LUFS +LUFS:-25 +PIN'`)).toBe("TUNER 1");
  await expect(page.locator('.slot[data-group="lufs"] [data-testid="strip"]')).toHaveCount(3, { timeout: 10_000 });
  return saved;
}

/** Every device stopped and removed, the Tuners gone, the layout reset, Live's values back. */
async function cleanUp(live: LiveClient, saved: Saved) {
  for (const n of [1, 2]) {
    await hostLine("band", `lufs track ${n} off`);
    await hostLine("band", `maxdev lufs track ${n} remove`);
  }
  for (const n of [1, 2, 3]) await hostLine("band", `tuner track ${n} remove`);
  await harness("/hub/layout/reset");
  await markersGone();
  for (const [i, t] of TRACKS.entries()) {
    if (!saved[i]) continue;
    await live.set("band", t, "mute", saved[i].mute);
    await live.set("band", `${t} mixer_device volume`, "value", saved[i].volume);
  }
}

/** The input device's `S` (and a `V` far from it, never shown), its strip's number once it shows `text`. */
async function shortTerm(page: Page, tenths: number, text: string) {
  expect(await hostLine("band", `lufs track 1 ${tenths} -100`)).toBe("LUFS 1");
  await expect(lufsStrip(page, "Vox 1").getByTestId("lufs")).toHaveText(text, { timeout: 10_000 });
}

/** A reading's look: no background, border or box shadow, its colour. */
async function quiet(el: Locator, colour: string) {
  await expect(el).toHaveCSS("color", colour);
  await expect(el).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(el).toHaveCSS("background-image", "none");
  for (const side of ["top", "right", "bottom", "left"]) await expect(el).toHaveCSS(`border-${side}-width`, "0px");
  await expect(el).toHaveCSS("box-shadow", "none");
}

/** Each bar's drawn level (1 − its cover's `scaleY`), as `strip.spec.ts` reads it. */
async function drawnBars(bars: Locator): Promise<{ level: number }[]> {
  return bars.evaluateAll((all) =>
    all.map((b) => {
      const cover = b.querySelector(".meter-cover") as HTMLElement;
      const scale = /scaleY\(([-\d.]+)\)/.exec(cover.style.transform);
      return { level: scale ? 1 - Number(scale[1]) : 0 };
    }),
  );
}

/** The caption's two lines. */
async function captionOf(s: Locator): Promise<string[]> {
  return s.getByTestId("lufs-cap").locator("span").allTextContents();
}

/** The reading's size (v10 A, strip.css): the info column's width (`strip-w − 38 − cap-w / 2`) / 2.25, 9 to 22 px; narrow × 0.8, 9 to 24 px. */
function sizeAt(w: number, narrow: boolean): number {
  const col = w - 38 - Math.min(32, w - 38) / 2;
  return narrow ? Math.min(24, Math.max(9, col * 0.8)) : Math.min(22, Math.max(9, col / 2.25));
}

/** The reading's characters (`behave::lufs::glyph_class`), one under another on a narrow strip: their texts and any overlap. */
async function stacked(lufs: Locator): Promise<{ text: string; problems: string[] }> {
  return lufs.evaluate((n) => {
    const problems: string[] = [];
    const nb = n.getBoundingClientRect();
    let above = nb.top;
    let text = "";
    for (const g of n.querySelectorAll(".g")) {
      const b = g.getBoundingClientRect();
      text += g.textContent;
      if (b.top < above - 0.01) problems.push(`"${g.textContent}" over the character above`);
      if (b.left < nb.left - 0.5 || b.right > nb.right + 0.5) problems.push(`"${g.textContent}" passes the number's box`);
      above = b.bottom;
    }
    if (above > nb.bottom + 0.01) problems.push("the last character passes the number's box");
    return { text, problems };
  });
}

/** The strip at `px` wide, the narrow flags as the page sets them on `.body`. */
async function widen(s: Locator, px: number) {
  await s.evaluate((el, w) => {
    const wrap = (el.closest(".slot") ?? el.parentElement) as HTMLElement;
    wrap.style.setProperty("--strip-w", `${w}px`);
    wrap.setAttribute("data-narrow", String(w < 78));
    wrap.setAttribute("data-narrow-wide", String(w * 1.1 < 78));
  }, px);
  await expect.poll(async () => (await s.boundingBox())!.width).toBeCloseTo(px, 0);
}

/**
 * Every shown part of a strip's reading inside the info column and its own
 * box: the number's laid-out text too (`clipped`; upright, its characters'
 * blocks, which clip the face's content area on purpose: `stacked`); a
 * caption line by its scroll sizes (its 8 px line is the face's whole
 * content area, so its text range is no measure: `.claude/rules/ui-rust.md`,
 * the group's name).
 */
async function inside(s: Locator): Promise<string[]> {
  const problems: string[] = [];
  const col = (await s.getByTestId("strip-col").boundingBox())!;
  const number = s.getByTestId("lufs");
  const lines = await s.getByTestId("lufs-cap").locator("span").all();
  for (const part of [number, ...lines]) {
    if (!(await seen(part))) continue;
    const text = await part.textContent();
    const b = (await part.boundingBox())!;
    if (b.x < col.x - 0.5 || b.x + b.width > col.x + col.width + 0.5 || b.y < col.y - 0.5 || b.y + b.height > col.y + col.height + 0.5) {
      problems.push(`${text} at ${JSON.stringify(b)} outside the column ${JSON.stringify(col)}`);
    }
    const upright = part === number && (await part.evaluate((e) => getComputedStyle(e).display === "flex"));
    const cut =
      part === number
        ? upright
          ? (await stacked(part)).problems
          : await clipped(part)
        : await part.evaluate((e) => (e.scrollWidth > e.clientWidth || e.scrollHeight > e.clientHeight ? [`scroll ${e.scrollWidth} x ${e.scrollHeight} > ${e.clientWidth} x ${e.clientHeight}`] : []));
    for (const c of cut) problems.push(`${text}: ${c}`);
  }
  return problems;
}

test.describe("The LUFS reading on a strip", () => {
  test.use({ viewport: { width: 1194, height: 834 } });
  let live: LiveClient;
  let saved: Saved = [];
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    await cleanUp(live, saved);
    saved = [];
    live.close();
  });

  test("the input shows S with one decimal, green within ±1 LU of its target and calm outside, silence shows −∞, a stopped device clears the number", async ({ page }) => {
    saved = await setUp(page, live);
    const s = lufsStrip(page, "Vox 1");
    const lufs = s.getByTestId("lufs");
    const cap = s.getByTestId("lufs-cap");
    await expect(s).toHaveAttribute("data-lufs", "in");
    await shortTerm(page, -250, "−25.0");
    await quiet(lufs, GREEN);
    await expect(lufs).toHaveAttribute("data-l", "in");
    await expect(cap).toBeVisible();
    expect(await captionOf(s)).toEqual(["LUFS", "IN"]);
    const row = (await lufs.boundingBox())!.height;
    const caption = (await cap.boundingBox())!.height;
    // v10 A: the row 1.25 em of the reading's size, the caption 8 + 8 px.
    expect(row).toBeCloseTo(1.25 * sizeAt((await s.boundingBox())!.width, false), 0);
    expect(caption).toBeCloseTo(16, 1);
    // V never shows: a V inside the band with S outside it is calm.
    expect(await hostLine("band", "lufs track 1 -330 -250")).toBe("LUFS 1");
    await expect(lufs).toHaveText("−33.0", { timeout: 10_000 });
    await expect(lufs).toHaveAttribute("data-l", "off");
    // The band's edges, both inside (|S − target| ≤ 1.0 LU).
    for (const [tenths, text, colour, state] of [
      [-240, "−24.0", GREEN, "in"],
      [-239, "−23.9", WHITE, "off"],
      [-260, "−26.0", GREEN, "in"],
      [-261, "−26.1", WHITE, "off"],
      [-259, "−25.9", GREEN, "in"],
      [-180, "−18.0", WHITE, "off"],
    ] as const) {
      await shortTerm(page, tenths, text);
      await expect(lufs).toHaveAttribute("data-l", state);
      await quiet(lufs, colour);
      expect(await captionOf(s)).toEqual(["LUFS", "IN"]);
    }
    // Silence: −∞ under its caption, calm; the rows keep their heights.
    await shortTerm(page, -700, "−∞");
    await expect(lufs).toHaveAttribute("data-l", "silent");
    await quiet(lufs, WHITE);
    await expect(lufs).toBeVisible();
    await expect(cap).toBeVisible();
    expect(await captionOf(s)).toEqual(["LUFS", "IN"]);
    expect((await lufs.boundingBox())!.height).toBeCloseTo(row, 1);
    expect((await cap.boundingBox())!.height).toBeCloseTo(caption, 1);
    // The device stops: the number goes within 1.5 s (the hub's 1 s stale
    // and its 100 ms tick). Read every 50 ms: `toHaveText`'s own back-off
    // looks at 850 ms, then at 1850 ms, so it cannot tell 1.5 s.
    await shortTerm(page, -250, "−25.0");
    expect(await hostLine("band", "lufs track 1 off")).toBe("LUFS 0");
    const stopped = Date.now();
    await expect.poll(() => lufs.textContent(), { timeout: 1500, intervals: [50] }).toBe("");
    test.info().annotations.push({ type: "cleared-ms", description: String(Date.now() - stopped) });
    await expect(lufs).toHaveAttribute("data-l", "none");
    await expect(lufs).toBeHidden();
    await expect(cap).toBeHidden();
    // A stopped device is no device: the strip's `!` and its words.
    await expect(s).toHaveAttribute("data-lufs-problem", "no LUFS meter", { timeout: 5000 });
    await expect(s.getByTestId("strip-flag")).toHaveText("!");
  });

  test("the output shows its calm short-term under LUFS OUT; a strip with no device shows ! and nothing in the row", async ({ page }) => {
    saved = await setUp(page, live);
    const out = lufsStrip(page, "Vocals");
    await expect(out).toHaveAttribute("data-lufs", "out");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 1");
    const lufs = out.getByTestId("lufs");
    await expect(lufs).toHaveText("−18.3", { timeout: 10_000 });
    await expect(lufs).toHaveAttribute("data-l", "plain");
    await quiet(lufs, WHITE);
    // Inside an input's band an output stays calm: it has no target.
    expect(await hostLine("band", "lufs track 2 -250 -700")).toBe("LUFS 1");
    await expect(lufs).toHaveText("−25.0", { timeout: 10_000 });
    await expect(lufs).toHaveAttribute("data-l", "plain");
    await quiet(lufs, WHITE);
    expect(await captionOf(out)).toEqual(["LUFS", "OUT"]);
    await expect(out.getByTestId("lufs-cap")).toHaveAttribute("data-kind", "out");
    // No device on Vox 3: `no LUFS meter` after the hub's grace, its `!`.
    const none = lufsStrip(page, "Vox 3");
    await expect(none).toHaveAttribute("data-lufs-problem", "no LUFS meter", { timeout: 10_000 });
    const flag = none.getByTestId("strip-flag");
    await expect(flag).toHaveText("!");
    expect(await seen(flag)).toBe(true);
    await expect(none.getByTestId("lufs")).toHaveText("");
    await expect(none.getByTestId("lufs")).toBeHidden();
    await expect(none.getByTestId("lufs-cap")).toBeHidden();
    const status = await hubStatus();
    const point = status.lufs.strips.find((p: any) => p.label === "Vox 3");
    expect(point.problem).toBe("no_lufs_meter");
    // The strips with a device carry no `!`.
    await expect(out.getByTestId("strip-flag")).toHaveCount(0);
  });

  test("the LUFS is bold and nearly white at the column's width, a digit half again as tall as 1.20.0's; a +LUFS strip's travel equals a plain strip's, at least 322 px", async ({ page }) => {
    // The owner's ruling of 29.10 (mockup v10 A): bigger and more prominent,
    // no longer quieter than Live's dB, which is unchanged.
    saved = await setUp(page, live);
    await shortTerm(page, -210, "−21.0");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
    await expect(lufsStrip(page, "Vocals").getByTestId("lufs")).toHaveText("−18.3", { timeout: 10_000 });
    await frames(page);
    const looks = await page.locator('[data-testid="strip"][data-lufs]').evaluateAll((all) =>
      all
        .filter((s) => (s.querySelector('[data-testid="lufs"]')?.textContent ?? "") !== "")
        .map((s) => {
          const n = s.querySelector('[data-testid="lufs"]')!;
          const l = getComputedStyle(n);
          const d = getComputedStyle(s.querySelector('[data-testid="db"]')!);
          // A digit's drawn height: the glyph "2"'s ink at the reading's face.
          const c = document.createElement("canvas");
          c.width = c.height = 400;
          const g = c.getContext("2d", { willReadFrequently: true })!;
          g.font = `${l.fontWeight} 200px ${l.fontFamily}`;
          g.fillText("2", 20, 300);
          const px = g.getImageData(0, 0, 400, 400).data;
          let first = -1;
          let last = -1;
          for (let y = 0; y < 400; y++)
            for (let x = 0; x < 400; x++)
              if (px[(y * 400 + x) * 4 + 3] > 0) {
                if (first < 0) first = y;
                last = y;
                break;
              }
          return {
            label: s.getAttribute("data-label"),
            strip: s.getBoundingClientRect().width,
            col: s.querySelector('[data-testid="strip-col"]')!.getBoundingClientRect().width,
            width: n.getBoundingClientRect().width,
            size: parseFloat(l.fontSize),
            weight: l.fontWeight,
            family: l.fontFamily,
            digit: ((last + 1 - first) * parseFloat(l.fontSize)) / 200,
            db: [d.fontSize, d.fontWeight, d.color],
          };
        }),
    );
    expect(looks.map((l) => l.label).sort()).toEqual(["Vocals", "Vox 1"]);
    console.log(`lufs look (${test.info().project.name}): ${JSON.stringify(looks)}`);
    for (const l of looks) {
      expect(l.weight, `${l.label}: bold`).toBe("700");
      expect(l.family, `${l.label}: the display face`).toMatch(/^"Barlow Condensed"/);
      expect(Math.abs(l.width - l.col), `${l.label}: the column's width`).toBeLessThan(0.5);
      expect(l.size, `${l.label}: the size`).toBeCloseTo(sizeAt(l.strip, false), 1);
      // 1.20.0's digit was 6.3 px on the iPad's strip, v10 A's 9.3 px.
      expect(l.digit, `${l.label}: a digit's height`).toBeGreaterThanOrEqual(9);
      // Live's dB as before.
      expect(l.db[1], `${l.label}: the dB's weight`).toBe("600");
    }
    // The travel: the reading costs the fader nothing.
    const travels = await page.getByTestId("strip").evaluateAll((all) =>
      all
        .filter((s) => !s.querySelector('[data-testid="strip-instance"]') && s.querySelector('[data-testid="fader"]'))
        .map((s) => ({
          label: s.getAttribute("data-label"),
          lufs: s.hasAttribute("data-lufs"),
          line: Array.from(document.querySelectorAll(".line")).indexOf(s.closest(".line")!),
          h: s.querySelector('[data-testid="fader"]')!.getBoundingClientRect().height,
        })),
    );
    const tagged = travels.filter((t) => t.lufs);
    expect(tagged.length).toBe(3);
    // A plain strip of the same line (another line may have another height).
    const plain = travels.filter((t) => !t.lufs && t.line === tagged[0].line);
    expect(plain.length).toBeGreaterThan(0);
    console.log(`lufs travel (${test.info().project.name}): +LUFS ${tagged.map((t) => t.h.toFixed(2)).join(", ")} px, plain ${[...new Set(plain.map((t) => t.h.toFixed(2)))].join(", ")} px`);
    for (const t of tagged) {
      expect(t.h, `${t.label}`).toBeGreaterThanOrEqual(322 - 0.5);
      expect(Math.abs(t.h - plain[0].h), `${t.label} against a plain strip`).toBeLessThan(0.5);
    }
  });

  test("every reading inside its box from 64 to 120 px: the caption from 78 px, under 78 px the number stacked upright", async ({ page }) => {
    saved = await setUp(page, live);
    // The widest text a reading shows: five characters, one decimal.
    await shortTerm(page, -259, "−25.9");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
    const out = lufsStrip(page, "Vocals");
    await expect(out.getByTestId("lufs")).toHaveText("−18.3", { timeout: 10_000 });
    for (const s of [lufsStrip(page, "Vox 1"), out]) {
      const kind = (await s.getAttribute("data-lufs")) === "in" ? "IN" : "OUT";
      for (const px of [64, 72, 76, 77.99, 78, 83.16, 88, 100, 120]) {
        await widen(s, px);
        await frames(page);
        const lufs = s.getByTestId("lufs");
        const cap = s.getByTestId("lufs-cap");
        expect(await seen(lufs), `${kind} ${px} px: the number seen`).toBe(true);
        expect(await inside(s), `${kind} ${px} px`).toEqual([]);
        if (px < 78) {
          await expect(cap).toHaveCSS("display", "none");
          await expect(lufs).toHaveCSS("writing-mode", "horizontal-tb");
          await expect(lufs).toHaveCSS("flex-direction", "column");
          const upright = await stacked(lufs);
          expect(upright.text, `${kind} ${px} px: one block per character`).toBe(await lufs.textContent());
          expect(upright.problems, `${kind} ${px} px: one under another`).toEqual([]);
          expect(parseFloat(await lufs.evaluate((e) => getComputedStyle(e).fontSize)), `${kind} ${px} px: the size`).toBeCloseTo(sizeAt(px, true), 1);
        } else {
          for (const line of await cap.locator("span").all()) expect(await seen(line), `${kind} ${px} px: the caption seen`).toBe(true);
          expect(await captionOf(s)).toEqual(["LUFS", kind]);
          await expect(lufs).toHaveCSS("writing-mode", "horizontal-tb");
          // Under the caption, above the group's name at 52 px.
          const c = (await cap.boundingBox())!;
          const l = (await lufs.boundingBox())!;
          const col = (await s.getByTestId("strip-col").boundingBox())!;
          expect(l.y, `${kind} ${px} px: the number under the caption`).toBeGreaterThanOrEqual(c.y + c.height - 0.5);
          expect(l.y + l.height - col.y, `${kind} ${px} px: the row ends by 52 px`).toBeLessThanOrEqual(52.5);
          expect(parseFloat(await lufs.evaluate((e) => getComputedStyle(e).fontSize)), `${kind} ${px} px: the size`).toBeCloseTo(sizeAt(px, false), 1);
          // `−25.9` fits its own box: no scroll.
          expect(await lufs.evaluate((e) => e.scrollWidth <= e.clientWidth), `${kind} ${px} px: the number fits`).toBe(true);
        }
        // Never over the group's name (Vox 1 is its run's first strip).
        const side = s.getByTestId("group-side");
        if ((await side.count()) > 0) {
          const l = (await lufs.boundingBox())!;
          const g = (await side.boundingBox())!;
          expect(l.y + l.height, `${kind} ${px} px: the number above the group's name`).toBeLessThanOrEqual(g.y + 0.5);
        }
      }
    }
  });

  test("an armed strip's bars follow its fader through its meter device; a strip without one keeps Live's meters", async ({ page }) => {
    // #151 Task 13 (spec §3.4): Live meters an armed track's input, so the
    // hub puts the meter device's peaks after the fader on the strip's meter
    // keys. SimLive's meters move 30 times a second: a strip that holds
    // still is fed by the device, one that moves by Live.
    saved = await setUp(page, live);
    const pan = `${IN_TRACK} mixer_device panning`;
    const savedPan = await live.get("band", pan, "value");
    const vox = lufsStrip(page, "Vox 1");
    const bars = () => vox.getByTestId("meter").getByTestId("meter-bar");
    const out = lufsStrip(page, "Vocals").getByTestId("meter").getByTestId("meter-bar");
    try {
      await live.set("band", `${IN_TRACK} mixer_device volume`, "value", 0.85);
      await live.set("band", pan, "value", 0);
      await live.set("band", IN_TRACK, "arm", true);
      expect(await hostLine("band", "maxdev meter track 1 add rack")).toBe("MAXDEV 1");
      expect(await hostLine("band", "level track 1 -120 -120")).toMatch(/^LEVEL [1-9]\d*$/);
      await until(
        async () => (await hubStatus()).lufs.devices,
        (d: any[]) => d.some((x) => x.kind === "meter" && x.state === "matched"),
        "the meter device matched",
        10_000,
      );
      // The bars settle on the device's level and stay there.
      const steady = async (): Promise<number[]> => {
        const a = (await drawnBars(bars())).map((b) => b.level);
        await page.waitForTimeout(250);
        const b = (await drawnBars(bars())).map((b) => b.level);
        return a.length === 2 && b.length === 2 && a.every((v, i) => v > 0 && Math.abs(v - b[i]) < 1e-3) ? b : [];
      };
      const before = await until(steady, (v) => v.length === 2, "the armed strip's bars steady on the device", 10_000);
      // A strip without a meter device keeps Live's moving meters.
      const moving = async () => {
        const a = (await drawnBars(out)).map((b) => b.level);
        await page.waitForTimeout(250);
        const b = (await drawnBars(out)).map((b) => b.level);
        return a.some((v, i) => Math.abs(v - b[i]) > 1e-3);
      };
      await until(moving, (m) => m, "Live's meters on the Vocals strip", 10_000);
      // A finger drags the fader down; Live's volume follows, and the bars fall.
      const fader = vox.getByTestId("fader");
      await ready(fader);
      const cap = await centre(vox.getByTestId("fader-cap"));
      await page.mouse.move(cap.x, cap.y);
      await page.mouse.down();
      for (let dy = 5; dy <= 100; dy += 5) {
        await page.mouse.move(cap.x, cap.y + dy);
        await frames(page);
      }
      await page.mouse.up();
      const lowered = await until(
        () => live.get("band", `${IN_TRACK} mixer_device volume`, "value"),
        (v: number) => value2db(v) <= -5,
        "Live's volume 5 dB down or more",
        10_000,
      );
      const after = await until(steady, (v) => v.length === 2 && v.every((x, i) => x < before[i] - 0.02), "the bars fallen", 10_000);
      test.info().annotations.push({ type: "bars", description: `${before} -> ${after} at ${value2db(lowered).toFixed(1)} dB` });
    } finally {
      await hostLine("band", "level track 1 off");
      await hostLine("band", "maxdev meter track 1 remove");
      await live.set("band", pan, "value", savedPan);
      await live.set("band", IN_TRACK, "arm", false);
    }
  });
});

// The phones and the iPad (spec §9: the phone sizes): on the real surface,
// a strip of 78 px or more shows the caption, a narrower one the number
// alone, stacked upright; every reading inside its column.
for (const size of [
  { width: 1194, height: 834 },
  { width: 844, height: 390 },
  { width: 390, height: 844 },
]) {
  test.describe(`The LUFS reading at ${size.width} × ${size.height}`, () => {
    test.use({ viewport: size });
    let live: LiveClient;
    let saved: Saved = [];
    test.beforeEach(async () => {
      live = await LiveClient.open();
    });
    test.afterEach(async () => {
      await cleanUp(live, saved);
      saved = [];
      live.close();
    });

    test("the caption only on a strip of 78 px or more, every reading inside its column", async ({ page }) => {
      saved = await setUp(page, live);
      await shortTerm(page, -250, "−25.0");
      expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
      const out = lufsStrip(page, "Vocals");
      await expect(out.getByTestId("lufs")).toHaveText("−18.3", { timeout: 10_000 });
      await frames(page);
      for (const s of [lufsStrip(page, "Vox 1"), out]) {
        // arrange::narrow: a strip under 78 px of its own width.
        const own = (await s.boundingBox())!.width;
        const narrow = own < 78;
        const lufs = s.getByTestId("lufs");
        expect(await seen(lufs), `${own} px: the number seen`).toBe(true);
        expect(await inside(s), `${own} px`).toEqual([]);
        if (narrow) {
          await expect(s.getByTestId("lufs-cap")).toHaveCSS("display", "none");
          await expect(lufs).toHaveCSS("flex-direction", "column");
          expect((await stacked(lufs)).problems, `${own} px: one under another`).toEqual([]);
        } else {
          for (const line of await s.getByTestId("lufs-cap").locator("span").all()) expect(await seen(line), `${own} px: the caption`).toBe(true);
          await expect(lufs).toHaveCSS("writing-mode", "horizontal-tb");
        }
      }
    });
  });
}
