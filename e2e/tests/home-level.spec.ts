import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  doubleTap,
  harness,
  hostLine,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  selectPage,
  shown,
  strip,
  track,
  until,
  value2db,
  volume,
  MBC_MARKERS,
} from "./support/live";

// A strip's home level (#225): a Tuner's `+HOME:<dB>` moves the strip's
// unity mark (`--u` on its fader zone, and the scale's brighter label) from
// 0 dB to that level, on the strip and in its channel detail, and a double
// tap glides there instead of to 0 dB. A strip without the tag keeps 0 dB.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
/** The second track of the test site (Hand2 #, band track 1). */
const HAND2 = track("Hand2 #");

/** The fixture with a MARKERS group (the strips of that tag group) in the FOH page's second row. */
function withMarkers(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "markers", title: "MARKERS", tags: "MARKERS" });
  return frame;
}

/** Waits until the hub holds no Tuner marker but the MBC set's. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === MBC_MARKERS, "the Tuner markers gone", 10_000);
}

/**
 * Where a fader zone's unity mark sits and which scale label is the brighter
 * one, read in one task in the page (#192): `--u` (the mark's position) and
 * the top of the scale's label at `db`, as a position (1 − top %), and the
 * `data-db` of every label with the `unity` class.
 */
async function markOf(page: Page, zone: string, db: number): Promise<{ u: number; tick: number; bright: string[] }> {
  return page.evaluate(
    ([sel, at]) => {
      const el = document.querySelector(sel) as HTMLElement | null;
      if (!el) return { u: NaN, tick: NaN, bright: [] as string[] };
      const u = Number(el.style.getPropertyValue("--u"));
      const scale = el.querySelector(".scale") ?? el.parentElement?.querySelector(".scale") ?? null;
      const label = scale?.querySelector(`.tick[data-db="${at}"]`) as HTMLElement | null;
      const top = label ? parseFloat(label.style.top) : NaN;
      const bright = Array.from(scale?.querySelectorAll(".tick.unity") ?? []).map((t) => t.getAttribute("data-db") ?? "");
      return { u, tick: 1 - top / 100, bright };
    },
    [zone, String(db)] as const,
  );
}

/** Expects the zone's unity mark on the scale's `db` label, and only that label bright. */
async function expectMarkAt(page: Page, zone: string, db: number) {
  const mark = await markOf(page, zone, db);
  expect(Number.isFinite(mark.u), `${zone}: --u is set`).toBe(true);
  expect(Math.abs(mark.u - mark.tick), `${zone}: --u ${mark.u} on the ${db} dB label at ${mark.tick}`).toBeLessThan(0.001);
  expect(mark.bright).toEqual([String(db)]);
}

/** Puts Live's volume of Hand2 # at 0.5 and waits until `fader` shows it. */
async function startAtHalf(live: LiveClient, fader: Locator) {
  await live.set("band", volume(HAND2), "value", 0.5);
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
}

test.describe("A strip's home level (#225)", () => {
  let live: LiveClient;
  let before: number;

  test.beforeEach(async () => {
    live = await LiveClient.open();
    before = await live.get("band", volume(HAND2), "value");
  });

  test.afterEach(async () => {
    await live.set("band", volume(HAND2), "value", before);
    live.close();
  });

  test("a plain strip keeps its mark at 0 dB and a double tap goes to 0 dB", async ({ page }) => {
    await openSurface(page);
    const plain = strip(page, "Hand2 #");
    const fader = plain.getByTestId("fader");
    await startAtHalf(live, fader);
    await expectMarkAt(page, '[data-testid="strip"][data-track="Hand2 #"][data-instance="band"] .strip-zone', 0);
    await doubleTap(fader);
    await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - 0.85) < 1e-6, "0 dB", 6000);
    await expect(plain.getByTestId("db")).toHaveText("0.0");
  });

  test("a +HOME:-6 strip shows its mark at -6 dB and a double tap lands on -6.0 dB, on the strip and in its detail", async ({ page }) => {
    await openSurface(page);
    const marker = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      expect(await hostLine("band", `tuner track 1 set '"CG" +G:MARKERS +HOME:-6'`)).toBe("TUNER 1");
      await expect(marker).toHaveCount(1, { timeout: 10_000 });
      await expect(marker).toHaveAttribute("data-label", "CG");
      // Composed without a problem: no TAG? mark.
      await expect(marker).not.toHaveAttribute("data-mark", /./);
      const zone = '.slot[data-group="markers"] [data-testid="strip"] .strip-zone';
      await expectMarkAt(page, zone, -6);

      // The strip's double tap: Live's −6.0 dB.
      const fader = marker.getByTestId("fader");
      await startAtHalf(live, fader);
      await doubleTap(fader);
      const landed = await until(
        () => live.get("band", volume(HAND2), "value"),
        (v) => Math.abs(value2db(v) + 6) < 0.05,
        "-6 dB",
        6000,
      );
      expect(Math.abs(value2db(landed) + 6)).toBeLessThan(0.01);
      await expect(marker.getByTestId("db")).toHaveText("−6.0");

      // Its channel detail: the same mark, and its double tap goes there too.
      const detail = await openDetail(page, "#2");
      await expectMarkAt(page, '[data-testid="detail"] .detail-fz', -6);
      const detailFader = detail.getByTestId("fader");
      await startAtHalf(live, detailFader);
      await doubleTap(detailFader);
      await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(value2db(v) + 6) < 0.05, "-6 dB from the detail", 6000);
      await expect(detail.getByTestId("db").first()).toHaveText("−6.0");
    } finally {
      await hostLine("band", "tuner track 1 remove");
      await harness("/hub/layout/reset");
      await markersGone();
    }
    await expect(marker).toHaveCount(0, { timeout: 10_000 });
  });
});
