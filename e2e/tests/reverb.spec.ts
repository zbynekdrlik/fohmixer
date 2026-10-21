import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  HUB_SOCKET,
  LiveClient,
  clipped,
  dispatchPointer,
  dispatchSnapped,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  pageEvents,
  ready,
  selectPage,
  strip as stripByTrack,
  until,
  value2db,
} from "./support/live";

// REVERB and REVERB AUT on the surface (#130, D18; mockup reverb-link-v2):
// the REVERB rail button (right after STAGE) lowers both reverb faders
// 10 dB on a tap and returns them exactly on the next; its 1 s hold turns
// REVERB AUT on or off at its lift (as the merged STAGE button's). The hub
// runs the rule (PR 4): Live's stop lowers them after 2 s, its start
// returns them, the hand wins. The faders show where they return (the
// dashed mark), `AUT −10` and `RUKA`; nothing takes a fader's height. The
// reverb faders are the `+REVERB` Tuners: Rev ST on band track 2 (Hand3 #),
// Rev PA on track 3 (Hand4 #), in Live's order.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
/** SimLive's band set: Rev ST on Hand3 # (track 2), Rev PA on Hand4 # (track 3). */
const ST = "live_set tracks 2 mixer_device volume";
const PA = "live_set tracks 3 mixer_device volume";
/** 10 dB down, within this (the rule's glide ends on Live's value of its dB law). */
const DB_CLOSE = 0.05;

/** The fixture with the REVERBS tags group and the REVERB button right after STAGE on the FOH rail. */
function withReverbButton(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "reverbs", title: "REVERBS", tags: "REVERBS" });
  foh.rail.splice(foh.rail.findIndex((c: any) => c.kind === "stage") + 1, 0, { kind: "reverb", label: "REVERB" });
  return frame;
}

/** Live's value of `target` once two reads 50 ms apart agree (within 8 s). */
async function settled(live: LiveClient, target: string): Promise<number> {
  const deadline = Date.now() + 8000;
  let last = await live.get("band", target, "value");
  for (;;) {
    await new Promise((r) => setTimeout(r, 50));
    const now = await live.get("band", target, "value");
    if (now === last) return now;
    if (Date.now() > deadline) throw new Error(`${target} never settled; last ${now}`);
    last = now;
  }
}

/** Whether `v` is `from` lowered by 10 dB on the strips' law. */
const tenDown = (from: number) => (v: number) => Math.abs(value2db(v) - (value2db(from) - 10)) < DB_CLOSE;

/** A strip of the REVERBS group by its label. */
const strip = (page: Page, label: string): Locator =>
  page.locator('.slot[data-group="reverbs"] [data-testid="strip"]').and(page.locator(`[data-label="${label}"]`));

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the markers gone", 10_000);
}

/** The page clock (the flight recorder's `t`) now. */
function pageNow(page: Page): Promise<number> {
  return page.evaluate(() => performance.timeOrigin + performance.now());
}

/** The REVERB button's `rail` steps the page recorded since `since`, once `ok` holds. */
async function railSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.button === "reverb" && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the REVERB rail steps uploaded",
    15_000,
  );
}

/** The hub's `reverb` steps since `since` (the test's clock: one runner clock with the hub's `ts`). */
async function reverbSteps(since: number): Promise<string[]> {
  return (await hubEvents()).filter((r: any) => r.ev === "reverb" && r.ts >= since).map((r: any) => r.step);
}

/** The button's second line reads `text`, and the button, its line and its label with the pill fit (118 × 44, #9). */
async function fits(button: Locator, text: string) {
  const sub = button.getByTestId("btn-sub");
  await expect(sub).toHaveText(text);
  expect(await clipped(button), `the button with "${text}"`).toEqual([]);
  expect(await clipped(sub), `its line "${text}"`).toEqual([]);
  // The label and the pill side by side within the row (a row is no
  // clipping box: its glyphs' content area may pass its line box).
  const row = await button.locator(".btn-row").evaluate((el) => [el.scrollWidth, el.clientWidth]);
  expect(row[0], `its label and pill side by side with "${text}"`).toBeLessThanOrEqual(row[1]);
}

