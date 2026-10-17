import type { Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  LiveClient,
  centre,
  clipped,
  dispatchPointer,
  dispatchSnapped,
  doubleTap,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  hubSubscriptions,
  openSurface,
  pageEvents,
  ready,
  selectPage,
  strip,
  track,
  until,
} from "./support/live";

// Solo, the merged STAGE button (the stage mics and STAGE AUT, #104, D18),
// TechAlert, bindings that stay current with no REFRESH ALL and the hub's
// unfold of the strips' groups (spec F3, F7, F14–F16, I5; #58).

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("Solo buttons", () => {
  test("a solo toggles its group's solo, independently of the others", async ({ page }) => {
    const stems = track("Stems grp#");
    const vocals = track("Vocals Repro grp#");
    await live.set("band", stems, "solo", false);
    await live.set("band", vocals, "solo", true);
    await openSurface(page);
    const solo = page.locator('[data-testid="solo"][data-track="Stems grp#"]');
    await ready(solo);
    await expect(solo).toHaveText("SOLO Stems");
    await expect(solo).toHaveAttribute("data-on", "false");
    await expect(solo).not.toHaveClass(/\bon\b/);
    await solo.click();
    await until(() => live.get("band", stems, "solo"), (v) => v === true, "Stems soloed");
    await expect(solo).toHaveAttribute("data-on", "true");
    await expect(solo).toHaveClass(/\bon\b/);
    // Independent: the other solo stays as it was.
    expect(await live.get("band", vocals, "solo")).toBe(true);
    await solo.click();
    await until(() => live.get("band", stems, "solo"), (v) => v === false, "Stems unsoloed");
    await live.set("band", vocals, "solo", false);
  });

  test("a solo whose group track is renamed turns red and takes no tap (I5)", async ({ page }) => {
    const stems = track("Stems grp#");
    await live.set("band", stems, "solo", false);
    await openSurface(page);
    const solo = page.locator('[data-testid="solo"][data-track="Stems grp#"]');
    await ready(solo);
    await expect(solo).toHaveAttribute("data-binding", "ready");
    try {
      expect(await hostLine("band", 'rename "Stems grp#" "Stems grp X"')).toBe("RENAMED 1");
      await expect(solo).toHaveAttribute("data-binding", "unresolved");
      await expect(solo).toHaveAttribute("aria-disabled", "true");
      await expect(solo).toHaveCSS("background-color", "rgb(255, 59, 48)");
      const { x, y } = await centre(solo);
      await page.mouse.click(x, y);
      await page.waitForTimeout(300);
      expect(await live.get("band", track("Stems grp X"), "solo")).toBe(false);
    } finally {
      await hostLine("band", 'rename "Stems grp X" "Stems grp#"');
    }
    await expect(solo).toHaveAttribute("data-binding", "ready");
    await ready(solo);
  });
});

