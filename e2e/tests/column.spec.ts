import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, clipped, harness, openSurface, ready, selectPage, strip, track, until } from "./support/live";

// The control column (#63, the owner's design of 2026-10-08): every screen is
// the page's rows cut in two by one column in the middle, which holds the
// status, the tabs, the arrows, the rail and TechAlert; nothing sits above
// the faders. Each row is a line while every line keeps 340 px; a lower
// screen shows one line of both rows, and a line that does not fit shows
// its pinned strips and a window the column's arrows move. One layout for
// every screen: these tests run both projects at the tablet's and desktop's
// own size and at a phone's.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fader's travel of `name`'s strip (px). */
async function travel(page: Page, name: string, instance = "band"): Promise<number> {
  const fader = strip(page, name, instance).getByTestId("fader");
  await ready(fader);
  return (await fader.boundingBox())!.height;
}

/** Nothing of a strip above its name button: the button is the strip's first part, 30 px high. */
async function headFirst(page: Page): Promise<string[]> {
  return page.getByTestId("strip").evaluateAll((strips) =>
    strips.flatMap((s) => {
      const head = s.firstElementChild;
      const mute = s.querySelector(".mute");
      if (!head || !head.classList.contains("strip-head") || !mute) return [`${s.getAttribute("data-track")}: no head first`];
      const h = mute.getBoundingClientRect().height;
      return Math.abs(h - 30) > 0.5 ? [`${s.getAttribute("data-track")}: a ${h} px name button`] : [];
    }),
  );
}

test.describe("At the tablet's and the desktop's own size", () => {
  test("the column stands in the middle, each row is a line of its own and nothing sits above the faders", async ({ page }) => {
    await openSurface(page);
    const viewport = page.viewportSize()!;
    const column = page.getByTestId("column");
    const box = (await column.boundingBox())!;
    expect(Math.abs(box.x + box.width / 2 - viewport.width / 2), "the column's centre (px off the screen's)").toBeLessThan(1);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
    // The column holds the status, the tabs, the rail and TechAlert; there is no top bar.
    for (const id of ["version", "dropouts", "tabbar", "rail", "alert-toggle"]) {
      await expect(column.getByTestId(id).first(), id).toBeVisible();
    }
    await expect(page.locator(".topbar")).toHaveCount(0);
    // Two lines, each cut by the column; no arrows.
    await expect(page.getByTestId("half")).toHaveCount(4);
    await expect(page.getByTestId("shift")).toHaveCount(0);
    expect(await headFirst(page)).toEqual([]);
    // The faders get the height: longer than TouchOSC's two rows (281 px) on
    // the tablet, and 225 px on the desktop's 720 px.
    const least = viewport.height >= 800 ? 285 : 225;
    expect(await travel(page, "Hand2 #"), "the fader's travel (px)").toBeGreaterThan(least);
    for (const tab of await page.getByTestId("tab").all()) {
      expect(await clipped(tab), `tab ${await tab.getAttribute("data-page")}`).toEqual([]);
    }
  });

  test("a pinned strip keeps its place and its values when the pager shows another sub-page", async ({ page }) => {
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    const stage = changed.pages[1].rows[0].sections[0].pages[0].sections[0];
    const keys = stage.controls.find((c: any) => c.binding.anchor.name === "Keys 1");
    keys.pinned = true;
    await openSurface(page);
    try {
      await harness("/hub/layout", { layout: changed });
      await selectPage(page, "stage");
      const pinned = strip(page, "Keys 1");
      await expect(pinned).toBeVisible({ timeout: 10_000 });
      const before = (await pinned.boundingBox())!;
      await selectPage(page, "others");
      await expect(strip(page, "Hand1 #", "master")).toBeVisible();
      await expect(strip(page, "Vocal 1 repro#")).toHaveCount(0);
      await expect(pinned).toBeVisible();
      const after = (await pinned.boundingBox())!;
      expect(Math.abs(after.x - before.x), "the pinned strip's place").toBeLessThan(0.5);
      // Subscribed on OTHERS too: bound, with Live's dB.
      await expect(pinned.getByTestId("status")).toHaveAttribute("data-state", "bound");
      await expect(pinned.getByTestId("db")).not.toHaveText("");
    } finally {
      await selectPage(page, "stage");
      await harness("/hub/layout/reset");
    }
  });
});

