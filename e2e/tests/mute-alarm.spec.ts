import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  resetMuteAlarms,
  setMuteAlarms,
  strip,
  token,
  track,
  until,
} from "./support/live";

// The mute alarm (#188; the owner's approved mockup
// docs/mockups/mute-alert-v1.html): a fader marked in its channel detail's
// MUTE ALARM blinks red while it is muted: its name button between
// #e5484d and #4a1015 once a second and a 2 px red frame round the strip
// in sync; steady red under reduced motion. An unmarked mute keeps its calm
// look, and a marked strip that sounds looks like any other. The hub keeps
// the marks (`mute_alarms.json`), so every tablet follows; nothing is
// written to Live.

const HAND2 = track("Hand2 #");
const BRIGHT = "rgb(229, 72, 77)";
const DARK = "rgb(74, 16, 21)";
/** The calm muted name button (strip.css). */
const CALM_MUTED = "rgb(16, 16, 24)";

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
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

type Look = {
  marked: string | null;
  muted: string | null;
  /** The name button's own background (the dark red while alarming). */
  fill: string;
  /** Its bright layer (`::before`): colour, animation, period. */
  bright: string;
  brightAnimation: string;
  brightDuration: string;
  /** The strip's frame: the still dark one (`::before`) and the blinking bright one (`::after`). */
  darkFrameWidth: string;
  darkFrameColour: string;
  frameWidth: string;
  frameColour: string;
  frameAnimation: string;
};

/** What a strip shows: its mark, its mute, the name button and the frame. */
async function lookOf(page: Page, name: string, instance = "band"): Promise<Look> {
  return strip(page, name, instance).evaluate((el) => {
    const mute = el.querySelector('[data-testid="mute"]') as HTMLElement;
    const button = getComputedStyle(mute);
    const layer = getComputedStyle(mute, "::before");
    const dark = getComputedStyle(el, "::before");
    const frame = getComputedStyle(el, "::after");
    return {
      marked: el.getAttribute("data-mute-alarm"),
      muted: mute.getAttribute("data-muted"),
      fill: button.backgroundColor,
      bright: layer.content === "none" ? "none" : layer.backgroundColor,
      brightAnimation: layer.animationName,
      brightDuration: layer.animationDuration,
      darkFrameWidth: dark.content === "none" ? "none" : dark.borderTopWidth,
      darkFrameColour: dark.borderTopColor,
      frameWidth: frame.borderTopWidth,
      frameColour: frame.borderTopColor,
      frameAnimation: frame.animationName,
    };
  });
}

/**
 * The bright layers' opacity (the button's and the frame's) at each `ms` of
 * their 1 s cycle (both animations moved to that time), and how far apart
 * the two animations run (ms).
 */
async function cycle(page: Page, name: string, at: number[]): Promise<{ fill: string[]; frame: string[]; apart: number }> {
  return strip(page, name).evaluate((el, times) => {
    const mute = el.querySelector('[data-testid="mute"]') as HTMLElement;
    const all = document.getAnimations() as CSSAnimation[];
    const on = (a: CSSAnimation, target: Element, pseudo: string) =>
      a.animationName === "mute-alarm" &&
      (a.effect as KeyframeEffect).target === target &&
      (a.effect as KeyframeEffect).pseudoElement === pseudo;
    const fill = all.find((a) => on(a, mute, "::before"))!;
    const frame = all.find((a) => on(a, el, "::after"))!;
    const apart = Math.abs(Number(fill.currentTime) - Number(frame.currentTime));
    const fills: string[] = [];
    const frames: string[] = [];
    for (const t of times) {
      fill.pause();
      frame.pause();
      fill.currentTime = t;
      frame.currentTime = t;
      fills.push(getComputedStyle(mute, "::before").opacity);
      frames.push(getComputedStyle(el, "::after").opacity);
    }
    fill.play();
    frame.play();
    return { fill: fills, frame: frames, apart };
  }, at);
}

