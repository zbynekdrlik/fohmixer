import type { Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { harness, hubStatus, openSurface, token, until } from "./support/live";

// A Live that is down, said in words (#178; the owner's approved
// docs/mockups/live-down-v2.html): no card or overlay anywhere. A strip of a
// Live that is not running (its host stopped) or has no set (its host on an
// empty set: none of the layout's names resolve) is dimmed with NOT RUNNING /
// NO SET under its name and `—` on its readout, never red; the column's foot
// says each Live in words with a one-line hint. I5's red stays for a single
// bad binding of a Live whose set is loaded. It clears by itself when Live
// comes back.

const IPAD = { width: 1194, height: 834 };
const UP = 15_000;

/** Every strip of `instance` on the page carries `live` (none: no attribute). */
async function stripsSay(page: Page, instance: string, live: string | null) {
  const strips = page.locator(`[data-testid="strip"][data-instance="${instance}"]`);
  await expect(strips.first()).toBeAttached();
  const n = await strips.count();
  expect(n).toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    if (live === null) await expect(strips.nth(i)).not.toHaveAttribute("data-live", /.*/, { timeout: UP });
    else await expect(strips.nth(i)).toHaveAttribute("data-live", live, { timeout: UP });
  }
}

/** The controls drawn red (I5) inside `scope`'s matches. */
async function redIn(page: Page, scope: string): Promise<number> {
  return page.locator(`${scope} [data-binding="unresolved"], ${scope}[data-binding="unresolved"]`).count();
}

const foot = (page: Page, instance: string) => page.locator(`[data-testid="badge"][data-instance="${instance}"]`);

/** The hosts a test stopped or moved, started again on the test site. */
const changed = new Set<string>();

async function host(name: string, how: "stop" | "start", site?: string) {
  changed.add(name);
  await harness(`/host/${name}/${how}`, site ? { site } : {});
}

test.describe("A Live that is down (#178)", () => {
  test.afterEach(async () => {
    for (const name of changed) await harness(`/host/${name}/start`);
    changed.clear();
    // The hub has both Lives back before the next spec opens a page (one
    // token: every login hashes the PIN).
    const bearer = await token();
    await until(
      () => hubStatus(bearer),
      (s) => s.instances.every((i: any) => i.online && i.set_name === "Test Site"),
      "both Lives online on the test site",
      15_000,
    );
  });

  test("Lives not running: the strips and the foot say so in words, nothing red, and it clears by itself", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(foot(page, "band").getByTestId("badge-word")).toHaveText("on");
    await expect(page.getByTestId("live-hint")).toHaveCount(0);
    const masterZone = page.locator('[data-testid="strip"][data-instance="master"] .strip-zone').first();
    const travel = (await masterZone.boundingBox())!.height;

    await host("band", "stop");
    await host("master", "stop");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, "off");
      await expect(foot(page, instance)).toHaveAttribute("data-live", "off");
      await expect(foot(page, instance).getByTestId("badge-word")).toHaveText("not running");
    }
    const first = page.locator('[data-testid="strip"][data-instance="band"]').first();
    await expect(first.getByTestId("strip-why")).toHaveText("NOT RUNNING");
    await expect(first.getByTestId("strip-down-db")).toHaveText("—");
    await expect(first.getByTestId("strip-down-db")).toBeVisible();
    await expect(first.getByTestId("db")).toBeHidden();
    await expect(page.getByTestId("live-hint")).toHaveText("Start Live on the FOH PC. The faders come back by themselves.");
    // Not red: no control, no TechAlert frame, no status light, no badge dot.
    expect(await redIn(page, "body")).toBe(0);
    await expect(page.locator(".status:visible")).toHaveCount(0);
    await expect(page.locator(".badge-dot")).toHaveCount(0);
    // The word is in the marks line's place: no height moved.
    expect((await masterZone.boundingBox())!.height).toBe(travel);
    const why = await first.getByTestId("strip-why").evaluate((e) => getComputedStyle(e).color);
    expect(why).toBe("rgb(245, 166, 35)");
    // Nothing covers the left side: an extra tab opens as usual.
    const tab = page.getByTestId("presenter-tab");
    await tab.click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "presenter");
    await expect(page.getByTestId("live-hint")).toBeVisible();
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");

    await host("band", "start");
    await host("master", "start");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, null);
      await expect(foot(page, instance)).toHaveAttribute("data-live", "on", { timeout: UP });
    }
    await expect(page.getByTestId("live-hint")).toHaveCount(0);
    await expect(page.getByTestId("strip-why")).toHaveCount(0);
    await expect(first.getByTestId("db")).toBeVisible();
  });

  test("a Live with no set: NO SET on its strips, its bindings not red, the other Live untouched", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "master")).toHaveAttribute("data-live", "on", { timeout: UP });
    // The fixture's master set has bindings that do not resolve (I5, red),
    // and so may the band's: the test needs the master's to be some.
    const masterRed = await redIn(page, '[data-testid="strip"][data-instance="master"]');
    expect(masterRed).toBeGreaterThan(0);
    const bandRed = await redIn(page, '[data-testid="strip"][data-instance="band"]');

    await host("band", "start", "empty");
    await stripsSay(page, "band", "noset");
    await expect(foot(page, "band")).toHaveAttribute("data-live", "noset");
    await expect(foot(page, "band").getByTestId("badge-word")).toHaveText("no set");
    await expect(page.locator('[data-testid="strip"][data-instance="band"]').first().getByTestId("strip-why")).toHaveText("NO SET");
    await expect(page.getByTestId("live-hint")).toHaveText("Open the set in Live.");
    // The band Live answers, yet none of its strips is red (it was all red,
    // #178's report); the master Live's own bindings keep I5 as they were.
    expect(await redIn(page, '[data-testid="strip"][data-instance="band"]')).toBe(0);
    await expect(page.locator('.alert-wash[data-binding="unresolved"]')).toHaveCount(0);
    await stripsSay(page, "master", null);
    expect(await redIn(page, '[data-testid="strip"][data-instance="master"]')).toBe(masterRed);
    await expect(foot(page, "master").getByTestId("badge-word")).toHaveText("on");

    await host("band", "start");
    await stripsSay(page, "band", null);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(page.getByTestId("live-hint")).toHaveCount(0);
    // A bad binding of the loaded set is red again (I5).
    await until(
      () => redIn(page, '[data-testid="strip"][data-instance="band"]'),
      (n) => n === bandRed,
      "the band's own red bindings back",
      UP,
    );
  });

  test("every Live on: the words take the dots' place and the rail keeps its room", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(foot(page, "master")).toHaveAttribute("data-live", "on");
    // The foot keeps the two rows it had with the dots, so every rail row
    // keeps its 44 px and the stage miniature its place.
    const height = (await page.getByTestId("column-foot").boundingBox())!.height;
    expect(height).toBeLessThanOrEqual(34);
    const rows = await page.locator(".rail-main").evaluate((e) => getComputedStyle(e).gridTemplateRows.split(" "));
    for (const row of rows) expect(row).toBe("44px");
    // Both words on the counter's row.
    const counter = (await page.getByTestId("dropouts").boundingBox())!;
    for (const instance of ["band", "master"]) {
      const b = (await foot(page, instance).boundingBox())!;
      expect(Math.abs(b.y + b.height / 2 - (counter.y + counter.height / 2))).toBeLessThan(2);
    }
  });
});
