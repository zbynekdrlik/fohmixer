import { test, expect, type Page } from "./support/fixtures";
import {
  BASE,
  LiveClient,
  centre,
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
// volumes) and, on the cue page, Vox 1 TU.

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

/** REPRO's target in a layout. */
function reproTarget(layout: any): any {
  const item = layout.pages
    .flatMap((p: any) => p.items)
    .find((i: any) => i.kind === "param_toggle" && i.label === "REPRO");
  return item.targets[0];
}

/** The layout the hub serves now. */
async function served(): Promise<any> {
  const response = await fetch(`${BASE}/api/layout`, { headers: { authorization: `Bearer ${await token()}` } });
  if (!response.ok) throw new Error(`layout: ${response.status}`);
  return (await response.json()).layout;
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
  test("Podklady All writes every target and shows the first one's display string", async ({ page }) => {
    const drums = volume(track("Drums #"));
    const bass = volume(track("Bass #"));
    await live.set("band", drums, "value", 0.4);
    await live.set("band", bass, "value", 0.4);
    await openSurface(page);
    const podklady = page.locator('[data-testid="param-fader"][data-label="Podklady All"]');
    const fader = podklady.getByTestId("fader");
    await ready(fader);
    await expect(podklady.getByTestId("param-display")).toHaveText(await live.display("band", drums, 0.4));
    const { x, y } = await centre(fader);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 6; i++) await page.mouse.move(x, y - 10 * i);
    await page.mouse.up();
    await until(() => live.get("band", drums, "value"), (v) => v > 0.45, "Drums up");
    // The sends settle (one per frame, the last on release): both targets
    // end on the same value.
    await page.waitForTimeout(400);
    const d = await live.get("band", drums, "value");
    expect(await live.get("band", bass, "value")).toBeCloseTo(d, 9);
    await expect(podklady.getByTestId("param-display")).toHaveText(await live.display("band", drums, d));
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
