import { test, expect } from "./support/fixtures";
import { frames, openSurface, ready, strip } from "./support/live";

// On a phone (#63): one screen at a time. Each row of the page is a screen
// and so is the rail; the screen bar switches them, so a row gets the whole
// height. The tablet layout is unchanged (no screen bar, every row).

/** How far the top bar's content runs past its own width (px; it wraps instead). */
async function topBarOverflow(page: import("@playwright/test").Page): Promise<number> {
  return page.locator(".topbar").evaluate((el) => el.scrollWidth - el.clientWidth);
}

test.describe("On a phone in landscape", () => {
  test.use({ viewport: { width: 844, height: 390 } });

  test("a row takes the whole height; the screen bar switches the rows and the rail", async ({ page }) => {
    await openSurface(page);
    const bar = page.getByTestId("screens");
    await expect(bar).toBeVisible();
    const screens = bar.getByTestId("screen");
    await expect(screens).toHaveText(["FUNKCIE", "STAGE", "EFFECTS · HANDS"]);
    const rows = page.getByTestId("row");
    await expect(screens.nth(1)).toHaveAttribute("data-selected", "true");
    await expect(rows.nth(0)).toBeVisible();
    await expect(rows.nth(1)).toBeHidden();
    await expect(page.getByTestId("rail")).toBeHidden();
    // The fader runs most of the screen's height (the tablet's two rows left
    // it ~20 px on this screen).
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    const box = (await fader.boundingBox())!;
    expect(box.height, "the fader's travel (px)").toBeGreaterThan(150);
    expect(box.y + box.height, "inside the screen").toBeLessThanOrEqual(390);
    const width = await page.locator(".rows").evaluate((el) => getComputedStyle(el).getPropertyValue("--strip-w"));

    // The second row.
    await screens.nth(2).click();
    await expect(screens.nth(2)).toHaveAttribute("data-selected", "true");
    await expect(rows.nth(1)).toBeVisible();
    await expect(rows.nth(0)).toBeHidden();
    await expect(strip(page, "Hand2 #", "master").getByTestId("fader")).toBeVisible();

    // The rail: its buttons over the whole width, no row.
    await screens.nth(0).click();
    const rail = page.getByTestId("rail");
    await expect(rail).toBeVisible();
    await expect(rows.nth(0)).toBeHidden();
    await expect(rows.nth(1)).toBeHidden();
    await expect(rail.getByTestId("solo").first()).toBeVisible();
    const railBox = (await rail.boundingBox())!;
    expect(railBox.width, "the rail fills the screen's width").toBeGreaterThan(700);

    // Back to the first row: the strips as wide as before the rail.
    await screens.nth(1).click();
    await expect(rows.nth(0)).toBeVisible();
    await expect(rail).toBeHidden();
    await frames(page);
    expect(await page.locator(".rows").evaluate((el) => getComputedStyle(el).getPropertyValue("--strip-w"))).toBe(width);
    expect(await topBarOverflow(page), "the top bar fits the screen").toBeLessThanOrEqual(0);
    const version = page.getByTestId("stage").getByTestId("version");
    await expect(version).toBeVisible();
    const label = (await version.boundingBox())!;
    expect(label.x + label.width, "the version inside the screen").toBeLessThanOrEqual(844);
  });
});

test.describe("On a phone upright", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("the top bar wraps inside the screen; a row gets the height, a wide one scrolls inside itself", async ({ page }) => {
    await openSurface(page);
    await expect(page.getByTestId("screens")).toBeVisible();
    await expect(page.getByTestId("rail")).toBeHidden();
    const rows = page.getByTestId("row");
    // The test layout's first row fits 370 px at the narrowest strips (about
    // 307 px), its second does not (about 468 px).
    await expect(rows.nth(0)).not.toHaveClass(/\bscrolls\b/);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    expect((await fader.boundingBox())!.height, "the fader's travel (px)").toBeGreaterThan(400);
    await page.getByTestId("screen").nth(2).click();
    await expect(rows.nth(1)).toBeVisible();
    await expect(rows.nth(1)).toHaveClass(/\bscrolls\b/);
    expect(await rows.nth(1).evaluate((el) => el.scrollWidth - el.clientWidth), "its scroll range (px)").toBeGreaterThan(50);
    expect(await topBarOverflow(page), "the top bar fits the screen").toBeLessThanOrEqual(0);
    const version = page.getByTestId("stage").getByTestId("version");
    await expect(version).toBeVisible();
    const label = (await version.boundingBox())!;
    expect(label.x + label.width, "the version inside the screen").toBeLessThanOrEqual(390);
  });
});

test("on the tablet there is no screen bar and every row shows", async ({ page }) => {
  await openSurface(page);
  await expect(page.getByTestId("screens")).toBeHidden();
  await expect(page.getByTestId("rail")).toBeVisible();
  const rows = page.getByTestId("row");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toBeVisible();
  await expect(rows.nth(1)).toBeVisible();
});