test.describe("The merged STAGE button (#104, D18)", () => {
  const mics = track("Mics Stage #");
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

  test.afterEach(async () => {
    live.setHub("stage_aut", false);
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
  });

  /** The page clock (the flight recorder's `t`) now. */
  function pageNow(page: Page): Promise<number> {
    return page.evaluate(() => performance.timeOrigin + performance.now());
  }

  /** The rail steps the page recorded since `since`, once uploaded. */
  async function railSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
    return until(
      async () =>
        pageEvents(await hubEvents())
          .filter((e: any) => e.ev === "rail" && e.t >= since)
          .map((e: any) => e.what),
      ok,
      "the rail steps uploaded",
      15_000,
    );
  }

  test("a tap acts on the lift, a 1 s hold turns STAGE AUT on, and the rule follows the transport until a tap overrides it", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    live.setHub("stage_aut", false);
    await openSurface(page);
    const since = await pageNow(page);
    const button = page.getByTestId("stage-mics");
    await ready(button);
    await expect(page.getByTestId("stage-aut")).toHaveCount(0);
    await expect(button.locator(".btn-text")).toHaveText("STAGE");
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(button).toHaveAttribute("data-muted", "false");
    // Lit while the mics are live, as TouchOSC's button and every strip's
    // mute (F12, lit when audible): a lit STAGE reads "the stage is open".
    await expect(button).toHaveClass(/\bon\b/);
    // Its touches are its own (#43 PR G).
    expect(
      await button.evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);

    // A tap on the lift: nothing while the finger is down.
    await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 400 }]);
    expect(await live.get("band", mics, "mute")).toBe(false);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    await until(() => live.get("band", mics, "mute"), (v) => v === true, "muted by the tap");
    await expect(button).not.toHaveClass(/\bon\b/);
    await button.click();
    await until(() => live.get("band", mics, "mute"), (v) => v === false, "live by the tap");
    await expect(button).toHaveClass(/\bon\b/);

    // A 1 s hold (the owner's ruling on #104) turns AUT on at its lift: the
    // fill (over RAIL_HOLD_MS, its CSS time from `--hold-ms`) and its line
    // before the full mark, the full line after it (both read in the page's
    // timer order, PR #133's review, M1), nothing sent and the mics untouched
    // until the lift.
    const on = await dispatchSnapped(button, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(on.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "drž… zapína AUT" });
    expect(on.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "pusť — zapne AUT" });
    expect(await button.locator(".hold").evaluate((el) => getComputedStyle(el).transitionDuration)).toBe("1s");
    expect(live.hub.get("stage_aut")).toBe(false);
    expect(await live.get("band", mics, "mute")).toBe(false);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("aut-badge")).toHaveText("AUT");
    await expect(button).toHaveAttribute("data-holding", "false");
    expect(await live.get("band", mics, "mute")).toBe(false);
    await expect(button.getByTestId("btn-sub")).toHaveText("stojí · zapnuté");

    // The rule: playing → muted, stopped → live.
    await live.call("band", "live_set", "start_playing");
    await until(() => live.get("band", mics, "mute"), (v) => v === true, "muted while playing");
    await expect(button.getByTestId("btn-sub")).toHaveText("hrá · stíšené");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button).not.toHaveClass(/\bon\b/);
    // A tap during the song overrides it until the next stop: outlined AUT.
    await button.click();
    await until(() => live.get("band", mics, "mute"), (v) => v === false, "live by hand during the song");
    await expect(button).toHaveAttribute("data-aut", "held");
    await expect(button.getByTestId("btn-sub")).toHaveText("ručne do zastavenia");
    await live.call("band", "live_set", "stop_playing");
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("btn-sub")).toHaveText("stojí · zapnuté");
    expect(await live.get("band", mics, "mute")).toBe(false);

    // The page's steps in its flight recorder.
    const steps = await railSteps(since, (s) => s.includes("hold") && s.filter((w) => w === "tap").length >= 3);
    expect(steps).toEqual(expect.arrayContaining(["press", "tap", "hold"]));
  });

  test("a slide or a hidden page ends a press with nothing done, and a second 1 s hold turns STAGE AUT off", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    live.setHub("stage_aut", true);
    await openSurface(page);
    const since = await pageNow(page);
    const button = page.getByTestId("stage-mics");
    await ready(button);
    await expect(button).toHaveAttribute("data-aut", "run");
    await expect(button.getByTestId("btn-sub")).toHaveText("stojí · zapnuté");

    // A slide past 10 px ends the press: no hold, no tap.
    await dispatchPointer(button, [
      { type: "pointerdown" },
      { type: "pointermove", dx: 0, dy: 0 },
      { type: "pointermove", dx: 0, dy: 11 },
      { wait: 1200 },
      { type: "pointerup", dy: 11 },
    ]);
    await expect(button).toHaveAttribute("data-holding", "false");
    expect(live.hub.get("stage_aut")).toBe(true);
    expect(await live.get("band", mics, "mute")).toBe(false);
    // A hidden page ends a press too.
    await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 300 }]);
    await expect(button).toHaveAttribute("data-holding", "true");
    await setHidden(page, true);
    await setHidden(page, false);
    await expect(button).toHaveAttribute("data-holding", "false");
    await dispatchPointer(button, [{ wait: 1000 }, { type: "pointerup" }]);
    expect(live.hub.get("stage_aut")).toBe(true);
    expect(await live.get("band", mics, "mute")).toBe(false);
    await expect(button).toHaveAttribute("data-aut", "run");

    // A second 1 s hold turns AUT off at its lift (its lines read in the
    // page's timer order, PR #133's review, M1); the mics stay as they are.
    const off = await dispatchSnapped(button, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(off.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "drž… vypína AUT" });
    expect(off.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "pusť — vypne AUT" });
    expect(live.hub.get("stage_aut")).toBe(true);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
    await expect(button).toHaveAttribute("data-aut", "off");
    await expect(button.getByTestId("aut-badge")).toHaveCount(0);
    await expect(button.getByTestId("btn-sub")).toHaveText("");
    expect(await live.get("band", mics, "mute")).toBe(false);

    // The page's steps in its flight recorder.
    const steps = await railSteps(since, (s) => s.includes("hold") && s.includes("lost"));
    expect(steps).toEqual(expect.arrayContaining(["press", "slid", "lost", "hold"]));
    expect(steps).not.toContain("tap");
  });

  test("a press whose end never came ends at the next primary finger elsewhere: no hold 1 s later (#124)", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    live.setHub("stage_aut", false);
    const volume = `${track("Hand2 #")} mixer_device volume`;
    const level = await live.get("band", volume, "value");
    try {
      await openSurface(page);
      const since = await pageNow(page);
      const button = page.getByTestId("stage-mics");
      await ready(button);
      const fader = strip(page, "Hand2 #").getByTestId("fader");
      await ready(fader);
      // A finger on STAGE whose end never comes: a dispatched pointer
      // captures nothing, as a finger the system took (#124).
      await dispatchPointer(button, [{ type: "pointerdown" }], 21);
      await expect(button).toHaveAttribute("data-holding", "true");
      // The next primary finger, on a fader's cap, ends it at once.
      await dispatchPointer(fader.locator(".fader-cap"), [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 22);
      await expect(button).toHaveAttribute("data-holding", "false");
      await page.waitForTimeout(1200);
      expect(live.hub.get("stage_aut")).toBe(false);
      expect(await live.get("band", mics, "mute")).toBe(false);
      await expect(button).toHaveAttribute("data-aut", "off");
      const steps = await railSteps(since, (s) => s.includes("lost"));
      expect(steps).toEqual(["press", "lost"]);
    } finally {
      await live.set("band", volume, "value", level);
    }
  });

  test("a hold with no lift stays full and sends nothing; the next primary finger ends it lost (#104)", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    live.setHub("stage_aut", false);
    const volume = `${track("Hand2 #")} mixer_device volume`;
    const level = await live.get("band", volume, "value");
    try {
      await openSurface(page);
      const since = await pageNow(page);
      const button = page.getByTestId("stage-mics");
      await ready(button);
      const fader = strip(page, "Hand2 #").getByTestId("fader");
      await ready(fader);
      // A finger on STAGE whose end never comes (a dispatched pointer
      // captures nothing, #124's state), and nothing else touched.
      await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 1200 }], 21);
      await expect(button).toHaveAttribute("data-full", "true");
      await expect(button).toHaveAttribute("data-holding", "true");
      await expect(button.getByTestId("btn-sub")).toHaveText("pusť — zapne AUT");
      expect(live.hub.get("stage_aut")).toBe(false);
      expect(await live.get("band", mics, "mute")).toBe(false);
      expect(await railSteps(since, (s) => s.includes("full"))).toEqual(["press", "full"]);
      // At 3 s still nothing.
      await page.waitForTimeout(1800);
      await expect(button).toHaveAttribute("data-full", "true");
      expect(live.hub.get("stage_aut")).toBe(false);
      expect(await live.get("band", mics, "mute")).toBe(false);
      // The next primary finger, on a fader's cap, ends it with nothing done.
      await dispatchPointer(fader.locator(".fader-cap"), [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 22);
      await expect(button).toHaveAttribute("data-full", "false");
      await expect(button).toHaveAttribute("data-holding", "false");
      await page.waitForTimeout(300);
      expect(live.hub.get("stage_aut")).toBe(false);
      await expect(button).toHaveAttribute("data-aut", "off");
      expect(await railSteps(since, (s) => s.includes("lost"))).toEqual(["press", "full", "lost"]);
    } finally {
      await live.set("band", volume, "value", level);
    }
  });

  test("a hold turns STAGE AUT on and off at its lift once full, never before (#104)", async ({ page }) => {
    await live.call("band", "live_set", "stop_playing");
    await live.set("band", mics, "mute", false);
    live.setHub("stage_aut", false);
    await openSurface(page);
    const since = await pageNow(page);
    const button = page.getByTestId("stage-mics");
    await ready(button);
    const sub = button.getByTestId("btn-sub");
    // On: full at 1 s, unchanged until the lift (about 1.3 s).
    await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 1100 }]);
    await expect(button).toHaveAttribute("data-full", "true");
    await expect(sub).toHaveText("pusť — zapne AUT");
    await page.waitForTimeout(200);
    expect(live.hub.get("stage_aut"), "nothing before the lift").toBe(false);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on at the lift");
    await expect(button).toHaveAttribute("data-full", "false");
    await expect(button).toHaveAttribute("data-aut", "run");
    // Off the same way.
    await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 1100 }]);
    await expect(button).toHaveAttribute("data-full", "true");
    await expect(sub).toHaveText("pusť — vypne AUT");
    await page.waitForTimeout(200);
    expect(live.hub.get("stage_aut"), "nothing before the lift").toBe(true);
    await dispatchPointer(button, [{ type: "pointerup" }]);
    await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off at the lift");
    await expect(button).toHaveAttribute("data-aut", "off");
    expect(await live.get("band", mics, "mute")).toBe(false);
    expect(
      await railSteps(since, (s) => s.filter((w) => w === "hold").length >= 2),
    ).toEqual(["press", "full", "hold", "press", "full", "hold"]);
  });

  test.describe("at the iPad's size", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    test("with STAGE AUT on, each of its four lines, the full press's two and the pill fit the button", async ({ page }) => {
      await live.call("band", "live_set", "stop_playing");
      await live.set("band", mics, "mute", false);
      live.setHub("stage_aut", true);
      await openSurface(page);
      const button = page.getByTestId("stage-mics");
      await ready(button);
      const sub = button.getByTestId("btn-sub");
      const fits = async (text: string, aut: string) => {
        await expect(sub).toHaveText(text);
        await expect(button).toHaveAttribute("data-aut", aut);
        expect(await clipped(button), `the button with "${text}"`).toEqual([]);
        expect(await clipped(sub), `its line "${text}"`).toEqual([]);
        // The label and the pill side by side within the row (a row is no
        // clipping box: its glyphs' content area may pass its line box).
        const row = await button.locator(".btn-row").evaluate((el) => [el.scrollWidth, el.clientWidth]);
        expect(row[0], `its label and pill side by side with "${text}"`).toBeLessThanOrEqual(row[1]);
      };
      await fits("stojí · zapnuté", "run");
      // A tap mutes the mics while Live stands: the override until the start.
      await button.click();
      await until(() => live.get("band", mics, "mute"), (v) => v === true, "muted by the tap");
      await fits("ručne do štartu", "held");
      await live.call("band", "live_set", "start_playing");
      await fits("hrá · stíšené", "run");
      // A tap opens them during the song: the override until the stop.
      await button.click();
      await until(() => live.get("band", mics, "mute"), (v) => v === false, "live by the tap");
      await fits("ručne do zastavenia", "held");
      // The full press's two lines (#104): a hold to full, then its lift.
      await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 1200 }]);
      await fits("pusť — vypne AUT", "held");
      await dispatchPointer(button, [{ type: "pointerup" }]);
      await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
      await dispatchPointer(button, [{ type: "pointerdown" }, { wait: 1200 }]);
      await fits("pusť — zapne AUT", "off");
      await dispatchPointer(button, [{ type: "pointerup" }]);
      await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on");
    });
  });
});

