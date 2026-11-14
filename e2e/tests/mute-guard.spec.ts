import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  clipped,
  dispatchPointer,
  dispatchSnapped,
  frames,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  resetMuteGuards,
  selectPage,
  setMuteGuards,
  strip,
  token,
  track,
  until,
} from "./support/live";

// The mute guard chosen on the tablet (#203; the owner, 2026-11-14: which
// fader's mute is protected by a hold is set through its SEL): the channel
// detail's MUTE GUARD (OFF / HOLD) marks the strip in the hub's
// `mute_guards.json`, and every page's mute of that strip then acts only by
// #162's 1 s hold: a tap writes nothing, a hold mutes at its lift. The
// layout's own guards (the frame's `double_click_mute`, a Tuner's `+MG`)
// keep guarding: the fixture's master Hand1 # is in the frame's list, its
// band namesake and band Hand2 # are not. Nothing is drawn on the strip but
// the hold's own fill; nothing about the guard is written to Live.

const HAND2 = track("Hand2 #");

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", HAND2, "mute", false);
});
test.afterEach(async () => {
  await live.set("band", HAND2, "mute", false);
  live.close();
  await resetMuteGuards();
});

/** Errors, warnings and page errors of a second page (the fixture guards the first). */
function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
  return problems;
}

/** A key of the detail's MUTE GUARD: HOLD (`true`) or OFF. */
function key(detail: Locator, on: boolean): Locator {
  return detail.locator(`[data-testid="guard-key"][data-on="${on}"]`);
}

/** The fader's travel of a strip (px). */
async function travel(page: Page, name: string): Promise<number> {
  return (await strip(page, name).getByTestId("fader").boundingBox())!.height;
}

/** Back to the mix from an open detail. */
async function closeDetail(page: Page) {
  await page.getByTestId("detail-exit").click();
  await expect(page.getByTestId("detail")).toHaveCount(0);
}

