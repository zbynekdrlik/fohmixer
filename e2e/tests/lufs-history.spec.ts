import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, dispatchPointer, harness, hostLine, hubStatus, openDetail, openSurface, MBC_MARKERS, until } from "./support/live";

// A fader's LUFS history in its channel detail (#219, the design on the
// ticket; the owner's approved mockup docs/mockups/lufs-history-v2.html):
// the detail's middle box gets tabs, EQ and, on a +LUFS strip, LUFS; each
// pane fills the box. The LUFS tab draws the point's short-term reading the
// hub keeps once a second (`/api/lufs/history`, `/api/lufs/days`), its live
// edge from the page's own readings, green within ±1.0 LU of an input
// point's target; 10 MIN / 1 H / 3 H, a drag back in time with NOW › to
// return, the days that hold the point, and a held finger's cursor. The
// tab and the window are kept for the page. Set up as lufs.spec.ts: SimLive's
// FohMixer LUFS device on band track 1 behind a `+LUFS:-25` Tuner.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fixture with a LUFS group (the `+G:LUFS` marker strips) in the FOH page's second row. */
function withLufs(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "lufs", title: "LUFS", tags: "LUFS" });
  return frame;
}

const IN_TRACK = "live_set tracks 1";

/** A `+LUFS` marker strip by its label. */
function lufsStrip(page: Page, label: string): Locator {
  return page.locator(`.slot[data-group="lufs"] [data-testid="strip"][data-label="${label}"]`);
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === MBC_MARKERS, "the Tuner markers gone", 10_000);
}

/** A tap of a key that acts on its down (a finger's down and up). */
async function tap(key: Locator) {
  await expect(key).toBeVisible();
  await dispatchPointer(key, [{ type: "pointerdown" }, { wait: 30 }, { type: "pointerup" }]);
}

/** Opens the detail of the `Vox 1` strip from its own SEL (the Tuner's label is not the track's name). */
async function openVox(page: Page): Promise<Locator> {
  const sel = lufsStrip(page, "Vox 1").getByTestId("detail-key");
  await expect(sel).toBeVisible();
  await dispatchPointer(sel, [{ type: "pointerdown" }, { wait: 600 }, { type: "pointerup" }]);
  const detail = page.getByTestId("detail");
  await expect(detail).toHaveAttribute("data-label", "Vox 1");
  await expect(detail).toHaveAttribute("data-guard", "false");
  return detail;
}

/** The graph's drawn runs (`data-runs`). */
async function runsOf(graph: Locator): Promise<number> {
  return Number(await graph.getAttribute("data-runs"));
}

