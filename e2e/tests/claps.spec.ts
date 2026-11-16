import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import type { PointerStep } from "./support/live";
import {
  LiveClient,
  MBC_MARKERS,
  dispatchPointer,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  openSurface,
  ready,
  selectPage,
  strip as stripByTrack,
  until,
} from "./support/live";

// Claps from the surface (#220; the approved mockup docs/mockups/claps-v2.html):
// SimLive's band track 0 (Hand1 #) tagged `"CLAPS" +G:CLAPS +CLAPS` is the
// Claps strip, shown in a CLAPS group the test adds to the FOH page. Its
// left column, above SEL: the calm state line, four beat dots and TAP. A tap
// acts on its lift (the hub snaps it to Live's beat), a bar with no tap
// gives the pattern to the device from the next bar, a dot switches its
// beat, a 1 s hold of TAP stops everything. The device itself is the hub's
// UDP peer (nothing listens in the E2E); the page reads the hub's `claps`.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
const TAP: PointerStep[] = [{ type: "pointerdown" }, { wait: 80 }, { type: "pointerup" }];
const HOLD: PointerStep[] = [{ type: "pointerdown" }, { wait: 1200 }, { type: "pointerup" }];

/** The fixture with a CLAPS group on the FOH page. */
function withClapsGroup(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "claps", title: "CLAPS", tags: "CLAPS" });
  return frame;
}

/** The Claps strip. */
const clapsStrip = (page: Page): Locator =>
  page.locator('.slot[data-group="claps"] [data-testid="strip"]').and(page.locator('[data-label="CLAPS"]'));

/** The hub's claps in `/api/status` once `ok` holds. */
async function clapsUntil(ok: (claps: any) => boolean, what: string): Promise<any> {
  return until(async () => (await hubStatus()).claps, ok, what, 12_000);
}

test.describe("Claps from the surface", () => {
  // The iPad's size, both projects.
  test.use({ viewport: { width: 1194, height: 834 } });

  let live: LiveClient;
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    await live.call("band", "live_set", "stop_playing");
    await hostLine("band", "tuner track 0 remove");
    await harness("/hub/layout/reset");
    await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === MBC_MARKERS, "the markers gone", 10_000);
    await clapsUntil((c) => c.track === null, "no Claps track");
    live.close();
  });

  test("taps choose the beats, a dot switches one, a hold stops; the fader keeps its travel", async ({ page }) => {
    await openSurface(page);
    await harness("/hub/layout", { layout: withClapsGroup() });
    await selectPage(page, "foh");
    expect(await hostLine("band", `tuner track 0 set '"CLAPS" +G:CLAPS +CLAPS'`)).toBe("TUNER 1");
    const claps = clapsStrip(page);
    await expect(claps).toHaveCount(1, { timeout: 10_000 });
    await expect(claps).toHaveAttribute("data-claps", "true");
    await ready(claps.getByTestId("fader"));
    await clapsUntil((c) => c.track === "band CLAPS" && c.signature === "4/4", "the Claps track followed");
    const column = claps.getByTestId("claps-col");
    const tap = claps.getByTestId("claps-tap");
    const line = claps.getByTestId("claps-state");
    const dots = claps.getByTestId("claps-dot");

    // The fader keeps its whole travel: the column sits beside it.
    const travel = (await claps.getByTestId("fader").boundingBox())!.height;
    const reference = (await stripByTrack(page, "Hand4 #", "master").getByTestId("fader").boundingBox())!.height;
    expect(travel, `the Claps strip's travel ${travel} equals an unmarked strip's ${reference}`).toBeCloseTo(reference, 0);
    const col = (await column.boundingBox())!;
    const cap = (await claps.getByTestId("fader-cap").boundingBox())!;
    expect(col.x + col.width, "the column ends left of the cap").toBeLessThanOrEqual(cap.x + 0.5);

    // Nothing lit while off: no words, no dot set or ringed; every part owns its touches.
    await expect(column).toHaveAttribute("data-state", "off");
    await expect(line).toHaveText("");
    await expect(dots).toHaveCount(4);
    for (let i = 0; i < 4; i++) {
      await expect(dots.nth(i)).toHaveAttribute("data-set", "false");
      await expect(dots.nth(i)).toHaveAttribute("data-now", "false");
    }
    for (const part of [tap, dots.nth(0)]) {
      expect(
        await part.evaluate((el) => {
          const e = new Event("touchstart", { bubbles: true, cancelable: true });
          el.dispatchEvent(e);
          return e.defaultPrevented;
        }),
      ).toBe(true);
    }

    // A tap while Live stands is refused by the hub: nothing changes.
    await dispatchPointer(tap, TAP);
    await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "claps" && r.step === "refused" && r.why === "Live does not play").length,
      (n: number) => n >= 1,
      "the stopped tap refused",
    );
    await expect(line).toHaveText("");

    // Live plays: two taps half a second apart (120 bpm: a beat).
    await live.call("band", "live_set", "start_playing");
    await until(
      async () => (await hubStatus()).instances.find((i: any) => i.name === "band")?.playing,
      (playing: boolean | undefined) => playing === true,
      "the hub heard Live play",
    );
    await dispatchPointer(tap, TAP);
    await expect(line).toHaveText("TAPPING");
    await expect(line).toHaveAttribute("data-lit", "true");
    await expect(tap).toHaveAttribute("data-state", "tapping");
    await page.waitForTimeout(500);
    await dispatchPointer(tap, TAP);
    const taps = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "claps" && r.step === "tap"),
      (t: any[]) => t.length === 2,
      "two taps recorded",
    );
    const beats = new Set(taps.map((t: any) => t.beat - 1));
    // A bar with no tap: the pattern goes to the device, from the next bar.
    const on = await clapsUntil((c) => c.words.on === true, "the device switched on");
    expect(on.words.pattern).toEqual([0, 1, 2, 3].map((i) => beats.has(i)));
    await expect(line).toHaveText(/^(NEXT BAR|CLAPS)$/);
    await expect(line).toHaveText("CLAPS", { timeout: 6_000 });
    for (let i = 0; i < 4; i++) {
      await expect(dots.nth(i)).toHaveAttribute("data-set", String(beats.has(i)));
    }
    // The ring moves with Live's beat.
    await expect(claps.locator('[data-testid="claps-dot"][data-now="true"]')).toHaveCount(1);

    // A dot set: its beat claps too.
    const free = [0, 1, 2, 3].find((i) => !beats.has(i))!;
    await dispatchPointer(dots.nth(free), [{ type: "pointerdown" }, { type: "pointerup" }]);
    const added = await clapsUntil((c) => c.words.pattern[free] === true, "the dot's beat set");
    expect(added.words.on).toBe(true);
    await expect(dots.nth(free)).toHaveAttribute("data-set", "true");

    // A 1 s hold of TAP: nothing claps, nothing lit.
    await dispatchPointer(tap, HOLD);
    await clapsUntil((c) => c.words.on === false && c.words.pattern.every((b: boolean) => !b), "the device switched off");
    await expect(line).toHaveText("");
    await expect(column).toHaveAttribute("data-state", "off");
    for (let i = 0; i < 4; i++) await expect(dots.nth(i)).toHaveAttribute("data-set", "false");
    await expect(claps.locator('[data-testid="claps-dot"][data-now="true"]')).toHaveCount(0);
    const steps = (await hubEvents()).filter((r: any) => r.ev === "claps").map((r: any) => r.step);
    expect(steps).toContain("stop");
  });
});
