import { test, expect, type Page } from "./support/fixtures";
import { LiveClient, centre, doubleTap, openSurface, ready, selectPage, track, until, volume } from "./support/live";

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
    await doubleTap(page, voc, 80);
    await until(() => live.get("band", VOC1, "mute"), (v) => v === false, "Vocal 1 on");
    await until(() => live.get("band", VOC2, "mute"), (v) => v === false, "Vocal 2 on");
    await expect(voc).toHaveAttribute("data-state", "on");
    await page.waitForTimeout(300);
    await doubleTap(page, voc, 80);
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
    await doubleTap(page, zvukar, 80);
    await until(() => live.get("band", HAND4, "mute"), (v) => v === false, "latched on");
    await page.waitForTimeout(500);
    expect(await live.get("band", HAND4, "mute")).toBe(false);
    await expect(zvukar).toHaveAttribute("data-state", "on");
    // A tap unlatches it.
    await zvukar.click();
    await until(() => live.get("band", HAND4, "mute"), (v) => v === true, "unlatched");
  });

  test("a toggle whose target the set lacks stays disabled", async ({ page }) => {
    await openSurface(page);
    const autotune = toggle(page, "AUTOTUNE");
    await expect(autotune).toHaveAttribute("data-state", "unknown");
    await expect(autotune).toHaveAttribute("aria-disabled", "true");
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
});
