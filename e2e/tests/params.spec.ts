import type { Locator } from "@playwright/test";
import { test, expect, type Page } from "./support/fixtures";
import {
  BASE,
  LiveClient,
  centre,
  clipped,
  dispatchPointer,
  frames,
  harness,
  hostLine,
  openSurface,
  ready,
  selectPage,
  toggleDoubleTap,
  token,
  track,
  until,
  volume,
} from "./support/live";

// The former MIDI controls (spec F17, F18, D10, X10): toggles and a fader that
// write their target parameters directly and show the targets' real state.
// In the synthetic layout: REVERB (toggle, a return's mute), VOC MIC (double
// tap latch, two tracks' mutes), ZVUKAR (pulse and double tap latch), AUTOTUNE
// (a rack parameter the set does not have), Podklady All (a fader on two
// track volumes: the backing-tracks fader, #58) and, on the cue page, Vox 1
// TU.

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

const toggle = (page: Page, label: string) => page.locator(`[data-testid="param-toggle"][data-label="${label}"]`);
const REVERB = "live_set return_tracks[name=A-Reverb #]";
const VOC1 = track("Vocal 1 repro#");
const VOC2 = track("Vocal 2 repro#");
const HAND4 = track("Hand4 #");
/** The stylesheet's red for an unresolved binding (`--danger`). */
const RED = "rgb(255, 59, 48)";

/** Every control of a layout (schema 2): rails, groups, every sub-page, global. */
function controls(layout: any): any[] {
  const groups = (sections: any[]): any[] =>
    sections.flatMap((s: any) => (s.kind === "pager" ? s.pages.flatMap((p: any) => groups(p.sections)) : [s]));
  return [
    ...layout.pages.flatMap((p: any) => [
      ...(p.rail ?? []),
      ...(p.rows ?? []).flatMap((r: any) => groups(r.sections).flatMap((g: any) => g.controls)),
    ]),
    ...(layout.global ?? []),
  ];
}

/** REPRO's target in a layout. */
function reproTarget(layout: any): any {
  const control = controls(layout).find((c: any) => c.kind === "param_toggle" && c.label === "REPRO");
  return control.targets[0];
}

/** The layout the hub serves now. */
async function served(): Promise<any> {
  const response = await fetch(`${BASE}/api/layout`, { headers: { authorization: `Bearer ${await token()}` } });
  if (!response.ok) throw new Error(`layout: ${response.status}`);
  return (await response.json()).layout;
}

/** The fixture's backing-tracks fader (#58). */
const podklady = (page: Page) => page.locator('[data-testid="param-fader"][data-label="Podklady All"]');

/** The group holding Podklady All in a layout. */
function podkladyGroup(layout: any): any {
  const groups = (sections: any[]): any[] =>
    sections.flatMap((s: any) => (s.kind === "pager" ? s.pages.flatMap((p: any) => groups(p.sections)) : [s]));
  const all = layout.pages.flatMap((p: any) => (p.rows ?? []).flatMap((r: any) => groups(r.sections)));
  const group = all.find((g: any) => (g.controls ?? []).some((c: any) => c.label === "Podklady All"));
  if (!group) throw new Error("no group holds Podklady All");
  return group;
}

/** A computed style property of an element. */
async function css(el: Locator, prop: string): Promise<string> {
  return el.evaluate((e, p) => (getComputedStyle(e) as any)[p], prop);
}

/**
 * How far the cap's centre is from the top of groove `i`'s fill (page px):
 * near 0 when the cap stands at that track's level.
 */
async function capOffGroove(strip: Locator, i: number): Promise<number> {
  return strip.evaluate((root, index) => {
    const cap = root.querySelector('[data-testid="fader-cap"]')!.getBoundingClientRect();
    const fill = root.querySelectorAll(".vca-fill")[index].getBoundingClientRect();
    return Math.abs(cap.top + cap.height / 2 - fill.top);
  }, i);
}