/**
 * The fader travel of both reverb strips equals an unmarked strip's of the
 * same row (master's Hand4 #, the HANDS group: one instance, no tag) and is
 * the iPad's 322 px (fader height first: nothing REVERB adds takes any).
 */
async function travelsPinned(page: Page, state: string) {
  const reference = (await stripByTrack(page, "Hand4 #", "master").getByTestId("fader").boundingBox())!.height;
  const travels: [string, number][] = [];
  for (const label of ["Rev ST", "Rev PA"]) {
    travels.push([label, (await strip(page, label).getByTestId("fader").boundingBox())!.height]);
  }
  console.log(`reverb travel (${test.info().project.name}, ${state}): ${travels.map(([l, h]) => `${l} ${h}`).join(", ")}, Hand4 # ${reference} px`);
  for (const [label, travel] of travels) {
    expect(travel, `${label}'s travel equals an unmarked strip's (${state})`).toBeCloseTo(reference, 0);
    expect(Math.abs(travel - 322), `${label}'s travel is 322 px (${state}): ${travel}`).toBeLessThanOrEqual(1);
  }
}

/** A 1 s hold, full at 1.2 s (read in the page before the lift, PR #133's review M1), then its lift. */
async function hold(button: Locator, line: string, live: LiveClient, aut: boolean) {
  const press = await dispatchSnapped(button, [{ type: "pointerdown" }, { wait: 1200 }, { snap: true }]);
  expect(press.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: line, failed: false });
  await expect(button).toHaveAttribute("data-full", "true");
  await fits(button, line);
  expect(live.reverb.aut, "nothing before the lift").toBe(!aut);
  await dispatchPointer(button, [{ type: "pointerup" }]);
  await until(async () => live.reverb.aut, (v: boolean) => v === aut, `REVERB AUT ${aut ? "on" : "off"}`);
  await expect(button).toHaveAttribute("data-full", "false");
}

