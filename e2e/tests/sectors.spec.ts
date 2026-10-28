import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  dispatchPointer,
  frames,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  pageEvents,
  resetSectors,
  setSectors,
  track,
  until,
  volume,
  type SectorsDoc,
  type StripRef,
} from "./support/live";

// The fader sectors (#148; spec docs/superpowers/specs/2026-10-28-sectors-design.md,
// the approved looks docs/mockups/sectors-v1.html and sectors-v2.html): with
// a sectors document that places faders, the FOH page is four sectors around
// the column (lu ru over ld rd), each paging through its own layouts in the
// column's LAYOUTS block; a strip's channel detail ticks the layouts it is
// in, ARRANGE reorders a sector's strips and renames, adds and removes its
// layouts (saved as one change), ↶ undoes the last change. The hub owns
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

const sectorsPage = (page: Page) => page.locator('[data-testid="page"][data-sectors="true"]');

/** A sector's slots on the sectors page, in their order. */
const slots = (page: Page, sector: string) => sectorsPage(page).locator(`.slot[data-sector="${sector}"]`);

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

    test("the fader travel is 322 px at 1194x834 in all four sectors, a mixed layout included", async ({ page }) => {
      await setSectors(MIXED_DOC);
      await openSectors(page);
      await expect.poll(async () => (await stripsOf(page, "ld")).length).toBe(3);
      await frames(page);
      const travels = await sectorsPage(page)
        .locator('.slot[data-sector] [data-testid="strip"]')
        .evaluateAll((all) =>
          all
            .filter((s) => s.querySelector('[data-testid="fader"]'))
            .map((s) => ({
              sector: s.closest(".slot")!.getAttribute("data-sector"),
              name: `${s.getAttribute("data-instance")}|${s.getAttribute("data-track")}`,
              h: s.querySelector('[data-testid="fader"]')!.getBoundingClientRect().height,
            })),
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
      const sides = await slots(page, "ld")
        .locator('[data-testid="strip"]')
        .evaluateAll((all) =>
          all.map((s) => {
            const side = s.querySelector('[data-testid="strip-col"] [data-testid="group-side"]') as HTMLElement | null;
            return side
              ? { text: side.textContent, color: getComputedStyle(side).color, events: getComputedStyle(side).pointerEvents }
              : null;
          }),
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
        const plain = await otherGaps.evaluateAll((all) =>
          all.map((e) => ({ outline: getComputedStyle(e).outlineStyle, border: getComputedStyle(e).borderTopStyle })),
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
        const travels = await sectorsPage(page)
          .locator('.slot[data-sector] [data-testid="fader"]')
          .evaluateAll((all) => all.map((f) => f.getBoundingClientRect().height));
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

        // One ↶ brings all of it back; the page's pick of a gone layout
        // falls back to its sector's first.
        const undo = page.getByTestId("sector-undo");
        await expect(undo).toHaveAttribute("aria-disabled", "false");
        await undo.click();
        await expect.poll(() => titles(page, "lu")).toEqual(["STAGE", "ALT"]);
        await expect.poll(() => titles(page, "ld")).toEqual(["HANDS", "FX"]);
        await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
        await expect(tabOf(page, "lu-1")).toHaveAttribute("aria-pressed", "true");
        const all = await until(
          () => sectorSteps(since),
          (s) => s.includes("undo"),
          "the hub's undo record",
        );
        expect(all).toEqual(["edit", "undo"]);
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

  test("a tick in SEL shows the strip in that layout and a second page sees it", async ({ page, context }) => {
    await setSectors(DOC);
    const other = await context.newPage();
    const otherConsole = watchConsole(other);
    await openSectors(other);
    await openSectors(page);
    const since = Date.now();
    const detail = await openDetail(page, "Hand4 #", "master");
    const layouts = detail.getByTestId("detail-layouts");
    await expect(layouts).toBeVisible();
    // Every layout of every sector, Hand4 #'s own ticked.
    const tick = (id: string) => layouts.locator(`[data-testid="sector-tick"][data-layout="${id}"]`);
    await expect(layouts.getByTestId("sector-tick")).toHaveCount(6);
    await expect(tick("ld-1")).toHaveAttribute("aria-checked", "true");
    await expect(tick("lu-1")).toHaveAttribute("aria-checked", "false");
    await expect(detail.getByTestId("detail-steps")).toBeVisible();

    await tick("lu-1").click();
    await expect(tick("lu-1")).toHaveAttribute("aria-checked", "true");
    await expect(tick("lu-1")).toHaveAttribute("data-refused", "false");
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

    expect(otherConsole, "the second page's console").toEqual([]);
    await other.close();
  });

  test("a full layout reads FULL and a tick is refused", async ({ page }) => {
    await setSectors(FULL_DOC);
    await openSectors(page);
    await expect.poll(async () => (await stripsOf(page, "lu")).length).toBe(6);
    const detail = await openDetail(page, "Hand2 #", "master");
    const tick = detail.locator('[data-testid="sector-tick"][data-layout="lu-1"]');
    await expect(tick).toHaveAttribute("aria-checked", "false");
    await expect(tick).toBeDisabled();
    await expect(tick).toContainText("FULL");
    // Its own layout is not full for it: it is in it already.
    const own = detail.locator('[data-testid="sector-tick"][data-layout="ld-1"]');
    await expect(own).toHaveAttribute("aria-checked", "true");
    await expect(own).toBeEnabled();

    // The hub refuses the tick itself (another client asking past the page).
    const live = await LiveClient.open();
    try {
      const before = (await hubStatus()).sectors.rev;
      const answer = await live.ask({ type: "sector_tick", layout: "lu-1", strip: master("Hand2 #"), on: true });
      expect(answer.type).toBe("error");
      expect(answer.message).toBe("FULL");
      expect((await hubStatus()).sectors.rev).toBe(before);
      await expect.poll(async () => (await stripsOf(page, "lu")).length).toBe(6);
    } finally {
      live.close();
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

  test("the detail's ◀ ▶ step to the pool's neighbouring strips", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    const since = await page.evaluate(() => performance.timeOrigin + performance.now());
    // The fixture's pool in document order: … band Keys 1, master Hand1 #
    // (the FOH page's OTHERS sub-page), band B-Main repro # (a return) …
    const detail = await openDetail(page, "Hand1 #", "master");
    await expect(detail.getByTestId("detail-steps")).toBeVisible();
    await detail.getByTestId("detail-next").click();
    await expect(detail).toHaveAttribute("data-track", "B-Main repro #");
    await expect(detail).toHaveAttribute("data-instance", "band");
    await detail.getByTestId("detail-prev").click();
    await expect(detail).toHaveAttribute("data-track", "Hand1 #");
    await expect(detail).toHaveAttribute("data-instance", "master");
    // Each step is an open recorded with why "step" and the strip's keys.
    const steps = await until(
      async () =>
        pageEvents(await hubEvents())
          .filter((e: any) => e.ev === "detail" && e.what === "open" && e.why === "step" && e.t >= since)
          .sort((a: any, b: any) => a.t - b.t),
      (all) => all.length >= 2,
      "the two steps in the event log",
      15_000,
    );
    expect(steps.length).toBe(2);
    const names = (e: any, instance: string, track: string) =>
      (e.keys as string[]).some((k) => k.startsWith(`${instance}|`) && k.includes(track));
    expect(names(steps[0], "band", "B-Main repro #"), JSON.stringify(steps[0])).toBe(true);
    expect(names(steps[1], "master", "Hand1 #"), JSON.stringify(steps[1])).toBe(true);
    expect(typeof steps[0].pointer).toBe("number");
  });

  test("undo restores the last change and a second undo redoes it", async ({ page }) => {
    await setSectors(DOC);
    await openSectors(page);
    await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
    const live = await LiveClient.open();
    try {
      const since = Date.now();
      // A change from another client: master Hand4 # into STAGE.
      const answer = await live.ask({ type: "sector_tick", layout: "lu-1", strip: master("Hand4 #"), on: true });
      expect(answer.type, JSON.stringify(answer)).toBe("result");
      const ticked = [name(VOCAL), name(KEYS), "master|Hand4 #"];
      await expect.poll(() => stripsOf(page, "lu")).toEqual(ticked);

      const undo = page.getByTestId("sector-undo");
      await expect(undo).toHaveAttribute("aria-disabled", "false");
      await undo.click();
      await expect.poll(() => stripsOf(page, "lu")).toEqual([name(VOCAL), name(KEYS)]);
      // The second ↶ redoes it.
      await expect(undo).toHaveAttribute("aria-disabled", "false");
      await undo.click();
      await expect.poll(() => stripsOf(page, "lu")).toEqual(ticked);
      await expect(undo).not.toHaveClass(/failed/);
      const steps = await until(
        () => sectorSteps(since),
        (s) => s.filter((w) => w === "undo").length === 2,
        "the hub's two undo records",
      );
      expect(steps).toEqual(["tick", "undo", "undo"]);
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
    await expect(page.getByTestId("sector-undo")).toHaveCount(0);
    const pager = page.locator('[data-testid="tabbar"][data-level="1"]');
    await expect(pager).toBeVisible();
    await expect(pager.locator('[data-testid="tab"][data-page="stage"]')).toBeVisible();
    await expect(pager.locator('[data-testid="tab"][data-page="others"]')).toBeVisible();
    await expect(page.locator('[data-testid="strip"][data-track="Vocal 1 repro#"][data-instance="band"]')).toBeVisible();
    // The detail shows no LAYOUTS and no ◀ ▶.
    const detail = await openDetail(page, "Vocal 1 repro#");
    await expect(detail.getByTestId("detail-layouts")).toHaveCount(0);
    await expect(detail.getByTestId("detail-steps")).toHaveCount(0);
  });
});
