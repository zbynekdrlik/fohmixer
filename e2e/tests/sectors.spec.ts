import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  clipped,
  columnHeadAndFoot,
  dispatchPointer,
  frames,
  harness,
  hubEvents,
  hubStatus,
  openDeck,
  openDetail,
  openSurface,
  pageEvents,
  resetSectors,
  rightHalfLive,
  servedLayout,
  setSectors,
  track,
  until,
  volume,
  type SectorsDoc,
  type StripRef,
  fixtureLayout,
} from "./support/live";
import {
  DOC,
  KEYS,
  SECTORS,
  VOCAL,
  band,
  master,
  name,
  openSectors,
  sectorSteps,
  sectorsPage,
  slots,
  slotsCss,
  stripWidth,
  stripsOf,
  watchConsole,
} from "./support/sectors";

// The fader sectors (#148; spec docs/superpowers/specs/2026-10-28-sectors-design.md,
// the approved looks docs/mockups/sectors-v1.html and sectors-v2.html): with
// a sectors document that places faders, the FOH page is four sectors around
// the column (lu ru over ld rd), each paging through its own layouts in the
// column's LAYOUTS block; ARRANGE reorders a sector's strips and renames,
// adds and removes its layouts (saved as one change); the channel detail's
// LAYOUTS and channel selector are sector-detail.spec.ts (#208, #205); there is no undo (the owner's ruling of
// 29.10.2026: the ↶ beside ARRANGE is gone, ARRANGE takes its row alone). The hub owns
// sectors.json; the harness starts it empty (sectors off) and each test puts
// the empty document back.
//
// The documents use the e2e fixture's frame strips (tools/import-tosc/
// fixtures/expected-layout.json), a frame strip's identity being its anchor's
// name (fohmixer_proto::sectors::strip_ref). Band Hand2 #, B-Main repro # and
// master A-Echo are wide (1.1 units); every other strip takes 1.

/** ld mixes master and band strips; the other sectors hold one Live set each. */
const MIXED_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "STAGE", strips: [VOCAL, KEYS] }],
    ru: [{ id: "ru-1", title: "MAIN", strips: [band("B-Main repro #")] }],
    ld: [
      { id: "ld-1", title: "EFFECTS", strips: [master("Hand1 #"), band("A-Reverb #"), master("A-Reverb #")] },
      { id: "ld-2", title: "FX", strips: [band("A-Reverb #")] },
    ],
    rd: [{ id: "rd-1", title: "MAIN", strips: [master("A-Echo")] }],
  },
};

/**
 * The slot model (1.24.0): lu holds 6 strips, so a side fits 6 strip widths,
 * and ru 4 strips with room for 2 more: the owner's case, BAND to RU's right
 * edge with empty space before it.
 */
const GAP_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [
      {
        id: "lu-1",
        title: "STAGE",
        strips: [VOCAL, KEYS, band("A-Reverb #"), master("A-Reverb #"), master("Hand1 #"), master("Hand4 #")],
      },
    ],
    ru: [{ id: "ru-1", title: "MAIN", strips: [master("Hand2 #"), VOCAL, KEYS, master("Hand4 #")] }],
  },
};

/**
 * #236: ru holds 5 strips of one unit, so in ARRANGE it shows one more
 * empty cell (its sixth unit); lu holds Vocal and the wide band Hand2 #
 * (1.1 units), which would make ru 6.1 there: FULL. ld holds 6 strips, so
 * every side's strips are as narrow as a 6-unit side's and ru's sixth cell
 * fits (a fader may sit in two layouts, never twice in one).
 */
const FULL_RU_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "STAGE", strips: [VOCAL, band("Hand2 #")] }],
    ru: [
      {
        id: "ru-1",
        title: "MAIN",
        strips: [KEYS, master("A-Reverb #"), master("Hand1 #"), master("Hand4 #"), master("Hand2 #")],
      },
    ],
    ld: GAP_DOC.sectors.lu!.map((layout) => ({ ...layout, id: "ld-1", title: "SIX" })),
  },
};

/** The note under ARRANGE while it is on (the approved mockup v2's). */
const ARRANGE_NOTE = "drag · ✎ ✕ + · tap to save";

/** A sector's cells in their order: a strip as `<instance>|<track>`, an empty slot as `_`. */
async function cellsOf(page: Page, sector: string): Promise<string[]> {
  return slots(page, sector).evaluateAll((all) =>
    all.map((slot) => {
      const strip = slot.querySelector('[data-testid="strip"]');
      return strip ? `${strip.getAttribute("data-instance")}|${strip.getAttribute("data-track")}` : "_";
    }),
  );
}

/**
 * Carries a strip of `sector` in ARRANGE onto the cell at `to` (its index
 * among the `target` sector's cells; its own sector's by default, #233
 * another visible one's): the moves and the lift dispatched on the strip's
 * own cover (a dispatched pointer has no capture), past the slop first,
 * the lift at the target cell's centre. With `lift` false the finger stays
 * on the target and the carry is returned for `cancelCarry`.
 */
async function carry(page: Page, sector: string, strip: string, to: number, target = sector, lift = true) {
  const cover = coverOf(page, sector, strip);
  await expect(cover).toBeVisible();
  const from = await centre(cover);
  const goal = await centre(slots(page, target).nth(to));
  const dx = goal.x - from.x;
  // `dy` is up from the cover's centre (`dispatchPointer`).
  const dy = from.y - goal.y;
  await dispatchPointer(cover, [
    { type: "pointerdown" },
    { type: "pointermove", dx: Math.sign(dx) * 10, dy: 0 },
    { type: "pointermove", dx, dy },
    ...(lift ? [{ type: "pointerup" as const, dx, dy }] : []),
  ]);
  return cover;
}

/** The ARRANGE cover of `strip` (by its track) in `sector`. */
function coverOf(page: Page, sector: string, strip: string): Locator {
  return slots(page, sector)
    .filter({ has: page.locator(`[data-testid="strip"][data-track="${strip}"]`) })
    .getByTestId("edit-cover");
}

/** The `data-drop` marks of every cell on the sectors page, as `<sector>/<index>=<mark>`. */
async function dropMarks(page: Page): Promise<string[]> {
  return page.evaluate((selector) => {
    const out: string[] = [];
    for (const sector of ["lu", "ld", "ru", "rd"]) {
      const cells = [...document.querySelectorAll(`${selector} .slot[data-sector="${sector}"]`)];
      cells.forEach((cell, i) => {
        const mark = cell.getAttribute("data-drop");
        if (mark != null) out.push(`${sector}/${i}=${mark}`);
      });
    }
    return out;
  }, SECTORS);
}

/**
 * ARRANGE's ⇄ key beside the page tab (#212, the owner's approved
 * docs/mockups/column-calm-v2.html): in the FOH tab's row, at its right,
 * about 30 px wide and as high as the tab, the tab taking the rest of the
 * row; named ARRANGE; nothing of it clipped; its centre hit-tests as the key
 * (reachable, nothing over it).
 */
async function arrangeKeyBesideTheTab(page: Page) {
  const key = page.getByTestId("arrange");
  await expect(key).toBeVisible();
  await expect(key).toHaveText("⇄");
  await expect(key).toHaveAttribute("aria-label", "ARRANGE");
  const fohTab = page.locator('[data-testid="page-row"] > [data-testid="tab"][data-page="foh"]');
  await expect(fohTab).toHaveCount(1);
  await expect(page.locator('[data-testid="page-row"] > [data-testid="arrange"]')).toHaveCount(1);
  const k = (await key.boundingBox())!;
  const t = (await fohTab.boundingBox())!;
  const row = (await page.getByTestId("page-row").boundingBox())!;
  expect(Math.abs(k.width - 30), "the key about 30 px wide").toBeLessThan(1);
  expect(Math.abs(k.y - t.y), "the key in the tab's row").toBeLessThan(0.5);
  expect(Math.abs(k.height - t.height), "the key as high as the tab").toBeLessThan(0.5);
  expect(k.x, "the key right of the tab").toBeGreaterThanOrEqual(t.x + t.width - 0.5);
  expect(Math.abs(k.x + k.width - (row.x + row.width)), "the key at the row's right end").toBeLessThan(0.5);
  expect(Math.abs(t.x - row.x), "the tab from the row's left end").toBeLessThan(0.5);
  expect(await clipped(key), "the key").toEqual([]);
  expect(await clipped(fohTab), "the FOH tab").toEqual([]);
  const hit = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-testid="arrange"]') != null, {
    x: k.x + k.width / 2,
    y: k.y + k.height / 2,
  });
  expect(hit, "the key's centre is the key").toBe(true);
}

