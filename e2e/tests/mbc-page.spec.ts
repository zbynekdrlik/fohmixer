import type { Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import {
  MBC_BASE,
  clipped,
  hostLine,
  openSurface,
  resetSectors,
  restoreMbcSectors,
  setMbcSectors,
  setSectors,
  type SectorsDoc,
} from "./support/live";

// STREAM LUFS and the MBC page's column (#184 PR C; the approved mockup
// docs/mockups/mbc-layout-v3.html): the frame's `lufs_readout` (`global`,
// `+CTRL:stream` on the MBC set's stream group, sim/fixtures/mbc-site.json
// track 8) reads #151's LUFS device on that track, judged against -14 (green
// within ±1.0 LU), drawn under ARRANGE: 40 px on the MBC hub's page (one Live
// set's faders), 30 px on the FOH column and only while a sector shows MBC
// faders. Every MBC strip with `+LUFS:<target>` shows #151's reading.

const IPAD = { width: 1194, height: 834 };
const UP = 15_000;
/** SimLive's MBC set: Hand 1 is track 0, the stream group (`Stream grp#`) track 8. */
const HAND_1 = 0;
const STREAM = 8;
const HAND_1_TUNER = '"Hand 1" +G:MBC:1';
const WHITE = "rgb(226, 226, 236)";
const GREEN = "rgb(47, 199, 154)";

/** The FOH hub's sectors: RD's first layout FOH faders, its second two MBC faders. */
const FOH_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "FOH", strips: [{ instance: "band", label: "Hand2 #" }] }],
    rd: [
      { id: "rd-1", title: "FX", strips: [{ instance: "master", label: "Hand4 #" }] },
      {
        id: "rd-2",
        title: "MBC",
        strips: [
          { instance: "mbc", label: "Hand 1" },
          { instance: "mbc", label: "Hand 2" },
        ],
      },
    ],
  },
};

const readout = (page: Page) => page.getByTestId("stream-lufs-box");
const sectorsPage = (page: Page) => page.locator('[data-testid="page"][data-sectors="true"]');
const mbcStrip = (page: Page, label: string) =>
  page.locator(`[data-testid="strip"][data-instance="mbc"][data-label="${label}"]`);

/** The MBC hub's page, its sectors shown. */
async function openMbc(page: Page) {
  await page.setViewportSize(IPAD);
  await openSurface(page, { base: MBC_BASE });
  await expect(sectorsPage(page)).toBeVisible({ timeout: UP });
}

/** A device on the stream group sending `S` (tenths). */
async function streamReads(tenths: number) {
  expect(await hostLine("mbc", `lufs track ${STREAM} ${tenths} -100`)).toBe("LUFS 1");
}

/** Nothing of the readout's lines leaves its box. */
async function whole(box: Locator) {
  for (const id of ["stream-lufs-cap", "stream-lufs", "stream-lufs-goal"]) {
    expect(await clipped(box.getByTestId(id)), id).toEqual([]);
  }
}

/** The column's rail rows (`page.css`: 44 px each while it has the room). */
async function railRows(page: Page): Promise<string[]> {
  return page.locator(".rail-main").evaluate((e) => getComputedStyle(e).gridTemplateRows.split(" "));
}

