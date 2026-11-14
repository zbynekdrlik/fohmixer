import type { Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import {
  LiveClient,
  MBC_BASE,
  frames,
  harness,
  hubStatus,
  openSurface,
  resetSectors,
  setSectors,
  token,
  until,
  type SectorsDoc,
} from "./support/live";

// The MBC hub and its instance link (#184, spec §6.1, §6.3; mockup v3): the
// harness runs a second hub on the MBC set (SimLive `mbc`, twelve Tuner
// strips `+G:MBC:<n>`), and the FOH hub reaches `mbc` through its link. The
// MBC hub's page is the four sectors with the twelve MBC strips (its seed:
// LU 6 + RU 6), no rail and no extra tab; on the FOH hub the MBC faders are in
// ARRANGE's pool, never a tab or a VIEWS button, and a fader moved on either
// page moves on the other. With the MBC hub gone the FOH page says MBC is not
// running on the MBC strips only. A down MBC strip's hint names the MBC PC,
// where the MBC Live runs, on either page; a FOH Live's the FOH PC.

const IPAD = { width: 1194, height: 834 };
const UP = 15_000;
const VOLUME = "live_set tracks 0 mixer_device volume";
const MBC_HINT = "Start Live on the MBC PC. The faders come back by themselves.";

/** Every MBC strip on the page is down and shows the MBC PC's hint, read in
 * one task inside the page (#192: the page re-creates a Live's strips when
 * it recomposes their lines). */
async function mbcStripsSayStartOnTheMbcPc(page: Page) {
  const wrong = () =>
    page.evaluate((hint) => {
      const strips = [...document.querySelectorAll('[data-testid="strip"][data-instance="mbc"]')];
      if (strips.length === 0) return ["no strip"];
      return strips
        .filter(
          (s) =>
            s.getAttribute("data-live") !== "off" ||
            s.querySelector('[data-testid="strip-hint"]')?.textContent !== hint,
        )
        .map((s) => `${s.getAttribute("data-label")}: ${s.getAttribute("data-live")}`);
    }, MBC_HINT);
  await expect.poll(wrong, { timeout: UP, message: "every MBC strip down with the MBC PC's hint" }).toEqual([]);
}

/** The FOH hub's sectors with RD paged to an MBC layout of two MBC faders. */
const FOH_DOC: SectorsDoc = {
  rev: 0,
  sectors: {
    lu: [{ id: "lu-1", title: "FOH", strips: [{ instance: "band", label: "Hand2 #" }] }],
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

const sectorsPage = (page: Page) => page.locator('[data-testid="page"][data-sectors="true"]');
const sectorStrips = (page: Page, sector: string) =>
  sectorsPage(page).locator(`.slot[data-sector="${sector}"] [data-testid="strip"]`);
const mbcStrip = (page: Page, label: string) =>
  page.locator(`[data-testid="strip"][data-instance="mbc"][data-label="${label}"]`);

/** The MBC hub's page, its sectors shown. */
async function openMbc(page: Page) {
  await page.setViewportSize(IPAD);
  await openSurface(page, { base: MBC_BASE });
  await expect(sectorsPage(page)).toBeVisible({ timeout: UP });
}

/** A fader's value as it draws it (`data-value`, four decimals). */
async function faderValue(fader: ReturnType<Page["locator"]>): Promise<number> {
  return Number(await fader.getAttribute("data-value"));
}

/** Whether `mbc` is online on the FOH hub. */
async function mbcOnline(bearer: string): Promise<boolean> {
  const status = await hubStatus(bearer);
  return status.instances.some((i: any) => i.name === "mbc" && i.online);
}

test.describe("The MBC hub and its instance link (#184)", () => {
  let live: LiveClient | null = null;
  /** A test stopped the MBC host (its Live): restarted on its own set. */
  let mbcHostStopped = false;

  test.afterEach(async () => {
    // The sectors are put back whatever fails before (later specs read them).
    try {
      if (mbcHostStopped) {
        mbcHostStopped = false;
        // `restart`, never `start`: `start` takes the FOH test site.
        await harness("/host/mbc/restart");
      }
      await harness("/mbc-hub/start").catch(() => undefined);
      const bearer = await token();
      await until(() => mbcOnline(bearer), (on) => on, "mbc back online on the FOH hub", UP);
      live ??= await LiveClient.open();
      await live.cmd("mbc", [{ target: VOLUME, name: "set_prop", args: { prop: "value", value: 0.85 } }]);
    } finally {
      live?.close();
      live = null;
      await resetSectors();
    }
  });

  test("the MBC hub's page shows twelve MBC strips in LU and RU", async ({ page }) => {
    await openMbc(page);
    await expect(sectorStrips(page, "lu")).toHaveCount(6, { timeout: UP });
    await expect(sectorStrips(page, "ru")).toHaveCount(6);
    await expect(sectorStrips(page, "ld")).toHaveCount(0);
    await expect(sectorStrips(page, "rd")).toHaveCount(0);
    const instances = await sectorsPage(page)
      .locator('[data-testid="strip"]')
      .evaluateAll((all) => [...new Set(all.map((s) => s.getAttribute("data-instance")))]);
    expect(instances).toEqual(["mbc"]);
    // The seed's order is the Tuners' places: Hand 1 first in LU.
    await expect(sectorStrips(page, "lu").first()).toHaveAttribute("data-label", "Hand 1");
    // No rail button, no extra tab, no FOH Live in the foot.
    await expect(page.locator('[data-testid="rail"] .btn')).toHaveCount(0);
    for (const id of ["deck-tab", "ableset-tab", "iem-row", "presenter-tab", "view"]) {
      await expect(page.getByTestId(id), id).toHaveCount(0);
    }
    const badges = await page.getByTestId("badge").evaluateAll((all) => all.map((b) => b.getAttribute("data-instance")));
    expect(badges).toEqual(["mbc"]);
  });

  test("MBC faders are in ARRANGE's pool on the FOH page, never a tab or a VIEWS button", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    const bearer = await token();
    await until(() => mbcOnline(bearer), (on) => on, "mbc online on the FOH hub", UP);
    // The composed layout holds the MBC view, pool-only.
    const { layout } = await (await fetch(`${new URL(page.url()).origin}/api/layout`, { headers: { authorization: `Bearer ${bearer}` } })).json();
    const view = layout.pages.find((p: any) => p.id === "view-MBC");
    expect(view?.pool_only, "the MBC group is a pool-only view").toBe(true);
    await expect(page.locator('[data-testid="tab"][data-page="view-MBC"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="view"][data-page="view-MBC"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="strip"][data-instance="mbc"]')).toHaveCount(0);
    // The hub takes an MBC fader into a layout: it is in the pool.
    await setSectors(FOH_DOC);
    live = await LiveClient.open();
    const tick = await live.ask({
      type: "sector_tick",
      id: "t1",
      layout: "lu-1",
      strip: { instance: "mbc", label: "Ambience" },
      on: true,
    });
    expect(tick.type, JSON.stringify(tick)).toBe("result");
    await expect(sectorStrips(page, "rd")).toHaveCount(2, { timeout: UP });
    await expect(sectorStrips(page, "lu").filter({ has: page.locator('[data-label="Ambience"]') })).toHaveCount(0);
    await expect(sectorsPage(page).locator('[data-testid="strip"][data-instance="mbc"][data-label="Ambience"]')).toHaveCount(1);
  });

  test("a fader moved on the MBC page moves on the FOH page", async ({ page, context }) => {
    await setSectors(FOH_DOC);
    live = await LiveClient.open();
    await live.cmd("mbc", [{ target: VOLUME, name: "set_prop", args: { prop: "value", value: 0.5 } }]);
    await page.setViewportSize(IPAD);
    await openSurface(page);
    const foh = mbcStrip(page, "Hand 1");
    await expect(foh).toBeVisible({ timeout: UP });
    const fohFader = foh.getByTestId("fader");
    await expect.poll(() => faderValue(fohFader), { timeout: UP }).toBeCloseTo(0.5, 3);

    const mbc = await context.newPage();
    const errors: string[] = [];
    mbc.on("console", (m) => {
      if (m.type() === "error" || m.type() === "warning") errors.push(m.text());
    });
    mbc.on("pageerror", (e) => errors.push(String(e)));
    await openMbc(mbc);
    const cap = mbcStrip(mbc, "Hand 1").getByTestId("fader-cap");
    const mbcFader = mbcStrip(mbc, "Hand 1").getByTestId("fader");
    await expect.poll(() => faderValue(mbcFader), { timeout: UP }).toBeCloseTo(0.5, 3);
    const box = (await cap.boundingBox())!;
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await mbc.mouse.move(x, y);
    await mbc.mouse.down();
    for (let i = 1; i <= 8; i++) {
      await mbc.mouse.move(x, y - i * 10);
      await frames(mbc, 1);
    }
    await mbc.mouse.up();
    // Live (read through the FOH hub's link) and the FOH page follow.
    const moved = await until(
      async () => Number(await live!.get("mbc", VOLUME, "value")),
      (v) => v > 0.55,
      "the MBC volume up",
    );
    await expect.poll(() => faderValue(fohFader), { timeout: UP }).toBeCloseTo(moved, 3);
    // And the other way: the FOH page's write reaches the MBC page.
    await live.cmd("mbc", [{ target: VOLUME, name: "set_prop", args: { prop: "value", value: 0.3 } }]);
    await expect.poll(() => faderValue(mbcFader), { timeout: UP }).toBeCloseTo(0.3, 3);
    await mbc.close();
    expect(errors).toEqual([]);
  });

  test("with the MBC hub stopped the FOH page shows MBC not running on MBC strips only", async ({ page }) => {
    await setSectors(FOH_DOC);
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(sectorStrips(page, "rd")).toHaveCount(2, { timeout: UP });
    await harness("/mbc-hub/stop");
    for (const strip of await sectorStrips(page, "rd").all()) {
      await expect(strip).toHaveAttribute("data-live", "off", { timeout: UP });
    }
    // The MBC Live runs on the MBC PC: its strips say start it there, on the
    // FOH page too (#184).
    await mbcStripsSayStartOnTheMbcPc(page);
    // The foot names no linked Live (spec C8): the MBC strips say it.
    await expect(page.locator('[data-testid="badge"][data-instance="mbc"]')).toHaveCount(0);
    // The FOH Lives say nothing.
    for (const strip of await sectorStrips(page, "lu").all()) await expect(strip).not.toHaveAttribute("data-live", /.*/);
    for (const instance of ["band", "master"]) {
      await expect(page.locator(`[data-testid="badge"][data-instance="${instance}"]`)).toHaveAttribute("data-live", "on");
    }
    await harness("/mbc-hub/start");
    for (const strip of await sectorStrips(page, "rd").all()) {
      await expect(strip).not.toHaveAttribute("data-live", /.*/, { timeout: UP });
    }
  });

  test("with the MBC Live not running the MBC page's strips say start it on the MBC PC", async ({ page }) => {
    await openMbc(page);
    await expect(sectorStrips(page, "lu")).toHaveCount(6, { timeout: UP });
    await expect(page.getByTestId("strip-hint")).toHaveCount(0);
    mbcHostStopped = true;
    await harness("/host/mbc/stop");
    await mbcStripsSayStartOnTheMbcPc(page);
    const hint = sectorStrips(page, "lu").first().getByTestId("strip-hint");
    await expect(hint).toHaveText(MBC_HINT);
    await expect(hint).toBeVisible();
    // Back by itself once Live runs again: every MBC strip on, no hint.
    await harness("/host/mbc/restart");
    mbcHostStopped = false;
    const stillDown = () =>
      page.evaluate(() => {
        const strips = [...document.querySelectorAll('[data-testid="strip"][data-instance="mbc"]')];
        if (strips.length === 0) return ["no strip"];
        return strips
          .filter((s) => s.hasAttribute("data-live") || s.querySelector('[data-testid="strip-hint"]'))
          .map((s) => `${s.getAttribute("data-label")}: ${s.getAttribute("data-live")}`);
      });
    await expect.poll(stillDown, { timeout: UP, message: "every MBC strip back on, no hint" }).toEqual([]);
    await expect(sectorStrips(page, "lu")).toHaveCount(6);
  });

  for (const size of [IPAD, { width: 844, height: 390 }]) {
    test(`the FOH foot names its own Lives only and keeps its room at ${size.width} × ${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await openSurface(page);
      const bearer = await token();
      await until(() => mbcOnline(bearer), (on) => on, "mbc online on the FOH hub", UP);
      await expect(page.locator('[data-testid="badge"][data-instance="band"]')).toHaveAttribute("data-live", "on", { timeout: UP });
      if (size.width < 1000) {
        await page.getByTestId("thin-burger").dispatchEvent("pointerdown");
        await expect(page.locator('[data-column-open="true"]')).toBeAttached();
      }
      // The foot names this hub's Lives only (spec C8, mockup v3: Band and
      // Master); a linked Live says it is down on its own strips.
      const badges = await page.getByTestId("badge").evaluateAll((all) => all.map((b) => b.getAttribute("data-instance")));
      expect(badges).toEqual(["band", "master"]);
      const foot = (await page.getByTestId("column-foot").boundingBox())!.height;
      expect(foot, "the foot keeps its two rows").toBeLessThanOrEqual(34);
      if (size === IPAD) {
        const rows = await page.locator(".rail-main").evaluate((e) => getComputedStyle(e).gridTemplateRows.split(" "));
        for (const row of rows) expect(row, "every rail row keeps 44 px").toBe("44px");
      }
    });
  }
});
