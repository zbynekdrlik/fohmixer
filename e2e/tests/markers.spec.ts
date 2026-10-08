import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { LiveClient, centre, clipped, frames, harness, hostLine, hubStatus, openSurface, ready, selectPage, until, volume } from "./support/live";

// Strips from Tuner markers (#68, spec D16): a Tuner in a track's device
// chain, renamed with the strip's quoted label and its tags, puts the strip
// on the surface, bound to that track by its index; a rename in Live
// follows within seconds. A tag group the frame does not show is a view,
// never a page tab.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fixture with a MARKERS group (the strips of that tag group) in the FOH page's second row. */
function withMarkers(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "markers", title: "MARKERS", tags: "MARKERS" });
  return frame;
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture, never rebuilt by a late read. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the Tuner markers gone", 10_000);
}

/** A mouse drag up the fader, from its centre (its first move only anchors, #43 PR F). */
async function dragUp(page: any, fader: any) {
  const { x, y } = await centre(fader);
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(x, y - 8 * i);
  await frames(page);
  await page.mouse.up();
}

test("a Tuner marker puts its strip on the surface and a rename in Live follows", async ({ page }) => {
  const frame = withMarkers();
  await openSurface(page);
  const marker = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
  try {
    await harness("/hub/layout", { layout: frame });
    await selectPage(page, "foh");
    // An empty tags group draws nothing: the strip itself proves the new
    // layout (the store composes the markers with whichever frame comes).
    // The second track of the test site (Hand2 #) gets a Tuner.
    expect(await hostLine("band", `tuner track 1 set '"Hand two" +G:MARKERS +MG'`)).toBe("TUNER 1");
    await expect(marker).toHaveCount(1, { timeout: 10_000 });
    await expect(marker).toHaveAttribute("data-label", "Hand two");
    await expect(marker).toHaveAttribute("data-track", "#2");
    await expect(marker.getByTestId("strip-label")).toHaveText("Hand two");
    await ready(marker.getByTestId("fader"));
    // Another Tuner's group the frame does not show, then a rename: once
    // the rename shows, the layout holds the view too, and it is no tab.
    expect(await hostLine("band", `tuner track 2 set '"Hand three" +G:SOLO'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 1 set '"Hand 2b" +G:MARKERS'`)).toBe("TUNER 1");
    await expect(marker).toHaveAttribute("data-label", "Hand 2b", { timeout: 10_000 });
    await expect(marker.getByTestId("strip-label")).toHaveText("Hand 2b");
    await expect(page.locator('[data-testid="tab"][data-page="view-SOLO"]')).toHaveCount(0);
    await expect(page.locator('[data-testid="tab"][data-page="foh"]')).toHaveCount(1);
  } finally {
    await hostLine("band", "tuner track 1 remove");
    await hostLine("band", "tuner track 2 remove");
    await harness("/hub/layout/reset");
    await markersGone();
  }
  await expect(marker).toHaveCount(0, { timeout: 10_000 });
});

// The approved mockup docs/mockups/surface-views-v4.html (#68).
test("a tag group the frame does not show is a view: its strips and the pins, a second tap returns", async ({ page }) => {
  await openSurface(page);
  const button = page.locator('[data-testid="view"][data-page="view-SOLO"]');
  try {
    await harness("/hub/layout", { layout: withMarkers() });
    await selectPage(page, "foh");
    expect(await hostLine("band", `tuner track 1 set '"Solo voice" +G:SOLO'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 2 set '"Pinned one" +G:MARKERS +PIN'`)).toBe("TUNER 1");
    await expect(button).toHaveCount(1, { timeout: 10_000 });
    await expect(page.locator('.slot[data-group="markers"] [data-label="Pinned one"]')).toHaveCount(1, { timeout: 10_000 });
    await expect(button).toHaveText("SOLO");
    await expect(page.getByTestId("views-title")).toBeVisible();
    await expect(button).toHaveAttribute("data-selected", "false");
    await button.click();
    const view = page.locator('[data-testid="page"][data-page="view-SOLO"]');
    await expect(view).toHaveCount(1);
    await expect(button).toHaveAttribute("data-selected", "true");
    // Its own strips, then every pinned strip.
    const labels = await view.locator('[data-testid="strip"]').evaluateAll((all) => all.map((e) => e.getAttribute("data-label")));
    expect(labels).toEqual(["Solo voice", "Pinned one"]);
    await ready(view.locator('[data-testid="strip"][data-label="Solo voice"]').getByTestId("fader"));
    // FOH's pager tabs stay, dimmed and inert.
    const pager = page.locator('[data-testid="tabbar"][data-level="1"]');
    await expect(pager).toHaveClass(/\bdim\b/);
    await expect(pager).toHaveCSS("pointer-events", "none");
    // A second tap: back to FOH.
    await button.click();
    await expect(page.locator('[data-testid="page"][data-page="foh"]')).toHaveCount(1);
    await expect(button).toHaveAttribute("data-selected", "false");
    await expect(pager).not.toHaveClass(/\bdim\b/);
  } finally {
    await hostLine("band", "tuner track 1 remove");
    await hostLine("band", "tuner track 2 remove");
    await harness("/hub/layout/reset");
    await markersGone();
  }
  await expect(button).toHaveCount(0, { timeout: 10_000 });
});