test.describe("STREAM LUFS (#184 PR C)", () => {
  test.afterEach(async () => {
    await hostLine("mbc", `lufs track ${STREAM} off`);
    await hostLine("mbc", `maxdev lufs track ${STREAM} remove`);
    await hostLine("mbc", `lufs track ${HAND_1} off`);
    await hostLine("mbc", `maxdev lufs track ${HAND_1} remove`);
    await hostLine("mbc", `tuner track ${HAND_1} set '${HAND_1_TUNER}'`);
    await resetSectors();
  });

  test("the MBC page shows STREAM LUFS under ARRANGE", async ({ page }) => {
    await openMbc(page);
    const box = readout(page);
    await expect(box).toBeVisible({ timeout: UP });
    await expect(box).toHaveAttribute("data-size", "full");
    await expect(box.getByTestId("stream-lufs-cap")).toHaveText("STREAM LUFS");
    // No device on the stream group yet: #151's problem, no number.
    await expect(box.getByTestId("stream-lufs-goal")).toHaveText("no LUFS meter", { timeout: UP });
    await expect(box.getByTestId("stream-lufs")).toHaveText("");
    expect(await hostLine("mbc", `maxdev lufs track ${STREAM} add rack`)).toBe("MAXDEV 1");
    await streamReads(-144);
    const n = box.getByTestId("stream-lufs");
    await expect(n).toHaveText("−14.4", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "in");
    await expect(n).toHaveCSS("color", GREEN);
    await expect(n).toHaveCSS("font-size", "40px");
    await expect(n).toHaveCSS("font-weight", "700");
    await expect(box.getByTestId("stream-lufs-goal")).toHaveText("target −14");
    await whole(box);
    await streamReads(-160);
    await expect(n).toHaveText("−16.0", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "off");
    await expect(n).toHaveCSS("color", WHITE);
    // ±1.0 LU, both ends included.
    await streamReads(-130);
    await expect(n).toHaveText("−13.0", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "in");
    // Under ARRANGE, inside the column, and it takes no touch.
    const arrange = (await page.getByTestId("arrange").boundingBox())!;
    const at = (await box.boundingBox())!;
    const column = (await page.getByTestId("column").boundingBox())!;
    expect(at.y).toBeGreaterThanOrEqual(arrange.y + arrange.height);
    expect(at.x).toBeGreaterThanOrEqual(column.x);
    expect(at.x + at.width).toBeLessThanOrEqual(column.x + column.width + 0.5);
    await expect(box).toHaveCSS("pointer-events", "none");
  });

  test("each MBC strip shows its own LUFS", async ({ page }) => {
    expect(await hostLine("mbc", `tuner track ${HAND_1} set '${HAND_1_TUNER} +LUFS:-18'`)).toBe("TUNER 1");
    expect(await hostLine("mbc", `maxdev lufs track ${HAND_1} add rack`)).toBe("MAXDEV 1");
    expect(await hostLine("mbc", `lufs track ${HAND_1} -183 -100`)).toBe("LUFS 1");
    await openMbc(page);
    const strip = mbcStrip(page, "Hand 1");
    await expect(strip).toHaveAttribute("data-lufs", "in", { timeout: UP });
    const n = strip.getByTestId("lufs");
    await expect(n).toHaveText("−18.3", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "in");
    await expect(strip.getByTestId("lufs-cap").locator("span")).toHaveText(["LUFS", "IN"]);
    expect(await hostLine("mbc", `lufs track ${HAND_1} -205 -100`)).toBe("LUFS 1");
    await expect(n).toHaveText("−20.5", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "off");
  });

  test("the FOH column shows STREAM LUFS only while a sector shows MBC faders", async ({ page }) => {
    expect(await hostLine("mbc", `maxdev lufs track ${STREAM} add rack`)).toBe("MAXDEV 1");
    await streamReads(-141);
    await setSectors(FOH_DOC);
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(sectorsPage(page)).toBeVisible({ timeout: UP });
    const rd = (id: string) => page.locator(`.lay-ed [data-testid="sector-tab"][data-layout="${id}"]`);
    // RD shows its FOH layout: no readout.
    await expect(rd("rd-1")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator('.slot[data-sector="rd"] [data-testid="strip"][data-instance="master"]')).toHaveCount(1);
    await expect(readout(page)).toHaveCount(0);
    const before = (await railRows(page)).map(parseFloat);
    const zone = page.locator('.slot[data-sector="lu"] [data-testid="strip"] .strip-zone').first();
    const travel = (await zone.boundingBox())!.height;
    // RD paged to the MBC faders: the readout at 30 px, the rail keeps its rows.
    await rd("rd-2").dispatchEvent("pointerdown", { pointerId: 1, isPrimary: true, pointerType: "touch" });
    await expect(page.locator('.slot[data-sector="rd"] [data-testid="strip"][data-instance="mbc"]')).toHaveCount(2, { timeout: UP });
    const box = readout(page);
    await expect(box).toBeVisible();
    await expect(box).toHaveAttribute("data-size", "compact");
    const n = box.getByTestId("stream-lufs");
    await expect(n).toHaveText("−14.1", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "in");
    await expect(n).toHaveCSS("font-size", "30px");
    await whole(box);
    // The FOH column is full (mockup v3): the readout's room comes from the
    // rail's rows (`page.css` lets them give up room down to 24 px, a rule
    // button's second line hides under its need), never from the faders.
    const after = (await railRows(page)).map(parseFloat);
    const took = (await box.boundingBox())!.height;
    console.log(`STREAM LUFS on the FOH column (${test.info().project.name}): ${took} px, rail rows ${before[0]} -> ${after[0]} px`);
    expect(after.length).toBe(before.length);
    for (const row of after) {
      expect(row, "a rail row keeps page.css's floor").toBeGreaterThanOrEqual(24);
      expect(row, "the readout never adds to a row").toBeLessThanOrEqual(before[0] + 0.01);
    }
    await expect(page.getByTestId("stage-mini")).toHaveCount(1);
    expect((await zone.boundingBox())!.height, "the faders keep their travel").toBeCloseTo(travel, 1);
    // The readout lies above the rail, overlapping nothing of it.
    const rail = (await page.getByTestId("rail").boundingBox())!;
    const at = (await box.boundingBox())!;
    expect(at.y + at.height).toBeLessThanOrEqual(rail.y + 0.5);
    // Paged back: gone.
    await rd("rd-1").dispatchEvent("pointerdown", { pointerId: 2, isPrimary: true, pointerType: "touch" });
    await expect(readout(page)).toHaveCount(0, { timeout: UP });
  });

  test("the MBC page's column matches the mockup", async ({ page }) => {
    await openMbc(page);
    const column = page.getByTestId("column");
    for (const id of ["sector-layouts-title", "arrange", "stream-lufs-box", "login-line", "logout", "version"]) {
      await expect(column.getByTestId(id), id).toBeVisible({ timeout: UP });
    }
    await expect(page.getByTestId("login-line")).toHaveText("logged in: ENGINEER");
    // Top to bottom as mockup v3: LAYOUTS, ARRANGE, STREAM LUFS, the login, LOG OUT.
    const ys: number[] = [];
    for (const id of ["sector-layouts-title", "arrange", "stream-lufs-box", "login-line", "logout"]) {
      ys.push((await column.getByTestId(id).boundingBox())!.y);
    }
    expect([...ys].sort((a, b) => a - b)).toEqual(ys);
    const badges = await page.getByTestId("badge").evaluateAll((all) => all.map((b) => b.getAttribute("data-instance")));
    expect(badges).toEqual(["mbc"]);
    for (const id of ["deck-tab", "ableset-tab", "iem-row", "presenter-tab", "stage-mini"]) {
      await expect(page.getByTestId(id), id).toHaveCount(0);
    }
    await expect(page.locator('[data-testid="rail"] .btn')).toHaveCount(0);
    for (const id of ["sector-layouts-title", "stream-lufs-cap", "stream-lufs-goal", "login-line", "logout"]) {
      expect(await clipped(column.getByTestId(id)), id).toEqual([]);
    }
  });
});