test.describe("TechAlert", () => {
  test("the overlay blinks while the TechAlert track is unmuted", async ({ page }) => {
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    await openSurface(page);
    const overlay = page.getByTestId("alert");
    await expect(overlay).toHaveAttribute("data-active", "false");
    await expect(overlay).toHaveAttribute("data-visible", "false");
    try {
      await live.set("band", alert, "mute", false);
      await expect(overlay).toHaveAttribute("data-active", "true");
      // Sampled inside the page every 15 ms for 900 ms (a Playwright read
      // per sample takes long enough in WebKit to alias the 300 ms blink).
      const seen = await overlay.evaluate(async (el) => {
        const states = new Set<string>();
        const end = performance.now() + 900;
        while (performance.now() < end) {
          states.add(el.getAttribute("data-visible") ?? "");
          await new Promise((done) => setTimeout(done, 15));
        }
        return [...states].sort();
      });
      expect(seen).toEqual(["false", "true"]);
      // The overlay never takes a touch.
      await expect(overlay).toHaveCSS("pointer-events", "none");
    } finally {
      await live.set("band", alert, "mute", true);
    }
    await expect(overlay).toHaveAttribute("data-active", "false");
    for (let i = 0; i < 10; i++) {
      await expect(overlay).toHaveAttribute("data-visible", "false");
      await page.waitForTimeout(70);
    }
    // Its button shows the same track's mute: dark while muted.
    await expect(page.getByTestId("alert-toggle")).toHaveAttribute("data-on", "false");
  });

  test("the TechAlert button unmutes and mutes its track; the wash follows", async ({ page }) => {
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    await openSurface(page);
    const button = page.getByTestId("rail").getByTestId("alert-toggle");
    const overlay = page.getByTestId("alert");
    await ready(button);
    try {
      await button.click();
      await until(() => live.get("band", alert, "mute"), (v) => v === false, "TechAlert on");
      await expect(button).toHaveAttribute("data-on", "true");
      await expect(overlay).toHaveAttribute("data-active", "true");
      await button.click();
      await until(() => live.get("band", alert, "mute"), (v) => v === true, "TechAlert off");
      await expect(button).toHaveAttribute("data-on", "false");
      await expect(overlay).toHaveAttribute("data-active", "false");
    } finally {
      await live.set("band", alert, "mute", true);
    }
  });
});