test.describe("A fader's LUFS history in its channel detail", () => {
  test.use({ viewport: { width: 1194, height: 834 } });
  let live: LiveClient;
  let mute: boolean | undefined;
  test.beforeEach(async () => {
    live = await LiveClient.open();
  });
  test.afterEach(async () => {
    await hostLine("band", "lufs track 1 off");
    await hostLine("band", "maxdev lufs track 1 remove");
    await hostLine("band", "tuner track 1 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    if (mute !== undefined) await live.set("band", IN_TRACK, "mute", mute);
    mute = undefined;
    live.close();
  });

  test("the LUFS tab draws the reading's history, green in the band; the windows, a drag back, NOW ›, a day and the hold's cursor; the tab and window are kept", async ({ page }) => {
    mute = await live.get("band", IN_TRACK, "mute");
    await live.set("band", IN_TRACK, "mute", false);
    await openSurface(page);
    await harness("/hub/layout", { layout: withLufs() });
    expect(await hostLine("band", "maxdev lufs track 1 add rack")).toBe("MAXDEV 1");
    expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:LUFS +LUFS:-25 +PIN'`)).toBe("TUNER 1");
    // In the band of −25: the strip shows −25.0 in green.
    expect(await hostLine("band", "lufs track 1 -250 -100")).toBe("LUFS 1");
    await expect(lufsStrip(page, "Vox 1").getByTestId("lufs")).toHaveText("−25.0", { timeout: 10_000 });

    const detail = await openVox(page);
    const middle = detail.getByTestId("detail-middle");
    const tabs = middle.getByTestId("detail-tab");
    await expect(tabs).toHaveText(["EQ", "LUFS"]);
    // A page starts on EQ, its cards as before.
    await expect(middle).toHaveAttribute("data-tab", "eq");
    await expect(tabs.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(middle.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(middle.getByTestId("detail-pane-lufs")).toHaveCount(0);

    await tap(tabs.nth(1));
    await expect(middle).toHaveAttribute("data-tab", "lufs");
    await expect(tabs.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(middle.getByTestId("detail-pane-eq")).toBeHidden();
    const history = middle.getByTestId("lufs-history");
    await expect(history).toBeVisible();
    // The pane fills the box below the tabs; nothing scrolls.
    const box = (await middle.boundingBox())!;
    const pane = (await middle.getByTestId("detail-pane-lufs").boundingBox())!;
    expect(pane.y + pane.height).toBeCloseTo(box.y + box.height - 10, 0);
    expect(await middle.evaluate((e) => e.scrollHeight <= e.clientHeight)).toBe(true);

    // The head: the caption, the reading as the strip shows it, the target.
    await expect(history.getByTestId("lufs-history-cap").locator("span")).toHaveText(["LUFS", "IN"]);
    const now = history.getByTestId("lufs-history-now");
    await expect(now).toHaveText("−25.0");
    await expect(now).toHaveAttribute("data-l", "in");
    await expect(now).toHaveCSS("color", "rgb(47, 199, 154)");
    await expect(history.getByTestId("lufs-history-target")).toHaveText("TARGET −25");

    // Live, 10 MIN: no NOW ›, the line drawn green within a few seconds.
    await expect(history).toHaveAttribute("data-state", "live");
    const windows = history.getByTestId("lufs-window");
    await expect(windows).toHaveText(["10 MIN", "1 H", "3 H"]);
    await expect(windows.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(history.getByTestId("lufs-now-chip")).toBeHidden();
    const graph = history.getByTestId("lufs-graph");
    await expect.poll(() => runsOf(graph), { timeout: 15_000 }).toBeGreaterThan(0);
    const lines = graph.getByTestId("lufs-line");
    await expect(lines.first()).toHaveAttribute("data-green", "true");
    await expect(lines.first()).toHaveCSS("stroke", "rgb(47, 199, 154)");
    await expect(history.getByTestId("lufs-history-empty")).toBeHidden();
    // Out of the band: the line goes on grey.
    expect(await hostLine("band", "lufs track 1 -300 -100")).toBe("LUFS 1");
    await expect(now).toHaveText("−30.0", { timeout: 10_000 });
    await expect(now).toHaveAttribute("data-l", "off");
    await expect(graph.locator('[data-testid="lufs-line"][data-green="false"]').first()).toBeAttached({ timeout: 15_000 });
    await expect(graph.locator('[data-testid="lufs-line"][data-green="false"]').first()).toHaveCSS("stroke", "rgb(154, 154, 180)");

    // A finger held still at the live edge of 10 MIN (2 s columns) shows
    // the cursor: its time and the reading there, −30.0 since the line went
    // grey.
    // At least 4 grey columns (8 s of −30.0) first: the finger sits 2-4 s
    // back from now.
    const greyPoints = () => graph.locator('[data-testid="lufs-line"][data-green="false"]').last().evaluate((e) => (e.getAttribute("points") ?? "").split(" ").length);
    await expect.poll(greyPoints, { timeout: 20_000 }).toBeGreaterThanOrEqual(5);
    const cursor = history.getByTestId("lufs-history-cursor");
    const edge = (await graph.boundingBox())!.width / 2 - 4;
    await dispatchPointer(graph, [{ type: "pointerdown", dx: edge }, { wait: 600 }], 22);
    await expect(graph.getByTestId("lufs-cursor")).toBeAttached();
    await expect(cursor).toHaveText(/^\d\d:\d\d:\d\d · −30\.0$/);
    await dispatchPointer(graph, [{ type: "pointerup", dx: edge }], 22);
    await expect(graph.getByTestId("lufs-cursor")).toHaveCount(0);
    await expect(cursor).toHaveText("");

    // The day that holds it: today.
    const days = history.getByTestId("lufs-days").getByTestId("lufs-day");
    await expect(days.first()).toHaveText("TODAY", { timeout: 15_000 });
    await expect(days.first()).toHaveAttribute("data-today", "true");

    // 1 H: the hub's history read for it, the line still drawn.
    await tap(windows.nth(1));
    await expect(windows.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(windows.nth(0)).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => runsOf(graph), { timeout: 10_000 }).toBeGreaterThan(0);

    // A drag right goes back in time: NOW › shows; it returns to the live edge.
    const nowChip = history.getByTestId("lufs-now-chip");
    const dragBack = () =>
      dispatchPointer(graph, [
        { type: "pointerdown" },
        { type: "pointermove", dx: 20 },
        { type: "pointermove", dx: 120 },
        { type: "pointerup", dx: 120 },
      ]);
    await dragBack();
    await expect(history).toHaveAttribute("data-state", "back");
    await expect(nowChip).toBeVisible();
    await tap(nowChip);
    await expect(history).toHaveAttribute("data-state", "live");
    await expect(nowChip).toBeHidden();

    // Back again, then today's chip: the live edge, the window still 1 H.
    await dragBack();
    await expect(history).toHaveAttribute("data-state", "back");
    await tap(days.first());
    await expect(history).toHaveAttribute("data-state", "live");
    await expect(nowChip).toBeHidden();
    await expect(windows.nth(1)).toHaveAttribute("aria-pressed", "true");

    // Back to the mix and in again: the LUFS tab and 1 H are kept, and the
    // fresh card draws the hub's history (its answer holds readings).
    await tap(detail.getByTestId("detail-exit"));
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const answer = page.waitForResponse((r) => r.url().includes("/api/lufs/history") && r.status() === 200);
    const again = await openVox(page);
    await expect(again.getByTestId("detail-middle")).toHaveAttribute("data-tab", "lufs");
    await expect(again.getByTestId("lufs-window").nth(1)).toHaveAttribute("aria-pressed", "true");
    const series = await (await answer).json();
    expect(series.step_ms).toBe(4000);
    expect(series.values.some((v: number | null) => v === -300), "the hub's history holds −30.0").toBe(true);
    await expect.poll(() => runsOf(again.getByTestId("lufs-graph")), { timeout: 10_000 }).toBeGreaterThan(0);

    // A strip without +LUFS: no tabs, its EQ pane, though LUFS is kept.
    await tap(again.getByTestId("detail-exit"));
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const plain = await openDetail(page, "Vocal 1 repro#");
    await expect(plain.getByTestId("detail-middle")).toHaveAttribute("data-tabs", "false");
    await expect(plain.getByTestId("detail-tabs")).toHaveCount(0);
    await expect(plain.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(plain.getByTestId("lufs-history")).toHaveCount(0);
    await tap(plain.getByTestId("detail-exit"));

    // In again: the LUFS tab still kept; EQ tapped shows its pane, the
    // history gone.
    const third = await openVox(page);
    await expect(third.getByTestId("detail-middle")).toHaveAttribute("data-tab", "lufs");
    await tap(third.getByTestId("detail-tab").nth(0));
    await expect(third.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(third.getByTestId("lufs-history")).toHaveCount(0);
  });

  test("a strip without +LUFS keeps its middle box as before: no tabs, the EQ cards", async ({ page }) => {
    await openSurface(page);
    const detail = await openDetail(page, "Hand2 #");
    const middle = detail.getByTestId("detail-middle");
    await expect(middle).toHaveAttribute("data-tabs", "false");
    await expect(middle.getByTestId("detail-tabs")).toHaveCount(0);
    await expect(middle.getByTestId("detail-pane-eq")).toBeVisible();
    await expect(middle.getByTestId("eq-cards")).toBeAttached();
    await expect(middle.getByTestId("lufs-history")).toHaveCount(0);
  });
});
