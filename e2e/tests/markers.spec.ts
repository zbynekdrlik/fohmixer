import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { harness, hostLine, openSurface, ready, selectPage } from "./support/live";

// Strips from Tuner markers (#68, spec D16): a Tuner in a track's device
// chain, renamed with the strip's quoted label and its tags, puts the strip
// on the surface, bound to that track by its index; a rename in Live
// follows within seconds. A tag group the frame does not show is a view,
// never a page tab.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

test("a Tuner marker puts its strip on the surface and a rename in Live follows", async ({ page }) => {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "markers", title: "MARKERS", tags: "MARKERS" });
  await openSurface(page);
  const marker = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
  try {
    await harness("/hub/layout", { layout: frame });
    await selectPage(page, "foh");
    await expect(page.locator('[data-testid="group"][data-group="markers"]')).toHaveCount(1, { timeout: 10_000 });
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
  }
  await expect(marker).toHaveCount(0, { timeout: 10_000 });
});