test.describe("A guarded TechAlert", () => {
  test("needs a second tap within 500 ms, as a guarded mute", async ({ page }) => {
    // #21 review: a TechAlert strip in double_click_mute keeps its guard.
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    changed.global[0].mute_guard = true;
    await openSurface(page);
    try {
      await harness("/hub/layout", { layout: changed });
      const button = page.getByTestId("rail").getByTestId("alert-toggle");
      await page.waitForTimeout(500);
      await ready(button);
      await button.click();
      await expect(button).toHaveClass(/\barmed\b/);
      await page.waitForTimeout(700);
      expect(await live.get("band", alert, "mute")).toBe(true);
      await expect(button).not.toHaveClass(/\barmed\b/);
      await doubleTap(button, 150);
      await until(() => live.get("band", alert, "mute"), (v) => v === false, "TechAlert on after the confirming tap");
    } finally {
      await live.set("band", alert, "mute", true);
      await harness("/hub/layout/reset");
    }
  });
});

test.describe("SOLO clear", () => {
  test("the pill shows while a solo of the page is on and clears only those", async ({ page }) => {
    // #21: the pill in the top bar turns off every solo Live reports on.
    const stems = track("Stems grp#");
    const vocals = track("Vocals Repro grp#");
    await live.set("band", stems, "solo", false);
    await live.set("band", vocals, "solo", false);
    await openSurface(page);
    const pill = page.getByTestId("solo-clear");
    await ready(page.locator('[data-testid="solo"][data-track="Stems grp#"]'));
    await expect(pill).toHaveAttribute("data-on", "false");
    await expect(pill).toBeHidden();
    try {
      await live.set("band", vocals, "solo", true);
      await expect(pill).toHaveAttribute("data-on", "true");
      await expect(pill).toBeVisible();
      await pill.click();
      await until(() => live.get("band", vocals, "solo"), (v) => v === false, "Vocals unsoloed");
      expect(await live.get("band", stems, "solo")).toBe(false);
      await expect(pill).toBeHidden();
      // Both on: both off. The pill clears the solos the page knows of, and
      // it shows as soon as one is on: wait until the page shows both (a
      // click before Stems' solo reached the page cleared only Vocals, #43
      // PR E's red run).
      await live.set("band", vocals, "solo", true);
      await live.set("band", stems, "solo", true);
      await expect(page.locator('[data-testid="solo"][data-track="Stems grp#"]')).toHaveAttribute("data-on", "true");
      await expect(page.locator('[data-testid="solo"][data-track="Vocals Repro grp#"]')).toHaveAttribute("data-on", "true");
      await expect(pill).toBeVisible();
      await pill.click();
      await until(() => live.get("band", stems, "solo"), (v) => v === false, "Stems unsoloed");
      await until(() => live.get("band", vocals, "solo"), (v) => v === false, "Vocals unsoloed");
      // A page without solos has no pill to show.
      await live.set("band", vocals, "solo", true);
      await page.locator('[data-testid="tab"][data-page="cue"]').click();
      await expect(pill).toBeHidden();
    } finally {
      await live.set("band", stems, "solo", false);
      await live.set("band", vocals, "solo", false);
    }
  });
});