test.describe("On a phone on its side", () => {
  test.use({ viewport: { width: 844, height: 390 } });

  test("both rows share one line with the column in the middle; its arrows move the strips", async ({ page }) => {
    await openSurface(page);
    const column = (await page.getByTestId("column").boundingBox())!;
    expect(Math.abs(column.x + column.width / 2 - 422), "the column's centre (px off the screen's)").toBeLessThan(1);
    expect(column.y + column.height).toBeLessThanOrEqual(390);
    await expect(page.getByTestId("half")).toHaveCount(2);
    await expect(page.locator(".topbar")).toHaveCount(0);
    expect(await headFirst(page)).toEqual([]);
    // The fader runs most of the screen's height (0.1.0-dev.43's screen bar
    // and overview left it 157 px).
    expect(await travel(page, "Vocal 1 repro#"), "the fader's travel (px)").toBeGreaterThan(250);
    // The line does not fit: arrows, the first window first.
    const shift = page.getByTestId("shift");
    await expect(shift).toHaveCount(1);
    const where = shift.getByTestId("shift-where");
    await expect(where).toHaveText("1–9/10");
    await expect(strip(page, "A-Echo", "master")).toHaveCount(0);
    // ▶ until the last window: the lower row's last strips show.
    for (let i = 0; i < 6; i++) await shift.getByTestId("shift-next").dispatchEvent("pointerdown");
    await expect(shift.getByTestId("shift-next")).toHaveClass(/\bend\b/);
    await expect(strip(page, "A-Echo", "master")).toBeVisible();
    await expect(strip(page, "Vocal 1 repro#")).toHaveCount(0);
    // ◀ back to the start.
    for (let i = 0; i < 6; i++) await shift.getByTestId("shift-back").dispatchEvent("pointerdown");
    await expect(shift.getByTestId("shift-back")).toHaveClass(/\bend\b/);
    await expect(strip(page, "Vocal 1 repro#")).toBeVisible();
    // The rail's buttons in the column, their words whole.
    const rail = page.getByTestId("rail");
    await expect(rail.getByTestId("solo").first()).toBeVisible();
    for (const button of await rail.locator(".btn").all()) {
      expect(await clipped(button), `rail button ${await button.textContent()}`).toEqual([]);
      const box = (await button.boundingBox())!;
      expect(box.y + box.height, `rail button ${await button.textContent()} on screen`).toBeLessThanOrEqual(390);
    }
  });

  test("TechAlert's wash blinks over the whole screen from the column", async ({ page }) => {
    // The wash lives in the rail's foot (AlertView), inside the column: no
    // ancestor may clip it or put it under the strips.
    const alert = track("TechAlert #");
    const live = await LiveClient.open();
    try {
      await live.set("band", alert, "mute", true);
      await openSurface(page);
      const overlay = page.getByTestId("alert");
      await live.set("band", alert, "mute", false);
      await expect(overlay).toHaveAttribute("data-active", "true");
      // Sampled in the page every 15 ms for 900 ms (the blink is 300 ms).
      const drawn = await overlay.evaluate(async (el) => {
        let seen = false;
        const end = performance.now() + 900;
        while (performance.now() < end) {
          const box = el.getBoundingClientRect();
          const on = el.getAttribute("data-visible") === "true" && getComputedStyle(el).visibility === "visible";
          if (on && box.width === window.innerWidth && box.height === window.innerHeight) seen = true;
          await new Promise((done) => setTimeout(done, 15));
        }
        return seen;
      });
      expect(drawn, "the wash drawn over the whole screen").toBe(true);
      const covered = await overlay.evaluate((el) => {
        const why: string[] = [];
        for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
          const css = getComputedStyle(a);
          if (Number(css.opacity) < 1) why.push(`${a.className} opacity ${css.opacity}`);
          if (css.zIndex !== "auto" && a.closest(".column")) why.push(`${a.className} z-index ${css.zIndex}`);
          if (css.transform !== "none" || css.filter !== "none" || css.contain !== "none") why.push(`${a.className} transform, filter or contain`);
        }
        return why;
      });
      expect(covered, "the wash's ancestors").toEqual([]);
    } finally {
      await live.set("band", alert, "mute", true);
      live.close();
    }
  });
});

test.describe("On a phone upright", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("each row is a line of two strips a side, the column narrow, its tabs and buttons whole", async ({ page }) => {
    await openSurface(page);
    const column = (await page.getByTestId("column").boundingBox())!;
    expect(Math.abs(column.x + column.width / 2 - 195), "the column's centre (px off the screen's)").toBeLessThan(1);
    await expect(page.getByTestId("half")).toHaveCount(4);
    await expect(page.getByTestId("shift")).toHaveCount(2);
    for (const half of await page.getByTestId("half").all()) {
      expect(await half.getByTestId("strip").count(), "strips in a half").toBeLessThanOrEqual(2);
    }
    expect(await headFirst(page)).toEqual([]);
    expect(await travel(page, "Vocal 1 repro#"), "the fader's travel (px)").toBeGreaterThan(250);
    for (const tab of await page.getByTestId("tab").all()) {
      expect(await clipped(tab), `tab ${await tab.getAttribute("data-page")}`).toEqual([]);
    }
    for (const button of await page.getByTestId("rail").locator(".btn").all()) {
      expect(await clipped(button), `rail button ${await button.textContent()}`).toEqual([]);
    }
    // The lower line's arrows reach its last strip.
    const lower = page.locator('[data-testid="shift"][data-line="1"]');
    for (let i = 0; i < 6; i++) await lower.getByTestId("shift-next").dispatchEvent("pointerdown");
    await until(
      () => strip(page, "A-Echo", "master").count(),
      (n) => n === 1,
      "the lower line's last strip on screen",
    );
  });
});
