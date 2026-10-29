import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  dispatchPointer,
  dispatchSnapped,
  hubEvents,
  openDetail,
  openSurface,
  pageEvents,
  ready,
  selectPage,
  strip,
  track,
  until,
} from "./support/live";

// A guarded mute acts by a hold (#162, the owner's request of 2026-10-29;
// it replaced spec F12's double tap within 500 ms, which nobody knew was
// needed): a press fills the name button over 1 s (RAIL_HOLD_MS, as the rail
// buttons), the line under the name says what the hold will do, at full
// what the lift does, and the lift mutes or unmutes, the only write. A lift
// before full shows the `hold` hint and writes nothing; a slide past 10 px,
// a hidden page and the next primary finger elsewhere (#124) end the press
// with nothing written. The channel detail's MUTE holds the same way. An
// unguarded mute stays one tap. The fixture's master Hand1 # (others) is in
// the TouchOSC project's `double_click_mute`, its band namesake is not.

const HAND1 = track("Hand1 #");
const HAND2 = track("Hand2 #");

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("master", HAND1, "mute", false);
});
test.afterEach(async () => {
  await live.set("master", HAND1, "mute", false);
  live.close();
});

/** The page clock (the flight recorder's `t`) now. */
function pageNow(page: Page): Promise<number> {
  return page.evaluate(() => performance.timeOrigin + performance.now());
}

/** The guarded mute's steps the page recorded since `since` (`rail` events of the button `mute`), once uploaded. */
async function muteSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.button === "mute" && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the mute's steps uploaded",
    15_000,
  );
}

/** The page hidden or shown, as an iPad's lock or app switch does (deck.spec's `setHidden`). */
async function setHidden(page: Page, hidden: boolean) {
  await page.evaluate((on) => {
    const doc = document as any;
    if (on) {
      Object.defineProperty(doc, "hidden", { configurable: true, get: () => true });
      Object.defineProperty(doc, "visibilityState", { configurable: true, get: () => "hidden" });
    } else {
      delete doc.hidden;
      delete doc.visibilityState;
    }
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  }, hidden);
}

/** The guarded Hand1 # name button on the others page, ready. */
async function guardedMute(page: Page): Promise<Locator> {
  await openSurface(page);
  await selectPage(page, "others");
  const mute = strip(page, "Hand1 #", "master").getByTestId("mute");
  await ready(mute);
  await expect(mute).toHaveAttribute("data-guarded", "true");
  return mute;
}