test("an equal label disables both strips; a tag problem marks a strip that still works", async ({ page }) => {
  const live = await LiveClient.open();
  const target = volume("live_set tracks 1");
  const before = await live.get("band", target, "value");
  await openSurface(page);
  const group = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
  try {
    await harness("/hub/layout", { layout: withMarkers() });
    await selectPage(page, "foh");
    expect(await hostLine("band", `tuner track 1 set '"Same" +G:MARKERS'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 2 set '"Same" +G:MARKERS'`)).toBe("TUNER 1");
    const conflict = page.locator('.slot[data-group="markers"] [data-testid="strip"][data-mark="conflict"]');
    await expect(conflict).toHaveCount(2, { timeout: 10_000 });
    await expect(conflict.first().getByTestId("strip-mark")).toHaveText("KONFLIKT");
    expect(await clipped(conflict.first().getByTestId("strip-mark"))).toEqual([]);
    await expect(conflict.first().getByTestId("db")).toBeHidden();
    // The first is track 1 (Live's order, `#2` by index): its fader is
    // bound and takes touches but for the conflict, and a drag on it
    // reaches nothing.
    await expect(conflict.first()).toHaveAttribute("data-track", "#2");
    await ready(conflict.first().getByTestId("fader"));
    await dragUp(page, conflict.first().getByTestId("fader"));
    await page.waitForTimeout(500);
    expect(await live.get("band", target, "value")).toBe(before);
    // A tag problem: marked, and its fader works.
    expect(await hostLine("band", `tuner track 1 set '"Other" +G:MARKERS +GX'`)).toBe("TUNER 1");
    const problem = group.and(page.locator('[data-label="Other"]'));
    await expect(problem).toHaveAttribute("data-mark", "problem", { timeout: 10_000 });
    await expect(problem.getByTestId("strip-mark")).toHaveText("ZNAČKA?");
    const same = group.and(page.locator('[data-label="Same"]'));
    await expect(same).toHaveCount(1);
    expect(await same.getAttribute("data-mark")).toBeNull();
    const fader = problem.getByTestId("fader");
    await ready(fader);
    await dragUp(page, fader);
    await until(() => live.get("band", target, "value"), (v) => v !== before, "the marked strip's fader reaches Live");
  } finally {
    await live.set("band", target, "value", before);
    live.close();
    await hostLine("band", "tuner track 1 remove");
    await hostLine("band", "tuner track 2 remove");
    await harness("/hub/layout/reset");
    await markersGone();
  }
  await expect(group).toHaveCount(0, { timeout: 10_000 });
});

test("the ZNAČKY chip opens the tag manual over the screen and closes it", async ({ page }) => {
  await openSurface(page);
  // The same page the tray opens.
  const served = await page.request.get("/znacky.html");
  expect(served.status()).toBe(200);
  expect(await served.text()).toContain("ZNAČKY V TUNERI");
  await page.getByTestId("manual-open").click();
  const manual = page.getByTestId("manual");
  await expect(manual).toBeVisible();
  const text = manual.getByTestId("znacky");
  await expect(text).toContainText("ZNAČKY V TUNERI", { timeout: 10_000 });
  await expect(text).toContainText("+G:VOCALS:2");
  // Over the whole screen, the column too.
  expect(Number(await manual.evaluate((e) => getComputedStyle(e).zIndex))).toBeGreaterThan(10);
  const box = (await manual.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(box.width).toBeGreaterThanOrEqual(viewport.width - 1);
  await page.getByTestId("manual-close").click();
  await expect(manual).toHaveCount(0);
});