test.describe("REVERB and REVERB AUT", () => {
  // The iPad's size (#130 PC check: the rail's buttons 118 × 44, nothing
  // clipped at 1194 × 834) in both projects.
  test.use({ viewport: { width: 1194, height: 834 } });

  let live: LiveClient;
  let before: number[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
    before = [await live.get("band", ST, "value"), await live.get("band", PA, "value")];
  });
  test.afterEach(async () => {
    // REVERB AUT off first, so the stop lowers nothing; the faders back up
    // (a return glides 0.5 s: its last writes would land on the volumes put
    // back, so wait until the rule holds nothing and Live has settled).
    live.setHub("reverb_aut", false);
    live.setHub("reverb_low", false);
    await live.call("band", "live_set", "stop_playing");
    await until(async () => (await hubStatus()).reverb.low ?? null, (low) => low === null, "REVERB at the song level");
    await settled(live, ST);
    await settled(live, PA);
    await live.set("band", ST, "value", before[0]);
    await live.set("band", PA, "value", before[1]);
    await hostLine("band", "tuner track 2 remove");
    await hostLine("band", "tuner track 3 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    live.close();
  });

  /** The REVERB button on the FOH rail, red until Rev ST and Rev PA are tagged `+REVERB`, then ready. */
  async function setUp(page: Page): Promise<Locator> {
    await openSurface(page);
    await harness("/hub/layout", { layout: withReverbButton() });
    await selectPage(page, "foh");
    const button = page.getByTestId("reverb");
    // No reverb fader yet (I5): red and disabled.
    await expect(button).toHaveAttribute("data-binding", "unresolved", { timeout: 10_000 });
    await expect(button).toHaveAttribute("aria-disabled", "true");
    expect(await hostLine("band", `tuner track 2 set '"Rev ST" +G:REVERBS +REVERB'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 3 set '"Rev PA" +G:REVERBS +REVERB'`)).toBe("TUNER 1");
    await expect(strip(page, "Rev PA")).toHaveCount(1, { timeout: 10_000 });
    await expect(strip(page, "Rev ST")).toHaveCount(1);
    await ready(strip(page, "Rev ST").getByTestId("fader"));
    await ready(strip(page, "Rev PA").getByTestId("fader"));
    await ready(button);
    await expect(button).toHaveAttribute("data-binding", "ready");
    return button;
  }

  test("a tap lowers both reverbs 10 dB with their marks, the next returns them exactly; no height", async ({ page }) => {
    await live.set("band", ST, "value", 0.85);
    await live.set("band", PA, "value", 0.6);
    const button = await setUp(page);
    const since = await pageNow(page);
    // Right after STAGE on the rail, in the rule's colour, lit at the song
    // level, its touches its own (#43 PR G).
    const rail = page.getByTestId("rail").locator(".rail-main > *");
    expect(await rail.nth(0).getAttribute("data-testid")).toBe("stage-mics");
    expect(await rail.nth(1).getAttribute("data-testid")).toBe("reverb");
    await expect(button.locator(".btn-text")).toHaveText("REVERB");
    // The rail's 118 × 44 (#130 PC check; review M4).
    const size = (await button.boundingBox())!;
    console.log(`reverb button (${test.info().project.name}): ${size.width} × ${size.height} px`);
    expect(Math.abs(size.width - 118), `118 px wide: ${size.width}`).toBeLessThanOrEqual(1);
    expect(Math.abs(size.height - 44), `44 px high: ${size.height}`).toBeLessThanOrEqual(1);
    await travelsPinned(page, "song level");
    await expect(button).toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button).toHaveAttribute("data-low", "false");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    expect(
      await button.evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);
    for (const label of ["Rev ST", "Rev PA"]) {
      for (const id of ["aut-span", "aut-ret", "aut-mark", "hand-mark"]) {
        await expect(strip(page, label).getByTestId(id)).not.toHaveClass(/\bshown\b/);
      }
      await expect(strip(page, label).getByTestId("strip-readout")).toHaveAttribute("data-aut", "");
    }
    const fader = strip(page, "Rev ST").getByTestId("fader");
    const travel = (await fader.boundingBox())!.height;
    // Where the cap stands at the song level: the return mark goes there.
    const capAt = async () => {
      const box = (await fader.getByTestId("fader-cap").boundingBox())!;
      return box.y + box.height / 2;
    };
    const songLevel = await capAt();

    // A tap on the lift: both glide 10 dB down.
    await button.click();
    await until(() => settled(live, ST), tenDown(0.85), "Rev ST −10 dB");
    await until(() => settled(live, PA), tenDown(0.6), "Rev PA −10 dB");
    await expect(button).not.toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-low", "true");
    await expect(button).toHaveAttribute("data-aut", "off");
    await fits(button, "ručne · −10 dB");
    await expect(button.getByTestId("btn-sub")).toHaveClass(/\blow\b/);
    for (const label of ["Rev ST", "Rev PA"]) {
      const s = strip(page, label);
      await expect(s.getByTestId("aut-span")).toHaveClass(/\bshown\b/);
      await expect(s.getByTestId("aut-ret")).toHaveClass(/\bshown\b/);
      await expect(s.getByTestId("aut-mark")).toHaveClass(/\bshown\b/);
      await expect(s.getByTestId("aut-mark")).toHaveText("AUT −10");
      await expect(s.getByTestId("hand-mark")).not.toHaveClass(/\bshown\b/);
      await expect(s.getByTestId("strip-readout")).toHaveAttribute("data-aut", "low");
    }
    // The dashed mark at the song level; the cap 10 dB under it; the
    // marks take no touch and no height.
    const ret = (await strip(page, "Rev ST").getByTestId("aut-ret").boundingBox())!;
    expect(Math.abs(ret.y + ret.height / 2 - songLevel), "the return mark at the song level").toBeLessThan(2);
    expect(await capAt(), "the cap under its return mark").toBeGreaterThan(songLevel + 10);
    expect(await strip(page, "Rev ST").getByTestId("aut-mark").evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
    expect((await fader.boundingBox())!.height, "no height").toBeCloseTo(travel, 0);
    await travelsPinned(page, "lowered");

    // The next tap: both exactly back.
    await button.click();
    await until(() => settled(live, ST), (v) => v === 0.85, "Rev ST back exactly");
    await until(() => settled(live, PA), (v) => v === 0.6, "Rev PA back exactly");
    await expect(button).toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-low", "false");
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    for (const label of ["Rev ST", "Rev PA"]) {
      await expect(strip(page, label).getByTestId("aut-ret")).not.toHaveClass(/\bshown\b/);
      await expect(strip(page, label).getByTestId("aut-mark")).not.toHaveClass(/\bshown\b/);
      await expect(strip(page, label).getByTestId("strip-readout")).toHaveAttribute("data-aut", "");
    }
    expect(await railSteps(since, (s) => s.filter((w) => w === "tap").length >= 2)).toEqual(["press", "tap", "press", "tap"]);
  });

  test("REVERB AUT: a 1 s hold and its lift turn it on, Live's stop lowers after 2 s, its start returns, the hand wins until the next stop", async ({ page }) => {
    await live.set("band", ST, "value", 0.85);
    await live.set("band", PA, "value", 0.6);
    await live.call("band", "live_set", "start_playing");
    const button = await setUp(page);
    const sincePage = await pageNow(page);
    await hold(button, "pusť — zapne AUT", live, true);
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("aut-badge")).toHaveText("AUT");
    await fits(button, "hrá · plný");
    // A tap during the song: lowered until the next stop, AUT stays on
    // (outlined); the next tap returns them.
    await button.click();
    await until(() => settled(live, ST), tenDown(0.85), "Rev ST lowered by the tap");
    await expect(button).toHaveAttribute("data-aut", "held");
    await fits(button, "ručne −10 · do zast.");
    await button.click();
    await until(() => settled(live, ST), (v) => v === 0.85, "Rev ST back by the tap");
    await until(() => settled(live, PA), (v) => v === 0.6, "Rev PA back by the tap");
    await expect(button).toHaveAttribute("data-aut", "run");
    await fits(button, "hrá · plný");

    // Live stops: 2 s of the last chord's tail, then 10 dB down over 2 s.
    const since = Date.now();
    await live.call("band", "live_set", "stop_playing");
    await fits(button, "stojí · plný");
    await expect(button.getByTestId("btn-sub")).toHaveText("stojí · −10 dB", { timeout: 4000 });
    await until(() => settled(live, ST), tenDown(0.85), "Rev ST lowered", 6000);
    await until(() => settled(live, PA), tenDown(0.6), "Rev PA lowered", 6000);
    await fits(button, "stojí · −10 dB");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(strip(page, "Rev PA").getByTestId("aut-mark")).toHaveText("AUT −10");

    // The engineer takes Rev PA: the rule lets it go (RUKA), its return
    // mark goes, Rev ST stays held.
    const fader = strip(page, "Rev PA").getByTestId("fader");
    await dispatchPointer(fader, [
      { type: "pointerdown" },
      { type: "pointermove", dy: 0 },
      { type: "pointermove", dy: 20 },
      { type: "pointerup", dy: 20 },
    ]);
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).toHaveClass(/\bshown\b/);
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).toHaveText("RUKA");
    await expect(strip(page, "Rev PA").getByTestId("aut-ret")).not.toHaveClass(/\bshown\b/);
    await expect(strip(page, "Rev PA").getByTestId("aut-mark")).not.toHaveClass(/\bshown\b/);
    await expect(strip(page, "Rev PA").getByTestId("strip-readout")).toHaveAttribute("data-aut", "");
    await expect(strip(page, "Rev ST").getByTestId("aut-ret")).toHaveClass(/\bshown\b/);
    // The button names no fader in the hand (the owner's ruling on #130 of
    // 2026-10-21): the faders show it; the line stays REVERB's own.
    await fits(button, "stojí · −10 dB");
    await expect(button).toHaveAttribute("data-aut", "run");
    const handLevel = await settled(live, PA);

    // Live plays: Rev ST back exactly, Rev PA stays where the hand left it,
    // RUKA through the song (F30: let go until the next stop).
    await live.call("band", "live_set", "start_playing");
    await until(() => settled(live, ST), (v) => v === 0.85, "Rev ST back exactly");
    await page.waitForTimeout(800);
    expect(await settled(live, PA), "the hand wins").toBe(handLevel);
    await expect(strip(page, "Rev ST").getByTestId("aut-ret")).not.toHaveClass(/\bshown\b/);
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).toHaveClass(/\bshown\b/);
    await fits(button, "hrá · plný");

    // A second hold turns it off at its lift.
    await hold(button, "pusť — vypne AUT", live, false);
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).toHaveClass(/\bshown\b/);
    // The next stop ends RUKA, with AUT off too, and lowers nothing.
    await live.call("band", "live_set", "stop_playing");
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).not.toHaveClass(/\bshown\b/);
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    expect(await settled(live, PA)).toBe(handLevel);
    expect(await settled(live, ST)).toBe(0.85);
    const steps = await until(
      () => reverbSteps(since),
      (s) => ["wait", "lower", "hand", "return", "aut"].every((w) => s.includes(w)),
      "the rule's steps in the event log",
      15_000,
    );
    for (const want of ["wait", "lower", "hand", "return", "aut"]) expect(steps).toContain(want);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "hold").length >= 2)).toEqual([
      "press",
      "full",
      "hold",
      "press",
      "tap",
      "press",
      "tap",
      "press",
      "full",
      "hold",
    ]);
  });

  test("a tap back while Live stands holds them full until the start; AUT off while lowered leaves them low, and a tap returns them exactly", async ({ page }) => {
    await live.set("band", ST, "value", 0.85);
    await live.set("band", PA, "value", 0.6);
    await live.call("band", "live_set", "start_playing");
    live.setHub("reverb_aut", true);
    await until(async () => live.reverb?.aut, (v: boolean) => v === true, "REVERB AUT on");
    const button = await setUp(page);
    await expect(button).toHaveAttribute("data-aut", "run");
    await live.call("band", "live_set", "stop_playing");
    await until(() => settled(live, ST), tenDown(0.85), "lowered", 6000);
    await fits(button, "stojí · −10 dB");
    // A tap back while Live stands: full until the start (outlined), back
    // exactly; a tap down again is the stop's lowering.
    await button.click();
    await until(() => settled(live, ST), (v) => v === 0.85, "back by the tap");
    await until(() => settled(live, PA), (v) => v === 0.6, "Rev PA back by the tap");
    await expect(button).toHaveAttribute("data-aut", "held");
    await fits(button, "ručne plný · do štartu");
    await button.click();
    await until(() => settled(live, ST), tenDown(0.85), "lowered by the tap", 6000);
    await fits(button, "stojí · −10 dB");
    await expect(button).toHaveAttribute("data-aut", "run");
    // AUT off while lowered: the faders stay, the lowering is a tap's.
    live.setHub("reverb_aut", false);
    await fits(button, "ručne · −10 dB");
    await expect(button).toHaveAttribute("data-aut", "off");
    await page.waitForTimeout(600);
    expect(Math.abs(value2db(await settled(live, ST)) - (value2db(0.85) - 10))).toBeLessThan(DB_CLOSE);
    await expect(strip(page, "Rev ST").getByTestId("aut-ret")).toHaveClass(/\bshown\b/);
    await button.click();
    await until(() => settled(live, ST), (v) => v === 0.85, "back exactly");
    await until(() => settled(live, PA), (v) => v === 0.6, "Rev PA back exactly");
    await expect(button).toHaveClass(/\bon\b/);
  });

  test("the channel detail's fader shows REVERB's return mark, AUT −10 and RUKA, with no height", async ({ page }) => {
    await live.set("band", ST, "value", 0.85);
    await live.set("band", PA, "value", 0.6);
    await setUp(page);
    // Rev PA (band track 3: `#4` by index) in its channel detail, which
    // covers the strips and the rail.
    const detail = await openDetail(page, "#4");
    const fader = detail.getByTestId("fader");
    await ready(fader);
    for (const id of ["aut-span", "aut-ret", "aut-mark", "hand-mark"]) {
      await expect(detail.getByTestId(id), `the detail's ${id}`).toHaveCount(1);
      await expect(detail.getByTestId(id)).not.toHaveClass(/\bshown\b/);
    }
    await expect(detail.getByTestId("detail-readout")).toHaveAttribute("data-aut", "");
    const travel = (await fader.boundingBox())!.height;
    const cap = async () => {
      const box = (await fader.getByTestId("fader-cap").boundingBox())!;
      return box.y + box.height / 2;
    };
    const songLevel = await cap();

    // Lowered (a page's tap, here the test's client: the detail covers the
    // rail): the return mark at the song level, AUT −10, the readout in the
    // rule's colour, no height.
    live.setHub("reverb_low", true);
    await until(() => settled(live, PA), tenDown(0.6), "Rev PA −10 dB");
    await expect(detail.getByTestId("aut-span")).toHaveClass(/\bshown\b/);
    await expect(detail.getByTestId("aut-ret")).toHaveClass(/\bshown\b/);
    await expect(detail.getByTestId("aut-mark")).toHaveClass(/\bshown\b/);
    await expect(detail.getByTestId("aut-mark")).toHaveText("AUT −10");
    await expect(detail.getByTestId("detail-readout")).toHaveAttribute("data-aut", "low");
    const ret = (await detail.getByTestId("aut-ret").boundingBox())!;
    expect(Math.abs(ret.y + ret.height / 2 - songLevel), "the return mark at the song level").toBeLessThan(2);
    expect(await cap(), "the cap under its return mark").toBeGreaterThan(songLevel + 10);
    expect((await fader.boundingBox())!.height, "no height").toBeCloseTo(travel, 0);

    // Dragged in the detail: the rule lets it go, RUKA there and on the strip.
    await dispatchPointer(fader, [
      { type: "pointerdown" },
      { type: "pointermove", dy: 0 },
      { type: "pointermove", dy: 20 },
      { type: "pointerup", dy: 20 },
    ]);
    await until(async () => live.reverb?.faders?.[1]?.hand, (v: boolean) => v === true, "the hub let Rev PA go");
    await expect(detail.getByTestId("hand-mark")).toHaveClass(/\bshown\b/);
    await expect(detail.getByTestId("hand-mark")).toHaveText("RUKA");
    await expect(detail.getByTestId("aut-ret")).not.toHaveClass(/\bshown\b/);
    await expect(detail.getByTestId("detail-readout")).toHaveAttribute("data-aut", "");
    expect((await fader.boundingBox())!.height, "no height").toBeCloseTo(travel, 0);
    await detail.getByTestId("detail-exit").click();
    await expect(strip(page, "Rev PA").getByTestId("hand-mark")).toHaveClass(/\bshown\b/);
    await expect(strip(page, "Rev ST").getByTestId("aut-ret")).toHaveClass(/\bshown\b/);
  });

  for (const [width, height] of [
    [844, 390],
    [390, 844],
  ]) {
    test.describe(`on a phone, ${width} × ${height}`, () => {
      test.use({ viewport: { width, height } });

      test("the REVERB button fits the phone's rail: its AUT pill a dot beside the label, no second line, every rail button whole", async ({ page }) => {
        await live.call("band", "live_set", "start_playing");
        await openSurface(page);
        await harness("/hub/layout", { layout: withReverbButton() });
        const button = page.getByTestId("reverb");
        await expect(button).toHaveCount(1, { timeout: 10_000 });
        expect(await hostLine("band", `tuner track 2 set '"Rev ST" +G:REVERBS +REVERB'`)).toBe("TUNER 1");
        expect(await hostLine("band", `tuner track 3 set '"Rev PA" +G:REVERBS +REVERB'`)).toBe("TUNER 1");
        await expect(button).toHaveAttribute("data-binding", "ready", { timeout: 10_000 });
        // REVERB AUT on while Live plays (nothing lowers).
        live.setHub("reverb_aut", true);
        await until(async () => live.reverb?.aut, (v: boolean) => v === true, "REVERB AUT on");
        // The phone's column behind ☰ (#63).
        await page.getByTestId("thin-burger").dispatchEvent("pointerdown");
        await expect(page.getByTestId("page")).toHaveAttribute("data-column-open", "true");
        await expect(page.getByTestId("column")).toBeVisible();
        await expect(button).toHaveAttribute("data-aut", "run");
        const place = await button.evaluate((el) => {
          const box = el.getBoundingClientRect();
          const dot = el.querySelector(".aut-badge")!.getBoundingClientRect();
          const range = document.createRange();
          range.selectNodeContents(el.querySelector(".btn-text")!);
          const text = range.getBoundingClientRect();
          return { box, dot, text };
        });
        // A 6 px dot inside the button, right of the label's text.
        expect(place.dot.width).toBeCloseTo(6, 0);
        expect(place.dot.height).toBeCloseTo(6, 0);
        expect(place.dot.left).toBeGreaterThanOrEqual(place.box.left);
        expect(place.dot.right).toBeLessThanOrEqual(place.box.right);
        expect(place.dot.top).toBeGreaterThanOrEqual(place.box.top);
        expect(place.dot.bottom).toBeLessThanOrEqual(place.box.bottom);
        expect(place.dot.left, "the dot beside the label's text").toBeGreaterThanOrEqual(place.text.right);
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
      });
    });
  }

  test("after a reconnect, until the hub tells REVERB's state again, a tap and a hold are refused and send nothing", async ({ page }) => {
    await live.set("band", ST, "value", 0.85);
    await live.set("band", PA, "value", 0.6);
    // The page's sockets: the first as it is, every later one without the
    // hub's `reverb` messages (the hub has not told this connection yet).
    const sockets: WebSocketRoute[] = [];
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      sockets.push(ws);
      const server = ws.connectToServer();
      if (sockets.length === 1) return;
      server.onMessage((message) => {
        if (JSON.parse(String(message)).type !== "reverb") ws.send(message);
      });
    });
    const button = await setUp(page);
    // The socket drops (a Wi-Fi blip): the page reconnects.
    await sockets[0].close({ code: 4000, reason: "gone" });
    await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    // The last state stays shown (stale): ready, lit.
    await expect(button).toHaveAttribute("data-binding", "ready");
    await expect(button).toHaveClass(/\bon\b/);
    const since = Date.now();
    const sincePage = await pageNow(page);
    // Every `failed` flash observed, never sampled (it lasts 400 ms).
    await button.evaluate((el) => {
      const w = window as any;
      w.reverbFlashes = 0;
      let on = el.classList.contains("failed");
      w.reverbWatch = new MutationObserver(() => {
        const now = el.classList.contains("failed");
        if (now && !on) w.reverbFlashes += 1;
        on = now;
      });
      w.reverbWatch.observe(el, { attributes: true, attributeFilter: ["class"] });
    });
    // A tap: refused with a flash.
    await button.click();
    await expect.poll(() => page.evaluate(() => (window as any).reverbFlashes)).toBe(1);
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
    await expect.poll(() => page.evaluate(() => (window as any).reverbFlashes)).toBe(2);
    await page.waitForTimeout(600);
    expect(await page.evaluate(() => (window as any).reverbFlashes)).toBe(2);
    await page.evaluate(() => (window as any).reverbWatch.disconnect());
    expect(await live.get("band", ST, "value")).toBe(0.85);
    expect(await live.get("band", PA, "value")).toBe(0.6);
    expect(live.reverb.aut).toBe(false);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "refused").length >= 2)).toEqual([
      "press",
      "refused",
      "press",
      "refused",
    ]);
    // Nothing reached the rule: no tap, no AUT switch since.
    const steps = await reverbSteps(since);
    expect(steps).not.toContain("tap");
    expect(steps).not.toContain("aut");
  });
});
