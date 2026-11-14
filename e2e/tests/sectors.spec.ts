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

// The fader sectors (#148; spec docs/superpowers/specs/2026-10-28-sectors-design.md,
// the approved looks docs/mockups/sectors-v1.html and sectors-v2.html): with
// a sectors document that places faders, the FOH page is four sectors around
// the column (lu ru over ld rd), each paging through its own layouts in the
// column's LAYOUTS block; a strip's channel detail lists the layouts it is
// in and adds it to another from ADD TO LAYOUT's sheet (#208), ARRANGE reorders a sector's strips and renames, adds and removes its
// layouts (saved as one change); there is no undo (the owner's ruling of
// 29.10.2026: the ↶ beside ARRANGE is gone, ARRANGE takes its row alone). The hub owns
// sectors.json; the harness starts it empty (sectors off) and each test puts
// the empty document back.
//
// The documents use the e2e fixture's frame strips (tools/import-tosc/
// fixtures/expected-layout.json), a frame strip's identity being its anchor's
// name (fohmixer_proto::sectors::strip_ref). Band Hand2 #, B-Main repro # and
// master A-Echo are wide (1.1 units); every other strip takes 1.

const band = (label: string): StripRef => ({ instance: "band", label });
const master = (label: string): StripRef => ({ instance: "master", label });

const VOCAL = band("Vocal 1 repro#");
const KEYS = band("Keys 1");

/** Every sector placed; lu and ld with two layouts each. */
const DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [
      { id: "lu-1", title: "STAGE", strips: [VOCAL, KEYS] },
      { id: "lu-2", title: "ALT", strips: [band("Hand2 #")] },
    ],
    ru: [{ id: "ru-1", title: "MAIN", strips: [band("B-Main repro #")] }],
    ld: [
      { id: "ld-1", title: "HANDS", strips: [master("Hand1 #"), master("Hand4 #")] },
      { id: "ld-2", title: "FX", strips: [band("A-Reverb #")] },
    ],
    rd: [{ id: "rd-1", title: "MAIN", strips: [master("A-Echo")] }],
  },
};

/** lu's STAGE at exactly 6 units (full: one more strip is refused) and master Hand2 # on the page in ld. */
const FULL_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [
      {
        id: "lu-1",
        title: "STAGE",
        strips: [VOCAL, KEYS, band("A-Reverb #"), master("A-Reverb #"), master("Hand1 #"), master("Hand4 #")],
      },
    ],
    ld: [{ id: "ld-1", title: "HANDS", strips: [master("Hand2 #")] }],
  },
};

/**
 * Every sector at its most layouts (4, `MAX_LAYOUTS`), long titles (8
 * characters, the most a rename takes); master Hand4 # in ld-1 alone (#208:
 * ADD's sheet then lists 15 layouts).
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

/** The note under ARRANGE while it is on (the approved mockup v2's). */
const ARRANGE_NOTE = "drag · ✎ ✕ + · tap to save";

/** The sectors page, as CSS (for a read in one task inside the page, #198). */
const SECTORS = '[data-testid="page"][data-sectors="true"]';
const sectorsPage = (page: Page) => page.locator(SECTORS);

/** A sector's slots on the sectors page, in their order: as CSS, and as a locator. */
const slotsCss = (sector: string) => `${SECTORS} .slot[data-sector="${sector}"]`;
const slots = (page: Page, sector: string) => page.locator(slotsCss(sector));

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
 * among the sector's cells): the moves and the lift dispatched on the
 * strip's own cover (a dispatched pointer has no capture), past the slop
 * first, the lift at the target cell's centre.
 */
async function carry(page: Page, sector: string, strip: string, to: number) {
  const cover = slots(page, sector)
    .filter({ has: page.locator(`[data-testid="strip"][data-track="${strip}"]`) })
    .getByTestId("edit-cover");
  await expect(cover).toBeVisible();
  const from = await centre(cover);
  const dx = (await centre(slots(page, sector).nth(to))).x - from.x;
  await dispatchPointer(cover, [
    { type: "pointerdown" },
    { type: "pointermove", dx: Math.sign(dx) * 10, dy: 0 },
    { type: "pointermove", dx, dy: 0 },
    { type: "pointerup", dx, dy: 0 },
  ]);
}

/** A sector's strips as `<instance>|<track>`, in their order. */
async function stripsOf(page: Page, sector: string): Promise<string[]> {
  return slots(page, sector)
    .locator('[data-testid="strip"]')
    .evaluateAll((all) => all.map((s) => `${s.getAttribute("data-instance")}|${s.getAttribute("data-track")}`));
}

