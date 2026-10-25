import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  HUB_SOCKET,
  LiveClient,
  clipped,
  dispatchSnapped,
  dispatchPointer,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  openSurface,
  pageEvents,
  ready,
  selectPage,
  strip,
  until,
} from "./support/live";

// AUTOTUNE and its singers' panel (#140 PR 3c; the approved mockup
// docs/mockups/autotune-v2.html): the AUTOTUNE rail button (where the former
// AUTOTUNE toggle stood) shows AUT, the state and the key in force on its
// second line (`off · C`, `stopped · C`, `no song`, `checking · C`,
// `tuning · C`, `tuning · C · 1 off`), lit while every singer tunes; a 1 s
// hold turns AUT on or off at its lift; a tap opens the singers' panel and
// sends nothing. The hub's rule switches each singer's tracks through the
// real hub and SimLive: SimLive's band track 6 (Vocal 2 repro#, a `Latencies`
// rack on top, an Auto-Tune Artist in its chain 0) is the singer VOX1's PA
// track (`"Vox 1" +ID:VOX1 +AUTOTUNE:0`: on 0, off 127), track 7 (Vocal 3
// repro#, a `Latencies` rack inside `Stream FX`, an AudioGridder in its
// chain 0) its stream track (`+MUTE:VOX1 +AUTOTUNE`: on 127, off 0); track
// 8 (Keys 1, no rack) `+AUTOTUNE` for a target with a problem. Both macros
// start at 0: track 6 on its chain, which the hub writes DRY (127) with AUT
// off, track 7 DRY. A song is a locator the test adds (`cue add`), the
// playhead the host's `song time`. Invented names only.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
/** The PA track's macro (on 0) and the stream track's (on 127). */
const PA = "live_set tracks 6 devices 0 parameters 1";
const STREAM = "live_set tracks 7 devices 0 chains 0 devices 0 parameters 1";
/** The stream track's AudioGridder slots that carry its Key and Scale. */
const STREAM_KEY = "live_set tracks 7 devices 0 chains 0 devices 0 chains 0 devices 0 parameters 1";
const STREAM_SCALE = "live_set tracks 7 devices 0 chains 0 devices 0 chains 0 devices 0 parameters 2";
/** Both macros on DRY (the PA's at 127, the stream's at 0), and both tuning. */
const DRY = [127, 0];
const ON = [0, 127];
/** The song the tests add: song 1 in C from 0 to 100. */
const SONG = ["cue add 0 001 Test One {C 120 4/4}", "cue add 100 SONG END"];

/** The fixture with the AUTOTUNE button where the FOH rail's AUTOTUNE toggle stood. */
function withAutotuneButton(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  const at = foh.rail.findIndex((c: any) => c.kind === "param_toggle" && c.label === "AUTOTUNE");
  foh.rail.splice(at, 1, { kind: "autotune", label: "AUTOTUNE" });
  return frame;
}

/** Live's two macros (PA, stream). */
async function macros(live: LiveClient): Promise<number[]> {
  return [await live.get("band", PA, "value"), await live.get("band", STREAM, "value")];
}

/** Waits until both macros read `want`. */
async function macrosAre(live: LiveClient, want: number[], what: string, timeout = 10_000): Promise<number[]> {
  return until(() => macros(live), (m: number[]) => m[0] === want[0] && m[1] === want[1], what, timeout);
}

/** The page clock (the flight recorder's `t`) now. */
function pageNow(page: Page): Promise<number> {
  return page.evaluate(() => performance.timeOrigin + performance.now());
}

/** The AUTOTUNE button's `rail` steps the page recorded since `since`, once `ok` holds. */
async function railSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.button === "autotune" && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the AUTOTUNE rail steps uploaded",
    15_000,
  );
}

/** The hub's `autotune` records since `since` (the test's clock: one runner clock with the hub's `ts`). */
async function autotuneRecords(since: number): Promise<any[]> {
  return (await hubEvents()).filter((r: any) => r.ev === "autotune" && r.ts >= since);
}

/** Waits until the hub heard the band's transport `playing` since `since` (its `transport` record). */
async function transportHeard(since: number, playing: boolean) {
  await until(
    async () => (await hubEvents()).some((r: any) => r.ev === "transport" && r.instance === "band" && r.playing === playing && r.ts >= since),
    (v: boolean) => v,
    `the hub heard Live ${playing ? "start" : "stop"}`,
    10_000,
  );
}

/** Waits until the hub found every one of `count` targets (a path or a problem each). */
async function targetsFound(count: number): Promise<any[]> {
  return until(
    async () => (await hubStatus()).autotune.targets,
    (t: any[]) => t.length === count && t.every((x) => x.state !== "waiting"),
    `${count} AUTOTUNE targets found`,
    10_000,
  );
}