test.describe("The mute guard (#203)", () => {
  // The iPad's FOH page, where a strip's travel is 322 px.
  test.use({ viewport: { width: 1194, height: 834 } });

  test("MUTE GUARD switched in SEL makes every page's mute hold-only: a tap writes nothing, a hold mutes; OFF makes it one tap again", async ({
    page,
    context,
  }) => {
    const other = await context.newPage();
    const bearer = await token();
    const guards = async () => (await hubStatus(bearer)).mute_guards.strips;
    try {
      await openSurface(page);
      const mute = strip(page, "Hand2 #").getByTestId("mute");
      await ready(mute);
      await expect(mute).toHaveAttribute("data-guarded", "false");
      const before = await travel(page, "Hand2 #");

      await other.setViewportSize({ width: 1194, height: 834 });
      const otherConsole = watchConsole(other);
      await openSurface(other);
      const otherMute = strip(other, "Hand2 #").getByTestId("mute");
      await ready(otherMute);
      await expect(otherMute).toHaveAttribute("data-guarded", "false");

      const since = Date.now();
      const detail = await openDetail(page, "Hand2 #");
      const section = detail.getByTestId("detail-guard");
      await expect(section.locator("h2")).toHaveText("MUTE GUARD");
      await expect(section.getByTestId("guard-now")).toHaveText("OFF");
      const keys = section.getByTestId("guard-key");
      await expect(keys).toHaveText(["OFF", "HOLD"]);
      await expect(key(detail, false)).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, true)).toHaveAttribute("aria-pressed", "false");
      // No layout note for a strip the layout does not guard.
      await expect(section.getByTestId("detail-guard-layout")).toHaveCount(0);
      // After MUTE ALARM, before COLOR.
      const order = await detail.getByTestId("detail-right").evaluate((box) =>
        [...box.querySelectorAll('[data-testid="detail-alarm"], [data-testid="detail-guard"], [data-testid="detail-colour"]')].map((e) =>
          e.getAttribute("data-testid"),
        ),
      );
      expect(order).toEqual(["detail-alarm", "detail-guard", "detail-colour"]);
      // A tap target of its own.
      expect(
        await key(detail, true).evaluate((el) => {
          const e = new Event("touchstart", { bubbles: true, cancelable: true });
          el.dispatchEvent(e);
          return e.defaultPrevented;
        }),
      ).toBe(true);
      // The detail's own MUTE is not guarded yet.
      await expect(detail.getByTestId("mute")).toHaveAttribute("data-guarded", "false");

      // Another strip's mute is not mounted again by this strip's guard.
      const masterMute = strip(other, "Hand2 #", "master").getByTestId("mute");
      await masterMute.evaluate((el) => el.setAttribute("data-e2e-kept", "1"));

      // One real tap: HOLD.
      await key(detail, true).click();
      await expect(key(detail, true)).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, false)).toHaveAttribute("aria-pressed", "false");
      await expect(section.getByTestId("guard-now")).toHaveText("HOLD");
      // The detail's MUTE and the other tablet's strip follow at once; the
      // master's Hand2 # stays one tap.
      await expect(detail.getByTestId("mute")).toHaveAttribute("data-guarded", "true");
      await expect(otherMute).toHaveAttribute("data-guarded", "true");
      await expect(masterMute).toHaveAttribute("data-guarded", "false");
      await expect(masterMute).toHaveAttribute("data-e2e-kept", "1");
      await until(guards, (n) => n === 1, "the hub keeping one guard");
      await closeDetail(page);
      await expect(mute).toHaveAttribute("data-guarded", "true");

      // On the second page: a tap writes nothing (the hint), a 1 s hold
      // mutes at its lift.
      await ready(otherMute);
      const muteKey = JSON.parse((await otherMute.getAttribute("data-keys"))!)[0] as string;
      const tapAt = Date.now();
      await dispatchPointer(otherMute, [{ type: "pointerdown" }, { wait: 200 }, { type: "pointerup" }]);
      await expect(otherMute.getByTestId("mute-hold")).toHaveText("hold");
      expect(await live.get("band", HAND2, "mute"), "a tap writes nothing").toBe(false);
      const held = await dispatchSnapped(otherMute, [{ type: "pointerdown" }, { wait: 1300 }, { snap: true }]);
      expect(held.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
      expect(await live.get("band", HAND2, "mute"), "nothing before the lift").toBe(false);
      await dispatchPointer(otherMute, [{ type: "pointerup" }]);
      await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted at the hold's lift");
      await expect(mute).toHaveAttribute("data-muted", "true");
      // The hub's records since the tap: one write of the mute, the lift's.
      const muteSets = await until(
        async () => (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= tapAt && r.key === muteKey),
        (sets: any[]) => sets.length >= 1,
        "the lift's write recorded",
      );
      expect(muteSets.map((r: any) => r.value), "the tap wrote nothing, the lift true").toEqual([true]);

      // OFF on the second page's detail: one tap unmutes again.
      const otherDetail = await openDetail(other, "Hand2 #");
      await expect(key(otherDetail, true)).toHaveAttribute("aria-pressed", "true");
      await key(otherDetail, false).click();
      await expect(otherDetail.getByTestId("guard-now")).toHaveText("OFF");
      await expect(otherDetail.getByTestId("mute")).toHaveAttribute("data-guarded", "false");
      await other.getByTestId("detail-exit").click();
      await expect(other.getByTestId("detail")).toHaveCount(0);
      await expect(mute).toHaveAttribute("data-guarded", "false");
      await until(guards, (n) => n === 0, "the hub keeping no guard");
      await ready(mute);
      await mute.click();
      await until(() => live.get("band", HAND2, "mute"), (v) => v === false, "one tap unmutes");

      // The hub's records: the two switches, nothing else of the guard;
      // the tap on the guarded mute added no write.
      const records = await hubEvents();
      const switched = records
        .filter((r: any) => r.ev === "mute_guards" && r.ts >= since && r.what === "set")
        .map((r: any) => [r.strip.label, r.on]);
      expect(switched).toEqual([
        ["Hand2 #", true],
        ["Hand2 #", false],
      ]);
      expect(await travel(page, "Hand2 #"), "the travel unchanged").toBeCloseTo(before, 1);
      expect(otherConsole, "the second page's console").toEqual([]);
    } finally {
      await other.close();
    }
  });

  test("a strip the layout guards says so: HOLD whatever the switch, the note under it", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "others");
    await ready(strip(page, "Hand1 #", "master").getByTestId("mute"));
    const detail = await openDetail(page, "Hand1 #", "master");
    const section = detail.getByTestId("detail-guard");
    await expect(section.getByTestId("guard-now")).toHaveText("HOLD (LAYOUT)");
    await expect(key(detail, false)).toHaveAttribute("aria-pressed", "true");
    await expect(section.getByTestId("detail-guard-layout")).toContainText("+MG");
    await expect(detail.getByTestId("mute")).toHaveAttribute("data-guarded", "true");
    // A guard set and cleared on the hub changes nothing here.
    await setMuteGuards({ mute_guards: { master: ["Hand1 #"] } });
    await expect(key(detail, true)).toHaveAttribute("aria-pressed", "true");
    await expect(section.getByTestId("guard-now")).toHaveText("HOLD (LAYOUT)");
    await resetMuteGuards();
    await expect(key(detail, false)).toHaveAttribute("aria-pressed", "true");
    await expect(detail.getByTestId("mute")).toHaveAttribute("data-guarded", "true");
  });

  test("MUTE GUARD fits the detail at the three screen sizes", async ({ page }) => {
    await openSurface(page);
    const detail = await openDetail(page, "Hand2 #");
    for (const size of [
      { width: 1194, height: 834 },
      { width: 844, height: 390 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(size);
      await frames(page);
      const right = detail.getByTestId("detail-right");
      const section = detail.getByTestId("detail-guard");
      await section.scrollIntoViewIfNeeded();
      const box = (await right.boundingBox())!;
      const area = (await section.boundingBox())!;
      const at = `${size.width}x${size.height}`;
      expect(area.x, `${at}: inside the right box`).toBeGreaterThanOrEqual(box.x - 0.5);
      expect(area.x + area.width, `${at}: inside the right box`).toBeLessThanOrEqual(box.x + box.width + 0.5);
      expect(await right.evaluate((el) => el.scrollWidth <= el.clientWidth), `${at}: no horizontal overflow`).toBe(true);
      for (const k of [key(detail, false), key(detail, true)]) {
        expect(await clipped(k), `${at}: a key's text whole`).toEqual([]);
      }
      expect(await clipped(section.getByTestId("guard-now")), `${at}: the state whole`).toEqual([]);
      // Calm: no bright fill on the switch.
      await expect(key(detail, true)).not.toHaveCSS("background-color", "rgb(229, 72, 77)");
    }
  });
});
