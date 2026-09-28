import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { harness, hubSubscriptions, openSurface, selectPage, strip, until } from "./support/live";

// Pages, pagers, the overlay and the layout (spec F1, F19; S4 design note §2,
// §5): the imported synthetic layout, only the visible pages subscribed.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
const layout = () => JSON.parse(readFileSync(LAYOUT, "utf-8"));

/** The subscriptions the page holds (its `data-subs`). */
async function pageSubs(page: import("@playwright/test").Page): Promise<number> {
  return Number(await page.getByTestId("surface").getAttribute("data-subs"));
}

/** The hub holds the page's subscriptions plus STAGE AUT's own (live_set is_playing). */
async function expectHubToHold(page: import("@playwright/test").Page, subs: number) {
  await expect(page.getByTestId("surface")).toHaveAttribute("data-subs", String(subs));
  await until(hubSubscriptions, (n) => n === subs + 1, `the hub to hold ${subs + 1} subscriptions`);
}

test.describe("Pages and tabs", () => {
  test("the tabs follow the layout and FOH is shown first", async ({ page }) => {
    await openSurface(page);
    const root = page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]');
    await expect(root).toHaveText(["Cue", "FOH", "Conf"]);
    await expect(root.nth(1)).toHaveAttribute("data-selected", "true");
    await expect(root.nth(0)).toHaveAttribute("data-selected", "false");
    const pager = page.locator('[data-testid="tabbar"][data-level="1"] [data-testid="tab"]');
    await expect(pager).toHaveText(["STAGE", "OTHERS"]);
    await expect(pager.nth(0)).toHaveAttribute("data-selected", "true");
  });

  test("the nested pager switches its pages and remembers them", async ({ page }) => {
    await openSurface(page);
    await expect(strip(page, "Keys 1")).toBeVisible();
    await expect(strip(page, "Hand1 #", "master")).toHaveCount(0);
    await selectPage(page, "others");
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await expect(strip(page, "Keys 1")).toHaveCount(0);
    // Another root page and back: the pager still shows OTHERS; so does a reload.
    await selectPage(page, "cue");
    await selectPage(page, "foh");
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await page.reload();
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await selectPage(page, "stage");
  });

  test("the overlay stays on every page", async ({ page }) => {
    await openSurface(page);
    for (const id of ["cue", "conf", "foh"]) {
      await selectPage(page, id);
      await expect(strip(page, "TechAlert #")).toBeVisible();
      await expect(page.getByTestId("refresh")).toHaveText("REFRESH ALL");
    }
    await selectPage(page, "cue");
    await expect(page.locator('[data-testid="param-toggle"][data-label="Vox 1 TU"]')).toBeVisible();
    await selectPage(page, "conf");
    await expect(page.getByTestId("label").first()).toContainText("unfold_band: 'Vocals Repro grp#'");
    await selectPage(page, "foh");
  });

  test("areas show their colours and titles, items sit on their frames", async ({ page }) => {
    await openSurface(page);
    const effects = page.locator('[data-testid="area"]', { hasText: "EFFECTS" });
    await expect(effects).toBeVisible();
    await expect(effects).toHaveCSS("background-color", "rgb(99, 99, 99)");
    // F19: the Hand2 # strip at its canvas frame (2180, 105, 161 × 700), scaled.
    const stage = await page.getByTestId("stage").boundingBox();
    const box = await strip(page, "Hand2 #").boundingBox();
    expect(stage && box).toBeTruthy();
    const scale = stage!.width / 2360;
    expect(Math.abs(box!.x - (stage!.x + 2180 * scale))).toBeLessThan(1.5);
    expect(Math.abs(box!.y - (stage!.y + 105 * scale))).toBeLessThan(1.5);
    expect(Math.abs(box!.width - 161 * scale)).toBeLessThan(1.5);
    expect(Math.abs(box!.height - 700 * scale)).toBeLessThan(1.5);
    // The stage fits the viewport, centred.
    const viewport = page.viewportSize()!;
    expect(Math.abs(stage!.x * 2 + stage!.width - viewport.width)).toBeLessThan(2);
    expect(Math.abs(stage!.y * 2 + stage!.height - viewport.height)).toBeLessThan(2);
    await expect(strip(page, "B-Main repro #")).toHaveAttribute("data-kind", "return");
  });
});

test.describe("Subscriptions follow the pages on screen", () => {
  test("a page switch subscribes the new page and releases the old one", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "stage");
    const foh = await pageSubs(page);
    expect(foh).toBe(28);
    await expectHubToHold(page, foh);
    await selectPage(page, "cue");
    await expectHubToHold(page, 3);
    await selectPage(page, "foh");
    await selectPage(page, "others");
    await expectHubToHold(page, 25);
    await selectPage(page, "stage");
    await expectHubToHold(page, foh);
  });

  test("a layout change while a page is open releases the old subscriptions", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "stage");
    await expectHubToHold(page, 28);
    const changed = layout();
    changed.pages[1].items = changed.pages[1].items.filter(
      (item: any) => !(item.kind === "strip" && item.binding.anchor.name === "Hand2 #"),
    );
    try {
      await harness("/hub/layout", { layout: changed });
      await expect(strip(page, "Hand2 #")).toHaveCount(0, { timeout: 10_000 });
      await expectHubToHold(page, 24);
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(strip(page, "Hand2 #")).toBeVisible({ timeout: 10_000 });
    await expectHubToHold(page, 28);
  });
});
