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

// AUTOTUNE and AUTOTUNE AUT on the surface (#104, D19; the approved mockup
// docs/mockups/autotune-v1.html): the AUTOTUNE rail button (where the
// former AUTOTUNE toggle stood) switches autotune on or off on every
// `+AUTOTUNE` track at once (the hub writes each `Latencies` rack's first
// macro through its ref), and its 1 s hold turns AUTOTUNE AUT on or off at
// its lift (Live plays: on; Live stops: off). SimLive's band set: Vocal 2
// repro# (track 6) has a `Latencies` rack on top, tagged `+AUTOTUNE:0`
// (on 0, off 127, the repro form); Vocal 3 repro# (track 7) has one inside
// `Stream FX`, tagged `+AUTOTUNE` (on 127, off 0, the stream form); Keys 1
// (track 8) has none. Both macros start at 0: track 6 on, track 7 off.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
/** The repro track's macro (on 0) and the stream track's (on 127). */
const REPRO = "live_set tracks 6 devices 0 parameters 1";
const STREAM = "live_set tracks 7 devices 0 chains 0 devices 0 parameters 1";
/** Live's two macros when every target is on, or off. */
const ON = [0, 127];
const OFF = [127, 0];

/** The fixture with the AUTOTUNE button where the FOH rail's AUTOTUNE toggle stood. */
function withAutotuneButton(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  const at = foh.rail.findIndex((c: any) => c.kind === "param_toggle" && c.label === "AUTOTUNE");
  foh.rail.splice(at, 1, { kind: "autotune", label: "AUTOTUNE" });
  return frame;
}

/** Live's two macros (repro, stream). */
async function macros(live: LiveClient): Promise<number[]> {
  return [await live.get("band", REPRO, "value"), await live.get("band", STREAM, "value")];
}