test.describe("Former MIDI toggles", () => {
  test("a toggle writes its target and shows Live's state", async ({ page }) => {
    await live.set("band", REVERB, "mute", false);
    await openSurface(page);
    const reverb = toggle(page, "REVERB");
    await ready(reverb);
    await expect(reverb).toHaveAttribute("data-state", "on");
    await reverb.click();
    await until(() => live.get("band", REVERB, "mute"), (v) => v === true, "the reverb muted (off)");
    await expect(reverb).toHaveAttribute("data-state", "off");
    await reverb.click();
    await until(() => live.get("band", REVERB, "mute"), (v) => v === false, "the reverb back on");
    // A change in Live (a song's automation) shows too.
    await live.set("band", REVERB, "mute", true);
    await expect(reverb).toHaveAttribute("data-state", "off");
  });

  test("a multi-target toggle shows mixed targets and a double tap writes every target", async ({ page }) => {
    await live.set("band", VOC1, "mute", true);
    await live.set("band", VOC2, "mute", false);
    await openSurface(page);
    const voc = toggle(page, "VOC MIC");
    await ready(voc);
    await expect(voc).toHaveAttribute("data-press", "double_tap_latch");
    await expect(voc).toHaveAttribute("data-state", "mixed");
    // A single tap does nothing: this toggle latches on a double tap only.
    await voc.click();
    await page.waitForTimeout(500);
    expect(await live.get("band", VOC1, "mute")).toBe(true);
    expect(await live.get("band", VOC2, "mute")).toBe(false);
    await toggleDoubleTap(page, voc);
    await until(() => live.get("band", VOC1, "mute"), (v) => v === false, "Vocal 1 on");
    await until(() => live.get("band", VOC2, "mute"), (v) => v === false, "Vocal 2 on");
    await expect(voc).toHaveAttribute("data-state", "on");
    await page.waitForTimeout(300);
    await toggleDoubleTap(page, voc);
    await until(() => live.get("band", VOC1, "mute"), (v) => v === true, "Vocal 1 off");
    await until(() => live.get("band", VOC2, "mute"), (v) => v === true, "Vocal 2 off");
    await expect(voc).toHaveAttribute("data-state", "off");
  });

  test("a pulse toggle is on while held; a double tap latches it", async ({ page }) => {
    await live.set("band", HAND4, "mute", true);
    await openSurface(page);
    const zvukar = toggle(page, "ZVUKAR");
    await ready(zvukar);
    await expect(zvukar).toHaveAttribute("data-state", "off");
    const { x, y } = await centre(zvukar);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await until(() => live.get("band", HAND4, "mute"), (v) => v === false, "on while held");
    await page.mouse.up();
    await until(() => live.get("band", HAND4, "mute"), (v) => v === true, "off on release");
    await page.waitForTimeout(300);
    await toggleDoubleTap(page, zvukar);
    await until(() => live.get("band", HAND4, "mute"), (v) => v === false, "latched on");
    await page.waitForTimeout(500);
    expect(await live.get("band", HAND4, "mute")).toBe(false);
    await expect(zvukar).toHaveAttribute("data-state", "on");
    // A tap unlatches it.
    await zvukar.click();
    await until(() => live.get("band", HAND4, "mute"), (v) => v === true, "unlatched");
  });

  test("a toggle whose target the set lacks stays disabled and red", async ({ page }) => {
    await openSurface(page);
    const autotune = toggle(page, "AUTOTUNE");
    await expect(autotune).toHaveAttribute("data-state", "unknown");
    await expect(autotune).toHaveAttribute("aria-disabled", "true");
    await expect(autotune).toHaveAttribute("data-binding", "unresolved");
    await expect(autotune).toHaveCSS("background-color", RED);
  });

  test("a write Live refuses is shown on the control, never retried (I6)", async ({ page }) => {
    const repro = "live_set tracks[name=Hand4 #] mixer_device sends 1";
    await live.set("band", repro, "value", 0.0);
    // REPRO writes 5.0 for "on": a send's range is 0..1, so Live refuses it.
    const refused = structuredClone(await served());
    reproTarget(refused).on = 5.0;
    await harness("/hub/layout", { layout: refused });
    try {
      await until(() => served(), (l) => reproTarget(l).on === 5, "the hub to serve the edited layout");
      await openSurface(page);
      const toggleEl = toggle(page, "REPRO");
      await ready(toggleEl);
      await expect(toggleEl).toHaveAttribute("data-state", "off");
      await toggleEl.click();
      await expect(toggleEl).toHaveClass(/failed/);
      await expect(toggleEl).not.toHaveClass(/failed/, { timeout: 2000 });
      // Never retried: no second flash and Live keeps its value.
      const deadline = Date.now() + 1500;
      while (Date.now() < deadline) {
        await expect(toggleEl).not.toHaveClass(/failed/, { timeout: 100 });
        await page.waitForTimeout(100);
      }
      expect(await live.get("band", repro, "value")).toBe(0);
      await expect(toggleEl).toHaveAttribute("data-state", "off");
    } finally {
      await harness("/hub/layout/reset");
      await until(() => served(), (l) => reproTarget(l).on === 1, "the original layout served again");
    }
  });

  test("the cue page's toggle writes its track", async ({ page }) => {
    const voc3 = track("Vocal 3 repro#");
    await live.set("band", voc3, "mute", true);
    await openSurface(page);
    await selectPage(page, "cue");
    const tu = toggle(page, "Vox 1 TU");
    await ready(tu);
    await expect(tu).toHaveAttribute("data-state", "off");
    await tu.click();
    await until(() => live.get("band", voc3, "mute"), (v) => v === false, "Vocal 3 on");
  });
});

