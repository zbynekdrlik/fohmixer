import type { Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { HARNESS, dispatchPointer, harness, hubEvents, openSurface, pageEvents, rightHalfLive, until } from "./support/live";

// Presenter's stage tab (#90; the owner's approved
// docs/mockups/presenter-stage-v1.html): the PRESENTER tab under IEM MIXER
// opens over the left half: a live preview of the stage display (Presenter's
// own stage page through the hub, scaled from 1920 x 1080), the layout on
// stage named in the header, and Presenter's layouts as keys, the one on
// stage lit. A key's 1 s hold switches the stage at its lift; a tap does
// nothing. Presenter down: the preview says so and the keys dim; it comes
// back by itself. The e2e job's harness runs the fake Presenter
// (e2e/harness/fake_presenter.py), the hub's [presenter].

/** The fake Presenter's state: `current`, `down`, `sockets`, `hits`. */
async function fakeState(): Promise<any> {
  const response = await fetch(`${HARNESS}/presenter`);
  if (!response.ok) throw new Error(`harness /presenter: ${response.status}`);
  return response.json();
}

const NAMES = ["WORSHIP", "OVERVIEW", "TIMER", "PREACH", "VIDEO", "BIBLE", "FULL TEXT", "API", "API + VIDEO"];

const keyOf = (page: Page, code: string): Locator => page.locator(`[data-testid="presenter-key"][data-name="${code}"]`);
const stageFrame = (page: Page) => page.frameLocator('[data-testid="presenter-frame"]');

/** The tab tapped, the stage page loaded in the preview. */
async function openPresenter(page: Page) {
  const tab = page.getByTestId("presenter-tab");
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
  await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "presenter");
  await expect(stageFrame(page).locator("body")).toHaveAttribute("data-app", "loaded");
  await expect(stageFrame(page).locator("body")).toHaveAttribute("data-ws", "open");
}

/** Puts the operator's layout back on stage (and Presenter up). */
async function restore() {
  await harness("/presenter/up");
  await harness("/presenter/switch", { code: "worship" });
}

/** The page's `rail` steps of the PRESENTER keys since `t` (page clock). */
async function steps(t: number): Promise<string[]> {
  const events = pageEvents(await hubEvents());
  return events.filter((e) => e.ev === "rail" && e.button === "presenter" && e.t >= t).map((e) => e.what);
}

const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

