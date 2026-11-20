import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, dispatchPointer, harness, hostLine, hubStatus, openDetail, openSurface, MBC_MARKERS, until } from "./support/live";

// A fader's LUFS history in its channel detail (#219, the design on the
// ticket; the owner's approved mockup docs/mockups/lufs-history-v2.html):
// the detail's middle box gets tabs, EQ and, on a +LUFS strip, LUFS; each
// pane fills the box. The LUFS tab draws the point's short-term reading the
// hub keeps once a second (`/api/lufs/history`, `/api/lufs/days`), its live
// edge from the page's own readings, green within ±1.0 LU of an input
// point's target; 10 MIN / 1 H / 3 H, a drag back in time with NOW › to
// return, the days that hold the point, and a held finger's cursor. The
// tab and the window are kept for the page. Set up as lufs.spec.ts: SimLive's
// FohMixer LUFS device on band track 1 behind a `+LUFS:-25` Tuner.
//
// #253 (the owner's approved mockup docs/mockups/lufs-history-v3.html): the
// line is a smooth 3 px curve (120 columns, each the gated energy mean of
// its seconds, glided 1-2-1), green the same path clipped to the band, a
// dot at its live end; a label and a grid line every 3 LU; the strip's
// fader as a 2 px blue line on its own dB scale at the right (0 dB on the
// target), its value in the head (`FADER −10.0 dB`) and on the cursor; the
// hub keeps the fader beside the reading.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fixture with a LUFS group (the `+G:LUFS` marker strips) in the FOH page's second row. */
function withLufs(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "lufs", title: "LUFS", tags: "LUFS" });
  return frame;
}

const IN_TRACK = "live_set tracks 1";
const IN_VOLUME = `${IN_TRACK} mixer_device volume`;

/** A `+LUFS` marker strip by its label. */
function lufsStrip(page: Page, label: string): Locator {
  return page.locator(`.slot[data-group="lufs"] [data-testid="strip"][data-label="${label}"]`);
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === MBC_MARKERS, "the Tuner markers gone", 10_000);
}

/** A tap of a key that acts on its down (a finger's down and up). */
async function tap(key: Locator) {
  await expect(key).toBeVisible();
  await dispatchPointer(key, [{ type: "pointerdown" }, { wait: 30 }, { type: "pointerup" }]);
}

/** Opens the detail of the `Vox 1` strip from its own SEL (the Tuner's label is not the track's name). */
async function openVox(page: Page): Promise<Locator> {
  const sel = lufsStrip(page, "Vox 1").getByTestId("detail-key");
  await expect(sel).toBeVisible();
  await dispatchPointer(sel, [{ type: "pointerdown" }, { wait: 600 }, { type: "pointerup" }]);
  const detail = page.getByTestId("detail");
  await expect(detail).toHaveAttribute("data-label", "Vox 1");
  await expect(detail).toHaveAttribute("data-guard", "false");
  return detail;
}

/** The graph's drawn runs (`data-runs`). */
async function runsOf(graph: Locator): Promise<number> {
  return Number(await graph.getAttribute("data-runs"));
}

