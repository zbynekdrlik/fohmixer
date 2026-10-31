import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { LiveClient, centre, clipped, frames, harness, hostLine, hubEvents, hubStatus, openSurface, ready, selectPage, servedLayout, until, volume } from "./support/live";

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
    await expect(conflict.first().getByTestId("strip-mark")).toHaveText("CONFLICT");
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
    // Its SEL is inert too (#71, #151): dimmed as the conflict dims the
    // strip's fader, and a finger held on it opens no detail.
    const key = conflict.first().getByTestId("detail-key");
    await expect(key).toHaveCSS("opacity", "0.35");
    await expect(conflict.first().getByTestId("fader")).toHaveCSS("opacity", "0.35");
    const menu = (await key.boundingBox())!;
    await page.mouse.move(menu.x + menu.width / 2, menu.y + menu.height / 2);
    await page.mouse.down();
    await page.waitForTimeout(700);
    await page.mouse.up();
    await expect(page.getByTestId("detail")).toHaveCount(0);
    // A tag problem: marked, and its fader works.
    expect(await hostLine("band", `tuner track 1 set '"Other" +G:MARKERS +GX'`)).toBe("TUNER 1");
    const problem = group.and(page.locator('[data-label="Other"]'));
    await expect(problem).toHaveAttribute("data-mark", "problem", { timeout: 10_000 });
    await expect(problem.getByTestId("strip-mark")).toHaveText("TAG?");
    // Beside the status light: the tighter fit of the two words.
    expect(await clipped(problem.getByTestId("strip-mark"))).toEqual([]);
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

test("a tags group's own controls follow its marker strips", async ({ page }) => {
  // #68 PR C: a migrated group that ended with a parameter fader keeps it
  // after its marker strips.
  const frame = withMarkers();
  const foh = frame.pages.find((p: any) => p.id === "foh");
  const volume = { instance: "band", anchor: { kind: "track", name: "Drums #" }, path: "mixer_device volume" };
  foh.rows[1].sections.find((s: any) => s.id === "markers").controls = [
    { kind: "param_fader", label: "OWN", targets: [{ binding: volume, prop: "value", scale: "cc_linear" }] },
  ];
  await openSurface(page);
  const slots = page.locator('.slot[data-group="markers"]');
  const own = slots.locator('[data-testid="param-fader"][data-label="OWN"]');
  const strip = slots.locator('[data-testid="strip"]');
  try {
    await harness("/hub/layout", { layout: frame });
    await selectPage(page, "foh");
    await expect(own).toHaveCount(1, { timeout: 10_000 });
    await expect(own.locator(".param-fader-label")).toHaveText("OWN");
    await expect(strip).toHaveCount(0);
    expect(await hostLine("band", `tuner track 1 set '"Hand two" +G:MARKERS'`)).toBe("TUNER 1");
    await expect(strip).toHaveCount(1, { timeout: 10_000 });
    await expect(strip).toHaveAttribute("data-label", "Hand two");
    await ready(strip.getByTestId("fader"));
    // The strip first, then the group's own fader, on screen.
    const stripBox = (await strip.boundingBox())!;
    const ownBox = (await own.boundingBox())!;
    expect(ownBox.x).toBeGreaterThan(stripBox.x + stripBox.width - 1);
  } finally {
    await hostLine("band", "tuner track 1 remove");
    await harness("/hub/layout/reset");
    await markersGone();
  }
  await expect(own).toHaveCount(0, { timeout: 10_000 });
});