test.describe("Presenter's stage tab", () => {
  test.afterEach(restore);

  test("is the tab under IEM MIXER, shows the stage over the left half, its layouts and the one on stage", async ({ page }) => {
    await restore();
    await openSurface(page);
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    const rows = await bar.evaluate((b) => [...b.children].map((c) => c.getAttribute("data-testid")));
    // The last row; IEM MIXER's row (when the hub has [iem]) right above it.
    expect(rows.at(-1)).toBe("presenter-tab");
    if (rows.includes("iem-row")) expect(rows.at(-2)).toBe("iem-row");
    const tab = page.getByTestId("presenter-tab");
    await expect(tab).toHaveText("PRESENTER");
    await expect(page.getByTestId("presenter-dot")).toHaveCount(0);
    const column = (await page.getByTestId("column").boundingBox())!;
    await openPresenter(page);
    // The column stays, the right half stays live.
    expect(await page.getByTestId("column").boundingBox()).toEqual(column);
    await rightHalfLive(page, page.locator('.slot[data-side="right"] [data-testid="strip"]'));
    const pane = (await page.getByTestId("presenter").boundingBox())!;
    expect(pane.x + pane.width).toBeLessThanOrEqual(column.x + 0.5);
    // The header and the caption are the mockup's.
    await expect(page.getByTestId("presenter-now")).toHaveText("on stage: WORSHIP");
    await expect(page.locator(".pst-caption")).toHaveText("STAGE LAYOUT · hold 1 s to switch");
    // The preview: Presenter's stage page through the hub, 1920 x 1080
    // scaled to the area's width, 16:9, no touch.
    const frame = page.getByTestId("presenter-frame");
    await expect(frame).toHaveAttribute("src", "/stage?preview=1");
    // No sandbox: one that allowed its scripts and its origin would warn
    // (the frame could lift it) and guard nothing.
    expect(await frame.getAttribute("sandbox")).toBeNull();
    await expect(stageFrame(page).locator("#fake-stage")).toHaveText("worship");
    const area = (await page.getByTestId("presenter-stage").boundingBox())!;
    expect(Math.abs(area.width / area.height - 16 / 9)).toBeLessThan(0.02);
    const shown = (await frame.boundingBox())!;
    expect(Math.abs(shown.width - area.width)).toBeLessThan(2.5);
    expect(Math.abs(shown.height - area.height)).toBeLessThan(2.5);
    expect(await frame.evaluate((f) => getComputedStyle(f).pointerEvents)).toBe("none");
    // The keys: Presenter's nine, in its order, in three columns; the one on
    // stage lit with "on stage now", the others Presenter's descriptions.
    const keys = page.getByTestId("presenter-key");
    await expect(keys).toHaveCount(9);
    await expect(page.locator('[data-testid="presenter-key"] .pst-name')).toHaveText(NAMES);
    await expect(keyOf(page, "worship")).toHaveAttribute("data-state", "on");
    await expect(keyOf(page, "worship").locator(".pst-note")).toHaveText("on stage now");
    await expect(keyOf(page, "timer")).toHaveAttribute("data-state", "off");
    await expect(keyOf(page, "timer").locator(".pst-note")).toHaveText("Countdown to the start");
    const tops = await keys.evaluateAll((all) => all.map((k) => Math.round(k.getBoundingClientRect().top)));
    expect(new Set(tops).size).toBe(3);
    // The hub's own socket and the frame's are Presenter's previews: two
    // sockets, both through the hub.
    await until(fakeState, (s) => s.sockets >= 2, "the hub's and the frame's sockets");
  });

  test("a key's 1 s hold switches the stage at the lift, a tap does nothing, and every page follows", async ({ page }) => {
    await restore();
    await openSurface(page);
    await openPresenter(page);
    const since = await pageNow(page);
    const timer = keyOf(page, "timer");
    // A tap: nothing.
    await dispatchPointer(timer, [{ type: "pointerdown" }, { wait: 120 }, { type: "pointerup" }], 31);
    await page.waitForTimeout(400);
    expect((await fakeState()).current).toBe("worship");
    await expect(timer).toHaveAttribute("data-state", "off");
    // A hold: the fill, "release — switch" once full, nothing sent yet.
    await dispatchPointer(timer, [{ type: "pointerdown" }, { wait: 300 }], 32);
    await expect(timer).toHaveAttribute("data-holding", "true");
    await expect(timer).toHaveAttribute("data-full", "true", { timeout: 3000 });
    await expect(timer.locator(".pst-release")).toBeVisible();
    await expect(timer.locator(".pst-release")).toHaveText("release — switch");
    await expect(timer.locator(".pst-note")).toBeHidden();
    expect((await fakeState()).current).toBe("worship");
    // The lift switches it.
    await dispatchPointer(timer, [{ type: "pointerup" }], 32);
    await until(fakeState, (s) => s.current === "timer", "the stage on TIMER");
    await expect(timer).toHaveAttribute("data-state", "on");
    await expect(keyOf(page, "worship")).toHaveAttribute("data-state", "off");
    await expect(page.getByTestId("presenter-now")).toHaveText("on stage: TIMER");
    // The preview follows through its own socket on the hub.
    await expect(stageFrame(page).locator("#fake-stage")).toHaveText("timer");
    // The operator's switch in Presenter shows at once too.
    await harness("/presenter/switch", { code: "bible" });
    await expect(keyOf(page, "bible")).toHaveAttribute("data-state", "on");
    await expect(page.getByTestId("presenter-now")).toHaveText("on stage: BIBLE");
    await expect(stageFrame(page).locator("#fake-stage")).toHaveText("bible");
    // The page's steps: the tap, then the press, full, hold.
    const recorded = await until(() => steps(since), (s) => s.includes("hold"), "the hold's steps", 15000);
    expect(recorded).toEqual(["press", "tap", "press", "full", "hold"]);
    // The hub recorded the switch with the page that asked.
    const records = await until(
      hubEvents,
      (r) => r.some((e) => e.ev === "presenter_switch" && e.code === "timer" && e.ts >= since - 5000),
      "the switch's record",
    );
    const switched = records.filter((e) => e.ev === "presenter_switch" && e.code === "timer").pop();
    expect(switched.ok).toBe(true);
    expect(switched.from).toBe("worship");
    expect(switched.on_stage).toBe("timer");
    expect(switched.peer).toBeTruthy();
    // Only the hub wrote to Presenter: one POST, never one of the page's.
    const posts = (await fakeState()).hits.filter((h: any) => h.method === "POST");
    expect(posts.every((h: any) => h.path === "/stage/layout" && h.link)).toBe(true);
  });

  test("a hold slid off its key does nothing", async ({ page }) => {
    await restore();
    await openSurface(page);
    await openPresenter(page);
    const preach = keyOf(page, "preach");
    // The first move only anchors the press (#43 PR F), the next slides it off.
    const slide = [
      { type: "pointerdown" as const },
      { wait: 300 },
      { type: "pointermove" as const, dy: 0 },
      { type: "pointermove" as const, dx: 40, dy: 30 },
      { wait: 900 },
      { type: "pointerup" as const },
    ];
    await dispatchPointer(preach, slide, 33);
    await page.waitForTimeout(400);
    expect((await fakeState()).current).toBe("worship");
    await expect(preach).toHaveAttribute("data-full", "false");
  });

  test("with Presenter down the preview says so and the keys dim, and it comes back by itself", async ({ page }) => {
    await restore();
    await openSurface(page);
    const since = await pageNow(page);
    await openPresenter(page);
    await harness("/presenter/down");
    const down = page.getByTestId("presenter-down");
    await expect(down).toBeVisible({ timeout: 15000 });
    await expect(down.locator(".pst-down-title")).toHaveText("Presenter can’t be reached");
    await expect(down.locator(".pst-down-text")).toHaveText("trying again…");
    await expect(page.getByTestId("presenter-frame")).toHaveCount(0);
    await expect(page.getByTestId("presenter-now")).toHaveText("on stage: —");
    await expect(page.getByTestId("presenter-tab")).toHaveAttribute("data-offline", "true");
    await expect(page.getByTestId("presenter-dot")).toBeVisible();
    const keys = page.getByTestId("presenter-key");
    await expect(keys).toHaveCount(9);
    for (const state of await keys.evaluateAll((all) => all.map((k) => k.getAttribute("data-state")))) {
      expect(state).toBe("dim");
    }
    expect(await keys.first().evaluate((k) => getComputedStyle(k).pointerEvents)).toBe("none");
    // Back: a fresh frame, the keys live again.
    await harness("/presenter/up");
    await expect(stageFrame(page).locator("body")).toHaveAttribute("data-app", "loaded", { timeout: 15000 });
    await expect(page.getByTestId("presenter-down")).toHaveCount(0);
    await expect(keyOf(page, "worship")).toHaveAttribute("data-state", "on");
    await expect(page.getByTestId("presenter-dot")).toHaveCount(0);
    // The page's recorder: the frame, the panel, the frame again.
    const whats = await until(
      async () => pageEvents(await hubEvents()).filter((e) => e.ev === "presenter" && e.t >= since).map((e) => e.what),
      (w) => w.includes("down") && w.filter((x) => x === "frame").length >= 2,
      "the tab's steps",
      15000,
    );
    expect(whats.indexOf("down")).toBeGreaterThan(whats.indexOf("frame"));
    expect(whats.lastIndexOf("frame")).toBeGreaterThan(whats.indexOf("down"));
  });
});