test.describe("A fader's LUFS history in its channel detail", () => {
  test.use({ viewport: { width: 1194, height: 834 } });
  let live: LiveClient;
  let mute: boolean | undefined;
  let volume: number | undefined;
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    await hostLine("band", "lufs track 1 off");
    await hostLine("band", "maxdev lufs track 1 remove");
    await hostLine("band", "tuner track 1 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    if (mute !== undefined) await live.set("band", IN_TRACK, "mute", mute);
    mute = undefined;
    if (volume !== undefined) await live.set("band", IN_VOLUME, "value", volume);
    volume = undefined;
    live.close();
  });

  test("the LUFS tab draws the reading's history, green in the band; the windows, a drag back, NOW ›, a day and the hold's cursor; the tab and window are kept", async ({ page }) => {
    // The graph's columns are 5 s (10 MIN) and 30 s (1 H), each glided over
    // its neighbours (#253): the dot reaching −30 and the 1 H column leaving
    // the band wait for real seconds of −30 to fill them (CI, 8b138005:
    // 5.0 + 7.0 + 8.0 s of the test's polls, 31.6 s in all), past the 30 s
    // default. The waits are the design's, not a slow page.
    test.setTimeout(60_000);
    mute = await live.get("band", IN_TRACK, "mute");
    await live.set("band", IN_TRACK, "mute", false);
    // The strip's fader at −10 dB (Live's 0.6).
    volume = await live.get("band", IN_VOLUME, "value");
    await live.set("band", IN_VOLUME, "value", 0.6);
    await openSurface(page);
    await harness("/hub/layout", { layout: withLufs() });
    expect(await hostLine("band", "maxdev lufs track 1 add rack")).toBe("MAXDEV 1");
    expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:LUFS +LUFS:-25 +PIN'`)).toBe("TUNER 1");
    // In the band of −25: the strip shows −25.0 in green.
    expect(await hostLine("band", "lufs track 1 -250 -100")).toBe("LUFS 1");
    await expect(lufsStrip(page, "Vox 1").getByTestId("lufs")).toHaveText("−25.0", { timeout: 10_000 });

    const detail = await openVox(page);
    const middle = detail.getByTestId("detail-middle");
    const tabs = middle.getByTestId("detail-tab");
    await expect(tabs).toHaveText(["EQ", "LUFS"]);
    // A page starts on EQ, its cards as before.
    await expect(middle).toHaveAttribute("data-tab", "eq");
    await expect(tabs.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(middle.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(middle.getByTestId("detail-pane-lufs")).toHaveCount(0);

    await tap(tabs.nth(1));
    await expect(middle).toHaveAttribute("data-tab", "lufs");
    await expect(tabs.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(middle.getByTestId("detail-pane-eq")).toBeHidden();
    const history = middle.getByTestId("lufs-history");
    await expect(history).toBeVisible();
    // The pane fills the box below the tabs; nothing scrolls.
    const box = (await middle.boundingBox())!;
    const pane = (await middle.getByTestId("detail-pane-lufs").boundingBox())!;
    expect(pane.y + pane.height).toBeCloseTo(box.y + box.height - 10, 0);
    expect(await middle.evaluate((e) => e.scrollHeight <= e.clientHeight)).toBe(true);

    // The head: the caption, the reading as the strip shows it, the target.
    await expect(history.getByTestId("lufs-history-cap").locator("span")).toHaveText(["LUFS", "IN"]);
    const now = history.getByTestId("lufs-history-now");
    await expect(now).toHaveText("−25.0");
    await expect(now).toHaveAttribute("data-l", "in");
    await expect(now).toHaveCSS("color", "rgb(47, 199, 154)");
    await expect(history.getByTestId("lufs-history-target")).toHaveText("TARGET −25");
    // The strip's fader beside the target, in its blue.
    const faderNow = history.getByTestId("lufs-history-fader");
    await expect(faderNow).toHaveText("FADER −10.0 dB");
    await expect(faderNow).toHaveCSS("color", "rgb(111, 184, 216)");

    // Live, 10 MIN: no NOW ›, the line drawn green within a few seconds.
    await expect(history).toHaveAttribute("data-state", "live");
    const windows = history.getByTestId("lufs-window");
    await expect(windows).toHaveText(["10 MIN", "1 H", "3 H"]);
    await expect(windows.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(history.getByTestId("lufs-now-chip")).toBeHidden();
    const graph = history.getByTestId("lufs-graph");
    await expect.poll(() => runsOf(graph), { timeout: 15_000 }).toBeGreaterThan(0);
    // One smooth 3 px curve in the calm grey, its green copy clipped to the band.
    const line = graph.getByTestId("lufs-line").first();
    await expect(line).toHaveCSS("stroke", "rgb(196, 196, 214)");
    await expect(line).toHaveCSS("stroke-width", "3px");
    expect(await line.getAttribute("d")).toMatch(/^M[\d.]+,[\d.]+C/);
    const greenCopy = graph.getByTestId("lufs-line-green").first();
    await expect(greenCopy).toHaveAttribute("clip-path", "url(#lh-band-clip)");
    await expect(greenCopy).toHaveCSS("stroke", "rgb(47, 199, 154)");
    // The same path (read at once: the line is drawn again each second).
    expect(
      await graph.evaluate((g) => {
        const base = g.querySelector('[data-testid="lufs-line"]')?.getAttribute("d");
        return base !== undefined && base === g.querySelector('[data-testid="lufs-line-green"]')?.getAttribute("d");
      }),
    ).toBe(true);
    await expect(graph.locator("clipPath#lh-band-clip rect")).toHaveCount(1);
    // The dot at the live end, green in the band.
    const dot = graph.getByTestId("lufs-dot");
    await expect(dot).toHaveAttribute("data-in", "true");
    await expect(dot).toHaveCSS("background-color", "rgb(47, 199, 154)");
    await expect(history.getByTestId("lufs-history-empty")).toBeHidden();

    // A label and a faint line every 3 LU at 13 px, the target's brighter;
    // the fader's dB at the same ticks on the right, 0 dB on the target.
    const yLabels = graph.getByTestId("lufs-y");
    await expect(yLabels).toHaveText(["−37", "−34", "−31", "−28", "−25", "−22", "−19", "−16", "−13"]);
    await expect(yLabels.first()).toHaveCSS("font-size", "13px");
    await expect(yLabels.nth(4)).toHaveAttribute("data-main", "true");
    await expect(yLabels.nth(4)).toHaveCSS("color", "rgb(196, 196, 214)");
    await expect(graph.getByTestId("lufs-fader-y")).toHaveText(["−12", "−9", "−6", "−3", "0", "+3", "+6", "+9", "+12"]);
    await expect(graph.getByTestId("lufs-axis")).toHaveText("LUFS");
    await expect(graph.getByTestId("lufs-fader-axis")).toHaveText("FADER dB");
    // Each label sits on its own line: the target's label and the fader's
    // 0 dB at the same height.
    const mid = async (l: Locator) => {
      const b = (await l.boundingBox())!;
      return b.y + b.height / 2;
    };
    expect(Math.abs((await mid(yLabels.nth(4))) - (await mid(graph.getByTestId("lufs-fader-y").nth(4))))).toBeLessThan(1);

    // The fader's line: 2 px of blue, its live end at −10 dB, 10 LU under
    // the target line (on the −35 LU height: 10/24 of the plot under its
    // middle). An earlier spec's minutes of this point may hold another
    // fader: only the live end is this test's.
    const faderLine = graph.getByTestId("lufs-fader-line").last();
    await expect(faderLine).toBeAttached({ timeout: 15_000 });
    await expect(faderLine).toHaveCSS("stroke", "rgb(111, 184, 216)");
    await expect(faderLine).toHaveCSS("stroke-width", "2px");
    const faderEnd = async () => {
      const points = (await faderLine.getAttribute("points")) ?? "";
      return Number(points.split(" ").at(-1)?.split(",")[1]);
    };
    await expect.poll(faderEnd, { timeout: 15_000 }).toBeCloseTo(916.7, 1);

    // Out of the band: the dot goes grey and, once the columns hold −30.0
    // (5 s each in 10 MIN, glided over their neighbours), sits on −30.
    expect(await hostLine("band", "lufs track 1 -300 -100")).toBe("LUFS 1");
    await expect(now).toHaveText("−30.0", { timeout: 10_000 });
    await expect(now).toHaveAttribute("data-l", "off");
    await expect(dot).toHaveAttribute("data-in", "false", { timeout: 15_000 });
    await expect(dot).toHaveCSS("background-color", "rgb(196, 196, 214)");
    // −30 on the −37..−13 scale: 70.833 % from the top.
    await expect.poll(() => dot.getAttribute("style"), { timeout: 30_000 }).toContain("top:70.833%");

    // A finger held still at the live edge shows the cursor: its time, the
    // reading there and the fader there.
    const cursor = history.getByTestId("lufs-history-cursor");
    const edge = (await graph.boundingBox())!.width / 2 - 4;
    await dispatchPointer(graph, [{ type: "pointerdown", dx: edge }, { wait: 600 }], 22);
    await expect(graph.getByTestId("lufs-cursor")).toBeAttached();
    // Its first line (#255: the set's transport is the second line's).
    await expect(cursor.getByTestId("lufs-cursor-text")).toHaveText(/^\d\d:\d\d:\d\d · −30\.0 · FADER −10\.0 dB$/);
    // TARGET and FADER step aside while it is held (#255).
    await expect(history.getByTestId("lufs-history-target")).toBeHidden();
    await expect(faderNow).toBeHidden();
    await dispatchPointer(graph, [{ type: "pointerup", dx: edge }], 22);
    await expect(history.getByTestId("lufs-history-target")).toBeVisible();
    await expect(graph.getByTestId("lufs-cursor")).toHaveCount(0);
    await expect(cursor).toHaveText("");

    // The fader moved in Live: the head follows at once.
    await live.set("band", IN_VOLUME, "value", 0.85);
    await expect(faderNow).toHaveText("FADER 0.0 dB");

    // The day that holds it: today.
    const days = history.getByTestId("lufs-days").getByTestId("lufs-day");
    await expect(days.first()).toHaveText("TODAY", { timeout: 15_000 });
    await expect(days.first()).toHaveAttribute("data-today", "true");

    // 1 H: the hub's history read for it (4 or 5 s a bucket), its live end
    // still drawn (30 s columns: the point's first minute may fill only one,
    // which draws the dot but no curve yet, and is no empty window).
    const span = (url: string) => {
      const q = new URL(url).searchParams;
      return Number(q.get("to")) - Number(q.get("from"));
    };
    const hourAnswer = page.waitForResponse((r) => r.url().includes("/api/lufs/history") && r.status() === 200 && span(r.url()) === 3_600_000);
    await tap(windows.nth(1));
    expect([4000, 5000]).toContain((await (await hourAnswer).json()).step_ms);
    await expect(windows.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(windows.nth(0)).toHaveAttribute("aria-pressed", "false");
    await expect(dot).toBeAttached({ timeout: 10_000 });
    await expect(history.getByTestId("lufs-history-empty")).toBeHidden();
    // Its last 30 s column glides with the one before (in the band at −25):
    // out of the band once about 14 s of −30 are in it.
    await expect(dot).toHaveAttribute("data-in", "false", { timeout: 20_000 });

    // A drag right goes back in time: NOW › shows; it returns to the live edge.
    const nowChip = history.getByTestId("lufs-now-chip");
    const dragBack = () =>
      dispatchPointer(graph, [
        { type: "pointerdown" },
        { type: "pointermove", dx: 20 },
        { type: "pointermove", dx: 120 },
        { type: "pointerup", dx: 120 },
      ]);
    await dragBack();
    await expect(history).toHaveAttribute("data-state", "back");
    await expect(nowChip).toBeVisible();
    // A window dragged back has no live end: no dot.
    await expect(graph.getByTestId("lufs-dot")).toHaveCount(0);
    await tap(nowChip);
    await expect(history).toHaveAttribute("data-state", "live");
    await expect(nowChip).toBeHidden();

    // Back again, then today's chip: the live edge, the window still 1 H.
    await dragBack();
    await expect(history).toHaveAttribute("data-state", "back");
    await tap(days.first());
    await expect(history).toHaveAttribute("data-state", "live");
    await expect(nowChip).toBeHidden();
    await expect(windows.nth(1)).toHaveAttribute("aria-pressed", "true");

    // Back to the mix and in again: the LUFS tab and 1 H are kept, and the
    // fresh card draws the hub's history (its answer holds readings).
    await tap(detail.getByTestId("detail-exit"));
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const answer = page.waitForResponse((r) => r.url().includes("/api/lufs/history") && r.status() === 200);
    const again = await openVox(page);
    await expect(again.getByTestId("detail-middle")).toHaveAttribute("data-tab", "lufs");
    await expect(again.getByTestId("lufs-window").nth(1)).toHaveAttribute("aria-pressed", "true");
    const series = await (await answer).json();
    // An hour asked from a time within a second: 3600 or 3601 s, so 4 or 5 s a value.
    expect([4000, 5000]).toContain(series.step_ms);
    expect(series.values.some((v: number | null) => v === -300), "the hub's history holds −30.0").toBe(true);
    // The hub keeps the strip's fader beside the readings (#253).
    expect(series.fader.length).toBe(series.values.length);
    expect(series.fader.some((v: number | null) => v === -100), "the hub's history holds the fader at −10 dB").toBe(true);
    // Drawn from the hub's history: in 10 MIN (5 s columns) a curve of six
    // columns or more within seconds of the card's open, which the card's
    // own seconds alone cannot make.
    await tap(again.getByTestId("lufs-window").nth(0));
    const againLine = again.getByTestId("lufs-graph").getByTestId("lufs-line");
    const segments = () => againLine.evaluateAll((ls) => Math.max(0, ...ls.map((l) => (l.getAttribute("d") ?? "").split("C").length - 1)));
    await expect.poll(segments, { timeout: 10_000 }).toBeGreaterThanOrEqual(5);

    // A strip without +LUFS: no tabs, its EQ pane, though LUFS is kept.
    await tap(again.getByTestId("detail-exit"));
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const plain = await openDetail(page, "Vocal 1 repro#");
    await expect(plain.getByTestId("detail-middle")).toHaveAttribute("data-tabs", "false");
    await expect(plain.getByTestId("detail-tabs")).toHaveCount(0);
    await expect(plain.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(plain.getByTestId("lufs-history")).toHaveCount(0);
    await tap(plain.getByTestId("detail-exit"));

    // In again: the LUFS tab still kept; EQ tapped shows its pane, the
    // history gone.
    const third = await openVox(page);
    await expect(third.getByTestId("detail-middle")).toHaveAttribute("data-tab", "lufs");
    await tap(third.getByTestId("detail-tab").nth(0));
    await expect(third.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(third.getByTestId("lufs-history")).toHaveCount(0);
  });

  test("a strip without +LUFS keeps its middle box as before: no tabs, the EQ cards", async ({ page }) => {
    await openSurface(page);
    const detail = await openDetail(page, "Hand2 #");
    const middle = detail.getByTestId("detail-middle");
    await expect(middle).toHaveAttribute("data-tabs", "false");
    await expect(middle.getByTestId("detail-tabs")).toHaveCount(0);
    await expect(middle.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(middle.getByTestId("eq-cards")).toBeAttached();
    await expect(middle.getByTestId("lufs-history")).toHaveCount(0);
  });
});

// #255 (the owner's approved mockup docs/mockups/lufs-history-v4.html): the
// point's Live set in the same graph. The hub keeps the set's transport and
// the song at its playhead once a second beside the reading (the song map
// and AUTOTUNE's playhead: the set needs an AUTOTUNE target, here band
// track 8's `+AUTOTUNE` with the AUTOTUNE button on the FOH rail); the
// graph draws a 6 px lane under the plot (a dim bar while the set plays, an
// empty track while it stands), a faint dashed line at a song's start with
// its `031 Name` at the plot's top, and the held cursor adds `· PLAY 031
// Name` or `· STOP`. Invented song names (a public repo).

/** The LUFS fixture with the AUTOTUNE button where the FOH rail's AUTOTUNE toggle stood. */
function withLufsAndAutotune(): any {
  const frame = withLufs();
  const foh = frame.pages.find((p: any) => p.id === "foh");
  const at = foh.rail.findIndex((c: any) => c.kind === "param_toggle" && c.label === "AUTOTUNE");
  foh.rail.splice(at, 1, { kind: "autotune", label: "AUTOTUNE" });
  return frame;
}

/** Two invented songs at 120 bpm: 031 from 0 to 400 beats (200 s), 057 after it. */
const SONGS = ["cue add 0 031 Morning Light {C 120 4/4}", "cue add 400 SONG END", "cue add 410 057 Quiet Hill", "cue add 800 SONG END"];

/** A run's right edge in percent of the lane, from its `left:` and `width:`. */
async function runEnds(runs: Locator): Promise<number[]> {
  return runs.evaluateAll((els) =>
    els.map((e) => {
      const style = e.getAttribute("style") ?? "";
      const num = (name: string) => Number(new RegExp(`${name}:([\\d.]+)%`).exec(style)?.[1] ?? NaN);
      return num("left") + num("width");
    }),
  );
}

test.describe("The LUFS history's transport and songs (#255)", () => {
  test.use({ viewport: { width: 1194, height: 834 } });
  let live: LiveClient;
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    await live.call("band", "live_set", "stop_playing");
    for (const index of [3, 2, 1, 0]) await hostLine("band", `cue remove ${index}`);
    expect(await hostLine("band", "song time 0")).toBe("SONG 0.0");
    await hostLine("band", "lufs track 1 off");
    await hostLine("band", "maxdev lufs track 1 remove");
    await hostLine("band", "tuner track 1 remove");
    await hostLine("band", "tuner track 8 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    await until(async () => (await hubStatus()).autotune.targets.length, (n: number) => n === 0, "no AUTOTUNE target", 10_000);
    live.close();
  });

  test("the lane shows the set playing and standing, a song's line and name at its start, and the hold's PLAY with the song and STOP", async ({ page }) => {
    // A song's name needs 40 px of its span: in 10 MIN (5 s columns) about
    // 45 s of play at the iPad's size, the design's, not a slow page.
    test.setTimeout(150_000);
    await live.call("band", "live_set", "stop_playing");
    expect(await hostLine("band", "song time 0")).toBe("SONG 0.0");
    for (const line of SONGS) expect(await hostLine("band", line)).toMatch(/^CUE \d+$/);
    await openSurface(page);
    await harness("/hub/layout", { layout: withLufsAndAutotune() });
    expect(await hostLine("band", "maxdev lufs track 1 add rack")).toBe("MAXDEV 1");
    expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:LUFS +LUFS:-25 +PIN'`)).toBe("TUNER 1");
    expect(await hostLine("band", "tuner track 8 set '+AUTOTUNE'")).toBe("TUNER 1");
    await until(async () => (await hubStatus()).autotune.targets.length, (n: number) => n === 1, "the AUTOTUNE target", 10_000);
    expect(await hostLine("band", "lufs track 1 -250 -100")).toBe("LUFS 1");
    await expect(lufsStrip(page, "Vox 1").getByTestId("lufs")).toHaveText("−25.0", { timeout: 10_000 });

    const detail = await openVox(page);
    await tap(detail.getByTestId("detail-middle").getByTestId("detail-tab").nth(1));
    const history = detail.getByTestId("lufs-history");
    const graph = history.getByTestId("lufs-graph");
    const windows = history.getByTestId("lufs-window");
    const lane = graph.getByTestId("lufs-lane");
    const tracks = lane.getByTestId("lufs-lane-track");
    const bars = lane.getByTestId("lufs-lane-play");
    // The lane: 6 px under the plot, 3 px apart, as wide as it.
    const laneBox = (await lane.boundingBox())!;
    expect(laneBox.height).toBeCloseTo(6, 0);
    const plotBox = (await graph.locator(".lh-plot").boundingBox())!;
    expect(laneBox.y - (plotBox.y + plotBox.height)).toBeCloseTo(3, 0);
    expect(laneBox.width).toBeCloseTo(plotBox.width, 0);
    // The set stands: the track reaches the live edge and no bar does (asked
    // again: a tap of 10 MIN reads the hub's history at once). The window
    // can hold an earlier spec's minutes of this point: only the live end
    // is this test's (`.claude/rules/e2e.md`).
    await expect
      .poll(
        async () => {
          await tap(windows.nth(0));
          return (await runEnds(tracks)).at(-1) ?? 0;
        },
        { timeout: 15_000, intervals: [2000] },
      )
      .toBeGreaterThan(99);
    expect((await runEnds(bars)).at(-1) ?? 0).toBeLessThan(98);
    await expect(tracks.last()).toHaveCSS("border-top-color", "rgb(42, 42, 64)");

    // It plays song 031: within seconds a dim bar at the live edge, with no
    // new answer from the hub (the page hears the start through AUTOTUNE's
    // and the recording watch's states), the song's dashed line at its
    // start, then, once its span is 40 px wide, its name.
    await live.call("band", "live_set", "start_playing");
    const started = Date.now();
    await expect.poll(async () => (await runEnds(bars)).at(-1) ?? 0, { timeout: 8_000, intervals: [250] }).toBeGreaterThan(99.5);
    console.log(`lufs history lane: the start seen in ${Date.now() - started} ms`);
    await expect(bars.last()).toHaveCSS("background-color", "rgb(58, 58, 82)");
    // The newest song's marks, read in ONE task in the page: the live window
    // moves a column every 5 s and redraws, so its positions are never read
    // across a redraw.
    const marks = () =>
      graph.evaluate((g) => {
        const left = (e: Element | undefined) => Number(/left:(?:calc\()?([\d.]+)%/.exec(e?.getAttribute("style") ?? "")?.[1] ?? NaN);
        const all = (id: string) => [...g.querySelectorAll(`[data-testid="${id}"]`)];
        const line = all("lufs-song-line").at(-1);
        const bar = all("lufs-lane-play").at(-1);
        const name = all("lufs-song").at(-1);
        const plot = g.querySelector(".lh-plot")!.getBoundingClientRect();
        const box = (e: Element | undefined) => {
          const r = e?.getBoundingClientRect();
          return r ? { x: r.x, y: r.y, right: r.right } : null;
        };
        return {
          lineLeft: left(line),
          barLeft: left(bar),
          nameLeft: left(name),
          song: name?.getAttribute("data-song") ?? null,
          strength: line?.getAttribute("data-line") ?? null,
          dashed: line ? getComputedStyle(line).borderLeftStyle : null,
          text: name?.textContent ?? null,
          colour: name ? getComputedStyle(name).color : null,
          line: box(line),
          name: box(name),
          plot: { x: plot.x, y: plot.y, right: plot.right },
          // The time grid's lines and labels, each its left in percent.
          grid: [...g.querySelectorAll(".lh-vline")].map(left),
          times: all("lufs-x").map(left),
        };
      });
    // The labels are exactly the grid lines outside the corners: none
    // within 10 px of the plot's left edge (`LUFS`) or 32 px of its right
    // (`FADER dB`), every other line labelled.
    const cornerProblems = (m: Awaited<ReturnType<typeof marks>>) => {
      const width = m.plot.right - m.plot.x;
      const shown = m.grid.filter((pct) => {
        const x = (pct / 100) * width;
        return x > 10 && x < width - 32;
      });
      return JSON.stringify(m.times) === JSON.stringify(shown) ? [] : [`labels ${JSON.stringify(m.times)}, lines ${JSON.stringify(m.grid)} of ${width} px`];
    };
    // The line first (faint while its span is narrow), where the newest bar starts.
    await expect
      .poll(async () => {
        const m = await marks();
        return m.dashed === "dashed" && Math.abs(m.lineLeft - m.barLeft) < 0.01;
      }, { timeout: 10_000, intervals: [250] })
      .toBe(true);
    // Then, once its span is 40 px wide, its name at the same start.
    await expect
      .poll(async () => {
        const m = await marks();
        return m.lineLeft === m.barLeft && m.nameLeft === m.lineLeft && m.song === "31" && m.strength === "full";
      }, { timeout: 80_000, intervals: [1000] })
      .toBe(true);
    // Its text, colour and place: at the plot's top, right of its line,
    // inside the plot; and no time label in the corners (`LUFS`, `FADER dB`).
    await expect
      .poll(async () => {
        const m = await marks();
        const problems: string[] = [];
        if (!/^031 Morning Light$|^031[^…]*…$/.test(m.text ?? "")) problems.push(`text ${m.text}`);
        if (m.colour !== "rgb(122, 122, 146)") problems.push(`colour ${m.colour}`);
        if (!m.name || !m.line) return [`no mark ${JSON.stringify(m)}`];
        if (m.nameLeft !== m.lineLeft) problems.push("the name is not the line's");
        if (!(m.name.x > m.line.x)) problems.push(`name at ${m.name.x}, line at ${m.line.x}`);
        if (!(m.name.y - m.plot.y < 6)) problems.push(`name ${m.name.y - m.plot.y} px under the plot's top`);
        if (!(m.name.right <= m.plot.right + 0.5)) problems.push(`name past the plot ${m.name.right - m.plot.right}`);
        if (m.times.length === 0) problems.push("no time label");
        problems.push(...cornerProblems(m));
        return problems;
      }, { timeout: 10_000, intervals: [250] })
      .toEqual([]);

    // A finger held at the live edge: PLAY with the song on the second line.
    const cursor = history.getByTestId("lufs-history-cursor");
    const edge = (await graph.boundingBox())!.width / 2 - 4;
    await dispatchPointer(graph, [{ type: "pointerdown", dx: edge }, { wait: 600 }], 22);
    await expect(cursor.getByTestId("lufs-cursor-set")).toHaveText("· PLAY 031 Morning Light");
    await expect(cursor.getByTestId("lufs-cursor-text")).toHaveText(/^\d\d:\d\d:\d\d · −25\.0/);
    await expect(history.getByTestId("lufs-history-target")).toBeHidden();
    await dispatchPointer(graph, [{ type: "pointerup", dx: edge }], 22);
    await expect(cursor).toHaveText("");

    // It stops: within seconds the bar ends before the live edge, the track
    // goes on, and the hold there says STOP.
    await live.call("band", "live_set", "stop_playing");
    const stopped = Date.now();
    await expect.poll(async () => (await runEnds(bars)).at(-1) ?? 100, { timeout: 8_000, intervals: [250] }).toBeLessThan(99.5);
    console.log(`lufs history lane: the stop seen in ${Date.now() - stopped} ms`);
    expect((await runEnds(tracks)).at(-1)).toBeCloseTo(100, 0);
    await dispatchPointer(graph, [{ type: "pointerdown", dx: edge }, { wait: 600 }], 22);
    await expect(cursor.getByTestId("lufs-cursor-set")).toHaveText("· STOP");
    await dispatchPointer(graph, [{ type: "pointerup", dx: edge }], 22);

    // The hub's answer carries the set beside the readings.
    const answer = page.waitForResponse((r) => r.url().includes("/api/lufs/history") && r.status() === 200);
    await tap(windows.nth(0));
    const series = await (await answer).json();
    expect(series.play.length).toBe(series.values.length);
    expect(series.play).toContain(true);
    expect(series.play).toContain(false);
    expect(series.song).toContain(31);
    expect(series.songs["31"]).toBe("Morning Light");

    // A grid line dragged into the right corner keeps its line and loses
    // its label (it would meet `FADER dB`): the window moved back by the
    // drag's px, a line at 16 px from the plot's right edge.
    const before = await marks();
    const width = before.plot.right - before.plot.x;
    const xs = before.grid.map((pct) => (pct / 100) * width).filter((x) => x < width - 48);
    expect(xs.length, "a grid line left of the corner").toBeGreaterThan(0);
    const dx = width - 16 - Math.max(...xs);
    await dispatchPointer(graph, [
      { type: "pointerdown" },
      { type: "pointermove", dx: 10 },
      { type: "pointermove", dx },
      { type: "pointerup", dx },
    ]);
    await expect(history).toHaveAttribute("data-state", "back");
    const after = await marks();
    const inCorner = after.grid.map((pct) => (pct / 100) * width).filter((x) => x >= width - 32);
    expect(inCorner.length, `a line in the right corner: ${JSON.stringify(after.grid)} of ${width} px`).toBeGreaterThan(0);
    expect(cornerProblems(after)).toEqual([]);
  });
});
