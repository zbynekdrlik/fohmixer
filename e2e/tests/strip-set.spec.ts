import type { Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { MBC_BASE, frames, openSurface, resetSectors, setSectors, type SectorsDoc } from "./support/live";
import { SECTORS, band, master, openSectors, stripsOf } from "./support/sectors";

// Every strip says which Live set it belongs to (#211, the owner's approved
// docs/mockups/strip-instance-v1.html): BAND, MASTER or MBC, calm and dim
// (`--faint`) vertical text in its info column (`strip-col`), under the
// LUFS row, in the side box with the group's name (`strip-side`,
// group.css): `strip-set`, or the group's name itself when that already
// says the strip's set (behave::label::set_text), never both, and no
// strip draws the old #63 instance pill (`strip-instance`) any more.
// Nothing lit, no pill, every strip's fader travel the full 322 px.
// On the rows page (titled, untitled and mixed groups) and the FOH sectors
// page (a band, a master, a mixed and an MBC layout: the master `Hand1 #`
// and the MBC `Hand 1`, the owner's case of two Lives' faders named alike,
// each with its own word) and the MBC hub's page, at the iPad's
// 1194 x 834 and both phones: the set seen whole or yielded whole
// (outside the side box's clip), never over the LUFS row, the group's
// name or SEL, never clipped.

const SIZES = [
  { width: 1194, height: 834 },
  { width: 844, height: 390 },
  { width: 390, height: 844 },
];

/** The FOH sectors: band only, master only, mixed, MBC only. */
const SET_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "STAGE", strips: [band("Vocal 1 repro#"), band("Keys 1")] }],
    ru: [{ id: "ru-1", title: "HANDS", strips: [master("Hand1 #"), master("Hand4 #")] }],
    ld: [{ id: "ld-1", title: "EFFECTS", strips: [master("A-Echo"), band("A-Reverb #"), master("A-Reverb #")] }],
    rd: [
      {
        id: "rd-1",
        title: "MBC",
        strips: [
          { instance: "mbc", label: "Hand 1" },
          { instance: "mbc", label: "Hand 2" },
        ],
      },
    ],
  },
};

type Seen = {
  problems: string[];
  /** Per instance: the strips on screen that say their set, and how (`set` or `side`). */
  says: Record<string, { set: number; side: number }>;
  /** Sets that yielded (a zone too short beside the group's name). */
  yielded: number;
  /** Every strip's fader travel on screen (px). */
  travels: number[];
};

/**
 * Every strip on screen, read in one task inside the page (#198): what
 * says its Live set, and where the set mark sits in its info column.
 */
function readSets(page: Page, scope: string): Promise<Seen> {
  return page.evaluate((scope) => {
    const probe = document.createElement("i");
    probe.style.color = "var(--faint)";
    document.body.append(probe);
    const faint = getComputedStyle(probe).color;
    probe.remove();
    const out: Seen = { problems: [], says: {}, yielded: 0, travels: [] };
    const at = (n: number) => n.toFixed(2);
    for (const s of document.querySelectorAll(`${scope} [data-testid="strip"]`)) {
      const strip = s.getBoundingClientRect();
      if (strip.width === 0 || strip.right <= 0 || strip.left >= innerWidth || getComputedStyle(s).visibility !== "visible") continue;
      const inst = s.getAttribute("data-instance") ?? "";
      const name = `${inst}|${s.getAttribute("data-track")}`;
      const fail = (what: string) => out.problems.push(`${name}: ${what}`);
      const want = inst.toUpperCase();
      const col = s.querySelector('[data-testid="strip-col"]')!;
      const cb = col.getBoundingClientRect();
      const set = col.querySelector('[data-testid="strip-set"]') as HTMLElement | null;
      const side = col.querySelector('[data-testid="group-side"]') as HTMLElement | null;
      const sideSays = !!side && (side.textContent ?? "").trim().toUpperCase() === want;
      const tally = (out.says[inst] ??= { set: 0, side: 0 });
      if (set && sideSays) fail("says its set twice");
      if (s.querySelector('[data-testid="strip-instance"], .strip-tag')) fail("draws an instance pill");
      if (!set && !sideSays) fail("says no set");
      if (sideSays) tally.side++;
      const fader = s.querySelector('[data-testid="fader"]');
      if (fader) out.travels.push(fader.getBoundingClientRect().height);
      if (!set) continue;
      const cs = getComputedStyle(set);
      if (set.textContent !== want) fail(`the set reads ${set.textContent}`);
      if (set.getAttribute("data-instance") !== inst) fail(`the set's instance is ${set.getAttribute("data-instance")}`);
      if (cs.color !== faint) fail(`the set is ${cs.color}, not --faint ${faint}`);
      if (cs.backgroundColor !== "rgba(0, 0, 0, 0)" || cs.borderTopStyle !== "none" || cs.boxShadow !== "none") fail("the set is drawn as a shape");
      if (cs.pointerEvents !== "none") fail("the set takes touches");
      const sb = set.getBoundingClientRect();
      const box = col.querySelector('[data-testid="strip-side"]')!.getBoundingClientRect();
      if (sb.left >= box.right - 0.01) {
        out.yielded++;
        continue;
      }
      tally.set++;
      if (sb.right > box.right + 0.5) fail("the set is half drawn");
      if (sb.left < cb.left - 0.5 || sb.right > cb.right + 0.5 || sb.top < cb.top - 0.5 || sb.bottom > cb.bottom + 0.5) fail("the set is outside its info column");
      if (set.scrollWidth > set.clientWidth || set.scrollHeight > set.clientHeight) fail("the set is clipped");
      const lufs = col.querySelector('[data-testid="lufs"]')!.getBoundingClientRect();
      if (lufs.height > 0 && sb.top < lufs.bottom - 0.5) fail(`the set is over the LUFS row (${at(sb.top - cb.top)} < ${at(lufs.bottom - cb.top)})`);
      const cap = col.querySelector('[data-testid="lufs-cap"]')!.getBoundingClientRect();
      if (cap.height > 0 && sb.top < cap.bottom - 0.5) fail("the set is over the LUFS caption");
      if (side) {
        const g = side.getBoundingClientRect();
        if (sb.top < g.bottom - 0.01 && sb.bottom > g.top + 0.01) fail("the set overlaps the group's name");
      }
      const sel = s.querySelector('[data-testid="detail-key"]')!.getBoundingClientRect();
      if (sb.bottom > sel.top + 0.5) fail(`the set reaches SEL (${at(sb.bottom - cb.top)} > ${at(sel.top - cb.top)})`);
    }
    return out;
  }, scope);
}