test.describe("Bindings stay current with no REFRESH ALL (#58)", () => {
  test("a renamed track turns its strip red; renamed back, the hub binds it again by itself", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("data-binding", "unresolved");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // The hub's name guard (F3) heals the live subscription.
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
  });

  test("a subscription made while the name is gone heals by itself when the track is renamed back", async ({ page }) => {
    await openSurface(page);
    const hand2 = strip(page, "Hand2 #");
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    try {
      expect(await hostLine("band", 'rename "Hand2 #" "Hand2 X"')).toBe("RENAMED 1");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
      // Away from the FOH page its strips are unsubscribed; back on it they
      // are subscribed again while the name is gone, so the new
      // subscription has no track whose name the hub could listen to.
      const before = await hubSubscriptions();
      await selectPage(page, "cue");
      await until(() => hubSubscriptions(), (n) => n < before, "the FOH page's subscriptions ended");
      await selectPage(page, "foh");
      await expect(hand2.getByTestId("fader")).toHaveAttribute("data-binding", "unresolved");
      await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "unbound");
    } finally {
      await hostLine("band", 'rename "Hand2 X" "Hand2 #"');
    }
    // Renamed back with no list change: the hub watches every track's name
    // while a binding is in error, so the strip heals with nothing pressed.
    await expect(hand2.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await ready(hand2.getByTestId("fader"));
    await expect(page.getByTestId("refresh")).toHaveCount(0);
  });

  test("the hub keeps a strip track's group unfolded, also after Live comes back", async () => {
    // Live sends no meter of a track inside a folded group (#58): the hub
    // holds the groups the strips' tracks sit in open, with no page.
    const group = track("Vocals Repro grp#");
    await until(hubStatus, (s) => s.instances[0].unfolded.includes("Vocals Repro grp#"), "the hub holds the strip's group");
    await live.set("band", group, "fold_state", true);
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded by the hub");
    // After a Live restart (a set load alike) it still unfolds it.
    await harness("/host/band/restart");
    await until(hubStatus, (s) => s.instances[0].online, "Live back", 10_000);
    await live.set("band", group, "fold_state", true);
    await until(() => live.get("band", group, "fold_state"), (v) => v === 0, "unfolded once Live is back");
  });
});