/** Waits until the PA's and the stream's keys stand `state` for `key` (the hub wrote and read them back). */
async function keysStand(key: string, states: [string, string]): Promise<any[]> {
  return until(
    async () => (await hubStatus()).autotune.targets,
    (t: any[]) => t.length >= 2 && t.slice(0, 2).every((x, i) => x.key?.want === key && x.key?.state === states[i]),
    `the keys ${key} ${states.join(", ")}`,
    10_000,
  );
}

/**
 * The layout revisions the page showed (its `layout revision N` console
 * line): a Tuner edit is a new layout, and the page replaces its controls
 * a moment after the hub serves it.
 */
function watchLayouts(page: Page): string[] {
  const lines: string[] = [];
  page.on("console", (msg) => {
    if (msg.text().startsWith("layout revision ")) lines.push(msg.text());
  });
  return lines;
}

/** Waits until the page shows the layout the hub serves now (a box read before it is the old page's). */
async function showsServedLayout(lines: string[]) {
  const rev = (await hubStatus()).layout.rev;
  await expect.poll(() => lines.includes(`layout revision ${rev}`), { timeout: 10_000 }).toBe(true);
}

/** Tags the singer VOX1's PA track (6) and stream track (7). */
async function tagSinger() {
  expect(await hostLine("band", `tuner track 6 set '"Vox 1" +ID:VOX1 +AUTOTUNE:0'`)).toBe("TUNER 1");
  expect(await hostLine("band", "tuner track 7 set '+MUTE:VOX1 +AUTOTUNE'")).toBe("TUNER 1");
}

/** Adds song 1 in C (its locators), and waits until the hub read the song map. */
async function addSongMap() {
  expect(await hostLine("band", SONG[0])).toBe("CUE 1");
  expect(await hostLine("band", SONG[1])).toBe("CUE 2");
  await until(
    async () => (await hubStatus()).songmap.find((m: any) => m.instance === "band"),
    (m: any) => m?.keyed === 1,
    "the song map read",
    10_000,
  );
}

/** Adds song 1 in C with the playhead in it, and waits until the hub wrote and read back its key. */
async function addSong() {
  await addSongMap();
  await keysStand("C", ["confirmed", "confirmed"]);
}

/** The second line's texts as the page draws them, every change recorded in the page from now on. */
async function watchLines(button: Locator) {
  await button.evaluate((el) => {
    const w = window as any;
    const sub = el.querySelector(".btn-sub")!;
    w.tuneLines = [sub.textContent];
    w.tuneLineWatch = new MutationObserver(() => {
      const now = sub.textContent;
      if (w.tuneLines[w.tuneLines.length - 1] !== now) w.tuneLines.push(now);
    });
    w.tuneLineWatch.observe(sub, { childList: true, characterData: true, subtree: true });
  });
}

/** The second line's texts recorded since `watchLines`. */
function watchedLines(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).tuneLines);
}

/** A 1 s hold of the button, lifted once full: its look at 1.2 s. */
async function hold(button: Locator) {
  const press = await dispatchSnapped(button, [{ type: "pointerdown" }, { wait: 1200 }, { snap: true }]);
  await dispatchPointer(button, [{ type: "pointerup" }]);
  return press.snaps[0];
}

/** A down and its up dispatched on the page's body: a tap outside every control. */
async function tapOutside(page: Page) {
  await page.evaluate(() => {
    for (const type of ["pointerdown", "pointerup"]) {
      document.body.dispatchEvent(new PointerEvent(type, { pointerId: 41, pointerType: "touch", isPrimary: true, bubbles: true, cancelable: true }));
    }
  });
}