// STREAM LUFS on the page without sectors (#184, the main session's design
// on the ticket): the MBC hub runs with its sectors off (the owner's rule
// "fader height first"), so the rows page draws the readout in its column,
// under the tabs, by the same rule: always on a page of one Live set's
// faders, never on the FOH rows page.
test.describe("STREAM LUFS on the page without sectors (#184)", () => {
  test.afterEach(async () => {
    await hostLine("mbc", `lufs track ${STREAM} off`);
    await hostLine("mbc", `maxdev lufs track ${STREAM} remove`);
    await restoreMbcSectors();
  });

  test("the MBC rows page shows STREAM LUFS in its column, the faders keep their travel", async ({ page }) => {
    await setMbcSectors({ rev: 0, sectors: {} });
    expect(await hostLine("mbc", `maxdev lufs track ${STREAM} add rack`)).toBe("MAXDEV 1");
    await streamReads(-142);
    await page.setViewportSize(IPAD);
    await openSurface(page, { base: MBC_BASE });
    const rows = page.locator('[data-testid="page"]:not([data-sectors])');
    await expect(rows).toBeVisible({ timeout: UP });
    await expect(sectorsPage(page)).toHaveCount(0);
    await expect(mbcStrip(page, "Hand 1")).toBeVisible({ timeout: UP });
    const box = readout(page);
    await expect(box).toBeVisible({ timeout: UP });
    await expect(box).toHaveCount(1);
    await expect(box).toHaveAttribute("data-size", "full");
    await expect(box.getByTestId("stream-lufs-cap")).toHaveText("STREAM LUFS");
    const n = box.getByTestId("stream-lufs");
    await expect(n).toHaveText("−14.2", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "in");
    await expect(n).toHaveCSS("color", GREEN);
    await expect(n).toHaveCSS("font-size", "40px");
    await expect(box.getByTestId("stream-lufs-goal")).toHaveText("target −14");
    await whole(box);
    await expect(box).toHaveCSS("pointer-events", "none");
    // In the column, above the rail and the foot.
    const column = page.getByTestId("column");
    const at = (await box.boundingBox())!;
    const col = (await column.boundingBox())!;
    expect(at.x).toBeGreaterThanOrEqual(col.x);
    expect(at.x + at.width).toBeLessThanOrEqual(col.x + col.width + 0.5);
    const rail = (await page.getByTestId("rail").boundingBox())!;
    expect(at.y + at.height).toBeLessThanOrEqual(rail.y + 0.5);
    // Nothing of the column is clipped: it fits its height, the foot whole
    // (its lines as "the MBC page's column matches the mockup" checks them;
    // the version's 10 px line is under its face's content area on every page).
    const fits = await column.evaluate((e) => e.scrollHeight <= e.clientHeight + 0.5);
    expect(fits, "the column fits its height").toBe(true);
    for (const id of ["login-line", "logout"]) {
      await expect(column.getByTestId(id), id).toBeVisible();
      expect(await clipped(column.getByTestId(id)), id).toEqual([]);
    }
    await expect(column.getByTestId("version")).toBeInViewport({ ratio: 1 });
    // The faders' travel is the page's without the readout.
    const zone = mbcStrip(page, "Hand 1").locator(".strip-zone");
    const travel = (await zone.boundingBox())!.height;
    await box.evaluate((e) => ((e as HTMLElement).style.display = "none"));
    const without = (await zone.boundingBox())!.height;
    await box.evaluate((e) => ((e as HTMLElement).style.display = ""));
    console.log(`STREAM LUFS on the MBC rows page (${test.info().project.name}): ${at.height} px, travel ${travel} px (${without} px without it)`);
    expect(travel, "the faders keep their travel").toBeCloseTo(without, 1);
    // A new reading follows.
    await streamReads(-160);
    await expect(n).toHaveText("−16.0", { timeout: UP });
    await expect(n).toHaveAttribute("data-l", "off");
  });

  test("the FOH rows page draws no STREAM LUFS", async ({ page }) => {
    expect(await hostLine("mbc", `maxdev lufs track ${STREAM} add rack`)).toBe("MAXDEV 1");
    await streamReads(-141);
    await resetSectors();
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(page.locator('[data-testid="page"]:not([data-sectors])')).toBeVisible({ timeout: UP });
    await expect(page.locator('[data-testid="strip"][data-instance="band"]').first()).toBeVisible({ timeout: UP });
    await expect(page.getByTestId("rail")).toBeVisible();
    await expect(readout(page)).toHaveCount(0);
  });
});