/** Waits until Live's two macros read `want`. */
async function macrosAre(live: LiveClient, want: number[], what: string) {
  await until(() => macros(live), (v) => v[0] === want[0] && v[1] === want[1], what);
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

/** The button's second line reads `text`, and the button, its line and its label with the pill fit (118 × 44). */
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

/** A 1 s hold, full at 1.2 s (read in the page before the lift), then its lift. */
async function hold(button: Locator, line: string, live: LiveClient, aut: boolean) {
  const press = await dispatchSnapped(button, [{ type: "pointerdown" }, { wait: 1200 }, { snap: true }]);
  expect(press.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: line, failed: false });
  await expect(button).toHaveAttribute("data-full", "true");
  await fits(button, line);
  expect(live.autotune.aut, "nothing before the lift").toBe(!aut);
  await dispatchPointer(button, [{ type: "pointerup" }]);
  await until(async () => live.autotune.aut, (v: boolean) => v === aut, `AUTOTUNE AUT ${aut ? "on" : "off"}`);
  await expect(button).toHaveAttribute("data-full", "false");
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

/** Tags the two singer tracks: track 6 the repro form, track 7 the stream form. */
async function tagSingers() {
  expect(await hostLine("band", "tuner track 6 set '+AUTOTUNE:0'")).toBe("TUNER 1");
  expect(await hostLine("band", "tuner track 7 set '+AUTOTUNE'")).toBe("TUNER 1");
}

test.describe("AUTOTUNE and AUTOTUNE AUT", () => {
  // The iPad's size (the rail's buttons 118 × 44, nothing clipped at
  // 1194 × 834) in both projects.
  test.use({ viewport: { width: 1194, height: 834 } });

  let live: LiveClient;
  let layouts: string[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    // AUTOTUNE AUT off first, so the stop writes nothing; then both macros
    // back to SimLive's 0 and the Tuners gone.
    live.setHub("autotune_aut", false);
    await until(async () => live.autotune?.aut, (v: boolean) => v === false, "AUTOTUNE AUT off");
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", REPRO, "value", 0);
    await live.set("band", STREAM, "value", 0);
    for (const track of [6, 7, 8]) await hostLine("band", `tuner track ${track} remove`);
    await harness("/hub/layout/reset");
    await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the markers gone", 10_000);
    await until(async () => (await hubStatus()).autotune.targets.length, (n: number) => n === 0, "no AUTOTUNE target", 10_000);
    live.close();
  });

  /** The AUTOTUNE button on the FOH rail, red until the singer tracks are tagged, then ready (mixed: 1 of 2 on). */
  async function setUp(page: Page): Promise<Locator> {
    await live.call("band", "live_set", "stop_playing");
    layouts = watchLayouts(page);
    await openSurface(page);
    await harness("/hub/layout", { layout: withAutotuneButton() });
    await selectPage(page, "foh");
    const button = page.getByTestId("autotune");
    // No tagged track yet (I5): red and disabled.
    await expect(button).toHaveAttribute("data-binding", "unresolved", { timeout: 10_000 });
    await expect(button).toHaveAttribute("aria-disabled", "true");
    await tagSingers();
    await targetsFound(2);
    await showsServedLayout(layouts);
    await ready(button);
    await expect(button).toHaveAttribute("data-binding", "ready");
    return button;
  }

  test("the button is red until tracks are tagged, then shows Live's autotune; 118 × 44, no fader height", async ({ page }) => {
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
    // SimLive's macros both 0: the repro track on, the stream track off.
    expect(await macros(live)).toEqual([0, 0]);
    await expect(button).toHaveAttribute("data-state", "mixed");
    await expect(button).toHaveAttribute("data-mixed", "true");
    await expect(button).not.toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button).toHaveAttribute("data-problem", "false");
    await expect(button.getByTestId("rule-flag")).toHaveCount(0);
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await fits(button, "1/2 zap.");
    // The mixed look: the orange ring.
    expect(await button.evaluate((el) => getComputedStyle(el).borderTopColor)).toBe("rgb(255, 159, 67)");
    const status = await hubStatus();
    expect(status.autotune.targets.map((t: any) => [t.label, t.on, t.state, t.path])).toEqual([
      ["#7", 0, "on", REPRO],
      ["#8", 127, "off", STREAM],
    ]);
  });

  test("a tap turns every target off from mixed, the next one on", async ({ page }) => {
    const button = await setUp(page);
    await expect(button).toHaveAttribute("data-state", "mixed");
    const since = Date.now();
    const sincePage = await pageNow(page);
    await button.click();
    await macrosAre(live, OFF, "both off");
    await expect(button).toHaveAttribute("data-state", "off");
    await expect(button).not.toHaveClass(/\bon\b/);
    await expect(button).toHaveAttribute("data-mixed", "false");
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    await button.click();
    await macrosAre(live, ON, "both on");
    await expect(button).toHaveAttribute("data-state", "on");
    await expect(button).toHaveClass(/\bon\b/);
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "tap").length >= 2)).toEqual(["press", "tap", "press", "tap"]);
    const records = await until(
      () => autotuneRecords(since),
      (r) => r.filter((x) => x.step === "write").length >= 2,
      "the taps' writes in the event log",
      15_000,
    );
    expect(records.map((r) => [r.step, r.on, r.why ?? null])).toEqual([
      ["tap", false, null],
      ["write", false, "tap"],
      ["tap", true, null],
      ["write", true, "tap"],
    ]);
    expect((await hubStatus()).autotune.taps).toBeGreaterThanOrEqual(2);
  });

  test("a 1 s hold turns AUTOTUNE AUT on at its lift; the rule follows Live's start and stop", async ({ page }) => {
    const button = await setUp(page);
    const since = Date.now();
    const sincePage = await pageNow(page);
    await hold(button, "pusť — zapne AUT", live, true);
    // Live stands: every target off at once.
    await macrosAre(live, OFF, "off while Live stands");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("aut-badge")).toHaveText("AUT");
    await expect(button).toHaveAttribute("data-state", "off");
    await fits(button, "stojí · vypnutý");
    // Live plays: every target on.
    await live.call("band", "live_set", "start_playing");
    await macrosAre(live, ON, "on while Live plays");
    await expect(button).toHaveAttribute("data-state", "on");
    await expect(button).toHaveClass(/\bon\b/);
    await fits(button, "hrá · zapnutý");
    await expect(button).toHaveAttribute("data-aut", "run");
    // Live stops: off again.
    await live.call("band", "live_set", "stop_playing");
    await macrosAre(live, OFF, "off at the stop");
    await fits(button, "stojí · vypnutý");
    // A second hold turns it off at its lift; Live's autotune stays.
    await hold(button, "pusť — vypne AUT", live, false);
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    expect(await macros(live)).toEqual(OFF);
    const records = await until(
      () => autotuneRecords(since),
      (r) => r.filter((x) => x.step === "flag").length >= 2,
      "the flags in the event log",
      15_000,
    );
    expect(records.map((r) => [r.step, r.on, r.why ?? null])).toEqual([
      ["flag", true, null],
      ["write", false, "flag"],
      ["write", true, "transport"],
      ["write", false, "transport"],
      ["flag", false, null],
    ]);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "hold").length >= 2)).toEqual([
      "press",
      "full",
      "hold",
      "press",
      "full",
      "hold",
    ]);
  });

  test("a tap overrides the rule until the next stop or start; Live's own switch shows mixed", async ({ page }) => {
    const button = await setUp(page);
    await live.call("band", "live_set", "start_playing");
    live.setHub("autotune_aut", true);
    await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
    await macrosAre(live, ON, "on while Live plays");
    await fits(button, "hrá · zapnutý");
    // A tap during the song: off by hand until the stop (outlined).
    await button.click();
    await macrosAre(live, OFF, "off by the tap");
    await expect(button).toHaveAttribute("data-aut", "held");
    await fits(button, "ručne vyp. · do zast.");
    // A song switches the stream track on by itself (its CC36): mixed.
    await live.set("band", STREAM, "value", 127);
    await expect(button).toHaveAttribute("data-state", "mixed");
    await expect(button).toHaveAttribute("data-aut", "held");
    await fits(button, "zmiešané · 1/2 zap.");
    // Live stops: the rule turns every target off.
    await live.call("band", "live_set", "stop_playing");
    await macrosAre(live, OFF, "off at the stop");
    await expect(button).toHaveAttribute("data-aut", "run");
    await fits(button, "stojí · vypnutý");
    // A tap while Live stands: on by hand until the start (outlined).
    await button.click();
    await macrosAre(live, ON, "on by the tap");
    await expect(button).toHaveAttribute("data-aut", "held");
    await fits(button, "ručne zap. · do štartu");
    // Live plays: the rule's own again.
    await live.call("band", "live_set", "start_playing");
    await expect(button).toHaveAttribute("data-aut", "run");
    await fits(button, "hrá · zapnutý");
    expect(await macros(live)).toEqual(ON);
  });

  test("a track without a Latencies rack shows the problem corner; the others still switch", async ({ page }) => {
    const button = await setUp(page);
    expect(await hostLine("band", "tuner track 8 set '+AUTOTUNE'")).toBe("TUNER 1");
    const targets = await targetsFound(3);
    await showsServedLayout(layouts);
    expect(targets[2]).toMatchObject({ label: "#9", state: "missing", problem: "no rack named Latencies" });
    await expect(button).toHaveAttribute("data-problem", "true");
    await expect(button).toHaveAttribute("data-binding", "ready");
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
    await fits(button, "1/2 zap.");
    // A tap still switches the two found targets.
    await button.click();
    await macrosAre(live, OFF, "both off");
    await expect(button).toHaveAttribute("data-state", "off");
    await expect(button).toHaveAttribute("data-problem", "true");
    expect(await clipped(button)).toEqual([]);
  });

  for (const [width, height] of [
    [844, 390],
    [390, 844],
  ]) {
    test.describe(`on a phone, ${width} × ${height}`, () => {
      test.use({ viewport: { width, height } });

      test("the AUTOTUNE button fits the phone's rail: its AUT dot and problem corner inside it, no second line, every rail button whole", async ({ page }) => {
        await live.call("band", "live_set", "start_playing");
        layouts = watchLayouts(page);
        await openSurface(page);
        await harness("/hub/layout", { layout: withAutotuneButton() });
        const button = page.getByTestId("autotune");
        await expect(button).toHaveCount(1, { timeout: 10_000 });
        await tagSingers();
        expect(await hostLine("band", "tuner track 8 set '+AUTOTUNE'")).toBe("TUNER 1");
        await targetsFound(3);
        await showsServedLayout(layouts);
        // AUTOTUNE AUT on while Live plays: every target on.
        live.setHub("autotune_aut", true);
        await until(async () => live.autotune?.aut, (v: boolean) => v === true, "AUTOTUNE AUT on");
        await macrosAre(live, ON, "on while Live plays");
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
            dot: el.querySelector(".aut-badge")!.getBoundingClientRect(),
            flag: el.querySelector(".rule-flag")!.getBoundingClientRect(),
            text: range.getBoundingClientRect(),
          };
        });
        for (const [name, mark] of [
          ["the AUT dot", place.dot],
          ["the problem corner", place.flag],
        ] as const) {
          expect(mark.width, `${name}: 6 px`).toBeCloseTo(6, 0);
          expect(mark.height, `${name}: 6 px`).toBeCloseTo(6, 0);
          expect(mark.left, `${name} inside the button`).toBeGreaterThanOrEqual(place.box.left);
          expect(mark.right, `${name} inside the button`).toBeLessThanOrEqual(place.box.right);
          expect(mark.top, `${name} inside the button`).toBeGreaterThanOrEqual(place.box.top);
          expect(mark.bottom, `${name} inside the button`).toBeLessThanOrEqual(place.box.bottom);
        }
        expect(place.dot.left, "the dot right of the label's text").toBeGreaterThanOrEqual(place.text.right);
        expect(place.flag.right, "the corner left of the label's text").toBeLessThanOrEqual(place.text.left);
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
    // The socket drops (a Wi-Fi blip): the page reconnects.
    await sockets[0].close({ code: 4000, reason: "gone" });
    await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    // The last state stays shown (stale): ready, mixed.
    await expect(button).toHaveAttribute("data-binding", "ready");
    await expect(button).toHaveAttribute("data-state", "mixed");
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
    // A tap: refused with a flash.
    await button.click();
    await expect.poll(() => page.evaluate(() => (window as any).tuneFlashes)).toBe(1);
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
    expect(await macros(live)).toEqual([0, 0]);
    expect(live.autotune.aut).toBe(false);
    expect(await railSteps(sincePage, (s) => s.filter((w) => w === "refused").length >= 2)).toEqual([
      "press",
      "refused",
      "press",
      "refused",
    ]);
    // Nothing reached the rule: no tap, no AUT switch since.
    const steps = (await autotuneRecords(since)).map((r) => r.step);
    expect(steps).not.toContain("tap");
    expect(steps).not.toContain("flag");
  });
});