test.describe("AUTOTUNE and its singers' panel", () => {
  // The iPad's size (the rail's buttons 118 × 44, nothing clipped at
  // 1194 × 834) in both projects.
  test.use({ viewport: { width: 1194, height: 834 } });

  let live: LiveClient;
  let layouts: string[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    // AUT off and the singer back on first; then Live stopped, the held
    // slots freed, the song gone, the Tuners gone (no target: the hub
    // writes nothing more), and only then both macros back to SimLive's 0.
    live.setHub("autotune_aut", false);
    await until(async () => live.autotune?.aut, (v: boolean) => v === false, "AUTOTUNE AUT off");
    live.setHub("autotune_singer", { id: "VOX1", off: false });
    await until(async () => (await hubStatus()).autotune.singers_off ?? [], (v: string[]) => v.length === 0, "no singer off");
    await live.call("band", "live_set", "stop_playing");
    for (const path of [STREAM_KEY, STREAM_SCALE]) expect(await hostLine("band", `param ${path} enabled 1`)).toBe("PARAM 1");
    for (const index of [1, 0]) await hostLine("band", `cue remove ${index}`);
    expect(await hostLine("band", "song time 0")).toBe("SONG 0.0");
    for (const track of [6, 7, 8]) await hostLine("band", `tuner track ${track} remove`);
    await harness("/hub/layout/reset");
    await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the markers gone", 10_000);
    await until(async () => (await hubStatus()).autotune.targets.length, (n: number) => n === 0, "no AUTOTUNE target", 10_000);
    await live.set("band", PA, "value", 0);
    await live.set("band", STREAM, "value", 0);
    live.close();
  });

  /** The AUTOTUNE button on the FOH rail, red until the singer is tagged, then ready: the hub writes the PA track DRY (AUT off), so it reads off. */
  async function setUp(page: Page): Promise<Locator> {
    await live.call("band", "live_set", "stop_playing");
    // The playhead at the set's start (another spec's play moved it).
    expect(await hostLine("band", "song time 0")).toBe("SONG 0.0");
    layouts = watchLayouts(page);
    await openSurface(page);
    await harness("/hub/layout", { layout: withAutotuneButton() });
    await selectPage(page, "foh");
    const button = page.getByTestId("autotune");
    // No tagged track yet (I5): red and disabled.
    await expect(button).toHaveAttribute("data-binding", "unresolved", { timeout: 10_000 });
    await expect(button).toHaveAttribute("aria-disabled", "true");
    await tagSinger();
    await targetsFound(2);
    // AUT off, the PA track on its chain: the hub writes it DRY (#140 PR 3b).
    await macrosAre(live, DRY, "the hub's DRY");
    await showsServedLayout(layouts);
    await ready(button);
    await expect(button).toHaveAttribute("data-binding", "ready");
    await expect(button).toHaveAttribute("data-state", "off");
    return button;
  }

  test("the button reads AUT, the state and the key; 118 × 44, no fader height lost; a 1 s hold turns AUT on and off at its lift", async ({ page }) => {
    const button = await setUp(page);
    // Where the AUTOTUNE toggle stood, its touches its own (#43 PR G).
    const rail = page.getByTestId("rail").locator(".rail-main > *");
    const at = withAutotuneButton().pages.find((p: any) => p.id === "foh").rail.findIndex((c: any) => c.kind === "autotune");
    expect(await rail.nth(at).getAttribute("data-testid")).toBe("autotune");
    await expect(button.locator(".btn-text")).toHaveText("AUTOTUNE");
    const size = (await button.boundingBox())!;
    console.log(`autotune button (${test.info().project.name}): ${size.width} × ${size.height} px`);
    expect(Math.abs(size.width - 118), `118 px wide: ${size.width}`).toBeLessThanOrEqual(1);
    expect(Math.abs(size.height - 44), `44 px high: ${size.height}`).toBeLessThanOrEqual(1);
    // Fader height first: a strip's travel is the iPad's 322 px.
    const travel = (await strip(page, "Hand4 #", "master").getByTestId("fader").boundingBox())!.height;
    expect(Math.abs(travel - 322), `a fader's travel is 322 px: ${travel}`).toBeLessThanOrEqual(1);
    expect(
      await button.evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);
    // AUT off, no song: dark, no pill, `off · —`.
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button).not.toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-mixed", "false");
    await expect(button).toHaveAttribute("data-problem", "false");
    await expect(button).toHaveAttribute("data-open", "false");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(button.getByTestId("btn-sub")).toHaveText("off · —");
    expect(await clipped(button)).toEqual([]);
    // A 1 s hold: full at 1 s, nothing sent before the lift; AUT on at it.
    const since = Date.now();
    const sincePage = await pageNow(page);
    const full = await hold(button);
    expect(full).toMatchObject({ holding: "true", full: "true", sub: "pusť — zapne AUT", failed: false });
    await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("aut-badge")).toHaveText("AUT");
    await expect(button.getByTestId("btn-sub")).toHaveText("stopped · —");
    expect(await clipped(button)).toEqual([]);
    const status = await hubStatus();
    expect(status.autotune.aut).toBe(true);
    // Taken, recorded and nothing refused (#140 PR 3c lifted the refusal).
    const records = await until(
      () => autotuneRecords(since),
      (r) => r.some((x) => x.step === "flag"),
      "the flag record",
    );
    expect(records.filter((r) => r.step === "flag").map((r) => r.on)).toEqual([true]);
    expect(records.filter((r) => r.step === "refused")).toEqual([]);
    // Stopped: nothing tunes.
    expect(await macros(live)).toEqual(DRY);
    // A second hold turns it off at its lift.
    const off = await hold(button);
    expect(off).toMatchObject({ holding: "true", full: "true", sub: "pusť — vypne AUT" });
    await until(async () => live.autotune?.aut, (v: boolean) => v === false, "AUTOTUNE AUT off again");
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button.getByTestId("btn-sub")).toHaveText("off · —");
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "hold").length >= 2)).toEqual(["press", "full", "hold", "press", "full", "hold"]);
  });

  test("a song played with AUT on tunes after the keys read back; a singer's off switch takes both tracks DRY; a stop goes DRY", async ({ page }) => {
    // Several of the rule's own round trips through SimLive (the keys, the
    // read-backs, the selectors) after the set-up.
    test.setTimeout(60_000);
    const button = await setUp(page);
    // The playhead after the song: no key in force, none written yet.
    expect(await hostLine("band", "song time 105")).toBe("SONG 105.0");
    await addSongMap();
    await expect(button.getByTestId("btn-sub")).toHaveText("off · —");
    // AUT on by the hold: Live stands, nothing tunes.
    await hold(button);
    await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
    await expect(button.getByTestId("btn-sub")).toHaveText("stopped · —");
    // Live plays between the songs: nothing tunes.
    const started = Date.now();
    await live.call("band", "live_set", "start_playing");
    await transportHeard(started, true);
    await expect(button.getByTestId("btn-sub")).toHaveText("no song");
    expect(await macros(live)).toEqual(DRY);
    // Into the song while playing: its key written (the first time) and
    // read back first (`checking · C`), then both tracks tune.
    await watchLines(button);
    const since = Date.now();
    expect(await hostLine("band", "song time 10")).toBe("SONG 10.0");
    await macrosAre(live, ON, "both tracks tuning");
    await expect(button).toHaveAttribute("data-state", "on");
    await expect(button).toHaveClass(/\bon\b/);
    await expect(button.getByTestId("btn-sub")).toHaveText("tuning · C");
    const lines = await watchedLines(page);
    expect(lines, `${lines}`).toContain("checking · C");
    expect(lines.indexOf("checking · C"), `${lines}`).toBeLessThan(lines.indexOf("tuning · C"));
    // The selectors went on only after both read-backs confirmed the key.
    const records = await until(
      () => autotuneRecords(since),
      (r) => r.filter((x) => x.step === "select" && x.on === true).length >= 2,
      "both tracks switched on",
    );
    for (const target of ["live_set tracks 6", "live_set tracks 7"]) {
      const confirmed = records.findIndex((r) => r.step === "readback" && r.target === target && r.state === "confirmed");
      const on = records.findIndex((r) => r.step === "select" && r.target === target && r.on === true);
      expect(confirmed, `${target}'s read-back`).toBeGreaterThanOrEqual(0);
      expect(confirmed, `${target}: on only after its key`).toBeLessThan(on);
    }
    const write = records.findIndex((r) => r.step === "write" && r.on.length === 2);
    expect(write, "one selector batch with both").toBeGreaterThan(records.findIndex((r) => r.step === "readback"));
    expect((await hubStatus()).autotune.targets.map((t: any) => t.decision)).toEqual([
      { on: true, why: "play" },
      { on: true, why: "play" },
    ]);
    // The singer off by hand from the panel: both tracks DRY.
    await button.click();
    const panel = page.getByTestId("autotune-panel");
    await expect(panel).toBeVisible();
    const row = panel.getByTestId("autotune-singer");
    await expect(row).toHaveCount(1);
    await expect(row.getByTestId("autotune-chip-stream")).toHaveText("stream TUNE");
    await expect(row.getByTestId("autotune-chip-pa")).toHaveText("PA TUNE");
    await expect(row.getByTestId("autotune-singer-key")).toHaveText("C ✓");
    const sw = row.getByTestId("autotune-singer-switch");
    await expect(sw).toHaveText("OFF");
    await sw.click();
    await macrosAre(live, DRY, "the singer off: both tracks DRY", 5_000);
    const status = await until(
      hubStatus,
      (s: any) => s.autotune.targets.every((t: any) => t.decision?.why === "singer_off"),
      "both tracks singer_off",
    );
    expect(status.autotune.singers_off).toEqual(["VOX1"]);
    await expect(row).toHaveAttribute("data-off", "true");
    await expect(sw).toHaveText("ON");
    await expect(row.getByTestId("autotune-chip-stream")).toHaveText("stream DRY");
    await expect(row.getByTestId("autotune-chip-pa")).toHaveText("PA DRY");
    await expect(button.getByTestId("btn-sub")).toHaveText("tuning · C · 1 off");
    await expect(button).toHaveAttribute("data-state", "off");
    // Back on: it tunes again.
    await sw.click();
    await macrosAre(live, ON, "the singer on again", 5_000);
    await expect(sw).toHaveText("OFF");
    await expect(button.getByTestId("btn-sub")).toHaveText("tuning · C");
    const singers = (await autotuneRecords(since)).filter((r) => r.step === "singer").map((r) => [r.id, r.off]);
    expect(singers).toEqual([
      ["VOX1", true],
      ["VOX1", false],
    ]);
    // AUTOTUNE again closes the panel.
    await button.click();
    await expect(panel).toHaveCount(0);
    // A stop in the middle of the song: DRY at once.
    const stopped = Date.now();
    await live.call("band", "live_set", "stop_playing");
    const dry = await macrosAre(live, DRY, "DRY at the stop", 2_000);
    expect(dry).toEqual(DRY);
    console.log(`autotune: every selector on DRY ${Date.now() - stopped} ms after the stop`);
    await expect(button.getByTestId("btn-sub")).toHaveText("stopped · C");
  });

  test("a tap opens the singers' panel with every singer beside the column; a tap outside it or on AUTOTUNE again closes it; it sends nothing", async ({ page }) => {
    const button = await setUp(page);
    // A target with no rack, naming no singer: a singer of its own, with a
    // problem and no manual off.
    expect(await hostLine("band", "tuner track 8 set '+AUTOTUNE'")).toBe("TUNER 1");
    await targetsFound(3);
    await showsServedLayout(layouts);
    await addSong();
    const since = Date.now();
    const sincePage = await pageNow(page);
    await button.click();
    const panel = page.getByTestId("autotune-panel");
    await expect(panel).toBeVisible();
    await expect(button).toHaveAttribute("data-open", "true");
    // Its head: the song and the key; AUT off, no pill.
    await expect(panel.getByTestId("autotune-panel-song")).toHaveText("song 001 · C");
    await expect(panel.getByTestId("autotune-panel-aut")).toHaveCount(0);
    // Every singer, in Live's order.
    const rows = panel.getByTestId("autotune-singer");
    await expect(rows).toHaveCount(2);
    const vox = rows.nth(0);
    await expect(vox).toHaveAttribute("data-id", "VOX1");
    await expect(vox.locator(".tp-name")).toHaveText("Vox 1");
    await expect(vox.getByTestId("autotune-chip-stream")).toHaveText("stream DRY");
    await expect(vox.getByTestId("autotune-chip-pa")).toHaveText("PA DRY");
    await expect(vox.getByTestId("autotune-singer-key")).toHaveText("C ✓");
    await expect(vox.getByTestId("autotune-singer-switch")).toHaveText("OFF");
    const lone = rows.nth(1);
    await expect(lone).toHaveAttribute("data-id", "");
    await expect(lone).toHaveAttribute("data-ok", "false");
    await expect(lone.getByTestId("autotune-singer-key")).toHaveText("!");
    await expect(lone.getByTestId("autotune-singer-switch")).toHaveCount(0);
    await expect(panel.getByTestId("autotune-singer-why")).toHaveCount(1);
    // Its why: the target's problem (no rack, no singer) in the hub's words.
    await expect(panel.getByTestId("autotune-singer-why")).toHaveText(/^#9: .+ · on DRY, rewrite at the next change$/);
    // The corner: a target with a problem.
    await expect(button).toHaveAttribute("data-problem", "true");
    // Its foot: the song map's counts and Live's bar.
    await expect(panel.getByTestId("autotune-panel-map")).toHaveText("Song map: 1 song with a key, 0 key changes, 0 problems.");
    const bar = (await hubStatus()).autotune.keys.find((k: any) => k.instance === "band")?.bar ?? "—";
    await expect(panel.getByTestId("autotune-panel-live")).toHaveText(`Live: ${bar}. Hold AUTOTUNE 1 s: AUT on/off.`);
    // Beside the column, inside the screen, its switch a tap target.
    const box = (await panel.boundingBox())!;
    const column = (await page.getByTestId("column").boundingBox())!;
    expect(box.x, "right of the column").toBeGreaterThanOrEqual(column.x + column.width);
    expect(box.x + box.width, "inside the screen").toBeLessThanOrEqual(1194);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height, "inside the screen").toBeLessThanOrEqual(834);
    for (const text of await panel.locator(".tp-name, .chip, .tp-key, .tp-sw, .tp-song").all()) {
      expect(await clipped(text), `${await text.textContent()}`).toEqual([]);
    }
    expect(
      await vox.getByTestId("autotune-singer-switch").evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);
    // A tap outside it closes it.
    await tapOutside(page);
    await expect(panel).toHaveCount(0);
    await expect(button).toHaveAttribute("data-open", "false");
    // Open again, then AUTOTUNE again closes it.
    await button.click();
    await expect(panel).toBeVisible();
    await button.click();
    await expect(panel).toHaveCount(0);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "tap").length >= 3)).toEqual(["press", "tap", "close", "press", "tap", "press", "tap"]);
    // Nothing reached the rule: no tap, no AUT switch, no singer.
    const steps = (await autotuneRecords(since)).map((r) => r.step);
    for (const step of ["refused", "tap", "flag", "singer", "write"]) expect(steps).not.toContain(step);
    expect(live.autotune.aut).toBe(false);
    expect((await hubStatus()).autotune.taps).toBe(0);
  });

  test("a tap outside the open panel only closes it: the mute under it is not touched (I1)", async ({ page }) => {
    const button = await setUp(page);
    // A strip on the screen's left half, away from the panel: its mute (the
    // name button) acts on its down.
    const mute = page.getByTestId("strip").first().getByTestId("mute");
    const key: string = JSON.parse((await mute.getAttribute("data-keys"))!)[0];
    const [instance, target] = key.split("|");
    const before = await live.get(instance, target, "mute");
    const since = Date.now();
    const sincePage = await pageNow(page);
    try {
      await button.click();
      const panel = page.getByTestId("autotune-panel");
      await expect(panel).toBeVisible();
      const box = (await panel.boundingBox())!;
      const at = (await mute.boundingBox())!;
      expect(at.x + at.width, "the mute is left of the panel").toBeLessThan(box.x);
      await mute.click();
      await expect(panel).toHaveCount(0);
      await page.waitForTimeout(500);
      expect(await live.get(instance, target, "mute"), "the mute as it was").toBe(before);
      const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since && r.key === key);
      expect(sets, "no write of the mute").toEqual([]);
      expect(await railSteps(sincePage, (st) => st.includes("close"))).toEqual(["press", "tap", "close"]);
    } finally {
      await live.set(instance, target, "mute", before);
    }
  });

  test("a singer's switch acts on a tap, never on its down or a slide (M1)", async ({ page }) => {
    const button = await setUp(page);
    await button.click();
    const panel = page.getByTestId("autotune-panel");
    const sw = panel.getByTestId("autotune-singer-switch");
    await expect(sw).toHaveText("OFF");
    const since = Date.now();
    // A down, then a slide of 30 px (a scroll), then the lift: nothing.
    await dispatchPointer(sw, [{ type: "pointerdown" }, { type: "pointermove", dy: 0 }, { type: "pointermove", dy: 30 }, { type: "pointerup", dy: 30 }]);
    await page.waitForTimeout(500);
    expect((await hubStatus()).autotune.singers_off ?? []).toEqual([]);
    expect((await autotuneRecords(since)).filter((r) => r.step === "singer")).toEqual([]);
    await expect(sw).toHaveText("OFF");
    // A down alone switches nothing; its lift does.
    await dispatchPointer(sw, [{ type: "pointerdown" }, { wait: 300 }]);
    await page.waitForTimeout(300);
    expect((await hubStatus()).autotune.singers_off ?? []).toEqual([]);
    await dispatchPointer(sw, [{ type: "pointerup" }]);
    await until(async () => (await hubStatus()).autotune.singers_off ?? [], (v: string[]) => v.length === 1, "the singer off at the lift");
    await expect(sw).toHaveText("ON");
    // A real tap puts it back on.
    await sw.click();
    await until(async () => (await hubStatus()).autotune.singers_off ?? [], (v: string[]) => v.length === 0, "the singer on again");
    expect((await autotuneRecords(since)).filter((r) => r.step === "singer").map((r) => r.off)).toEqual([true, false]);
  });

  test("a hold on a red AUTOTUNE does nothing: no fill, AUT stays off (M2)", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    layouts = watchLayouts(page);
    await openSurface(page);
    await harness("/hub/layout", { layout: withAutotuneButton() });
    await selectPage(page, "foh");
    const button = page.getByTestId("autotune");
    // No tagged track (I5): red.
    await expect(button).toHaveAttribute("data-binding", "unresolved", { timeout: 10_000 });
    const since = Date.now();
    const press = await dispatchSnapped(button, [{ type: "pointerdown" }, { wait: 400 }, { snap: true }, { wait: 800 }, { snap: true }]);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    expect(press.snaps[0]).toMatchObject({ holding: "false", full: "false" });
    expect(press.snaps[1]).toMatchObject({ holding: "false", full: "false" });
    await page.waitForTimeout(500);
    expect(live.autotune?.aut ?? false).toBe(false);
    expect((await hubStatus()).autotune.aut).toBe(false);
    expect((await autotuneRecords(since)).filter((r) => r.step === "flag")).toEqual([]);
    // A tap on it opens nothing either.
    await button.click();
    await expect(page.getByTestId("autotune-panel")).toHaveCount(0);
  });

  test("a mismatch shows the yellow corner and its why in the panel", async ({ page }) => {
    const button = await setUp(page);
    // The stream's Key and Scale held as a Max map holds them: its key write
    // is refused (one retry), the PA's reads back.
    for (const path of [STREAM_KEY, STREAM_SCALE]) expect(await hostLine("band", `param ${path} enabled 0`)).toBe("PARAM 0");
    expect(await hostLine("band", SONG[0])).toBe("CUE 1");
    expect(await hostLine("band", SONG[1])).toBe("CUE 2");
    await keysStand("C", ["confirmed", "refused"]);
    await expect(button).toHaveAttribute("data-problem", "true");
    const flag = button.getByTestId("rule-flag");
    await expect(flag).toHaveText("!");
    // The corner inside the button, left of the label: it takes no width.
    const place = await button.evaluate((el) => {
      const range = document.createRange();
      range.selectNodeContents(el.querySelector(".btn-text")!);
      return {
        box: el.getBoundingClientRect(),
        flag: el.querySelector(".rule-flag")!.getBoundingClientRect(),
        text: range.getBoundingClientRect(),
      };
    });
    expect(place.flag.width).toBeCloseTo(11, 0);
    expect(place.flag.left).toBeGreaterThanOrEqual(place.box.left);
    expect(place.flag.top).toBeGreaterThanOrEqual(place.box.top);
    expect(place.flag.right, "the corner left of the label").toBeLessThanOrEqual(place.text.left);
    expect(await clipped(button)).toEqual([]);
    // AUT on and Live playing in the song: the PA tunes, the stream stays DRY.
    live.setHub("autotune_aut", true);
    await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
    await live.call("band", "live_set", "start_playing");
    await macrosAre(live, [ON[0], DRY[1]], "the PA tuning, the stream DRY");
    await expect(button.getByTestId("btn-sub")).toHaveText("tuning · C · 1 !");
    await expect(button).toHaveAttribute("data-state", "mixed");
    await expect(button).toHaveAttribute("data-mixed", "true");
    expect(await clipped(button)).toEqual([]);
    await button.click();
    const panel = page.getByTestId("autotune-panel");
    const row = panel.getByTestId("autotune-singer");
    await expect(row.getByTestId("autotune-singer-key")).toHaveText("!");
    await expect(row.getByTestId("autotune-chip-stream")).toHaveText("stream DRY");
    await expect(row.getByTestId("autotune-chip-pa")).toHaveText("PA TUNE");
    await expect(panel.getByTestId("autotune-singer-why")).toHaveText("Vox 1: stream Key refused, wrote C · on DRY, rewrite at the next change");
    expect((await hubStatus()).autotune.targets.map((t: any) => t.decision?.why)).toEqual(["play", "mismatch"]);
  });

  for (const [width, height] of [
    [844, 390],
    [390, 844],
  ]) {
    test.describe(`on a phone, ${width} × ${height}`, () => {
      test.use({ viewport: { width, height } });

      test("the AUTOTUNE button and its panel fit the phone: the AUT dot and the corner inside the button, no second line, every rail button whole", async ({ page }) => {
        await live.call("band", "live_set", "start_playing");
        layouts = watchLayouts(page);
        await openSurface(page);
        await harness("/hub/layout", { layout: withAutotuneButton() });
        const button = page.getByTestId("autotune");
        await expect(button).toHaveCount(1, { timeout: 10_000 });
        await tagSinger();
        expect(await hostLine("band", "tuner track 8 set '+AUTOTUNE'")).toBe("TUNER 1");
        await targetsFound(3);
        await showsServedLayout(layouts);
        live.setHub("autotune_aut", true);
        await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
        // The phone's column behind ☰ (#63).
        await page.getByTestId("thin-burger").dispatchEvent("pointerdown");
        await expect(page.getByTestId("page")).toHaveAttribute("data-column-open", "true");
        await expect(page.getByTestId("column")).toBeVisible();
        await expect(button).toHaveAttribute("data-aut", "run");
        await expect(button).toHaveAttribute("data-problem", "true");
        const place = await button.evaluate((el) => {
          const range = document.createRange();
          range.selectNodeContents(el.querySelector(".btn-text")!);
          return {
            box: el.getBoundingClientRect(),
            flag: el.querySelector(".rule-flag")!.getBoundingClientRect(),
            dot: el.querySelector(".aut-badge")!.getBoundingClientRect(),
            text: range.getBoundingClientRect(),
          };
        });
        for (const [name, mark] of [
          ["the problem corner", place.flag],
          ["the AUT dot", place.dot],
        ] as const) {
          expect(mark.width, `${name}: 6 px`).toBeCloseTo(6, 0);
          expect(mark.height, `${name}: 6 px`).toBeCloseTo(6, 0);
          expect(mark.left, `${name} inside the button`).toBeGreaterThanOrEqual(place.box.left);
          expect(mark.right, `${name} inside the button`).toBeLessThanOrEqual(place.box.right);
          expect(mark.top, `${name} inside the button`).toBeGreaterThanOrEqual(place.box.top);
          expect(mark.bottom, `${name} inside the button`).toBeLessThanOrEqual(place.box.bottom);
        }
        expect(place.flag.right, "the corner left of the label's text").toBeLessThanOrEqual(place.text.left);
        expect(place.dot.left, "the dot right of the label's text").toBeGreaterThanOrEqual(place.text.right);
        await expect(button.getByTestId("btn-sub")).toBeHidden();
        // Every rail button whole, inside the rail's part that holds it, on screen.
        const rail = page.getByTestId("rail");
        for (const part of [".rail-main", ".rail-foot"]) {
          const area = (await rail.locator(part).boundingBox())!;
          for (const each of await rail.locator(`${part} .btn`).all()) {
            const name = await each.textContent();
            expect(await clipped(each), `rail button ${name}`).toEqual([]);
            const box = (await each.boundingBox())!;
            expect(box.y, `rail button ${name}: top inside ${part}`).toBeGreaterThanOrEqual(area.y - 0.5);
            expect(box.y + box.height, `rail button ${name}: bottom inside ${part}`).toBeLessThanOrEqual(area.y + area.height + 0.5);
            expect(box.y + box.height, `rail button ${name} on screen`).toBeLessThanOrEqual(height);
          }
        }
        // The panel: upright at the screen's full width, on its side beside
        // the column; inside the screen, nothing wider than it.
        await button.click();
        const panel = page.getByTestId("autotune-panel");
        await expect(panel).toBeVisible();
        await expect(panel.getByTestId("autotune-singer")).toHaveCount(2);
        const box = (await panel.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width, "inside the screen").toBeLessThanOrEqual(width + 0.5);
        expect(box.y).toBeGreaterThanOrEqual(0);
        expect(box.y + box.height, "inside the screen").toBeLessThanOrEqual(height + 0.5);
        const column = (await page.getByTestId("column").boundingBox())!;
        if (width > height) {
          expect(box.x, "beside the column").toBeGreaterThanOrEqual(column.x + column.width);
        } else {
          expect(box.width, "the screen's width less its 8 px edges").toBeCloseTo(width - 16, 0);
        }
        const wide = await panel.evaluate((el) => {
          const out: string[] = [];
          if (el.scrollWidth > el.clientWidth + 0.5) out.push(`the panel: ${el.scrollWidth} > ${el.clientWidth}`);
          for (const part of el.querySelectorAll(".tp-row > *, .chip")) {
            const r = part.getBoundingClientRect();
            const p = el.getBoundingClientRect();
            if (r.right > p.right + 0.5 || r.left < p.left - 0.5) out.push(`${part.textContent}: outside the panel`);
            if ((part as HTMLElement).scrollWidth > (part as HTMLElement).clientWidth + 0.5) out.push(`${part.textContent}: wider than its box`);
          }
          return out;
        });
        expect(wide).toEqual([]);
      });
    });
  }

  test("after a reconnect, until the hub tells AUTOTUNE's state again, a tap and a hold are refused and send nothing", async ({ page }) => {
    // The page's sockets: the first as it is, every later one without the
    // hub's `autotune` messages (the hub has not told this connection yet).
    const sockets: WebSocketRoute[] = [];
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      sockets.push(ws);
      const server = ws.connectToServer();
      if (sockets.length === 1) return;
      server.onMessage((message) => {
        if (JSON.parse(String(message)).type !== "autotune") ws.send(message);
      });
    });
    const button = await setUp(page);
    // The panel open before the drop closes with it (no stale singer).
    await button.click();
    await expect(page.getByTestId("autotune-panel")).toBeVisible();
    // The socket drops (a Wi-Fi blip): the page reconnects.
    await sockets[0].close({ code: 4000, reason: "gone" });
    await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await expect(page.getByTestId("autotune-panel")).toHaveCount(0);
    await expect(button).toHaveAttribute("data-open", "false");
    // The last state stays shown (stale): ready, off.
    await expect(button).toHaveAttribute("data-binding", "ready");
    await expect(button).toHaveAttribute("data-state", "off");
    const since = Date.now();
    const sincePage = await pageNow(page);
    // Every `failed` flash observed, never sampled (it lasts 400 ms).
    await button.evaluate((el) => {
      const w = window as any;
      w.tuneFlashes = 0;
      let on = el.classList.contains("failed");
      w.tuneWatch = new MutationObserver(() => {
        const now = el.classList.contains("failed");
        if (now && !on) w.tuneFlashes += 1;
        on = now;
      });
      w.tuneWatch.observe(el, { attributes: true, attributeFilter: ["class"] });
    });
    // A tap: refused with a flash, no panel.
    await button.click();
    await expect.poll(() => page.evaluate(() => (window as any).tuneFlashes)).toBe(1);
    await expect(page.getByTestId("autotune-panel")).toHaveCount(0);
    // A hold: no fill, refused at the 1 s mark, its lift does nothing.
    const press = await dispatchSnapped(button, [
      { type: "pointerdown" },
      { wait: 400 },
      { snap: true },
      { wait: 1100 },
      { snap: true },
      { type: "pointerup" },
      { wait: 100 },
    ]);
    expect(press.snaps[0]).toMatchObject({ holding: "false", full: "false" });
    expect(press.snaps[0].sub).not.toMatch(/drž|pusť/);
    expect(press.snaps[1].full).toBe("false");
    await expect.poll(() => page.evaluate(() => (window as any).tuneFlashes)).toBe(2);
    await page.waitForTimeout(600);
    expect(await page.evaluate(() => (window as any).tuneFlashes)).toBe(2);
    await page.evaluate(() => (window as any).tuneWatch.disconnect());
    await expect(page.getByTestId("autotune-panel")).toHaveCount(0);
    expect(await macros(live)).toEqual(DRY);
    expect(live.autotune.aut).toBe(false);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "refused").length >= 2)).toEqual([
      "press",
      "refused",
      "press",
      "refused",
    ]);
    // Nothing reached the rule: no tap, no AUT switch, no singer since.
    const steps = (await autotuneRecords(since)).map((r) => r.step);
    for (const step of ["refused", "tap", "flag", "singer"]) expect(steps).not.toContain(step);
  });
});