test.afterEach(async () => {
  await resetSectors();
});

test.describe("The fader sectors", () => {
  test("paging one sector leaves the other three and the strip width", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
    await expect.poll(() => stripsOf(page, "ru")).toEqual(["band|B-Main repro #"]);
    await expect.poll(() => stripsOf(page, "ld")).toEqual(["master|Hand1 #", "master|Hand4 #"]);
    await expect.poll(() => stripsOf(page, "rd")).toEqual(["master|A-Echo"]);
    // The LAYOUTS block: each sector's first layout shown, the pager's tabs gone.
    const tab = (id: string) => page.locator(`[data-testid="sector-tab"][data-layout="${id}"]`);
    for (const id of ["lu-1", "ru-1", "ld-1", "rd-1"]) await expect(tab(id)).toHaveAttribute("aria-pressed", "true");
    for (const id of ["lu-2", "ld-2"]) await expect(tab(id)).toHaveAttribute("aria-pressed", "false");
    await expect(page.locator('[data-testid="tabbar"][data-level="1"]')).toHaveCount(0);

    await frames(page);
    const width = await stripWidth(page);
    expect(width).toMatch(/px$/);
    for (const sector of ["ru", "ld", "rd"]) {
      await slots(page, sector).evaluateAll((all) => all.forEach((s) => s.setAttribute("data-e2e-kept", "1")));
    }
    await tab("lu-2").click();
    await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "true");
    await expect(tab("lu-1")).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => stripsOf(page, "lu")).toEqual(["band|Hand2 #"]);
    // The other sectors' strips are the same elements (nothing rebuilt) and the width is unchanged.
    for (const sector of ["ru", "ld", "rd"]) {
      const kept = await slots(page, sector).evaluateAll((all) => all.map((s) => s.getAttribute("data-e2e-kept")));
      expect(kept.length, sector).toBeGreaterThan(0);
      expect(kept, sector).toEqual(kept.map(() => "1"));
    }
    await frames(page);
    expect(await stripWidth(page)).toBe(width);
    for (const id of ["ru-1", "ld-1", "rd-1"]) await expect(tab(id)).toHaveAttribute("aria-pressed", "true");
  });

  test.describe("at the iPad's size", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    test("STREAM DECK opens in the left sectors: the column stays in the middle and RU and RD stay live (#148)", async ({ page }) => {
      // The owner (29.10.2026): the deck and AbleSet in the left sectors, the
      // column never moving right; the right sectors keep working.
      await setSectors(DOC);
      await openSectors(page);
      const before = (await page.getByTestId("column").boundingBox())!;
      await openDeck(page);
      await expect(sectorsPage(page)).toHaveAttribute("data-extra", "deck");
      const viewport = page.viewportSize()!;
      const column = (await page.getByTestId("column").boundingBox())!;
      expect(Math.abs(column.x + column.width / 2 - viewport.width / 2), "the column's centre (px off the screen's)").toBeLessThan(1);
      expect(Math.abs(column.x - before.x), "the column did not move").toBeLessThan(0.5);
      // Every key left of the column, none right of it.
      const keys = await page.getByTestId("deck-key").evaluateAll((all) => all.map((k) => k.getBoundingClientRect().right));
      expect(keys).toHaveLength(32);
      for (const right of keys) expect(right, "a key right of the column").toBeLessThanOrEqual(column.x + 0.5);
      // RU and RD shown, LU and LD under the deck.
      for (const sector of ["ru", "rd"]) await expect(slots(page, sector).getByTestId("strip").first(), sector).toBeVisible();
      for (const sector of ["lu", "ld"]) await expect(slots(page, sector).getByTestId("strip").first(), sector).toBeHidden();
      // A fader in RU still moves (#148 review): its cap's centre is the
      // cap's (nothing over it) and a real drag from there reaches the hub
      // as a set and raises Live's volume.
      await rightHalfLive(page, `${slotsCss("ru")} [data-testid="strip"]`);
      await expect(page.getByTestId("deck-key")).toHaveCount(32);
      // The controls that act only on the hidden left sectors are inert
      // (#148 review): LU's and LD's layouts and ARRANGE dimmed and
      // aria-disabled, a dispatched tap on them changes nothing; RU's and
      // RD's stay live.
      const cell = (sector: string) => page.locator(`.sec-cell[data-sector="${sector}"]`);
      for (const sector of ["lu", "ld"]) await expect(cell(sector), sector).toHaveAttribute("aria-disabled", "true");
      for (const sector of ["ru", "rd"]) await expect(cell(sector), sector).toHaveAttribute("aria-disabled", "false");
      const arrange = page.getByTestId("arrange");
      await expect(arrange).toHaveAttribute("aria-disabled", "true");
      expect(Number(await cell("lu").evaluate((e) => getComputedStyle(e).opacity))).toBeLessThan(0.5);
      const tab = (id: string) => page.locator(`[data-testid="sector-tab"][data-layout="${id}"]`);
      await tab("lu-2").dispatchEvent("pointerdown");
      await arrange.dispatchEvent("pointerdown");
      await frames(page);
      await expect(tab("lu-1")).toHaveAttribute("aria-pressed", "true");
      await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "false");
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      // A layout tab leaves the deck; LU's layouts and ARRANGE are live again.
      await page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-page="foh"]').click();
      await expect(page.getByTestId("deck")).toHaveCount(0);
      await expect(cell("lu")).toHaveAttribute("aria-disabled", "false");
      await expect(arrange).toHaveAttribute("aria-disabled", "false");
      await tab("lu-2").click();
      await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "true");
    });

    test("an open deck stays mounted while the sectors switch on and off under it (#148 review)", async ({ page }) => {
      // Another tablet's ARRANGE save or a hand edit can switch the default
      // page between its rows and the sectors: the page is mounted anew,
      // the deck beside it is not (its unmount would lift a held key).
      await openSurface(page);
      await expect(page.locator('[data-testid="page"][data-page="foh"]')).toBeVisible();
      await expect(sectorsPage(page)).toHaveCount(0);
      await openDeck(page);
      await page.getByTestId("deck").evaluate((e) => e.setAttribute("data-e2e-kept", "1"));
      await page.getByTestId("deck-key").evaluateAll((all) => all.forEach((e) => e.setAttribute("data-e2e-kept", "1")));
      const kept = async () => {
        await expect(page.getByTestId("deck")).toHaveAttribute("data-e2e-kept", "1");
        await expect(page.locator('[data-testid="deck-key"][data-e2e-kept="1"]')).toHaveCount(32);
        await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "deck");
        await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "true");
      };
      // Sectors on: the sectors page, the deck the same elements.
      await page.getByTestId("page").evaluate((e) => e.setAttribute("data-e2e-old", "1"));
      await setSectors(DOC);
      await expect(sectorsPage(page)).toBeVisible();
      await expect(page.locator('[data-testid="page"][data-e2e-old]')).toHaveCount(0);
      await kept();
      const column = (await page.getByTestId("column").boundingBox())!;
      const keys = await page.getByTestId("deck-key").evaluateAll((all) => all.map((k) => k.getBoundingClientRect().right));
      for (const right of keys) expect(right, "a key right of the column").toBeLessThanOrEqual(column.x + 0.5);
      // Sectors off again: the rows, the deck still the same elements.
      await resetSectors();
      await expect(sectorsPage(page)).toHaveCount(0);
      await expect(page.locator('[data-testid="page"][data-page="foh"]')).toBeVisible();
      await kept();
    });

    test("the column's head: FOH a centred row of its own above STREAM DECK and ABLESET, the status at its foot, nothing overflowing (#148)", async ({ page }) => {
      // The site's column (the owner's report of 29.10.2026): the LAYOUTS
      // block, ARRANGE alone on its row, the rail STAGE, REVERB, AUTOTUNE,
      // TALKBACK and TECHALERT at its foot.
      const fixture = fixtureLayout();
      const rails = fixture.pages.find((p: any) => p.id === "foh").rail.length;
      const layout = structuredClone(fixture);
      const foh = layout.pages.find((p: any) => p.id === "foh");
      const stage = foh.rail.find((c: any) => c.kind === "stage");
      const talkback = { ...foh.rail.find((c: any) => c.label === "ZVUKAR"), label: "TALKBACK" };
      foh.rail = [stage, { kind: "reverb", label: "REVERB" }, { kind: "autotune", label: "AUTOTUNE" }, talkback];
      await harness("/hub/layout", { layout });
      try {
        await until(servedLayout, (l) => l.pages.find((p: any) => p.id === "foh").rail.length === 4, "the hub serving the site's rail");
        await setSectors(DOC);
        await openSectors(page);
        await expect(page.getByTestId("sector-layouts")).toBeVisible();
        await expect(page.getByTestId("reverb")).toBeVisible();
        await expect(page.getByTestId("deck-tab")).toBeVisible();
        await expect(page.getByTestId("sector-undo")).toHaveCount(0);
        await frames(page);
        expect(await columnHeadAndFoot(page)).toEqual([]);
        // #212 (the owner's approved docs/mockups/column-calm-v2.html): no
        // LAYOUTS caption, and ARRANGE a small ⇄ key at the right of the FOH
        // tab, in its row; the tab takes the rest and keeps today's size.
        await expect(page.getByTestId("sector-layouts-title")).toHaveCount(0);
        await expect(page.locator(".sector-actions, .arrange-btn")).toHaveCount(0);
        const fohTab = page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-page="foh"]');
        await arrangeKeyBesideTheTab(page);
        expect(await fohTab.evaluate((t) => getComputedStyle(t).fontSize)).toBe("19px");
        for (const id of ["deck-tab", "ableset-tab"]) {
          expect(await page.getByTestId(id).evaluate((t) => getComputedStyle(t).fontSize), id).toBe("10px");
        }
        // The selected FOH tab keeps today's mark.
        await expect(fohTab).toHaveAttribute("data-selected", "true");
        expect(await fohTab.evaluate((t) => getComputedStyle(t).boxShadow)).toMatch(/inset/);
        // The head's freed room goes to the rail (#212): with the site's rail
        // and every extra tab the stage miniature is the rail's whole width at
        // 16:9 (the picture fills its box) and every rail row keeps 44 px.
        for (const id of ["ableset-tab", "iem-tab", "presenter-tab"]) await expect(page.getByTestId(id), id).toBeVisible();
        const mini = page.getByTestId("stage-mini");
        await expect(mini).toBeVisible();
        await frames(page, 4);
        const fit = await page.evaluate(() => {
          const box = (s: string) => document.querySelector(s)!.getBoundingClientRect();
          return {
            rail: box('[data-testid="rail"]').width,
            mini: box('[data-testid="stage-mini"]').width,
            area: box('[data-testid="mini-stage"]'),
            rows: getComputedStyle(document.querySelector('[data-testid="rail"] .rail-main')!).gridTemplateRows,
          };
        });
        console.log(`sectors column (${test.info().project.name}): rail ${fit.rail.toFixed(1)} px, picture ${fit.area.width.toFixed(1)} x ${fit.area.height.toFixed(1)}, rows ${fit.rows}`);
        expect(Math.abs(fit.area.width - (fit.rail - 10)), "the picture as wide as the rail less its box").toBeLessThan(0.6);
        expect(Math.abs(fit.area.width - (fit.mini - 10)), "the picture fills its box").toBeLessThan(0.6);
        expect(Math.abs(fit.area.width / fit.area.height - 16 / 9), "16:9").toBeLessThan(0.02);
        expect(fit.rows.split(" ").every((t) => t === "44px"), fit.rows).toBe(true);
      } finally {
        await harness("/hub/layout/reset");
        await until(servedLayout, (l) => l.pages.find((p: any) => p.id === "foh").rail.length === rails, "the hub serving the fixture again");
      }
    });

    test("the fader travel is 322 px at 1194x834 in all four sectors, a mixed layout included", async ({ page }) => {
      await setSectors(MIXED_DOC);
      await openSectors(page);
      await expect.poll(async () => (await stripsOf(page, "ld")).length).toBe(3);
      await frames(page);
      // One task inside the page (#198): a strip re-created between
      // `evaluateAll`'s query and its read would measure 0 px.
      const travels = await page.evaluate((selector) =>
        [...document.querySelectorAll(selector)]
          .filter((s) => s.querySelector('[data-testid="fader"]'))
          .map((s) => ({
            sector: s.closest(".slot")!.getAttribute("data-sector"),
            name: `${s.getAttribute("data-instance")}|${s.getAttribute("data-track")}`,
            h: s.querySelector('[data-testid="fader"]')!.getBoundingClientRect().height,
          })),
        `${SECTORS} .slot[data-sector] [data-testid="strip"]`,
      );
      // Every strip of lu, ru, ld (band and master mixed) and rd.
      expect(travels.length).toBe(7);
      expect([...new Set(travels.map((t) => t.sector))].sort()).toEqual(["ld", "lu", "rd", "ru"]);
      console.log(`sectors travel (${test.info().project.name}): ${[...new Set(travels.map((t) => t.h.toFixed(2)))].join(", ")} px`);
      for (const t of travels) expect(t.h, `${t.sector} ${t.name}`).toBeGreaterThanOrEqual(322 - 0.5);
    });

    test("a mixed layout names each strip's Live set as calm side text, never a pill", async ({ page }) => {
      await setSectors(MIXED_DOC);
      await openSectors(page);
      await expect.poll(() => stripsOf(page, "ld")).toEqual(["master|Hand1 #", "band|A-Reverb #", "master|A-Reverb #"]);
      // No instance pill anywhere on the sectors page.
      await expect(sectorsPage(page).locator('[data-testid="strip-instance"]')).toHaveCount(0);
      // In the mixed ld each strip's info column names its own Live set.
      // One task inside the page (#198): a re-created strip's side text
      // would read an empty computed colour through `evaluateAll`.
      const sides = await page.evaluate((selector) =>
        [...document.querySelectorAll(selector)].map((s) => {
          const side = s.querySelector('[data-testid="strip-col"] [data-testid="group-side"]') as HTMLElement | null;
          return side
            ? { text: side.textContent, color: getComputedStyle(side).color, events: getComputedStyle(side).pointerEvents }
            : null;
        }),
        `${slotsCss("ld")} [data-testid="strip"]`,
      );
      const faint = await page.evaluate(() => {
        const probe = document.createElement("span");
        probe.style.color = "var(--faint)";
        document.body.append(probe);
        const color = getComputedStyle(probe).color;
        probe.remove();
        return color;
      });
      expect(sides.map((s) => s?.text)).toEqual(["MASTER", "BAND", "MASTER"]);
      for (const side of sides) {
        expect(side!.color).toBe(faint);
        expect(side!.events).toBe("none");
      }
      // A layout of one Live set shows no side text.
      for (const sector of ["lu", "ru", "rd"]) {
        await expect(slots(page, sector).locator('[data-testid="group-side"]'), sector).toHaveCount(0);
      }
      // Paging ld to a layout of one set takes the side text away, also
      // from the band strip the page keeps (the same element, marked).
      const reverb = slots(page, "ld").filter({
        has: page.locator('[data-testid="strip"][data-track="A-Reverb #"][data-instance="band"]'),
      });
      await reverb.evaluate((slot) => slot.setAttribute("data-e2e-kept", "1"));
      await page.locator('[data-testid="sector-tab"][data-layout="ld-2"]').click();
      await expect.poll(() => stripsOf(page, "ld")).toEqual(["band|A-Reverb #"]);
      await expect(reverb).toHaveAttribute("data-e2e-kept", "1");
      await expect(slots(page, "ld").locator('[data-testid="group-side"]')).toHaveCount(0);
    });
  });

  test.describe("the column's picks go to the event log (#212)", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    test("a page tab, a layout tab and the ARRANGE key each leave a tab event, a REMOVE? confirm and an inert tap none", async ({ page }) => {
      await setSectors(DOC);
      await openSectors(page);
      const since = await page.evaluate(() => performance.timeOrigin + performance.now());
      const tab = (id: string) => page.locator(`[data-testid="sector-tab"][data-layout="${id}"]`);
      const pageTab = (id: string) => page.locator(`[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-page="${id}"]`);
      // A real switch to CUE and back to FOH, then a re-tap of FOH (same).
      await pageTab("cue").click();
      await expect(pageTab("cue")).toHaveAttribute("data-selected", "true");
      await expect(sectorsPage(page)).toHaveCount(0);
      await pageTab("foh").click();
      await expect(sectorsPage(page)).toBeVisible();
      await pageTab("foh").click();
      await expect(pageTab("foh")).toHaveAttribute("data-selected", "true");
      // A real layout switch, then a re-tap of the layout shown (same).
      await tab("lu-2").click();
      await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "true");
      await tab("lu-2").click();
      await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "true");
      const arrange = page.getByTestId("arrange");
      await arrange.click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      await tab("ld-2").click();
      await expect(tab("ld-2")).toHaveAttribute("aria-pressed", "true");
      // ✕ asks on ld-2's tab; the tab's tap confirms the removal: no pick.
      await page.locator('.lay-ed[data-layout="ld-2"] [data-testid="remove"]').click();
      await expect(tab("ld-2")).toHaveText("REMOVE?");
      await tab("ld-2").click();
      await expect(tab("ld-2")).toHaveCount(0);
      // The second press saves the edit.
      await arrange.click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      // Under STREAM DECK LU's layouts and ARRANGE are inert: a dispatched
      // down on them changes nothing and records nothing; the FOH tab that
      // closes the deck is the next pick.
      await openDeck(page);
      await tab("lu-1").dispatchEvent("pointerdown");
      await arrange.dispatchEvent("pointerdown");
      await frames(page);
      await expect(tab("lu-2")).toHaveAttribute("aria-pressed", "true");
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      await pageTab("foh").click();
      await expect(page.getByTestId("deck")).toHaveCount(0);
      const picks = await until(
        async () =>
          pageEvents(await hubEvents())
            .filter((e) => e.ev === "tab" && e.t >= since)
            .map(({ ev, t, ...rest }) => rest),
        (p) => p.length >= 9,
        "the page's tab events",
        15000,
      );
      // `same`: whether the pick changed anything (a re-tap did not; the FOH
      // tab that closes the deck did: it shows the page again).
      expect(picks).toEqual([
        { what: "page", id: "cue", same: false },
        { what: "page", id: "foh", same: false },
        { what: "page", id: "foh", same: true },
        { what: "layout", sector: "lu", id: "lu-2", title: "ALT", arrange: false, same: false },
        { what: "layout", sector: "lu", id: "lu-2", title: "ALT", arrange: false, same: true },
        { what: "arrange", act: "start" },
        { what: "layout", sector: "ld", id: "ld-2", title: "FX", arrange: true, same: false },
        { what: "arrange", act: "save" },
        { what: "page", id: "foh", same: false },
      ]);
    });
  });

  // Phones (#212): the ⇄ key sits in the ☰ column beside the page tab, whole
  // and reachable, and a tap on it starts and leaves ARRANGE.
  for (const size of [
    { width: 844, height: 390 },
    { width: 390, height: 844 },
  ]) {
    test.describe(`on a phone, ${size.width}×${size.height}`, () => {
      test.use({ viewport: size });

      test("the ARRANGE key is whole and reachable in the ☰ column", async ({ page }) => {
        await setSectors(DOC);
        await openSectors(page);
        await page.getByTestId("thin-burger").dispatchEvent("pointerdown");
        await expect(sectorsPage(page)).toHaveAttribute("data-column-open", "true");
        await expect(page.getByTestId("column")).toBeVisible();
        await frames(page);
        await arrangeKeyBesideTheTab(page);
        expect(await columnHeadAndFoot(page)).toEqual([]);
        const column = (await page.getByTestId("column").boundingBox())!;
        const key = (await page.getByTestId("arrange").boundingBox())!;
        expect(key.x).toBeGreaterThanOrEqual(column.x - 0.5);
        expect(key.x + key.width).toBeLessThanOrEqual(column.x + column.width + 0.5);
        expect(key.y + key.height).toBeLessThanOrEqual(size.height);
        // A real tap at its centre starts the mode; the column stays open.
        await page.mouse.click(key.x + key.width / 2, key.y + key.height / 2);
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
        await expect(page.getByTestId("arrange")).toHaveAttribute("aria-pressed", "true");
        await expect(sectorsPage(page)).toHaveAttribute("data-column-open", "true");
        await page.mouse.click(key.x + key.width / 2, key.y + key.height / 2);
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      });
    });
  }

  test.describe("ARRANGE with empty slots (the slot model)", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    test("ARRANGE puts a strip at the right edge with a gap", async ({ page, context }) => {
      await setSectors(GAP_DOC);
      await openSectors(page);
      const ru = ["master|Hand2 #", name(VOCAL), name(KEYS), "master|Hand4 #"];
      await expect.poll(() => cellsOf(page, "ru")).toEqual(ru);
      // Outside ARRANGE no empty cell is drawn.
      await expect(sectorsPage(page).getByTestId("sector-gap")).toHaveCount(0);
      const live = await LiveClient.open();
      try {
        const HAND4_VOLUME = volume(track("Hand4 #"));
        const volumeBefore = await live.get("master", HAND4_VOLUME, "value");
        const revBefore = (await hubStatus()).sectors.rev;
        const since = Date.now();
        await frames(page);
        const width = await stripWidth(page);

        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
        // Every position ru can show is a cell: its 4 strips and 2 empty
        // ones, drawn as calm dashed outlines; lu is full, so none there.
        await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "_", "_"]);
        await expect(slots(page, "lu").filter({ has: page.getByTestId("edit-cover") })).toHaveCount(6);
        await expect(slots(page, "lu").and(page.getByTestId("sector-gap"))).toHaveCount(0);
        const gap = slots(page, "ru").nth(5);
        await expect(gap).toHaveAttribute("data-testid", "sector-gap");
        const look = await gap.evaluate((e) => ({ style: getComputedStyle(e).outlineStyle, events: getComputedStyle(e).pointerEvents }));
        expect(look).toEqual({ style: "dashed", events: "none" });
        await frames(page);
        expect(await stripWidth(page), "ARRANGE resizes nothing").toBe(width);

        // Hand4 # to the sixth cell: its old place stays empty.
        await carry(page, "ru", "Hand4 #", 5);
        await expect.poll(() => cellsOf(page, "ru")).toEqual(["master|Hand2 #", name(VOCAL), name(KEYS), "_", "_", "master|Hand4 #"]);
        expect((await hubStatus()).sectors.rev).toBe(revBefore);

        // The second tap saves: the hub's document holds the gaps as null.
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
        await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the order");
        const steps = await until(() => sectorSteps(since), (s) => s.includes("edit"), "the hub's edit record");
        expect(steps).toEqual(["edit"]);
        await until(
          async () => live.sectors?.doc?.sectors?.ru?.[0]?.strips,
          (strips: any) => Array.isArray(strips) && strips.length === 6,
          "the hub's document with ru's six slots",
        );
        expect(live.sectors.doc.sectors.ru[0].strips).toEqual([master("Hand2 #"), VOCAL, KEYS, null, null, master("Hand4 #")]);

        // Nothing reached Live.
        const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
        expect(sets, JSON.stringify(sets)).toEqual([]);
        expect(await live.get("master", HAND4_VOLUME, "value")).toBe(volumeBefore);

        // A second page shows Hand4 # at ru's right edge (the line's), with
        // two strip widths of empty space before it.
        const other = await context.newPage();
        await other.setViewportSize({ width: 1194, height: 834 });
        const otherConsole = watchConsole(other);
        await openSectors(other);
        await expect.poll(() => cellsOf(other, "ru")).toEqual(["master|Hand2 #", name(VOCAL), name(KEYS), "_", "_", "master|Hand4 #"]);
        await frames(other);
        // Outside ARRANGE a saved gap is plain space: no outline, no border.
        const otherGaps = sectorsPage(other).getByTestId("sector-gap");
        await expect(otherGaps).toHaveCount(2);
        // One task inside the page (#198): a gap re-created between
        // `evaluateAll`'s query and its read has an empty computed style.
        const plain = await other.evaluate((selector) =>
          [...document.querySelectorAll(selector)].map((e) => ({
            outline: getComputedStyle(e).outlineStyle,
            border: getComputedStyle(e).borderTopStyle,
          })),
          `${SECTORS} [data-testid="sector-gap"]`,
        );
        expect(plain).toEqual([
          { outline: "none", border: "none" },
          { outline: "none", border: "none" },
        ]);
        const placed =await sectorsPage(other).evaluate((body) => {
          const slot = (track: string) =>
            [...body.querySelectorAll('.slot[data-sector="ru"]')].find(
              (s) => s.querySelector(`[data-testid="strip"][data-track="${track}"]`) != null,
            )!.getBoundingClientRect();
          const line = body.querySelector('.slot[data-sector="ru"]')!.closest(".line")!.getBoundingClientRect();
          const strip = parseFloat((body as HTMLElement).style.getPropertyValue("--strip-w"));
          return { lineRight: line.right, hand4: slot("Hand4 #"), keys: slot("Keys 1"), strip };
        });
        expect(Math.abs(placed.hand4.right - placed.lineRight), JSON.stringify(placed)).toBeLessThan(2);
        expect(placed.hand4.left - placed.keys.right, JSON.stringify(placed)).toBeGreaterThanOrEqual(2 * placed.strip);
        expect(otherConsole, "the second page's console").toEqual([]);
        await other.close();
      } finally {
        live.close();
      }
    });

    test("ARRANGE swaps a strip dropped on another", async ({ page }) => {
      await setSectors(GAP_DOC);
      await openSectors(page);
      await expect.poll(() => cellsOf(page, "ru")).toEqual(["master|Hand2 #", name(VOCAL), name(KEYS), "master|Hand4 #"]);
      const revBefore = (await hubStatus()).sectors.rev;
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      await expect.poll(async () => (await cellsOf(page, "ru")).length).toBe(6);
      // Hand2 # dropped on Keys 1: the two swap, the others stay.
      await carry(page, "ru", "Hand2 #", 2);
      await expect.poll(() => cellsOf(page, "ru")).toEqual([name(KEYS), name(VOCAL), "master|Hand2 #", "master|Hand4 #", "_", "_"]);
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the order");
      await expect.poll(() => cellsOf(page, "ru")).toEqual([name(KEYS), name(VOCAL), "master|Hand2 #", "master|Hand4 #"]);
    });

    test("ARRANGE moves a strip onto another sector's cell and back (#233)", async ({ page }) => {
      await setSectors(GAP_DOC);
      await openSectors(page);
      const lu = [name(VOCAL), name(KEYS), "band|A-Reverb #", "master|A-Reverb #", "master|Hand1 #", "master|Hand4 #"];
      const ru = ["master|Hand2 #", name(VOCAL), name(KEYS), "master|Hand4 #"];
      await expect.poll(() => cellsOf(page, "lu")).toEqual(lu);
      await expect.poll(() => cellsOf(page, "ru")).toEqual(ru);
      const live = await LiveClient.open();
      try {
        const HAND1_VOLUME = volume(track("Hand1 #"));
        const volumeBefore = await live.get("master", HAND1_VOLUME, "value");
        const since = Date.now();
        const revBefore = (await hubStatus()).sectors.rev;
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
        await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "_", "_"]);

        // Carried just past the slop, over its own slot: no mark (it only
        // goes back there).
        const own = coverOf(page, "lu", "Hand1 #");
        await dispatchPointer(own, [
          { type: "pointerdown" },
          { type: "pointermove", dx: 10, dy: 0 },
        ]);
        await expect(slots(page, "lu").nth(4)).toHaveAttribute("data-carried", "true");
        expect(await dropMarks(page)).toEqual([]);
        await dispatchPointer(own, [{ type: "pointercancel" }]);

        // A layout that already holds the strip takes no drop: Keys 1 held
        // over RU's empty cell marks it `no`, and its lift there moves
        // nothing.
        const keys = await carry(page, "lu", "Keys 1", 4, "ru", false);
        await expect.poll(() => dropMarks(page)).toEqual(["ru/4=no"]);
        await dispatchPointer(keys, [{ type: "pointercancel" }]);
        await expect.poll(() => dropMarks(page)).toEqual([]);
        await carry(page, "lu", "Keys 1", 4, "ru");
        await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "_", "_"]);
        await expect.poll(() => cellsOf(page, "lu")).toEqual(lu);

        // Hand1 # from LU onto RU's first empty cell: held there, the cell
        // is marked `yes`; at the lift it moves, its LU slot left empty.
        const hand1 = await carry(page, "lu", "Hand1 #", 4, "ru", false);
        await expect.poll(() => dropMarks(page)).toEqual(["ru/4=yes"]);
        await dispatchPointer(hand1, [{ type: "pointercancel" }]);
        await carry(page, "lu", "Hand1 #", 4, "ru");
        const luMoved = [name(VOCAL), name(KEYS), "band|A-Reverb #", "master|A-Reverb #", "_", "master|Hand4 #"];
        await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "master|Hand1 #", "_"]);
        await expect.poll(() => cellsOf(page, "lu")).toEqual(luMoved);
        await expect.poll(() => dropMarks(page)).toEqual([]);
        expect((await hubStatus()).sectors.rev, "nothing saved before the second tap").toBe(revBefore);

        // The second tap saves it as ONE edit, moved 1.
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
        await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the move");
        const steps = await until(() => sectorSteps(since), (s) => s.includes("edit"), "the hub's edit record");
        expect(steps).toEqual(["edit"]);
        const record = (await hubEvents()).find((r: any) => r.ev === "sectors" && r.what === "edit" && r.ts >= since);
        expect({ moved: record.moved, added: record.added, removed: record.removed, reordered: record.reordered }).toEqual({
          moved: 1,
          added: 0,
          removed: 0,
          reordered: 2,
        });
        // Shown in RU, gone from LU (its slot a plain gap now).
        await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "master|Hand1 #"]);
        await expect.poll(() => cellsOf(page, "lu")).toEqual(luMoved);
        await expect(slots(page, "lu").filter({ has: page.locator('[data-track="Hand1 #"]') })).toHaveCount(0);
        await until(
          async () => live.sectors?.doc?.sectors,
          (s: any) => s?.ru?.[0]?.strips?.length === 5,
          "the hub's document with Hand1 # in ru",
        );
        expect(live.sectors.doc.sectors.ru[0].strips).toEqual([master("Hand2 #"), VOCAL, KEYS, master("Hand4 #"), master("Hand1 #")]);
        expect(live.sectors.doc.sectors.lu[0].strips).toEqual([
          VOCAL,
          KEYS,
          band("A-Reverb #"),
          master("A-Reverb #"),
          null,
          master("Hand4 #"),
        ]);

        // Back: Hand1 # from RU onto LU's empty cell, saved: the fixture's
        // document as it was.
        const backSince = Date.now();
        const revMoved = (await hubStatus()).sectors.rev;
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
        await expect.poll(() => cellsOf(page, "lu")).toEqual(luMoved);
        await carry(page, "ru", "Hand1 #", 4, "lu");
        await expect.poll(() => cellsOf(page, "lu")).toEqual(lu);
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
        await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revMoved, "the hub taking the move back");
        await until(
          async () => (await hubEvents()).find((r: any) => r.ev === "sectors" && r.what === "edit" && r.ts >= backSince),
          (r: any) => r != null,
          "the move back's edit record",
        ).then((r: any) => expect(r.moved).toBe(1));
        await expect.poll(() => cellsOf(page, "ru")).toEqual(ru);
        await expect.poll(() => cellsOf(page, "lu")).toEqual(lu);
        await until(
          async () => live.sectors?.doc?.sectors,
          (s: any) => s?.ru?.[0]?.strips?.length === 4,
          "the hub's document as the fixture's",
        );
        expect(live.sectors.doc.sectors).toEqual(GAP_DOC.sectors);

        // Nothing reached Live.
        const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
        expect(sets, JSON.stringify(sets)).toEqual([]);
        expect(await live.get("master", HAND1_VOLUME, "value")).toBe(volumeBefore);
      } finally {
        live.close();
      }
    });

    test("ARRANGE refuses a drop that would make a layout FULL, and loses nothing (#236)", async ({ page }) => {
      await setSectors(FULL_RU_DOC);
      await openSectors(page);
      const lu = [name(VOCAL), "band|Hand2 #"];
      const ru = [name(KEYS), "master|A-Reverb #", "master|Hand1 #", "master|Hand4 #", "master|Hand2 #"];
      const since = Date.now();
      const revBefore = (await hubStatus()).sectors.rev;
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      // LU's further cells up to its 6 units (2.1 + three), RU's one.
      await expect.poll(() => cellsOf(page, "lu")).toEqual([...lu, "_", "_", "_"]);
      await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "_"]);

      // A first, unsaved change of the session: Vocal and Hand2 # swap in LU.
      await carry(page, "lu", "Hand2 #", 0);
      const luSwapped = ["band|Hand2 #", name(VOCAL)];
      await expect.poll(() => cellsOf(page, "lu")).toEqual([...luSwapped, "_", "_", "_"]);

      // RU holds 5 units: the wide Hand2 # on its sixth cell would make it
      // 6.1, past the 6 a sector holds (the hub would refuse the save FULL).
      // Held there the cell reads `no`; its lift moves nothing.
      const wide = await carry(page, "lu", "Hand2 #", 5, "ru", false);
      await expect.poll(() => dropMarks(page)).toEqual(["ru/5=no"]);
      await dispatchPointer(wide, [{ type: "pointercancel" }]);
      await expect.poll(() => dropMarks(page)).toEqual([]);
      await carry(page, "lu", "Hand2 #", 5, "ru");
      await expect.poll(() => cellsOf(page, "ru")).toEqual([...ru, "_"]);
      await expect.poll(() => cellsOf(page, "lu")).toEqual([...luSwapped, "_", "_", "_"]);
      await expect(page.getByTestId("arrange-note")).toHaveText(ARRANGE_NOTE);

      // Vocal (one unit) fits there: held over the same cell it reads `yes`.
      const vocal = await carry(page, "lu", VOCAL.label, 5, "ru", false);
      await expect.poll(() => dropMarks(page)).toEqual(["ru/5=yes"]);
      await dispatchPointer(vocal, [{ type: "pointercancel" }]);
      await expect.poll(() => dropMarks(page)).toEqual([]);

      // The save keeps the session's change: one edit, taken (no FULL).
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the swap");
      expect(await sectorSteps(since)).toEqual(["edit"]);
      await expect.poll(() => cellsOf(page, "lu")).toEqual(luSwapped);
      await expect.poll(() => cellsOf(page, "ru")).toEqual(ru);
      const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
      expect(sets, JSON.stringify(sets)).toEqual([]);
    });
  });

  test.describe("ARRANGE edits the layouts (mockup v2)", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    /** A sector's cell in the LAYOUTS block. */
    const cell = (page: Page, sector: string) => page.locator(`.sec-cell[data-sector="${sector}"]`);
    /** A layout's tab, its field and its keys. */
    const row = (page: Page, id: string) => page.locator(`.lay-ed[data-layout="${id}"]`);
    const tabOf = (page: Page, id: string) => row(page, id).getByTestId("sector-tab");
    /** A sector's tabs' texts, in their order. */
    const titles = (page: Page, sector: string) => cell(page, sector).getByTestId("sector-tab").allTextContents();

    /** Whether `locator`'s text fits its box (nothing cut). */
    const whole = (locator: Locator) => locator.evaluate((e) => e.scrollWidth <= e.clientWidth + 0.5);

    test("rename, add and remove save as one change, a second page sees it, ↶ restores it, nothing reaches Live", async ({
      page,
      context,
    }) => {
      await setSectors(DOC);
      await openSectors(page);
      await expect.poll(() => titles(page, "lu")).toEqual(["STAGE", "ALT"]);
      const live = await LiveClient.open();
      try {
        const VOCAL_VOLUME = volume(track("Vocal 1 repro#"));
        const volumeBefore = await live.get("band", VOCAL_VOLUME, "value");
        const revBefore = (await hubStatus()).sectors.rev;
        const since = Date.now();
        // An added layout's id names the revision its save makes and the
        // edit's add count, so no id is ever used again (#148 review).
        const [firstAdd, secondAdd] = [1, 2].map((n) => `lu-r${revBefore + 1}-${n}`);

        // Outside ARRANGE the block only pages: no ✎, ✕ or +.
        await expect(page.getByTestId("add-layout")).toHaveCount(0);
        for (const id of ["lu-1", "lu-2", "ru-1", "ld-1", "ld-2", "rd-1"]) await expect(row(page, id).getByTestId("rename"), id).toBeHidden();
        await expect(row(page, "lu-1").getByTestId("remove")).toBeHidden();

        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
        await expect(page.getByTestId("arrange-note")).toHaveText(ARRANGE_NOTE);
        // ✎ on every tab; ✕ only where a sector has more than one layout.
        await expect(page.getByTestId("rename")).toHaveCount(6);
        for (const id of ["lu-1", "lu-2", "ru-1", "ld-1", "ld-2", "rd-1"]) await expect(row(page, id).getByTestId("rename"), id).toBeVisible();
        for (const id of ["lu-1", "lu-2", "ld-1", "ld-2"]) await expect(row(page, id).getByTestId("remove")).toBeVisible();
        for (const id of ["ru-1", "rd-1"]) await expect(row(page, id).getByTestId("remove"), id).toHaveCount(0);
        await expect(page.getByTestId("add-layout")).toHaveCount(4);

        // Rename lu-1 in its field: 8 capitals at most, Enter keeps it.
        await row(page, "lu-1").getByTestId("rename").click();
        await expect(row(page, "lu-1")).toHaveAttribute("data-renaming", "true");
        const field = row(page, "lu-1").getByTestId("rename-input");
        await expect(field).toBeFocused();
        await expect(field).toHaveValue("STAGE");
        await field.fill("monitors2");
        await expect(field).toHaveValue("MONITORS");
        expect(await whole(field), "the field holds 8 capitals whole").toBe(true);
        await field.press("Enter");
        await expect(row(page, "lu-1")).toHaveAttribute("data-renaming", "false");
        await expect(field).not.toBeFocused();
        await expect.poll(() => titles(page, "lu")).toEqual(["MONITORS", "ALT"]);

        // Rename ld-2, kept by a tap elsewhere.
        await row(page, "ld-2").getByTestId("rename").click();
        const fx = row(page, "ld-2").getByTestId("rename-input");
        await expect(fx).toBeFocused();
        await fx.fill("effects");
        await page.getByTestId("arrange-note").click();
        await expect(row(page, "ld-2")).toHaveAttribute("data-renaming", "false");
        await expect.poll(() => titles(page, "ld")).toEqual(["HANDS", "EFFECTS"]);

        // + adds NEW, shown at once (empty); at 4 layouts the + is gone.
        await cell(page, "lu").getByTestId("add-layout").click();
        await expect.poll(() => titles(page, "lu")).toEqual(["MONITORS", "ALT", "NEW"]);
        await expect(tabOf(page, firstAdd)).toHaveAttribute("aria-pressed", "true");
        await expect.poll(() => stripsOf(page, "lu")).toEqual([]);
        await cell(page, "lu").getByTestId("add-layout").click();
        await expect.poll(() => titles(page, "lu")).toEqual(["MONITORS", "ALT", "NEW", "NEW 2"]);
        await expect(tabOf(page, secondAdd)).toHaveAttribute("aria-pressed", "true");
        await expect(cell(page, "lu").getByTestId("add-layout")).toHaveCount(0);

        // Four layouts with their keys fit the column: the block whole, the
        // rows in their order inside the column, the rail's first button
        // whole, and the faders keep their 322 px.
        await frames(page);
        const fit = await page.getByTestId("column").evaluate((column) => {
          const box = (e: Element | null) => e!.getBoundingClientRect();
          const col = box(column);
          const block = column.querySelector('[data-testid="sector-layouts"]') as HTMLElement;
          const order = [
            block,
            column.querySelector('[data-testid="arrange-note"]'),
            column.querySelector('[data-testid="rail"]'),
          ].map(box);
          const inBlock = [...block.querySelectorAll(".tab, .tool, .tab-add")]
            .filter((e) => (e as HTMLElement).offsetParent !== null)
            .map(box)
            .every((r) => r.top >= box(block).top - 0.5 && r.bottom <= box(block).bottom + 0.5);
          const railMain = column.querySelector(".rail-main")!;
          const firstButton = railMain.firstElementChild;
          return {
            blockWhole: block.scrollHeight <= block.clientHeight + 0.5,
            inBlock,
            ordered: order.every((r, i) => i === 0 || r.top >= order[i - 1].bottom - 0.5),
            inColumn: order.every((r) => r.top >= col.top - 0.5 && r.bottom <= col.bottom + 0.5),
            railButton: firstButton == null || box(firstButton).bottom <= box(railMain).bottom + 0.5,
            blockHeight: box(block).height,
          };
        });
        console.log(`sectors ARRANGE block at 4 layouts (${test.info().project.name}): ${fit.blockHeight.toFixed(1)} px`);
        expect(fit).toEqual({ ...fit, blockWhole: true, inBlock: true, ordered: true, inColumn: true, railButton: true });
        // One task inside the page (#198): ARRANGE re-arranges the cells,
        // and a re-created fader read through `evaluateAll` measures 0 px.
        const travels = await page.evaluate((selector) =>
          [...document.querySelectorAll(selector)].map((f) => f.getBoundingClientRect().height),
          `${SECTORS} .slot[data-sector] [data-testid="fader"]`,
        );
        expect(travels.length).toBeGreaterThan(0);
        for (const h of travels) expect(h).toBeGreaterThanOrEqual(322 - 0.5);

        // ✕ asks on its tab, a tap elsewhere takes the question back.
        await row(page, "lu-2").getByTestId("remove").click();
        await expect(tabOf(page, "lu-2")).toHaveText("REMOVE?");
        await expect(tabOf(page, "lu-2")).toHaveClass(/confirm/);
        expect(await whole(tabOf(page, "lu-2")), "REMOVE? whole").toBe(true);
        await page.getByTestId("arrange-note").click();
        await expect(tabOf(page, "lu-2")).toHaveText("ALT");
        // Asked again, the tab's second tap removes it; the + is back.
        await row(page, "lu-2").getByTestId("remove").click();
        await expect(tabOf(page, "lu-2")).toHaveText("REMOVE?");
        await tabOf(page, "lu-2").click();
        await expect(row(page, "lu-2")).toHaveCount(0);
        await expect.poll(() => titles(page, "lu")).toEqual(["MONITORS", "NEW", "NEW 2"]);
        await expect(cell(page, "lu").getByTestId("add-layout")).toHaveCount(1);

        // Nothing is saved before the second tap.
        expect((await hubStatus()).sectors.rev).toBe(revBefore);

        // The second tap saves all of it as one change.
        await page.getByTestId("arrange").click();
        await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
        await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the edit");
        const steps = await until(() => sectorSteps(since), (s) => s.includes("edit"), "the hub's edit record");
        expect(steps).toEqual(["edit"]);
        const record = (await hubEvents()).find((r: any) => r.ev === "sectors" && r.what === "edit" && r.ts >= since);
        expect({ added: record.added, removed: record.removed, renamed: record.renamed, reordered: record.reordered }).toEqual({
          added: 2,
          removed: 1,
          renamed: 2,
          reordered: 0,
        });
        await until(
          async () => live.sectors?.doc?.sectors?.lu?.map((l: any) => l.title),
          (lu: any) => Array.isArray(lu) && lu.length === 3,
          "the hub's document with lu's three layouts",
        );
        expect(live.sectors.doc.sectors.lu.map((l: any) => [l.id, l.title, l.strips])).toEqual([
          ["lu-1", "MONITORS", [VOCAL, KEYS]],
          [firstAdd, "NEW", []],
          [secondAdd, "NEW 2", []],
        ]);
        expect(live.sectors.doc.sectors.ld.map((l: any) => l.title)).toEqual(["HANDS", "EFFECTS"]);
        // Outside ARRANGE again: the new names, no keys.
        await expect.poll(() => titles(page, "lu")).toEqual(["MONITORS", "NEW", "NEW 2"]);
        await expect(page.getByTestId("add-layout")).toHaveCount(0);
        for (const id of ["lu-1", firstAdd, secondAdd, "ru-1", "ld-1", "ld-2", "rd-1"]) await expect(row(page, id).getByTestId("rename"), id).toBeHidden();

        // Nothing reached Live.
        const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
        expect(sets, JSON.stringify(sets)).toEqual([]);
        expect(await live.get("band", VOCAL_VOLUME, "value")).toBe(volumeBefore);

        // A second page sees the new layouts (this browser's picks are
        // shared: its lu shows NEW 2, empty, as this page does).
        const other = await context.newPage();
        await other.setViewportSize({ width: 1194, height: 834 });
        const otherConsole = watchConsole(other);
        await openSectors(other);
        await expect.poll(() => titles(other, "lu")).toEqual(["MONITORS", "NEW", "NEW 2"]);
        await expect.poll(() => titles(other, "ld")).toEqual(["HANDS", "EFFECTS"]);
        await expect(tabOf(other, secondAdd)).toHaveAttribute("aria-pressed", "true");
        await expect.poll(() => stripsOf(other, "lu")).toEqual([]);
        expect(otherConsole, "the second page's console").toEqual([]);
        await other.close();
      } finally {
        live.close();
      }
    });

    test("a sector's last layout has no ✕ and a removed layout another device shows falls back to its first", async ({
      page,
      context,
    }) => {
      await setSectors(DOC);
      // A second page shows lu's ALT.
      const other = await context.newPage();
      await other.setViewportSize({ width: 1194, height: 834 });
      const otherConsole = watchConsole(other);
      await openSectors(other);
      await tabOf(other, "lu-2").click();
      await expect.poll(() => stripsOf(other, "lu")).toEqual(["band|Hand2 #"]);

      await openSectors(page);
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      // lu-2 removed: lu-1 is lu's last, so its ✕ goes.
      await expect(row(page, "lu-1").getByTestId("remove")).toBeVisible();
      await row(page, "lu-2").getByTestId("remove").click();
      await tabOf(page, "lu-2").click();
      await expect(row(page, "lu-2")).toHaveCount(0);
      await expect(row(page, "lu-1").getByTestId("remove")).toHaveCount(0);
      await page.getByTestId("arrange").click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");

      // The other page's lu shows its first layout now; its pick is gone.
      await expect.poll(() => titles(other, "lu")).toEqual(["STAGE"]);
      await expect.poll(() => stripsOf(other, "lu")).toEqual([name(VOCAL), name(KEYS)]);
      await expect(tabOf(other, "lu-1")).toHaveAttribute("aria-pressed", "true");
      expect(otherConsole, "the second page's console").toEqual([]);
      await other.close();
    });
  });

  test("ARRANGE reorders, saves, a second page sees it, and nothing reaches Live", async ({ page, context, browserName }) => {
    await setSectors(DOC);
    await openSectors(page);
    await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
    const live = await LiveClient.open();
    try {
      const VOCAL_VOLUME = volume(track("Vocal 1 repro#"));
      const volumeBefore = await live.get("band", VOCAL_VOLUME, "value");
      const revBefore = (await hubStatus()).sectors.rev;

      const since = Date.now();
      const arrange = page.getByTestId("arrange");
      await arrange.click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      await expect(arrange).toHaveAttribute("aria-pressed", "true");
      await expect(page.getByTestId("arrange-note")).toHaveText(ARRANGE_NOTE);
      // The ⇄ key is lit as the big button was (#212: the panel's lighter
      // ground and the yellow underline) and stays live at full opacity
      // while the rest of the head dims and takes no touch.
      const head = await page.evaluate(() => {
        const key = document.querySelector('[data-testid="arrange"]')!;
        const tab = document.querySelector('[data-testid="page-row"] > [data-testid="tab"]')!;
        const deck = document.querySelector('[data-testid="deck-tab"]')!;
        const style = (e: Element) => getComputedStyle(e);
        return {
          lit: key.classList.contains("selected"),
          shadow: style(key).boxShadow,
          keyOpacity: style(key).opacity,
          keyTouch: style(key).pointerEvents,
          tabOpacity: style(tab).opacity,
          tabTouch: style(tab).pointerEvents,
          deckOpacity: style(deck).opacity,
        };
      });
      expect(head).toEqual({ ...head, lit: true, keyOpacity: "1", keyTouch: "auto", tabOpacity: "0.3", tabTouch: "none", deckOpacity: "0.3" });
      expect(head.shadow).toMatch(/inset/);

      // Every strip lies under its cover: a real, hit-tested gesture on
      // Vocal 1 repro#'s fader cap and on its SEL meets the cover, moves no
      // fader and opens no detail (a mouse drag down the cap and a hold on
      // SEL in Chromium, a finger's tap on each on the iPad).
      const vocalSlot = slots(page, "lu").nth(0);
      const cap = await centre(vocalSlot.getByTestId("fader-cap"));
      const sel = await centre(vocalSlot.getByTestId("detail-key"));
      for (const [what, at] of [["the cap", cap], ["SEL", sel]] as const) {
        const hit = await page.evaluate(({ x, y }) => {
          const el = document.elementFromPoint(x, y);
          return {
            cover: el?.closest('[data-testid="edit-cover"]') != null,
            track: el?.closest(".slot")?.querySelector('[data-testid="strip"]')?.getAttribute("data-track") ?? null,
          };
        }, at);
        expect(hit, `what is under ${what}`).toEqual({ cover: true, track: "Vocal 1 repro#" });
      }
      if (browserName === "chromium") {
        await page.mouse.move(cap.x, cap.y);
        await page.mouse.down();
        await page.mouse.move(cap.x, cap.y + 20, { steps: 4 });
        await page.mouse.move(cap.x, cap.y + 40, { steps: 4 });
        await page.mouse.up();
        await page.mouse.move(sel.x, sel.y);
        await page.mouse.down();
        await page.waitForTimeout(700);
        await page.mouse.up();
      } else {
        await page.touchscreen.tap(cap.x, cap.y);
        await page.touchscreen.tap(sel.x, sel.y);
      }
      await frames(page);
      await expect(page.getByTestId("detail")).toHaveCount(0);
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "true");
      expect(await stripsOf(page, "lu"), "a vertical gesture moves no strip").toEqual([name(VOCAL), name(KEYS)]);
      const touched = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
      expect(touched, JSON.stringify(touched)).toEqual([]);
      expect(await live.get("band", VOCAL_VOLUME, "value")).toBe(volumeBefore);

      // Vocal 1 repro#'s cover carried past Keys 1's centre (the moves and
      // the lift on the same cover: a dispatched pointer has no capture).
      const covers = slots(page, "lu").getByTestId("edit-cover");
      await expect(covers).toHaveCount(2);
      const first: Locator = covers.nth(0);
      await expect(first).toBeVisible();
      const from = await centre(first);
      const past = (await centre(covers.nth(1))).x - from.x + 12;
      await dispatchPointer(first, [
        { type: "pointerdown" },
        { type: "pointermove", dx: 10, dy: 0 },
        { type: "pointermove", dx: past, dy: 0 },
        { type: "pointerup", dx: past, dy: 0 },
      ]);
      // The page shows the edit; nothing is saved yet.
      await expect.poll(() => stripsOf(page, "lu")).toEqual([name(KEYS), name(VOCAL)]);
      expect((await hubStatus()).sectors.rev).toBe(revBefore);

      // The second tap saves and leaves the mode.
      await arrange.click();
      await expect(sectorsPage(page)).toHaveAttribute("data-arrange", "false");
      await until(async () => (await hubStatus()).sectors.rev, (rev) => rev > revBefore, "the hub taking the order");
      const steps = await until(() => sectorSteps(since), (s) => s.includes("edit"), "the hub's edit record");
      expect(steps).toEqual(["edit"]);
      await expect.poll(() => stripsOf(page, "lu")).toEqual([name(KEYS), name(VOCAL)]);
      await expect(page.getByTestId("arrange-note")).toHaveText("");

      // Nothing reached Live: no write of any page since the first tap, the volume as it was.
      const sets = (await hubEvents()).filter((r: any) => r.ev === "set" && r.ts >= since);
      expect(sets, JSON.stringify(sets)).toEqual([]);
      expect(await live.get("band", VOCAL_VOLUME, "value")).toBe(volumeBefore);

      // A second page opened now shows the saved order.
      const other = await context.newPage();
      const otherConsole = watchConsole(other);
      await openSectors(other);
      await expect.poll(() => stripsOf(other, "lu")).toEqual([name(KEYS), name(VOCAL)]);
      expect(otherConsole, "the second page's console").toEqual([]);
      await other.close();
    } finally {
      live.close();
    }
  });

  test("the existing FOH page is unchanged with the empty document", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    await expect(page.getByTestId("sector-layouts")).toBeVisible();
    // The empty document while the page is open: the FOH page as before.
    await resetSectors();
    await expect(page.locator('[data-testid="page"][data-sectors]')).toHaveCount(0);
    await expect(page.getByTestId("page")).toHaveAttribute("data-page", "foh");
    await expect(page.getByTestId("sector-layouts")).toHaveCount(0);
    await expect(page.getByTestId("arrange")).toHaveCount(0);
    const pager = page.locator('[data-testid="tabbar"][data-level="1"]');
    await expect(pager).toBeVisible();
    await expect(pager.locator('[data-testid="tab"][data-page="stage"]')).toBeVisible();
    await expect(pager.locator('[data-testid="tab"][data-page="others"]')).toBeVisible();
    await expect(page.locator('[data-testid="strip"][data-track="Vocal 1 repro#"][data-instance="band"]')).toBeVisible();
    // The detail shows no LAYOUTS and no channel selector: the name chip
    // as before.
    const detail = await openDetail(page, "Vocal 1 repro#");
    await expect(detail.getByTestId("detail-layouts")).toHaveCount(0);
    await expect(detail.getByTestId("chan-open")).toHaveCount(0);
    await expect(detail.getByTestId("detail-name")).toHaveText("Vocal");
  });
});