/** A key of the detail's MUTE ALARM: `on` or `off`. */
function key(detail: Locator, on: boolean): Locator {
  return detail.locator(`[data-testid="alarm-key"][data-on="${on}"]`);
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

test.describe("The mute alarm (#188)", () => {
  // The iPad's FOH page, where a strip's travel is 322 px.
  test.use({ viewport: { width: 1194, height: 834 } });

  test("MUTE ALARM switched in SEL marks the strip on every page, OFF clears it, nothing reaches Live", async ({ page, context }) => {
    await live.set("band", HAND2, "mute", false);
    await resetMuteAlarms();
    const other = await context.newPage();
    const bearer = await token();
    const marks = async () => (await hubStatus(bearer)).mute_alarms.strips;
    try {
      await openSurface(page);
      await ready(strip(page, "Hand2 #").getByTestId("mute"));
      expect((await lookOf(page, "Hand2 #")).marked, "no strip marked at first").toBeNull();
      const before = await travel(page, "Hand2 #");

      await other.setViewportSize({ width: 1194, height: 834 });
      const otherConsole = watchConsole(other);
      await openSurface(other);
      await ready(strip(other, "Hand2 #").getByTestId("mute"));

      const since = Date.now();
      const detail = await openDetail(page, "Hand2 #");
      const section = detail.getByTestId("detail-alarm");
      await expect(section.locator("h2")).toHaveText("MUTE ALARM");
      await expect(section.getByTestId("alarm-now")).toHaveText("OFF");
      // Two keys, OFF then ON; OFF chosen.
      const keys = section.getByTestId("alarm-key");
      await expect(keys).toHaveCount(2);
      await expect(keys).toHaveText(["OFF", "ON"]);
      await expect(key(detail, false)).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, true)).toHaveAttribute("aria-pressed", "false");
      // It sits in the right box before COLOR.
      const order = await detail.getByTestId("detail-right").evaluate((box) =>
        [...box.querySelectorAll('[data-testid="detail-alarm"], [data-testid="detail-colour"]')].map((e) =>
          e.getAttribute("data-testid"),
        ),
      );
      expect(order).toEqual(["detail-alarm", "detail-colour"]);
      // A tap target of its own (Pointer Events, its touches its own).
      expect(
        await key(detail, true).evaluate((el) => {
          const e = new Event("touchstart", { bubbles: true, cancelable: true });
          el.dispatchEvent(e);
          return e.defaultPrevented;
        }),
      ).toBe(true);
      // Calm: no red in the switch.
      await expect(key(detail, true)).not.toHaveCSS("background-color", BRIGHT);

      // One tap: ON, marked like the selected tab.
      await key(detail, true).click();
      await expect(key(detail, true)).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, false)).toHaveAttribute("aria-pressed", "false");
      await expect(section.getByTestId("alarm-now")).toHaveText("ON");
      await expect(key(detail, true)).toHaveCSS("box-shadow", /inset/);
      // The other tablet at once; the master's Hand2 # stays unmarked.
      await expect(strip(other, "Hand2 #")).toHaveAttribute("data-mute-alarm", "true");
      await expect(strip(other, "Hand2 #", "master")).not.toHaveAttribute("data-mute-alarm", "true");
      await until(marks, (n) => n === 1, "the hub keeping one mark");
      await closeDetail(page);
      await expect(strip(page, "Hand2 #")).toHaveAttribute("data-mute-alarm", "true");
      // Marked and sounding: like any other strip.
      const sounding = await lookOf(page, "Hand2 #");
      expect(sounding.muted).toBe("false");
      expect(sounding.bright).toBe("none");
      expect(sounding.darkFrameWidth).toBe("none");
      expect(sounding.frameAnimation).toBe("none");
      expect(sounding.frameWidth).toBe("0px");

      // OFF clears it everywhere.
      const again = await openDetail(page, "Hand2 #");
      await expect(key(again, true)).toHaveAttribute("aria-pressed", "true");
      await key(again, false).click();
      await expect(again.getByTestId("alarm-now")).toHaveText("OFF");
      await expect(strip(other, "Hand2 #")).not.toHaveAttribute("data-mute-alarm", "true");
      await closeDetail(page);
      await until(marks, (n) => n === 0, "the hub keeping no mark");

      // Nothing reached Live: no write recorded; the hub's records.
      const records = await hubEvents();
      const sets = records.filter((r: any) => r.ev === "set" && r.ts >= since);
      expect(sets, JSON.stringify(sets)).toEqual([]);
      const switched = records
        .filter((r: any) => r.ev === "mute_alarms" && r.ts >= since && r.what === "set")
        .map((r: any) => [r.strip.label, r.on]);
      expect(switched).toEqual([
        ["Hand2 #", true],
        ["Hand2 #", false],
      ]);
      expect(await travel(page, "Hand2 #"), "the travel unchanged").toBeCloseTo(before, 1);
      expect(otherConsole, "the second page's console").toEqual([]);
    } finally {
      await other.close();
      await resetMuteAlarms();
    }
  });

  test("a marked strip blinks red while muted, an unmarked mute stays calm", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    await live.set("master", HAND2, "mute", false);
    await setMuteAlarms({ mute_alarms: { band: ["Hand2 #"] } });
    try {
      await openSurface(page);
      await ready(strip(page, "Hand2 #").getByTestId("mute"));
      await expect(strip(page, "Hand2 #")).toHaveAttribute("data-mute-alarm", "true");
      const before = await travel(page, "Hand2 #");

      // Both Hand2 # muted: the band's marked, the master's not.
      await live.set("band", HAND2, "mute", true);
      await live.set("master", HAND2, "mute", true);
      await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveAttribute("data-muted", "true");
      await expect(strip(page, "Hand2 #", "master").getByTestId("mute")).toHaveAttribute("data-muted", "true");

      // The dark red drawn still, the bright red a layer over it whose
      // opacity blinks once a second (the compositor's work, no repaint).
      const alarm = await lookOf(page, "Hand2 #");
      expect(alarm.fill).toBe(DARK);
      expect(alarm.bright).toBe(BRIGHT);
      expect(alarm.brightAnimation).toBe("mute-alarm");
      expect(alarm.brightDuration).toBe("1s");
      expect([alarm.darkFrameWidth, alarm.darkFrameColour]).toEqual(["2px", DARK]);
      expect([alarm.frameWidth, alarm.frameColour]).toEqual(["2px", BRIGHT]);
      expect(alarm.frameAnimation).toBe("mute-alarm");
      // Bright, dark, bright again over the second, the frame with the
      // button, both cycles together.
      const seen = await cycle(page, "Hand2 #", [0, 500, 1000]);
      expect(seen.fill).toEqual(["1", "0", "1"]);
      expect(seen.frame).toEqual(["1", "0", "1"]);
      expect(seen.apart, "the frame blinks in sync").toBeLessThan(50);
      // The name and MUTED in white.
      await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveCSS("color", "rgb(255, 255, 255)");
      await expect(strip(page, "Hand2 #").getByTestId("mute-mark")).toHaveCSS("color", "rgb(255, 255, 255)");

      // The unmarked mute: today's calm look, nothing blinks.
      const calm = await lookOf(page, "Hand2 #", "master");
      expect(calm.marked).toBeNull();
      expect(calm.fill).toBe(CALM_MUTED);
      expect(calm.bright).toBe("none");
      expect(calm.darkFrameWidth).toBe("none");
      expect(calm.frameAnimation).toBe("none");
      expect(calm.frameWidth).toBe("0px");
      await expect(strip(page, "Hand2 #", "master").getByTestId("mute-mark")).toHaveCSS("color", "rgb(168, 71, 79)");

      // The detail's own MUTE keeps its look (detail.css: red while muted),
      // with no blinking layer.
      const detail = await openDetail(page, "Hand2 #");
      const detailMute = detail.getByTestId("mute");
      const own = await detailMute.evaluate((m) => ({
        bg: getComputedStyle(m).backgroundColor,
        layer: getComputedStyle(m, "::before").content,
        animation: getComputedStyle(m, "::before").animationName,
      }));
      expect(own.layer).toBe("none");
      expect(own.animation).toBe("none");
      expect(own.bg).not.toBe(DARK);
      await closeDetail(page);

      // Unmuted again: the alarm ends.
      await live.set("band", HAND2, "mute", false);
      await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveAttribute("data-muted", "false");
      const ended = await lookOf(page, "Hand2 #");
      expect(ended.bright).toBe("none");
      expect(ended.darkFrameWidth).toBe("none");
      expect(ended.frameWidth).toBe("0px");
      expect(await travel(page, "Hand2 #"), "nothing moves").toBeCloseTo(before, 1);
    } finally {
      await live.set("band", HAND2, "mute", false);
      await live.set("master", HAND2, "mute", false);
      await resetMuteAlarms();
    }
  });

  test("under reduced motion the alarm is steady red", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await live.set("band", HAND2, "mute", true);
    await setMuteAlarms({ mute_alarms: { band: ["Hand2 #"] } });
    try {
      await openSurface(page);
      await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveAttribute("data-muted", "true");
      await expect(strip(page, "Hand2 #")).toHaveAttribute("data-mute-alarm", "true");
      const look = await lookOf(page, "Hand2 #");
      expect(look.brightAnimation).toBe("none");
      expect(look.frameAnimation).toBe("none");
      // The bright layers stay whole over the dark red.
      expect(look.bright).toBe(BRIGHT);
      expect(await strip(page, "Hand2 #").getByTestId("mute").evaluate((m) => getComputedStyle(m, "::before").opacity)).toBe("1");
      expect([look.frameWidth, look.frameColour]).toEqual(["2px", BRIGHT]);
      expect(await strip(page, "Hand2 #").evaluate((s) => getComputedStyle(s, "::after").opacity)).toBe("1");
    } finally {
      await live.set("band", HAND2, "mute", false);
      await resetMuteAlarms();
    }
  });
});