test.describe("The former MIDI fader", () => {
  test("another parameter fader keeps its linear fader and its target's display string (OWN)", async ({ page }) => {
    // A single volume target (the markers spec's OWN, #68) is no
    // backing-tracks fader (#58): today's fader, TouchOSC's first move kept.
    const drums = volume(track("Drums #"));
    const own = {
      kind: "param_fader",
      label: "OWN",
      targets: [{ binding: { instance: "band", anchor: { kind: "track", name: "Drums #" }, path: "mixer_device volume" }, prop: "value", scale: "cc_linear" }],
    };
    const layout = structuredClone(await served());
    podkladyGroup(layout).controls.push(own);
    await harness("/hub/layout", { layout });
    try {
      await until(() => served(), (l) => controls(l).some((c: any) => c.label === "OWN"), "the hub to serve OWN");
      await live.set("band", drums, "value", 0.4);
      await openSurface(page);
      const ownFader = page.locator('[data-testid="param-fader"][data-label="OWN"]');
      const fader = ownFader.getByTestId("fader");
      await ready(fader);
      await expect(ownFader.getByTestId("param-groove")).toHaveCount(0);
      expect(await ownFader.getAttribute("data-moved")).toBeNull();
      await expect(ownFader.getByTestId("param-display")).toHaveText(await live.display("band", drums, 0.4));
      // Podklady All beside it is the backing-tracks fader.
      await expect(podklady(page).getByTestId("param-groove")).toHaveCount(2);
      // Linear on the parameter's range, 1:1 with the finger, its first
      // move counted: 60 px up from 0.4.
      const travel = (await fader.boundingBox())!.height;
      const { x, y } = await centre(fader);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 6; i++) await page.mouse.move(x, y - 10 * i);
      await page.mouse.up();
      await until(() => live.get("band", drums, "value"), (v) => v > 0.45, "Drums up");
      // The sends settle (one per frame, the last on release).
      await page.waitForTimeout(400);
      const d = await live.get("band", drums, "value");
      expect(Math.abs(d - (0.4 + 60 / travel)), `${d} after 60 px of ${travel}`).toBeLessThan(0.005);
      await expect(ownFader.getByTestId("param-display")).toHaveText(await live.display("band", drums, d));
    } finally {
      await harness("/hub/layout/reset");
      await until(() => served(), (l) => !controls(l).some((c: any) => c.label === "OWN"), "the original layout served again");
    }
  });

  test("Podklady All waits for every target: one that does not resolve turns it red", async ({ page }) => {
    await openSurface(page);
    const fader = page.locator('[data-testid="param-fader"][data-label="Podklady All"]').getByTestId("fader");
    await ready(fader);
    await expect(fader).toHaveAttribute("data-binding", "ready");
    const drums = volume(track("Drums #"));
    const before = await live.get("band", drums, "value");
    try {
      // The second target goes: the fader must not write the first alone.
      expect(await hostLine("band", 'rename "Bass #" "Bass X"')).toBe("RENAMED 1");
      await expect(fader).toHaveAttribute("data-binding", "unresolved");
      await expect(fader).toHaveAttribute("aria-disabled", "true");
      await expect(fader).toHaveCSS("background-color", RED);
      const { x, y } = await centre(fader);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 6; i++) await page.mouse.move(x, y - 10 * i);
      await page.mouse.up();
      await page.waitForTimeout(300);
      expect(await live.get("band", drums, "value")).toBe(before);
    } finally {
      await hostLine("band", 'rename "Bass X" "Bass #"');
    }
    await expect(fader).toHaveAttribute("data-binding", "ready");
    await ready(fader);
  });
});

