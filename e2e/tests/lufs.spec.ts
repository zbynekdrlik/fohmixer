import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, clipped, frames, harness, hostLine, hubStatus, openSurface, seen, until } from "./support/live";

// The strip's loudness (#151, spec §6.1, §6.2, §9's E2E bullet; mockup
// docs/mockups/lufs-v9.html): SimLive's FohMixer LUFS devices send their
// readings to the hub, which serves them to the `+LUFS` strips. The number
// is quiet text under the caption `LUFS` over `IN` (a mic, judged against
// its target) or `OUT` (an output, plain grey); tinted text only out of
// range, nothing at all when silent; the row keeps its height; under 78 px
// the number stands upright and the caption leaves. A strip whose device is
// missing shows `!` and no number.

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

/** The tints (spec §6.2, v5): the quiet grey, the muted amber and the muted red. */
const GREY = "rgb(110, 110, 136)";
const AMBER = "rgb(168, 137, 74)";
const RED = "rgb(176, 88, 95)";

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

/** The input device's `V` (and a fixed `S`), its strip's number once it shows `text`. */
async function voice(page: Page, tenths: number, text: string) {
  expect(await hostLine("band", `lufs track 1 -300 ${tenths}`)).toBe("LUFS 1");
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

/** The caption's two lines. */
async function captionOf(s: Locator): Promise<string[]> {
  return s.getByTestId("lufs-cap").locator("span").allTextContents();
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
 * box: the number's laid-out text too (`clipped`); a caption line by its
 * scroll sizes (its 9 px line is the face's whole content area, so its text
 * range is no measure: `.claude/rules/ui-rust.md`, the group's name).
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
    const cut =
      part === number
        ? await clipped(part)
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

  test("the input reading takes its tint, silence shows nothing and keeps the row, a stopped device clears the number", async ({ page }) => {
    saved = await setUp(page, live);
    const s = lufsStrip(page, "Vox 1");
    const lufs = s.getByTestId("lufs");
    const cap = s.getByTestId("lufs-cap");
    await expect(s).toHaveAttribute("data-lufs", "in");
    await voice(page, -250, "−25");
    await quiet(lufs, GREY);
    await expect(lufs).toHaveAttribute("data-l", "fine");
    await expect(cap).toBeVisible();
    expect(await captionOf(s)).toEqual(["LUFS", "IN"]);
    const row = (await lufs.boundingBox())!.height;
    const caption = (await cap.boundingBox())!.height;
    expect(row).toBeCloseTo(16, 1);
    expect(caption).toBeCloseTo(18, 1);
    for (const [tenths, text, colour, state] of [
      [-210, "−21", AMBER, "warn"],
      [-300, "−30", AMBER, "warn"],
      [-170, "−17", RED, "far"],
      [-330, "−33", RED, "far"],
    ] as const) {
      await voice(page, tenths, text);
      await expect(lufs).toHaveAttribute("data-l", state);
      await quiet(lufs, colour);
      expect(await captionOf(s)).toEqual(["LUFS", "IN"]);
    }
    // Silence: no text, no caption, no box; the rows keep their heights.
    await voice(page, -700, "");
    await expect(lufs).toHaveAttribute("data-l", "silent");
    await expect(lufs).toBeHidden();
    await expect(cap).toBeHidden();
    expect(await captionOf(s)).toEqual(["", ""]);
    expect((await lufs.boundingBox())!.height).toBeCloseTo(row, 1);
    expect((await cap.boundingBox())!.height).toBeCloseTo(caption, 1);
    // The device stops: the number goes within 1.5 s (the hub's 1 s stale
    // and its 100 ms tick). Read every 50 ms: `toHaveText`'s own back-off
    // looks at 850 ms, then at 1850 ms, so it cannot tell 1.5 s.
    await voice(page, -250, "−25");
    expect(await hostLine("band", "lufs track 1 off")).toBe("LUFS 0");
    const stopped = Date.now();
    await expect.poll(() => lufs.textContent(), { timeout: 1500, intervals: [50] }).toBe("");
    test.info().annotations.push({ type: "cleared-ms", description: String(Date.now() - stopped) });
    await expect(lufs).toHaveAttribute("data-l", "none");
    await expect(cap).toBeHidden();
    // A stopped device is no device: the strip's `!` and its words.
    await expect(s).toHaveAttribute("data-lufs-problem", "no LUFS meter", { timeout: 5000 });
    await expect(s.getByTestId("strip-flag")).toHaveText("!");
  });

  test("the output shows its plain short-term under LUFS OUT; a strip with no device shows ! and nothing in the row", async ({ page }) => {
    saved = await setUp(page, live);
    const out = lufsStrip(page, "Vocals");
    await expect(out).toHaveAttribute("data-lufs", "out");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 1");
    const lufs = out.getByTestId("lufs");
    await expect(lufs).toHaveText("−18", { timeout: 10_000 });
    await expect(lufs).toHaveAttribute("data-l", "plain");
    await quiet(lufs, GREY);
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

  test("the LUFS is quieter than the dB; a +LUFS strip's travel equals a plain strip's, at least 322 px", async ({ page }) => {
    saved = await setUp(page, live);
    await voice(page, -210, "−21");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
    await expect(lufsStrip(page, "Vocals").getByTestId("lufs")).toHaveText("−18", { timeout: 10_000 });
    await frames(page);
    const looks = await page.locator('[data-testid="strip"][data-lufs]').evaluateAll((all) => {
      const rgb = (c: string) => (c.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
      const lum = (c: string) => {
        const [r, g, b] = rgb(c).map((v) => {
          const s = v / 255;
          return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const contrast = (a: string, b: string) => {
        const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
        return (x + 0.05) / (y + 0.05);
      };
      return all
        .filter((s) => (s.querySelector('[data-testid="lufs"]')?.textContent ?? "") !== "")
        .map((s) => {
          const ground = getComputedStyle(s).backgroundColor;
          const l = getComputedStyle(s.querySelector('[data-testid="lufs"]')!);
          const d = getComputedStyle(s.querySelector('[data-testid="db"]')!);
          return {
            label: s.getAttribute("data-label"),
            size: [parseFloat(l.fontSize), parseFloat(d.fontSize)],
            weight: [Number(l.fontWeight), Number(d.fontWeight)],
            contrast: [contrast(l.color, ground), contrast(d.color, ground)],
          };
        });
    });
    expect(looks.map((l) => l.label).sort()).toEqual(["Vocals", "Vox 1"]);
    for (const l of looks) {
      expect(l.size[0], `${l.label}: smaller`).toBeLessThan(l.size[1]);
      expect(l.weight[0], `${l.label}: lighter`).toBeLessThan(l.weight[1]);
      expect(l.contrast[0], `${l.label}: lower contrast`).toBeLessThan(l.contrast[1]);
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

  test("every reading inside its box from 64 to 120 px: the caption from 78 px, the number upright under it", async ({ page }) => {
    saved = await setUp(page, live);
    await voice(page, -330, "−33");
    expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
    const out = lufsStrip(page, "Vocals");
    await expect(out.getByTestId("lufs")).toHaveText("−18", { timeout: 10_000 });
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
          await expect(lufs).toHaveCSS("writing-mode", "vertical-rl");
        } else {
          for (const line of await cap.locator("span").all()) expect(await seen(line), `${kind} ${px} px: the caption seen`).toBe(true);
          expect(await captionOf(s)).toEqual(["LUFS", kind]);
          await expect(lufs).toHaveCSS("writing-mode", "horizontal-tb");
          // Under the caption, above the group's name at 40 px.
          const c = (await cap.boundingBox())!;
          const l = (await lufs.boundingBox())!;
          const col = (await s.getByTestId("strip-col").boundingBox())!;
          expect(l.y, `${kind} ${px} px: the number under the caption`).toBeGreaterThanOrEqual(c.y + c.height - 0.5);
          expect(l.y + l.height - col.y, `${kind} ${px} px: the row ends by 36 px`).toBeLessThanOrEqual(36.5);
        }
      }
    }
  });
});

// The phones and the iPad (spec §9: the phone sizes): on the real surface,
// a strip of 78 px or more shows the caption, a narrower one the number
// alone, upright; every reading inside its column.
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
      await voice(page, -250, "−25");
      expect(await hostLine("band", "lufs track 2 -183 -700")).toBe("LUFS 2"); // the input's device sends too (SimLive counts every device sending)
      const out = lufsStrip(page, "Vocals");
      await expect(out.getByTestId("lufs")).toHaveText("−18", { timeout: 10_000 });
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
          await expect(lufs).toHaveCSS("writing-mode", "vertical-rl");
        } else {
          for (const line of await s.getByTestId("lufs-cap").locator("span").all()) expect(await seen(line), `${own} px: the caption`).toBe(true);
          await expect(lufs).toHaveCSS("writing-mode", "horizontal-tb");
        }
      }
    });
  });
}
