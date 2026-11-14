import type { Locator } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  clipped,
  dispatchPointer,
  frames,
  hubEvents,
  hubStatus,
  openDetail,
  pageEvents,
  resetSectors,
  setSectors,
  until,
  type SectorsDoc,
  type StripRef,
} from "./support/live";
import {
  DOC,
  FULL_DOC,
  KEYS,
  VOCAL,
  band,
  master,
  name,
  openSectors,
  sectorSteps,
  stripsOf,
  watchConsole,
} from "./support/sectors";

// The channel detail with the fader sectors on (#148; split from
// sectors.spec.ts): its LAYOUTS block (#208, the owner's approved mockup
// docs/mockups/scenes-v3.html frame 1: only the layouts the fader is in,
// plus + ADD TO LAYOUT and its sheet) and its channel selector (#205,
// docs/mockups/channel-select-v1.html: the name chip opens every fader of
// the pool, NOT PLACED marked; it replaced ◀ ▶). The hub owns sectors.json;
// the harness starts it empty and each test puts the empty document back.

/**
 * Every sector at its most layouts (4, `MAX_LAYOUTS`), long titles (8
 * characters, the most a rename takes); master Hand4 # in ld-1 alone (#208:
 * ADD's sheet then lists 15 layouts, 16 once it is in none).
 */
const FOUR_EACH_DOC: SectorsDoc = {
  rev: 0,
  sectors: Object.fromEntries(
    ["lu", "ld", "ru", "rd"].map((s) => [
      s,
      ["MONITORS", "OVERHEAD", "BACKLINE", "SPEAKERS"].map((title, i) => ({
        id: `${s}-${i + 1}`,
        title,
        strips: s === "ld" && i === 0 ? [master("Hand4 #")] : s === "lu" && i === 0 ? [VOCAL] : [],
      })),
    ]),
  ),
};

/** The detail's LAYOUTS rows (#208) as `<sector>|<title>`, in their order. */
async function layoutRows(layouts: Locator): Promise<string[]> {
  return layouts
    .getByTestId("lay-row")
    .evaluateAll((all) =>
      all.map((r) => `${r.getAttribute("data-sector")}|${r.querySelector(".lay-what")?.textContent ?? ""}`),
    );
}

/** ADD TO LAYOUT's picks (#208) as `<layout>|<title>|<sector>|<full>`, in their order. */
async function layoutPicks(sheet: Locator): Promise<string[]> {
  return sheet
    .getByTestId("lay-pick")
    .evaluateAll((all) =>
      all.map(
        (p) =>
          `${p.getAttribute("data-layout")}|${p.querySelector(".sheet-pick-name")?.textContent ?? ""}|` +
          `${p.getAttribute("data-sector")}|${p.getAttribute("data-full")}`,
      ),
    );
}

/** The channel selector's pick of `r` (#205). */
const pickOf = (sheet: Locator, r: StripRef) =>
  sheet.locator(`[data-testid="chan-pick"][data-instance="${r.instance}"][data-label="${r.label}"]`);

/** The channel selector's picks (#205) that are `current`, `placed` or `unplaced`, as `<instance>|<label>`. */
async function channelPicks(sheet: Locator, which: "current" | "placed" | "unplaced"): Promise<string[]> {
  const selector =
    which === "current" ? '[data-current="true"]' : which === "placed" ? '[data-placed="true"]' : '[data-placed="false"]';
  return sheet
    .locator(`[data-testid="chan-pick"]${selector}`)
    .evaluateAll((all) => all.map((p) => `${p.getAttribute("data-instance")}|${p.getAttribute("data-label")}`));
}

/** Whether `inner`'s box lies inside `outer`'s (half a pixel of rounding). */
async function insideOf(inner: Locator, outer: Locator): Promise<boolean> {
  const a = await inner.boundingBox();
  const b = await outer.boundingBox();
  if (!a || !b) return false;
  return (
    a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5
  );
}

test.afterEach(async () => {
  await resetSectors();
});