// The backing-tracks fader (#58 item 4): a parameter fader of two or more
// track volumes, as the owner approved it in the mockup
// docs/mockups/podklady-fader-v1.html. The fixture's Podklady All writes
// two (Drums #, Bass #); the site's writes five.
test.describe("The backing-tracks fader", () => {
  const DRUMS = volume(track("Drums #"));
  const BASS = volume(track("Bass #"));
  /** The mockup's colours: a groove at 0 dB, a track off it, the panel and the amber-tinted panel. */
  const TEAL = "rgb(60, 200, 180)";
  const AMBER = "rgb(255, 176, 32)";
  const PANEL = "rgb(26, 26, 40)";
  const AMBER_PANEL = "rgb(63, 50, 39)";
  let before: number[] = [];

  test.beforeEach(async () => {
    before = [await live.get("band", DRUMS, "value"), await live.get("band", BASS, "value")];
  });
  test.afterEach(async () => {
    await live.set("band", DRUMS, "value", before[0]);
    await live.set("band", BASS, "value", before[1]);
  });

  test("at 0 dB it looks like any strip: teal grooves at 0 dB, the cap on them, no badge", async ({ page }) => {
    await live.set("band", DRUMS, "value", 0.85);
    await live.set("band", BASS, "value", 0.85);
    await openSurface(page);
    const strip = podklady(page);
    await ready(strip.getByTestId("fader"));
    const grooves = strip.getByTestId("param-groove");
    await expect(grooves).toHaveCount(2);
    await expect(grooves.nth(0)).toHaveAttribute("data-db", "0.000");
    await expect(grooves.nth(1)).toHaveAttribute("data-db", "0.000");
    await expect(strip.getByTestId("param-display")).toHaveText("0.0 dB");
    await expect(strip.getByTestId("param-values")).toHaveText("0 0");
    await expect(strip).toHaveAttribute("data-moved", "false");
    await expect(strip.getByTestId("param-badge")).toBeHidden();
    expect(await css(strip, "backgroundColor")).toBe(PANEL);
    for (const i of [0, 1]) {
      await expect(grooves.nth(i)).toHaveAttribute("data-moved", "false");
      expect(await css(grooves.nth(i).locator(".vca-fill"), "backgroundColor")).toBe(TEAL);
    }
    await expect.poll(() => capOffGroove(strip, 0), { message: "the cap at the grooves' level" }).toBeLessThan(1.5);
  });

  test("tracks pulled down in Live (a prayer mode) turn it amber, each groove at its own level", async ({ page }) => {
    await live.set("band", DRUMS, "value", 0.85);
    await live.set("band", BASS, "value", 0.85);
    await openSurface(page);
    const strip = podklady(page);
    await ready(strip.getByTestId("fader"));
    await expect(strip).toHaveAttribute("data-moved", "false");
    // The Live set's prayer mode moves the tracks themselves, unevenly:
    // −14 and −12 dB.
    await live.set("band", DRUMS, "value", 0.5);
    await live.set("band", BASS, "value", 0.55);
    const grooves = strip.getByTestId("param-groove");
    await expect(strip).toHaveAttribute("data-moved", "true");
    await expect(grooves.nth(0)).toHaveAttribute("data-db", "-14.000");
    await expect(grooves.nth(1)).toHaveAttribute("data-db", "-12.000");
    await expect(strip.getByTestId("param-display")).toHaveText("-12.0 dB");
    await expect(strip.getByTestId("param-values")).toHaveText("-14 -12");
    const badge = strip.getByTestId("param-badge");
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("≠ 0 dB");
    expect(await css(strip, "backgroundColor")).toBe(AMBER_PANEL);
    expect(await css(strip, "boxShadow")).toContain(AMBER);
    expect(await css(strip.locator(".param-fader-label"), "color")).toBe(AMBER);
    expect(await css(strip.getByTestId("param-display"), "color")).toBe(AMBER);
    for (const i of [0, 1]) {
      await expect(grooves.nth(i)).toHaveAttribute("data-moved", "true");
      expect(await css(grooves.nth(i).locator(".vca-fill"), "backgroundColor")).toBe(AMBER);
    }
    // The cap stands at the loudest track, Bass.
    await expect.poll(() => capOffGroove(strip, 1), { message: "the cap at the loudest groove" }).toBeLessThan(1.5);
    for (const part of ["param-display", "param-values"]) {
      expect(await clipped(strip.getByTestId(part)), part).toEqual([]);
    }
    expect(await clipped(strip.locator(".param-fader-label")), "the label").toEqual([]);
    // One track back at 0 dB: still amber, that groove teal.
    await live.set("band", BASS, "value", 0.85);
    await expect(grooves.nth(1)).toHaveAttribute("data-db", "0.000");
    await expect(strip.getByTestId("param-display")).toHaveText("0.0 dB");
    await expect(strip.getByTestId("param-values")).toHaveText("-14 0");
    await expect(strip).toHaveAttribute("data-moved", "true");
    await expect(grooves.nth(1)).toHaveAttribute("data-moved", "false");
    expect(await css(grooves.nth(1).locator(".vca-fill"), "backgroundColor")).toBe(TEAL);
    // Both back: a normal strip again.
    await live.set("band", DRUMS, "value", 0.85);
    await expect(strip).toHaveAttribute("data-moved", "false");
    await expect(strip.getByTestId("param-badge")).toBeHidden();
    expect(await css(strip, "backgroundColor")).toBe(PANEL);
  });

  test("a drag moves every track by the same dB, and back to its start restores them exactly", async ({ page }) => {
    // −6 and −2 dB: the drag stays on the law's linear part (0.4 and up,
    // 40 dB per unit of Live's volume), where the same dB is the same
    // change of volume.
    await live.set("band", DRUMS, "value", 0.7);
    await live.set("band", BASS, "value", 0.8);
    await openSurface(page);
    const strip = podklady(page);
    const fader = strip.getByTestId("fader");
    await ready(fader);
    await expect(strip.getByTestId("param-display")).toHaveText("-2.0 dB");
    // A finger down; its first move only anchors (#43 PR F); then 40 px
    // down in moves too large to shape (each over 3 % of TouchOSC's 355 px
    // strip), so the cap follows the finger 1:1. The finger stays down.
    await dispatchPointer(fader, [
      { type: "pointerdown" },
      { type: "pointermove", dy: 0 },
      { type: "pointermove", dy: -20 },
      { type: "pointermove", dy: -40 },
    ]);
    const [drums, bass] = await until(
      () => Promise.all([live.get("band", DRUMS, "value"), live.get("band", BASS, "value")]),
      ([d, b]) => d < 0.69 && b < 0.79,
      "both tracks down",
    );
    expect(drums, "Drums on the linear part").toBeGreaterThan(0.4);
    // The loudest (Bass) at the finger, Drums moved by the same dB.
    expect(Math.abs(drums - 0.7 - (bass - 0.8)), `Drums ${drums}, Bass ${bass}`).toBeLessThan(1e-4);
    expect(bass, "at least 2 dB down").toBeLessThan(0.75);
    // The surface shows what Live holds.
    await frames(page);
    const grooves = strip.getByTestId("param-groove");
    await expect.poll(async () => Math.abs(Number(await grooves.nth(0).getAttribute("data-db")) - (40 * drums - 34))).toBeLessThan(0.001);
    await expect.poll(async () => Math.abs(Number(await grooves.nth(1).getAttribute("data-db")) - (40 * bass - 34))).toBeLessThan(0.001);
    await expect(strip).toHaveAttribute("data-moved", "true");
    // Back up to where the finger started, and lifted: the touch's start
    // values, exactly.
    await dispatchPointer(fader, [
      { type: "pointermove", dy: -20 },
      { type: "pointermove", dy: 0 },
      { type: "pointerup", dy: 0 },
    ]);
    await until(
      () => Promise.all([live.get("band", DRUMS, "value"), live.get("band", BASS, "value")]),
      ([d, b]) => d === 0.7 && b === 0.8,
      "Drums 0.7 and Bass 0.8 again, exactly",
    );
    await expect(strip.getByTestId("param-groove").nth(0)).toHaveAttribute("data-db", "-6.000");
    await expect(strip.getByTestId("param-display")).toHaveText("-2.0 dB");
  });
});