/** Every strip of `scope` on screen says its set right at each size; returns the iPad's read. */
async function everySize(page: Page, scope: string): Promise<Seen> {
  let ipad: Seen | null = null;
  for (const size of SIZES) {
    await page.setViewportSize(size);
    await frames(page);
    await expect
      .poll(async () => (await readSets(page, scope)).problems, { message: `${size.width}x${size.height}` })
      .toEqual([]);
    const seen = await readSets(page, scope);
    const strips = Object.values(seen.says).reduce((n, t) => n + t.set + t.side, 0) + seen.yielded;
    console.log(
      `strip set (${test.info().project.name}, ${size.width}x${size.height}): ${JSON.stringify(seen.says)} yielded ${seen.yielded} of ${strips}`,
    );
    expect(strips, `${size.width}x${size.height}: strips on screen`).toBeGreaterThan(0);
    if (size.width === 1194) ipad = seen;
  }
  return ipad!;
}

test.describe("Every strip says its Live set (#211)", () => {
  test.afterEach(async () => {
    await resetSectors();
  });

  test("on the rows page: a titled group's strips under their name, an untitled one's by its name, a mixed one's each", async ({ page }) => {
    await page.setViewportSize(SIZES[0]);
    await openSurface(page);
    const ipad = await everySize(page, '[data-testid="page"]:not([data-sectors="true"])');
    // At the iPad's size every zone holds the set: none yielded; band and
    // master both say it by the mark, and an untitled shared group's first
    // strip by its name.
    expect(ipad.yielded).toBe(0);
    expect(ipad.says.band?.set).toBeGreaterThan(0);
    expect(ipad.says.master?.set).toBeGreaterThan(0);
    expect((ipad.says.band?.side ?? 0) + (ipad.says.master?.side ?? 0)).toBeGreaterThan(0);
    // The v8 travel (strip.spec.ts' floor) on every strip, a mixed group's
    // included: no instance pill takes its ~16 px any more.
    await page.setViewportSize(SIZES[0]);
    await frames(page);
    const travels = await page.evaluate(() =>
      [...document.querySelectorAll('[data-testid="strip"]')]
        .filter((s) => s.querySelector('[data-testid="fader"]'))
        .map((s) => s.querySelector('[data-testid="fader"]')!.getBoundingClientRect().height)
        .filter((h) => h > 0),
    );
    expect(travels.length).toBeGreaterThan(0);
    for (const h of travels) expect(h).toBeGreaterThanOrEqual(322 - 0.5);
  });

  test("on the FOH sectors page: a band, a master, a mixed and an MBC layout, a master and an MBC fader named alike told apart", async ({ page }) => {
    await setSectors(SET_DOC);
    await page.setViewportSize(SIZES[0]);
    await openSectors(page);
    await expect(page.locator(`${SECTORS} .slot[data-sector="rd"] [data-testid="strip"][data-instance="mbc"]`)).toHaveCount(2);
    await expect.poll(async () => (await stripsOf(page, "ld")).length).toBe(3);
    const ipad = await everySize(page, SECTORS);
    expect(ipad.yielded).toBe(0);
    // Each Live set says itself on the iPad: a one-set layout by the mark,
    // the mixed layout by the strips' side texts (sector_side).
    expect(ipad.says.band?.set).toBe(2);
    expect(ipad.says.master?.set).toBe(2);
    expect(ipad.says.mbc?.set).toBe(2);
    expect(ipad.says.band?.side).toBe(1);
    expect(ipad.says.master?.side).toBe(2);
    // The owner's case: the master `Hand1 #` and the MBC `Hand 1`, faders
    // of two Lives named alike, each with its own word.
    const strips = `${SECTORS} [data-testid="strip"]`;
    await expect(page.locator(`${strips}[data-instance="mbc"][data-label="Hand 1"]`).getByTestId("strip-set")).toHaveText("MBC");
    await expect(page.locator(`${strips}[data-instance="master"][data-track="Hand1 #"]`).getByTestId("strip-set")).toHaveText("MASTER");
    // The travel unchanged: every strip of the four sectors (sectors.spec.ts' floor).
    await page.setViewportSize(SIZES[0]);
    await frames(page);
    const travels = (await readSets(page, SECTORS)).travels;
    expect(travels.length).toBe(9);
    for (const h of travels) expect(h).toBeGreaterThanOrEqual(322 - 0.5);
  });

  test("on the MBC hub's page every strip says MBC", async ({ page }) => {
    await page.setViewportSize(SIZES[0]);
    await openSurface(page, { base: MBC_BASE });
    await expect(page.locator('[data-testid="strip"][data-instance="mbc"]').first()).toBeVisible();
    const ipad = await everySize(page, '[data-testid="page"]');
    expect(Object.keys(ipad.says)).toEqual(["mbc"]);
    expect(ipad.says.mbc.set).toBeGreaterThan(0);
    expect(ipad.yielded).toBe(0);
  });
});