test.describe("The channel detail with the sectors on", () => {
  test("ADD TO LAYOUT in SEL puts the strip in a layout, ✕ takes it out, and a second page sees both", async ({
    page,
    context,
  }) => {
    await setSectors(DOC);
    const other = await context.newPage();
    const otherConsole = watchConsole(other);
    await openSectors(other);
    await openSectors(page);
    const since = Date.now();
    const detail = await openDetail(page, "Hand4 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    await expect(layouts).toBeVisible();
    // Only Hand4 #'s own layout, one row; the others wait in ADD's sheet (#208).
    await expect.poll(() => layoutRows(layouts)).toEqual(["LD|HANDS"]);
    await expect(layouts.getByTestId("lay-count")).toHaveText("1");
    await expect(layouts.getByTestId("lay-empty")).toHaveCount(0);
    await expect(detail.getByTestId("lay-sheet")).toHaveCount(0);
    await expect(detail.getByTestId("chan-open")).toBeVisible();

    await layouts.getByTestId("lay-add").click();
    const sheet = detail.getByTestId("lay-sheet");
    await expect(sheet).toBeVisible();
    // Every other layout, in the sectors' order (LU, LD, RU, RD), none full.
    await expect.poll(() => layoutPicks(sheet)).toEqual([
      "lu-1|STAGE|LU|false",
      "lu-2|ALT|LU|false",
      "ld-2|FX|LD|false",
      "ru-1|MAIN|RU|false",
      "rd-1|MAIN|RD|false",
    ]);
    await sheet.locator('[data-testid="lay-pick"][data-layout="lu-1"]').click();
    // One tap: the sheet goes and the row comes.
    await expect(sheet).toHaveCount(0);
    await expect.poll(() => layoutRows(layouts)).toEqual(["LU|STAGE", "LD|HANDS"]);
    await expect(layouts.getByTestId("lay-count")).toHaveText("2");
    // It joins STAGE at its end, on both pages; it stays in HANDS.
    const stage = [name(VOCAL), name(KEYS), "master|Hand4 #"];
    await expect.poll(() => stripsOf(page, "lu")).toEqual(stage);
    await expect.poll(() => stripsOf(other, "lu")).toEqual(stage);
    await expect.poll(() => stripsOf(other, "ld")).toEqual(["master|Hand1 #", "master|Hand4 #"]);
    const steps = await until(() => sectorSteps(since), (s) => s.includes("tick"), "the hub's tick record");
    expect(steps).toEqual(["tick"]);
    const record = (await hubEvents()).find((r: any) => r.ev === "sectors" && r.ts >= since);
    expect(record.layout).toBe("lu-1");
    expect(record.strip).toEqual(master("Hand4 #"));
    expect(record.on).toBe(true);

    // ✕ on STAGE's row takes it out again, on both pages.
    const out = Date.now();
    await layouts.locator('[data-testid="lay-row"][data-layout="lu-1"]').getByTestId("lay-remove").click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LD|HANDS"]);
    await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
    await expect.poll(() => stripsOf(other, "lu")).toEqual([name(VOCAL), name(KEYS)]);
    const removed = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "sectors" && r.ts >= out),
      (all) => all.length > 0,
      "the hub's record of the ✕",
    );
    expect(removed.map((r: any) => [r.what, r.layout, r.on])).toEqual([["tick", "lu-1", false]]);
    expect(removed[0].strip).toEqual(master("Hand4 #"));

    expect(otherConsole, "the second page's console").toEqual([]);
    await other.close();
  });

  test("a full layout reads FULL in ADD's sheet and a tick is refused", async ({ page }) => {
    await setSectors(FULL_DOC);
    await openSectors(page);
    await expect.poll(async () => (await stripsOf(page, "lu")).length).toBe(6);
    const detail = await openDetail(page, "Hand2 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    // Its own layout is a row: it is in it already.
    await expect.poll(() => layoutRows(layouts)).toEqual(["LD|HANDS"]);
    await layouts.getByTestId("lay-add").click();
    const sheet = detail.getByTestId("lay-sheet");
    await expect(sheet).toBeVisible();
    const pick = sheet.locator('[data-testid="lay-pick"][data-layout="lu-1"]');
    await expect.poll(() => layoutPicks(sheet)).toEqual(["lu-1|STAGE|LU|true"]);
    await expect(pick).toBeDisabled();
    await expect(pick).toContainText("STAGE");
    await expect(pick).toContainText("LU · FULL");
    // Disabled, it takes no touch (whether its handler would refuse it too
    // is the unit test a_full_pick_asks_nothing_and_says_full): a click at
    // its centre leaves the sheet open and asks the hub nothing (no
    // `sectors` record of any kind, a refused tick's included).
    const before = (await hubStatus()).sectors.rev;
    const touched = Date.now();
    const at = await centre(pick);
    await page.mouse.click(at.x, at.y);
    await frames(page);
    await expect(sheet).toBeVisible();
    // ✕ closes the sheet.
    await sheet.getByTestId("sheet-close").click();
    await expect(sheet).toHaveCount(0);

    const live = await LiveClient.open();
    try {
      // A marker after the touch: a tick the test's own client asks (Hand1 #
      // into HANDS, which has room). The hub logs in order, so its record
      // being the only one since the touch proves the page asked nothing.
      const marker = await live.ask({ type: "sector_tick", layout: "ld-1", strip: master("Hand1 #"), on: true });
      expect(marker.type).toBe("result");
      const steps = await until(() => sectorSteps(touched), (s) => s.length > 0, "the marker's record");
      expect(steps, "the page asked nothing").toEqual(["tick"]);
      // The hub refuses the tick itself (another client asking past the page).
      const ticked = (await hubStatus()).sectors.rev;
      const answer = await live.ask({ type: "sector_tick", layout: "lu-1", strip: master("Hand2 #"), on: true });
      expect(answer.type).toBe("error");
      expect(answer.message).toBe("FULL");
      expect((await hubStatus()).sectors.rev).toBe(ticked);
      expect(ticked).toBeGreaterThan(before);
      await expect.poll(async () => (await stripsOf(page, "lu")).length).toBe(6);
    } finally {
      live.close();
    }
  });

  test("a strip in no layout shows the empty line, and ADD's sheet holds every layout", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    const detail = await openDetail(page, "Hand4 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    await layouts.locator('[data-testid="lay-row"][data-layout="ld-1"]').getByTestId("lay-remove").click();
    // Out of its only layout: no row, the empty line, the detail still open.
    await expect(layouts.getByTestId("lay-empty")).toHaveText("This fader is in no layout.");
    await expect(layouts.getByTestId("lay-row")).toHaveCount(0);
    await expect(layouts.getByTestId("lay-count")).toHaveText("0");
    await expect(detail).toHaveAttribute("data-track", "Hand4 #");
    await expect.poll(() => stripsOf(page, "ld")).toEqual(["master|Hand1 #"]);
    // The sheet holds all six; the shade closes it.
    await layouts.getByTestId("lay-add").click();
    const sheet = detail.getByTestId("lay-sheet");
    await expect(sheet.getByTestId("lay-pick")).toHaveCount(6);
    await expect(sheet.locator(".sheet-note")).toHaveText("It joins the layout at its end; ARRANGE moves it.");
    const shade = await sheet.getByTestId("sheet-shade").boundingBox();
    if (!shade) throw new Error("the shade has no box");
    await page.mouse.click(shade.x + 4, shade.y + 4);
    await expect(sheet).toHaveCount(0);
    // A pick acts at its lift, never after a slide (a finger scrolling the
    // list): down, 30 px down and up on HANDS changes nothing; the sheet
    // stays open.
    await layouts.getByTestId("lay-add").click();
    const slid = Date.now();
    await dispatchPointer(sheet.locator('[data-testid="lay-pick"][data-layout="ld-1"]'), [
      { type: "pointerdown" },
      { type: "pointermove", dy: 0 },
      { type: "pointermove", dy: -30 },
      { type: "pointerup", dy: -30 },
    ]);
    await frames(page);
    await expect(sheet).toBeVisible();
    expect(await sectorSteps(slid), "no tick after a slide").toEqual([]);
    await sheet.getByTestId("sheet-close").click();
    await expect(sheet).toHaveCount(0);
    // Back into HANDS from the sheet: at its end.
    await layouts.getByTestId("lay-add").click();
    await sheet.locator('[data-testid="lay-pick"][data-layout="ld-1"]').click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LD|HANDS"]);
    await expect(layouts.getByTestId("lay-empty")).toHaveCount(0);
    await expect.poll(() => stripsOf(page, "ld")).toEqual(["master|Hand1 #", "master|Hand4 #"]);
  });

  test("LAYOUTS and its sheet fit the tablet and both phones", async ({ page }) => {
    await setSectors(FOUR_EACH_DOC);
    await openSectors(page);
    const detail = await openDetail(page, "Hand4 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    // In two layouts, so the block has two rows, and fourteen picks; at the
    // end, in none: all sixteen.
    await layouts.getByTestId("lay-add").click();
    await detail.locator('[data-testid="lay-pick"][data-layout="lu-1"]').click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LU|MONITORS", "LD|MONITORS"]);
    const fits = async (where: string, count: number) => {
      const sheet = detail.getByTestId("lay-sheet");
      await expect(sheet.getByTestId("lay-pick")).toHaveCount(count);
      const body = await sheet
        .getByTestId("sheet-body")
        .evaluate((e) => ({ scroll: e.scrollHeight, client: e.clientHeight }));
      expect(body.scroll, `the picks' list scrolls at ${where}`).toBeLessThanOrEqual(body.client);
      for (const pick of await sheet.getByTestId("lay-pick").all()) {
        expect(await clipped(pick.locator(".sheet-pick-name")), `a pick's title at ${where}`).toEqual([]);
        expect(await clipped(pick.locator("small")), `a pick's sector at ${where}`).toEqual([]);
        expect(await insideOf(pick, sheet.locator(".sheet-panel")), `a pick inside the panel at ${where}`).toBe(true);
      }
    };
    for (const size of [
      { width: 1194, height: 834 },
      { width: 844, height: 390 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(size);
      await frames(page);
      const where = `${size.width}x${size.height}`;
      for (const row of await layouts.getByTestId("lay-row").all()) {
        await row.scrollIntoViewIfNeeded();
        expect(await clipped(row.locator(".lay-what")), `a row's title at ${where}`).toEqual([]);
        expect(await insideOf(row.getByTestId("lay-remove"), row), `a row's ✕ at ${where}`).toBe(true);
      }
      const add = layouts.getByTestId("lay-add");
      await add.scrollIntoViewIfNeeded();
      expect(await clipped(add), `ADD at ${where}`).toEqual([]);
      await add.click();
      const sheet = detail.getByTestId("lay-sheet");
      await expect(sheet).toBeVisible();
      const panel = await sheet.locator(".sheet-panel").boundingBox();
      if (!panel) throw new Error("the sheet's panel has no box");
      expect(panel.x, `the panel's left at ${where}`).toBeGreaterThanOrEqual(0);
      expect(panel.y, `the panel's top at ${where}`).toBeGreaterThanOrEqual(0);
      expect(panel.x + panel.width, `the panel's right at ${where}`).toBeLessThanOrEqual(size.width + 0.5);
      expect(panel.y + panel.height, `the panel's bottom at ${where}`).toBeLessThanOrEqual(size.height + 0.5);
      expect(await clipped(sheet.locator(".sheet-title")), `the title at ${where}`).toEqual([]);
      await fits(where, 14);
      await sheet.getByTestId("sheet-close").click();
      await expect(sheet).toHaveCount(0);
    }
    // Out of both: the most a document holds (4 layouts a sector), all
    // sixteen, fit a phone on its side and upright without a scroll.
    for (const id of ["lu-1", "ld-1"]) {
      await layouts.locator(`[data-testid="lay-row"][data-layout="${id}"]`).getByTestId("lay-remove").click();
    }
    await expect(layouts.getByTestId("lay-empty")).toBeVisible();
    for (const size of [
      { width: 844, height: 390 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(size);
      await frames(page);
      const add = layouts.getByTestId("lay-add");
      await add.scrollIntoViewIfNeeded();
      await add.click();
      await fits(`${size.width}x${size.height}`, 16);
      await detail.getByTestId("lay-sheet").getByTestId("sheet-close").click();
    }
  });

  test("the channel chip lists the pool by Live set and a pick opens that channel", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    const since = await page.evaluate(() => performance.timeOrigin + performance.now());
    const detail = await openDetail(page, "Hand1 #", "master");
    await expect(detail.getByTestId("detail-name")).toHaveText("Hand1");
    await detail.getByTestId("chan-open").click();
    const sheet = detail.getByTestId("chan-sheet");
    await expect(sheet).toBeVisible();
    await expect(sheet.locator(".sheet-title")).toHaveText("CHANNEL");
    // Grouped by Live set in the pool's order: band first, then master.
    const groups = await sheet
      .getByTestId("chan-group")
      .evaluateAll((all) => all.map((g) => `${g.getAttribute("data-instance")}|${g.textContent}`));
    expect(groups.slice(0, 2)).toEqual(["band|BAND", "master|MASTER"]);
    // The one shown is outlined, alone.
    await expect.poll(() => channelPicks(sheet, "current")).toEqual(["master|Hand1 #"]);
    // Every strip DOC places reads placed, with no NOT PLACED.
    const placedNow = await channelPicks(sheet, "placed");
    for (const r of [VOCAL, KEYS, band("Hand2 #"), band("B-Main repro #"), master("Hand1 #"), master("Hand4 #"), band("A-Reverb #"), master("A-Echo")]) {
      expect(placedNow, `${name(r)} placed`).toContain(name(r));
      await expect(pickOf(sheet, r)).not.toContainText("NOT PLACED");
    }
    // A pick opens its channel (at its lift) and the sheet goes with the old detail.
    await pickOf(sheet, band("B-Main repro #")).click();
    await expect(detail).toHaveAttribute("data-track", "B-Main repro #");
    await expect(detail).toHaveAttribute("data-instance", "band");
    await expect(detail.getByTestId("chan-sheet")).toHaveCount(0);
    await detail.getByTestId("chan-open").click();
    await pickOf(detail.getByTestId("chan-sheet"), master("Hand1 #")).click();
    await expect(detail).toHaveAttribute("data-track", "Hand1 #");
    await expect(detail).toHaveAttribute("data-instance", "master");
    // A pick of the one shown only closes the sheet.
    await detail.getByTestId("chan-open").click();
    await pickOf(detail.getByTestId("chan-sheet"), master("Hand1 #")).click();
    await expect(detail.getByTestId("chan-sheet")).toHaveCount(0);
    await expect(detail).toHaveAttribute("data-track", "Hand1 #");
    // A last pick (A-Echo) closes the record: the page's events reach the
    // log in order, so a wrong open from the pick of the one shown would
    // sit before it.
    await detail.getByTestId("chan-open").click();
    await pickOf(detail.getByTestId("chan-sheet"), master("A-Echo")).click();
    await expect(detail).toHaveAttribute("data-track", "A-Echo");
    // Each pick is an open recorded with why "pick" and the strip's keys.
    const picks = await until(
      async () =>
        pageEvents(await hubEvents())
          .filter((e: any) => e.ev === "detail" && e.what === "open" && e.why === "pick" && e.t >= since)
          .sort((a: any, b: any) => a.t - b.t),
      (all) => all.length >= 3,
      "the three picks in the event log",
      15_000,
    );
    expect(picks.length).toBe(3);
    const names = (e: any, instance: string, track: string) =>
      (e.keys as string[]).some((k) => k.startsWith(`${instance}|`) && k.includes(track));
    expect(names(picks[0], "band", "B-Main repro #"), JSON.stringify(picks[0])).toBe(true);
    expect(names(picks[1], "master", "Hand1 #"), JSON.stringify(picks[1])).toBe(true);
    expect(names(picks[2], "master", "A-Echo"), JSON.stringify(picks[2])).toBe(true);
    expect(typeof picks[0].pointer).toBe("number");
  });

  test("a fader no layout holds is reachable from the chip, and ADD TO LAYOUT places it", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    const detail = await openDetail(page, "Hand1 #", "master");
    await detail.getByTestId("chan-open").click();
    const sheet = detail.getByTestId("chan-sheet");
    // DOC places 8 faders; the pool holds more: the first one it leaves out.
    const unplaced = await channelPicks(sheet, "unplaced");
    expect(unplaced.length, "a fader no layout holds").toBeGreaterThan(0);
    const [instance, label] = unplaced[0].split("|");
    const loose = { instance, label };
    const chip = pickOf(sheet, loose);
    await expect(chip.locator(".chan-np")).toHaveText("NOT PLACED");
    await expect(chip.locator(".chan-np")).toHaveCSS("color", "rgb(245, 166, 35)");
    await chip.click();
    await expect(detail).toHaveAttribute("data-instance", instance);
    await expect(detail).toHaveAttribute("data-track", label);
    // Its detail: in no layout; ADD's sheet puts it into lu's ALT.
    const layouts = detail.getByTestId("detail-layouts");
    await expect(layouts.getByTestId("lay-empty")).toBeVisible();
    const since = Date.now();
    await layouts.getByTestId("lay-add").click();
    await detail.locator('[data-testid="lay-pick"][data-layout="lu-2"]').click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LU|ALT"]);
    const record = (
      await until(
        async () => (await hubEvents()).filter((r: any) => r.ev === "sectors" && r.ts >= since),
        (all) => all.length > 0,
        "the hub's tick record",
      )
    )[0];
    expect([record.what, record.layout, record.on]).toEqual(["tick", "lu-2", true]);
    expect(record.strip).toEqual(loose);
    // The chip's list now reads it placed.
    await detail.getByTestId("chan-open").click();
    await expect(pickOf(detail.getByTestId("chan-sheet"), loose)).toHaveAttribute("data-placed", "true");
    await expect(pickOf(detail.getByTestId("chan-sheet"), loose).locator(".chan-np")).toHaveCount(0);
  });

  test("a flick whose first move is already past 10 px takes nothing out, adds nothing and opens no channel", async ({
    page,
  }) => {
    // The integrated review's finding: a key's press was anchored at its
    // first move, so a flick whose first pointermove landed 30 px away still
    // acted at its lift. The flicks: down, then the first move 30 px below,
    // then the up there.
    const flick = [
      { type: "pointerdown" as const },
      { type: "pointermove" as const, dy: -30 },
      { type: "pointerup" as const, dy: -30 },
    ];
    await setSectors(DOC);
    await openSectors(page);
    const since = await page.evaluate(() => performance.timeOrigin + performance.now());
    const detail = await openDetail(page, "Hand4 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    const touched = Date.now();
    // ✕ on HANDS: the fader stays in it.
    await dispatchPointer(layouts.locator('[data-testid="lay-row"][data-layout="ld-1"]').getByTestId("lay-remove"), flick);
    // A pick of STAGE in ADD's sheet: the sheet stays open.
    await layouts.getByTestId("lay-add").click();
    const sheet = detail.getByTestId("lay-sheet");
    await dispatchPointer(sheet.locator('[data-testid="lay-pick"][data-layout="lu-1"]'), flick);
    await frames(page);
    await expect(sheet).toBeVisible();
    await sheet.getByTestId("sheet-close").click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LD|HANDS"]);
    // A marker after both: a tick the test's own client asks (Hand1 # into
    // FX). The hub logs in order, so its record being the only `sectors`
    // record since the flicks proves neither flick asked anything.
    const live = await LiveClient.open();
    try {
      const marker = await live.ask({ type: "sector_tick", layout: "ld-2", strip: master("Hand1 #"), on: true });
      expect(marker.type).toBe("result");
    } finally {
      live.close();
    }
    const records = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "sectors" && r.ts >= touched),
      (all) => all.length > 0,
      "the marker's record",
    );
    expect(
      records.map((r: any) => [r.what, r.layout, r.on]),
      "no tick but the marker's",
    ).toEqual([["tick", "ld-2", true]]);

    // A pick in the channel list: no channel opens; then a real pick
    // (A-Echo) closes the record, so a wrong open would sit before it.
    await detail.getByTestId("chan-open").click();
    const chans = detail.getByTestId("chan-sheet");
    await dispatchPointer(pickOf(chans, band("B-Main repro #")), flick);
    await frames(page);
    await expect(chans).toBeVisible();
    await expect(detail).toHaveAttribute("data-track", "Hand4 #");
    await pickOf(chans, master("A-Echo")).click();
    await expect(detail).toHaveAttribute("data-track", "A-Echo");
    const opens = await until(
      async () =>
        pageEvents(await hubEvents()).filter(
          (e: any) => e.ev === "detail" && e.what === "open" && e.why === "pick" && e.t >= since,
        ),
      (all) => all.some((e: any) => (e.keys as string[]).some((k) => k.startsWith("master|") && k.includes("A-Echo"))),
      "the real pick's open in the event log",
      15_000,
    );
    expect(opens.length, JSON.stringify(opens)).toBe(1);
  });

  test("the channel list scrolls under a dragged finger and a drag opens nothing", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    const detail = await openDetail(page, "Hand1 #", "master");
    await page.setViewportSize({ width: 844, height: 390 });
    await frames(page);
    await detail.getByTestId("chan-open").click();
    const sheet = detail.getByTestId("chan-sheet");
    const body = sheet.getByTestId("sheet-body");
    const panel = await sheet.locator(".sheet-panel").boundingBox();
    if (!panel) throw new Error("the panel has no box");
    expect(panel.y + panel.height, "the panel inside the screen").toBeLessThanOrEqual(390.5);
    const sizes = await body.evaluate((e) => ({ scroll: e.scrollHeight, client: e.clientHeight, top: e.scrollTop }));
    expect(sizes.scroll, "the pool overflows a phone on its side").toBeGreaterThan(sizes.client);
    expect(sizes.top).toBe(0);
    // A finger down on a pick, dragged 120 px up and lifted (buttons held,
    // as a touch's moves carry them): the list scrolls, no channel opens.
    const scrolled = await sheet
      .getByTestId("chan-pick")
      .nth(2)
      .evaluate(async (el) => {
        const box = el.getBoundingClientRect();
        const x = box.left + box.width / 2;
        const y = box.top + box.height / 2;
        const fire = (type: string, dy: number) =>
          el.dispatchEvent(
            new PointerEvent(type, {
              pointerId: 31,
              pointerType: "touch",
              isPrimary: true,
              buttons: type === "pointerup" ? 0 : 1,
              clientX: x,
              clientY: y - dy,
              bubbles: true,
              cancelable: true,
            }),
          );
        fire("pointerdown", 0);
        for (const dy of [0, 20, 60, 120]) fire("pointermove", dy);
        fire("pointerup", 120);
        await new Promise((r) => setTimeout(r, 50));
        return (el.closest('[data-testid="sheet-body"]') as HTMLElement).scrollTop;
      });
    expect(scrolled, "the list scrolled with the finger").toBeGreaterThan(0);
    await expect(sheet).toBeVisible();
    await expect(detail).toHaveAttribute("data-track", "Hand1 #");
  });
});