test.describe("A guarded mute (#162)", () => {
  test("a lift before 1 s shows the hint and writes nothing; a hold fills, says what the lift does and mutes, then unmutes, at the lift", async ({ page }) => {
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    const line = mute.getByTestId("mute-hold");
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);

    // A tap (a lift at 200 ms): the hint in the marks line's place,
    // nothing written, the hint gone after its 1.2 s (the recorder's steps,
    // below, show no write either).
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 200 }, { type: "pointerup" }]);
    await expect(line).toHaveText("hold");
    await expect(mute.getByTestId("strip-marks")).toBeHidden();
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);
    await expect(mute.getByTestId("strip-marks")).toBeAttached();
    expect(await live.get("master", HAND1, "mute")).toBe(false);

    // A 1 s hold: filling with its line before the full mark, full with the
    // lift's line after it (read in the page's timer order), nothing written
    // until the lift.
    const on = await dispatchSnapped(mute, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(on.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "hold… mute" });
    expect(on.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
    expect(await mute.locator(".hold").evaluate((el) => getComputedStyle(el).transitionDuration)).toBe("1s");
    expect(await live.get("master", HAND1, "mute"), "nothing before the lift").toBe(false);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === true, "muted at the lift");
    await expect(mute).toHaveAttribute("data-muted", "true");
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);
    // The band's track of the same name is another instance's (spec F2, X4).
    expect(await live.get("band", HAND1, "mute")).toBe(false);

    // The same hold unmutes.
    const off = await dispatchSnapped(mute, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(off.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "hold… unmute" });
    expect(off.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "lift: unmute" });
    expect(await live.get("master", HAND1, "mute")).toBe(true);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === false, "unmuted at the lift");
    await expect(mute).toHaveAttribute("data-muted", "false");

    // The flight recorder: the tap's hint, then each hold's press, full mark
    // and lift; no write but the two lifts.
    expect(await muteSteps(since, (s) => s.filter((w) => w === "hold").length >= 2)).toEqual([
      "press",
      "hint",
      "press",
      "full",
      "hold",
      "press",
      "full",
      "hold",
    ]);
  });

  test("a slide, a hidden page or the next primary finger elsewhere ends a press, full or not, with nothing written", async ({ page }) => {
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    // Another control: the same strip's fader.
    const other = strip(page, "Hand1 #", "master").getByTestId("fader");
    await ready(other);

    // A slide past 10 px after the full mark: nothing, no hint.
    await dispatchPointer(mute, [
      { type: "pointerdown" },
      { type: "pointermove", dx: 0, dy: 0 },
      { wait: 1200 },
      { type: "pointermove", dx: 0, dy: -11 },
      { type: "pointerup", dy: -11 },
    ]);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    // A hidden page ends a press; its later lift writes nothing.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 300 }]);
    await expect(mute).toHaveAttribute("data-holding", "true");
    await setHidden(page, true);
    await setHidden(page, false);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await dispatchPointer(mute, [{ wait: 1000 }, { type: "pointerup" }]);
    // A full press whose end never came (a dispatched pointer captures
    // nothing, #124's state) ends at the next primary finger on another
    // control.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 1200 }], 21);
    await expect(mute).toHaveAttribute("data-full", "true");
    await expect(mute.getByTestId("mute-hold")).toHaveText("lift: mute");
    await dispatchPointer(other.locator(".fader-cap"), [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 22);
    await expect(mute).toHaveAttribute("data-full", "false");
    await expect(mute).toHaveAttribute("data-holding", "false");

    expect(await muteSteps(since, (s) => s.filter((w) => w === "lost").length >= 2)).toEqual([
      "press",
      "full",
      "slid",
      "press",
      "lost",
      "press",
      "full",
      "lost",
    ]);
    expect(await live.get("master", HAND1, "mute"), "nothing written").toBe(false);
  });

  test("the channel detail's MUTE holds the same way, its line in the label's place", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "others");
    const detail = await openDetail(page, "Hand1 #", "master");
    const mute = detail.getByTestId("mute");
    await ready(mute);
    await expect(mute).toHaveAttribute("data-guarded", "true");
    const since = await pageNow(page);
    const label = mute.getByTestId("strip-label");
    await expect(label).toBeVisible();
    // A tap: the hint in the label's place, nothing written.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await expect(mute.getByTestId("mute-hold")).toHaveText("hold");
    await expect(label).toBeHidden();
    await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    await expect(label).toBeVisible();
    // A hold to full, then the lift mutes.
    const held = await dispatchSnapped(mute, [{ type: "pointerdown" }, { wait: 1300 }, { snap: true }]);
    expect(held.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
    expect(await live.get("master", HAND1, "mute")).toBe(false);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === true, "muted at the detail's lift");
    await expect(mute).toHaveAttribute("data-muted", "true");
    expect(await muteSteps(since, (s) => s.includes("hold"))).toEqual(["press", "hint", "press", "full", "hold"]);
  });

  test("an unguarded mute stays one tap on its down", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    try {
      await openSurface(page);
      const mute = strip(page, "Hand2 #").getByTestId("mute");
      await ready(mute);
      await expect(mute).toHaveAttribute("data-guarded", "false");
      await expect(mute.locator(".hold")).toHaveCount(0);
      // The down writes, before any lift.
      await dispatchPointer(mute, [{ type: "pointerdown" }]);
      await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted on the down");
      await dispatchPointer(mute, [{ type: "pointerup" }]);
      expect(await mute.getAttribute("data-holding"), "no hold state").toBeNull();
      await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    } finally {
      await live.set("band", HAND2, "mute", false);
    }
  });
});
