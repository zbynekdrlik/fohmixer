import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import type { PointerStep } from "./support/live";
import { LiveClient, dispatchPointer, hubEvents, openSurface, pageEvents, strip, until } from "./support/live";

// The recording watch (#172): a strip's ARM key above SEL and the column
// foot's REC key, both a 1 s hold that acts at its lift, on SimLive's band.

const TRACK = "live_set tracks 1";
const NAME = "Hand2 #";
const HOLD: PointerStep[] = [{ type: "pointerdown" }, { wait: 1200 }, { type: "pointerup" }];
const TAP: PointerStep[] = [{ type: "pointerdown" }, { wait: 150 }, { type: "pointerup" }];
/** Long past the hold, after a slide off the key. */
const SLID: PointerStep[] = [
  { type: "pointerdown" },
  { type: "pointermove", dy: 0 },
  { type: "pointermove", dy: 20 },
  { wait: 1200 },
  { type: "pointerup" },
];

let live: LiveClient;

test.beforeEach(async () => {
  live = await LiveClient.open();
});

test.afterEach(async () => {
  await live.call("band", "live_set", "stop_playing");
  await live.set("band", "live_set", "record_mode", false);
  await live.set("band", TRACK, "arm", false);
  live.close();
});

const armKey = (page: Page): Locator => strip(page, NAME).getByTestId("arm-key");
const recKey = (page: Page): Locator => page.getByTestId("rec-key");
const pageNow = (page: Page): Promise<number> => page.evaluate(() => performance.timeOrigin + performance.now());

/** The page's `rail` steps of `button` since `since`, once they read `ok`. */
async function steps(button: string, since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.button === button && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the rail steps uploaded",
    15_000,
  );
}

test.describe("The recording watch", () => {
  test("a hold on the ARM key arms the track and disarms it; a tap and a slide do nothing", async ({ page }) => {
    await openSurface(page);
    const key = armKey(page);
    await expect(key).toHaveAttribute("data-state", "off");
    await expect(recKey(page)).toHaveAttribute("data-state", "off");
    const since = await pageNow(page);

    await dispatchPointer(key, TAP);
    await dispatchPointer(key, SLID);
    expect(await live.get("band", TRACK, "arm")).toBe(false);
    await expect(key).toHaveAttribute("data-state", "off");

    await dispatchPointer(key, HOLD);
    await expect(key).toHaveAttribute("data-state", "armed");
    await expect(recKey(page)).toHaveAttribute("data-state", "ready");
    await expect(recKey(page)).toHaveText("REC");
    expect(await live.get("band", TRACK, "arm")).toBe(true);
    // No box: the dot alone, 6 px, on the invisible touch area.
    const dot = key.locator(".rec-dot");
    await expect(dot).toHaveCSS("width", "6px");
    await expect(key).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    const area = (await key.boundingBox())!;
    expect(area.height).toBe(34);
    // Directly above SEL.
    const sel = (await strip(page, NAME).getByTestId("detail-key").boundingBox())!;
    expect(Math.abs(area.y + area.height - sel.y)).toBeLessThan(1);

    await dispatchPointer(key, HOLD);
    await expect(key).toHaveAttribute("data-state", "off");
    await expect(recKey(page)).toHaveAttribute("data-state", "off");
    expect(await live.get("band", TRACK, "arm")).toBe(false);

    const want = ["press", "tap", "press", "slid", "press", "full", "hold", "press", "full", "hold"];
    expect(await steps("arm", since, (s) => s.length >= want.length)).toEqual(want);
  });

  test("a return's strip has no ARM key", async ({ page }) => {
    await openSurface(page);
    await expect(armKey(page)).toBeVisible();
    const back = strip(page, "B-Main repro #");
    await expect(back).toHaveAttribute("data-kind", "return");
    await expect(back.getByTestId("arm-key")).toHaveCount(0);
  });

  test("the foot's hold switches the record button and it says what Live does", async ({ page }) => {
    await openSurface(page);
    const key = armKey(page);
    const foot = recKey(page);
    await expect(foot).toHaveText("REC");
    const since = await pageNow(page);

    await dispatchPointer(foot, TAP);
    await dispatchPointer(foot, HOLD);
    await until(() => live.get("band", "live_set", "record_mode"), (on) => on === true, "the record button on");

    // Playing with the record button on and nothing armed.
    await live.call("band", "live_set", "start_playing");
    await expect(foot).toHaveAttribute("data-state", "no_arm");
    await expect(foot).toHaveText("NO ARM");

    await dispatchPointer(key, HOLD);
    await expect(foot).toHaveAttribute("data-state", "recording");
    await expect(foot).toHaveText("REC");
    await expect(key).toHaveAttribute("data-state", "recording");
    // The slow soft pulse of the dot, and no filled box on the foot.
    await expect(key.locator(".rec-dot")).toHaveCSS("animation-duration", "2.4s");
    await expect(foot).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");

    // The record button off under a playing, armed set.
    await dispatchPointer(foot, HOLD);
    await expect(foot).toHaveAttribute("data-state", "not_rec");
    await expect(foot).toHaveText("NOT REC");
    await expect(key).toHaveAttribute("data-state", "armed");
    await expect(key.locator(".rec-dot")).toHaveCSS("animation-name", "none");
    expect(await live.get("band", "live_set", "record_mode")).toBe(false);

    await live.call("band", "live_set", "stop_playing");
    await expect(foot).toHaveAttribute("data-state", "ready");

    const want = ["press", "tap", "press", "full", "hold", "press", "full", "hold"];
    expect(await steps("rec", since, (s) => s.length >= want.length)).toEqual(want);
    // The hub's log says when and why each change came.
    const records = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "rec" && r.ts >= since),
      (all) => all.some((r: any) => r.why === "transport" && r.prev === "not_rec"),
      "the recording watch's records",
    );
    const stop = records.find((r: any) => r.prev === "recording")!;
    expect(stop).toMatchObject({ state: "not_rec", why: "record_mode", is_playing: true, record_mode: false, armed: 1 });
  });
});
