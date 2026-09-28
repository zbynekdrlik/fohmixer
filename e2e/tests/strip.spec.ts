import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  doubleTap,
  hostLine,
  harness,
  openSurface,
  panning,
  ready,
  selectPage,
  shown,
  strip,
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
  test("shows Live's display string for the volume", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.85);
    await openSurface(page);
    const db = strip(page, "Hand2 #").getByTestId("db");
    await expect(db).toHaveText(await live.display("band", volume(HAND2), 0.85));
    await expect(db).toHaveText("0.0 dB");
    await live.set("band", volume(HAND2), "value", 0.7);
    await expect(db).toHaveText(await live.display("band", volume(HAND2), 0.7));
    await expect(strip(page, "Hand2 #").getByTestId("strip-label")).toHaveText("Hand2");
    await expect(strip(page, "B-Main repro #").getByTestId("strip-label")).toHaveText("Main");
    await expect(strip(page, "Hand2 #").getByTestId("strip-instance")).toHaveText("band");
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

  test("a double tap glides the fader to 0 dB", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.5);
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    await doubleTap(page, fader);
    await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - 0.85) < 1e-6, "0 dB", 6000);
    await expect(strip(page, "Hand2 #").getByTestId("db")).toHaveText("0.0 dB");
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
    await doubleTap(page, pan);
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
    await doubleTap(page, mute, 150);
    await until(() => live.get("master", hand1, "mute"), (v) => v === true, "muted after the confirming tap");
    // The band's track of the same name is another instance's (spec F2, X4).
    expect(await live.get("band", hand1, "mute")).toBe(false);
    await live.set("master", hand1, "mute", false);
  });

  test("the meter moves with Live's meter", async ({ page }) => {
    await openSurface(page);
    const meter = strip(page, "Hand2 #").getByTestId("meter");
    const first = await until(async () => Number(await meter.getAttribute("data-level")), (v) => v > 0, "a level");
    await until(async () => Number(await meter.getAttribute("data-level")), (v) => v !== first, "the level to move");
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
    await expect(master.getByTestId("strip-instance")).toHaveText("master");
    await expect(master.getByTestId("db")).toHaveText(await live.display("master", volume(track("Hand1 #")), 0.6));
  });
});

test.describe("Controls wait for Live's value (I8)", () => {
  test("right after load the controls are disabled until their first value", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    // The band host's main thread stops: its values are held back.
    await hostLine("band", "stall 4000");
    await openSurfaceDuringStall(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await expect(fader).toHaveAttribute("aria-disabled", "true");
    await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveAttribute("aria-disabled", "true");
    await expect(fader).toHaveAttribute("aria-disabled", "false", { timeout: 10_000 });
    await ready(strip(page, "Hand2 #").getByTestId("mute"));
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
    await expect(strip(page, "Hand2 #").getByTestId("db")).toHaveText(await live.display("band", volume(HAND2), 0.8));
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