const name = (r: StripRef) => `${r.instance}|${r.label}`;

/** Opens the surface and waits for the sectors page. */
async function openSectors(page: Page) {
  await openSurface(page);
  await expect(sectorsPage(page)).toBeVisible();
}

/** The page's `--strip-w`. */
function stripWidth(page: Page): Promise<string> {
  return sectorsPage(page).evaluate((e) => (e as HTMLElement).style.getPropertyValue("--strip-w"));
}

/** A second page's console (the fixture's guard watches the test's own page only). */
function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
  return problems;
}

/** The hub's `sectors` records since `since` (the runner's clock), each `what`. */
async function sectorSteps(since: number): Promise<string[]> {
  return (await hubEvents()).filter((r: any) => r.ev === "sectors" && r.ts >= since).map((r: any) => r.what);
}

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
        // ARRANGE takes its row alone: as wide as the LAYOUTS block.
        const arrange = (await page.locator(".arrange-btn").boundingBox())!;
        const block = (await page.getByTestId("sector-layouts").boundingBox())!;
        expect(Math.abs(arrange.width - block.width), "ARRANGE as wide as the LAYOUTS block").toBeLessThan(1);
        // The selected FOH tab keeps today's mark.
        const fohTab = page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-page="foh"]');
        await expect(fohTab).toHaveAttribute("data-selected", "true");
        expect(await fohTab.evaluate((t) => getComputedStyle(t).boxShadow)).toMatch(/inset/);
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
        await page.getByTestId("sector-layouts-title").click();
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
            column.querySelector(".sector-actions"),
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
        await page.getByTestId("sector-layouts-title").click();
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
    // its centre leaves the sheet open and the document as it was.
    const before = (await hubStatus()).sectors.rev;
    const at = await centre(pick);
    await page.mouse.click(at.x, at.y);
    await frames(page);
    await expect(sheet).toBeVisible();
    expect((await hubStatus()).sectors.rev).toBe(before);
    // ✕ closes the sheet.
    await sheet.getByTestId("sheet-close").click();
    await expect(sheet).toHaveCount(0);

    // The hub refuses the tick itself (another client asking past the page).
    const live = await LiveClient.open();
    try {
      const answer = await live.ask({ type: "sector_tick", layout: "lu-1", strip: master("Hand2 #"), on: true });
      expect(answer.type).toBe("error");
      expect(answer.message).toBe("FULL");
      expect((await hubStatus()).sectors.rev).toBe(before);
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
    // In two layouts, so the block has two rows, and fourteen picks.
    await layouts.getByTestId("lay-add").click();
    await detail.locator('[data-testid="lay-pick"][data-layout="lu-1"]').click();
    await expect.poll(() => layoutRows(layouts)).toEqual(["LU|MONITORS", "LD|MONITORS"]);
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
      // The most layouts a document holds (4 a sector) fit without a scroll.
      await expect(sheet.getByTestId("lay-pick")).toHaveCount(14);
      const body = await sheet
        .getByTestId("sheet-body")
        .evaluate((e) => ({ scroll: e.scrollHeight, client: e.clientHeight }));
      expect(body.scroll, `the picks' list scrolls at ${where}`).toBeLessThanOrEqual(body.client);
      for (const pick of await sheet.getByTestId("lay-pick").all()) {
        expect(await clipped(pick.locator(".sheet-pick-name")), `a pick's title at ${where}`).toEqual([]);
        expect(await clipped(pick.locator("small")), `a pick's sector at ${where}`).toEqual([]);
        expect(await insideOf(pick, sheet.locator(".sheet-panel")), `a pick inside the panel at ${where}`).toBe(true);
      }
      await sheet.getByTestId("sheet-close").click();
      await expect(sheet).toHaveCount(0);
    }
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
    // Each pick is an open recorded with why "pick" and the strip's keys.
    const picks = await until(
      async () =>
        pageEvents(await hubEvents())
          .filter((e: any) => e.ev === "detail" && e.what === "open" && e.why === "pick" && e.t >= since)
          .sort((a: any, b: any) => a.t - b.t),
      (all) => all.length >= 2,
      "the two picks in the event log",
      15_000,
    );
    expect(picks.length).toBe(2);
    const names = (e: any, instance: string, track: string) =>
      (e.keys as string[]).some((k) => k.startsWith(`${instance}|`) && k.includes(track));
    expect(names(picks[0], "band", "B-Main repro #"), JSON.stringify(picks[0])).toBe(true);
    expect(names(picks[1], "master", "Hand1 #"), JSON.stringify(picks[1])).toBe(true);
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