test("the TAGS chip opens the tag manual over the screen and closes it", async ({ page }) => {
  await openSurface(page);
  // The same page the tray opens.
  const served = await page.request.get("/znacky.html");
  expect(served.status()).toBe(200);
  expect(await served.text()).toContain("TAGS IN THE TUNER");
  await page.getByTestId("manual-open").click();
  const manual = page.getByTestId("manual");
  await expect(manual).toBeVisible();
  const text = manual.getByTestId("znacky");
  await expect(text).toContainText("TAGS IN THE TUNER", { timeout: 10_000 });
  await expect(text).toContainText("+G:VOCALS:2");
  // Over the whole screen, the column too.
  expect(Number(await manual.evaluate((e) => getComputedStyle(e).zIndex))).toBeGreaterThan(10);
  const box = (await manual.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(box.width).toBeGreaterThanOrEqual(viewport.width - 1);
  await page.getByTestId("manual-close").click();
  await expect(manual).toHaveCount(0);
});

test("the tag manual stays open through a new layout (a Tuner edited in Live)", async ({ page }) => {
  await openSurface(page);
  const tab = page.locator('[data-testid="tab"][data-page="foh"]');
  await expect(tab).toHaveText("FOH");
  try {
    await page.getByTestId("manual-open").click();
    const manual = page.getByTestId("manual");
    await expect(manual.getByTestId("znacky")).toContainText("TAGS IN THE TUNER", { timeout: 10_000 });
    await manual.evaluate((e) => e.setAttribute("data-e2e-kept", "1"));
    // A new layout (the markers' compositions come the same way): the
    // page under the manual is rebuilt, the manual is the same element.
    const frame = withMarkers();
    frame.pages.find((p: any) => p.id === "foh").title = "FOH 2";
    await harness("/hub/layout", { layout: frame });
    await expect(tab).toHaveText("FOH 2", { timeout: 10_000 });
    await expect(manual).toHaveAttribute("data-e2e-kept", "1");
    await expect(manual.getByTestId("znacky")).toContainText("TAGS IN THE TUNER");
    await page.getByTestId("manual-close").click();
    await expect(manual).toHaveCount(0);
  } finally {
    await harness("/hub/layout/reset");
  }
  await expect(tab).toHaveText("FOH", { timeout: 10_000 });
});

/** The fixture with STAGE, TechAlert and one toggle named by `+CTRL` ids instead of their tracks (#104). */
function withCtrls(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const ctrl = (id: string) => ({ instance: "band", anchor: { kind: "ctrl", id } });
  const rail = frame.pages.find((p: any) => p.id === "foh").rail;
  rail.find((c: any) => c.kind === "stage").binding = ctrl("stage");
  rail.find((c: any) => c.label === "ZVUKAR").targets = [{ binding: ctrl("talkback"), prop: "mute", on: false, off: true }];
  frame.global.find((c: any) => c.kind === "alert").binding = ctrl("alert");
  return frame;
}

test("a control naming a +CTRL id is red until its Tuner comes, then follows that track", async ({ page }) => {
  await openSurface(page);
  const stage = page.getByTestId("stage-mics");
  const alert = page.getByTestId("rail").getByTestId("alert-toggle");
  const toggle = page.locator('[data-testid="param-toggle"][data-label="ZVUKAR"]');
  const live = await LiveClient.open();
  const mutes = [1, 2, 3].map((i) => `live_set tracks ${i}`);
  const before = await Promise.all(mutes.map((target) => live.get("band", target, "mute")));
  try {
    await harness("/hub/layout", { layout: withCtrls() });
    await selectPage(page, "foh");
    for (const button of [stage, alert, toggle]) {
      await expect(button).toHaveAttribute("data-binding", "unresolved", { timeout: 10_000 });
    }
    // A press on a red control writes nothing (read at the end: the first
    // sets after these presses are the filled controls').
    const since = Date.now();
    for (const button of [stage, alert, toggle]) {
      const { x, y } = await centre(button);
      await page.mouse.click(x, y);
    }
    expect(await hostLine("band", "tuner track 1 set '+CTRL:stage'")).toBe("TUNER 1");
    expect(await hostLine("band", "tuner track 2 set '+CTRL:alert'")).toBe("TUNER 1");
    expect(await hostLine("band", "tuner track 3 set '+CTRL:talkback'")).toBe("TUNER 1");
    for (const button of [stage, alert, toggle]) {
      await expect(button).toHaveAttribute("data-binding", "ready", { timeout: 10_000 });
    }
    // The button reads the tagged track, by its index.
    await live.set("band", "live_set tracks 1", "mute", true);
    await expect(stage).toHaveAttribute("data-muted", "true");
    await live.set("band", "live_set tracks 1", "mute", false);
    await expect(stage).toHaveAttribute("data-muted", "false");
    // A press on the filled alert and the filled toggle reaches its track.
    await live.set("band", mutes[1], "mute", true);
    await live.set("band", mutes[2], "mute", true);
    await expect(alert).toHaveAttribute("data-on", "false");
    await ready(alert);
    await alert.click();
    await until(() => live.get("band", mutes[1], "mute"), (v) => v === false, "the alert reaches its track");
    await expect(alert).toHaveAttribute("data-on", "true");
    await alert.click();
    await until(() => live.get("band", mutes[1], "mute"), (v) => v === true, "the alert off");
    await ready(toggle);
    const { x, y } = await centre(toggle);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await until(() => live.get("band", mutes[2], "mute"), (v) => v === false, "the toggle reaches its track");
    await page.mouse.up();
    await until(() => live.get("band", mutes[2], "mute"), (v) => v === true, "the toggle off");
    const keys = await until(
      async () => new Set((await hubEvents()).filter((e) => e.ev === "set" && e.ts >= since).map((e) => e.key)),
      (found) => found.size >= 2,
      "the presses' sets in the event log",
    );
    expect([...keys].sort()).toEqual([`band|${mutes[1]}|mute`, `band|${mutes[2]}|mute`]);
  } finally {
    for (const [i, target] of mutes.entries()) await live.set("band", target, "mute", before[i]);
    live.close();
    for (const track of [1, 2, 3]) await hostLine("band", `tuner track ${track} remove`);
    await harness("/hub/layout/reset");
    // Only the fixture names STAGE's track: `markersGone` alone can return
    // while the hub still serves `withCtrls()`.
    await until(servedLayout, (l) => l.pages.find((p: any) => p.id === "foh").rail.find((c: any) => c.kind === "stage").binding.anchor.kind === "track", "the hub serving the fixture layout again");
    await markersGone();
  }
});
