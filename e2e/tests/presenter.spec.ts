import type { Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { HARNESS, LiveClient, dispatchPointer, frames, harness, hubEvents, openSurface, pageEvents, rightHalfLive, until } from "./support/live";

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
    // The hub's own socket and the pane's frame are Presenter's previews:
    // two sockets, both through the hub (the column's miniature, and its
    // socket, are gone while the tab is open).
    await expect(mini(page)).toHaveCount(0);
    await until(fakeState, (s) => s.sockets === 2, "the hub's and the pane's sockets");
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

// The column's stage miniature (1.37.0; way A of the owner's approved
// docs/mockups/presenter-stage-v2.html): the same stage frame as the
// PRESENTER pane, scaled to the column's width, in the rail's free room
// above TECHALERT; a dim STAGE caption with the layout on stage; a tap
// opens PRESENTER, and while it is open the miniature is not there. Where
// the rail has no free room, and on a phone, it is not there either:
// nothing gets shorter for it.
const mini = (page: Page) => page.getByTestId("stage-mini");
const miniFrame = (page: Page) => page.frameLocator('[data-testid="mini-frame"]');

/** The rail's rows (its resolved tracks) and the travel of every fader on the screen (px). */
async function heights(page: Page): Promise<{ rows: string; travel: number[] }> {
  const rows = await page.locator('[data-testid="rail"] .rail-main').evaluate((m) => getComputedStyle(m).gridTemplateRows);
  const travel = await page
    .getByTestId("fader")
    .evaluateAll((all) => all.filter((f) => f.getBoundingClientRect().height > 0).map((f) => f.getBoundingClientRect().height));
  return { rows, travel };
}

test.describe("Presenter's stage miniature in the column", () => {
  test.afterEach(restore);

  test.describe("on the iPad's screen", () => {
    test.use({ viewport: { width: 1194, height: 834 } });

    test("shows the stage above TECHALERT, a tap opens PRESENTER, hidden while it is open, nothing shorter", async ({ page }) => {
      await restore();
      await openSurface(page);
      const box = mini(page);
      await expect(box).toBeVisible();
      await expect(box.locator(".mini-cap span")).toHaveText("STAGE");
      await expect(page.getByTestId("mini-note")).toHaveText("WORSHIP");
      await expect(box).toHaveAttribute("data-offline", "false");
      // Calm: dim text, no fill.
      expect(await box.evaluate((b) => getComputedStyle(b).backgroundColor)).toBe("rgb(16, 18, 24)");
      // The stage page through the hub, scaled from 1920 x 1080, no touch.
      const frame = page.getByTestId("mini-frame");
      await expect(frame).toHaveAttribute("src", "/stage?preview=1");
      expect(await frame.getAttribute("sandbox")).toBeNull();
      await expect(miniFrame(page).locator("body")).toHaveAttribute("data-app", "loaded");
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("worship");
      expect(await frame.evaluate((f) => getComputedStyle(f).pointerEvents)).toBe("none");
      const area = (await page.getByTestId("mini-stage").boundingBox())!;
      expect(Math.abs(area.width / area.height - 16 / 9)).toBeLessThan(0.02);
      const shown = (await frame.boundingBox())!;
      expect(Math.abs(shown.width - area.width)).toBeLessThan(1.5);
      expect(Math.abs(shown.height - area.height)).toBeLessThan(1.5);
      // In the rail, between its buttons and TECHALERT, the column's width.
      const rail = page.getByTestId("rail");
      await expect(rail.getByTestId("stage-mini")).toHaveCount(1);
      const at = (await box.boundingBox())!;
      const main = (await rail.locator(".rail-main").boundingBox())!;
      const foot = (await rail.locator(".rail-foot").boundingBox())!;
      const railBox = (await rail.boundingBox())!;
      expect(at.y).toBeGreaterThanOrEqual(main.y + main.height);
      expect(at.y + at.height).toBeLessThanOrEqual(foot.y);
      expect(Math.abs(at.width - railBox.width)).toBeLessThan(1);
      // Every rail row at its full 44 px with it there.
      const withMini = await heights(page);
      expect(withMini.rows.split(" ").every((t) => t === "44px"), withMini.rows).toBe(true);
      // A tap opens PRESENTER; the miniature goes while it is open.
      const pageTab = page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-selected="true"]');
      await expect(pageTab).toHaveCount(1);
      const pageId = (await pageTab.getAttribute("data-page"))!;
      const since = await pageNow(page);
      await box.dispatchEvent("pointerdown");
      await expect(page.getByTestId("presenter-tab")).toHaveAttribute("data-selected", "true");
      await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "presenter");
      await expect(mini(page)).toHaveCount(0);
      await expect(page.getByTestId("mini-frame")).toHaveCount(0);
      // Nothing changed height: the rail's rows and the right half's faders.
      const open = await heights(page);
      expect(open.rows).toBe(withMini.rows);
      const right = await page
        .locator('.slot[data-side="right"] [data-testid="fader"]')
        .evaluateAll((all) => all.map((f) => f.getBoundingClientRect().height));
      expect(right.length).toBeGreaterThan(0);
      for (const h of right) expect(withMini.travel).toContain(h);
      // Back to the mixer page: the miniature again.
      await page.locator(`[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-page="${pageId}"]`).dispatchEvent("pointerdown");
      await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");
      await expect(mini(page)).toBeVisible();
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("worship");
      expect(await heights(page)).toEqual(withMini);
      // The page's recorder: the miniature's frame and its tap.
      const whats = await until(
        async () => pageEvents(await hubEvents()).filter((e) => e.ev === "presenter" && e.t >= since).map((e) => e.what),
        (w) => w.includes("mini_tap") && w.includes("mini_frame"),
        "the miniature's steps",
        15000,
      );
      expect(whats.indexOf("mini_tap")).toBeLessThan(whats.indexOf("open"));
      // At the iPad's full size the picture is the rail's whole width (less its 10 px box).
      const shownFits = await until(
        async () =>
          pageEvents(await hubEvents()).filter((e) => e.ev === "presenter" && e.what === "mini_fit" && e.t >= since && e.w !== undefined && e.vw === 1194),
        (f) => f.length > 0,
        "a shown mini_fit at 1194 x 834",
        15000,
      );
      const fit = shownFits.at(-1)!;
      expect(fit.w).toBeCloseTo(railBox.width - 10, 0);
      expect(fit.vh).toBe(834);
    });

    test("follows a switch of the stage's layout", async ({ page }) => {
      await restore();
      await openSurface(page);
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("worship");
      await harness("/presenter/switch", { code: "timer" });
      await expect(page.getByTestId("mini-note")).toHaveText("TIMER");
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("timer");
      await harness("/presenter/switch", { code: "bible" });
      await expect(page.getByTestId("mini-note")).toHaveText("BIBLE");
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("bible");
    });

    test("with Presenter down says offline in red and dims, and comes back by itself", async ({ page }) => {
      await restore();
      await openSurface(page);
      await expect(miniFrame(page).locator("body")).toHaveAttribute("data-app", "loaded");
      await harness("/presenter/down");
      const box = mini(page);
      await expect(box).toHaveAttribute("data-offline", "true", { timeout: 15000 });
      await expect(page.getByTestId("mini-note")).toHaveText("offline");
      expect(await page.getByTestId("mini-note").evaluate((e) => getComputedStyle(e).color)).toBe("rgb(255, 59, 48)");
      await expect(page.getByTestId("mini-frame")).toHaveCount(0);
      const stage = page.getByTestId("mini-stage");
      await expect(stage).toHaveAttribute("data-preview", "down");
      expect(await stage.evaluate((s) => getComputedStyle(s).opacity)).toBe("0.35");
      await harness("/presenter/up");
      await expect(miniFrame(page).locator("body")).toHaveAttribute("data-app", "loaded", { timeout: 15000 });
      await expect(box).toHaveAttribute("data-offline", "false");
      await expect(page.getByTestId("mini-note")).toHaveText("WORSHIP");
    });
  });

  // Shorter screens (#90, 1.38.2: the owner's iPad over the internet never
  // showed it; Safari's bars take height, another model is smaller; the
  // main session's option A): the miniature never goes below its least
  // picture (40 px high, 16:9, narrower than the rail), the rail's rows give
  // that room up (shorter than 44 px, never under their 24 px floor), and
  // the faders keep their height.
  for (const size of [
    { width: 1180, height: 740 },
    { width: 1024, height: 700 },
  ]) {
    test.describe(`on a shorter screen, ${size.width}×${size.height}`, () => {
      test.use({ viewport: size });

      test("is there at least at its least size, the rail's rows give up room, the faders keep their height", async ({ page }) => {
        await restore();
        // Page times are epoch ms on the runner's one clock (performance.timeOrigin + now).
        const since = Date.now();
        await openSurface(page);
        const box = mini(page);
        await expect(box).toBeVisible();
        await expect(miniFrame(page).locator("#fake-stage")).toHaveText("worship");
        await expect(page.getByTestId("mini-note")).toHaveText("WORSHIP");
        const rail = page.getByTestId("rail");
        // The settled size: the drawn picture equal to the page's last shown mini_fit (a first measure can come before the column settles).
        const lastShown = async () =>
          pageEvents(await hubEvents())
            .filter((e) => e.ev === "presenter" && e.what === "mini_fit" && e.t >= since && e.vw === size.width && e.vh === size.height && e.w !== undefined)
            .at(-1);
        await until(
          async () => ({ fit: await lastShown(), box: await page.getByTestId("mini-stage").boundingBox() }),
          ({ fit, box }) => !!fit && !!box && Math.abs(fit.w - box.width) < 0.6 && Math.abs(fit.h - box.height) < 0.6,
          "the drawn picture at the last mini_fit's size",
          15000,
        );
        const railBox = (await rail.boundingBox())!;
        const area = (await page.getByTestId("mini-stage").boundingBox())!;
        test.info().annotations.push({ type: `mini ${size.width}x${size.height}`, description: `${area.width.toFixed(1)} x ${area.height.toFixed(1)} in a ${railBox.width.toFixed(1)} px rail` });
        // Smaller than the rail's width less the box (10 px), at least 40 px high, 16:9.
        expect(area.width).toBeLessThan(railBox.width - 10 - 1);
        expect(area.height).toBeGreaterThanOrEqual(39.9);
        expect(Math.abs(area.width / area.height - 16 / 9)).toBeLessThan(0.02);
        // Centred under the caption, the stage scaled to the picture.
        const at = (await box.boundingBox())!;
        expect(Math.abs(area.x + area.width / 2 - (at.x + at.width / 2))).toBeLessThan(1);
        const shown = (await page.getByTestId("mini-frame").boundingBox())!;
        expect(Math.abs(shown.width - area.width)).toBeLessThan(1.5);
        expect(Math.abs(shown.height - area.height)).toBeLessThan(1.5);
        // Between the buttons and TECHALERT; the rows shorter than 44 px for it, never under 24 px.
        const main = (await rail.locator(".rail-main").boundingBox())!;
        const foot = (await rail.locator(".rail-foot").boundingBox())!;
        expect(at.y).toBeGreaterThanOrEqual(main.y + main.height - 0.5);
        expect(at.y + at.height).toBeLessThanOrEqual(foot.y + 0.5);
        const withMini = await heights(page);
        const rows = withMini.rows.split(" ").map((t) => parseFloat(t));
        test.info().annotations.push({ type: `rows ${size.width}x${size.height}`, description: withMini.rows });
        expect(rows.length).toBeGreaterThan(0);
        for (const r of rows) {
          expect(r).toBeGreaterThanOrEqual(24);
          expect(r).toBeLessThan(44);
        }
        // The faders' height does not depend on it: PRESENTER open (the miniature gone) leaves the right half's as they were.
        await page.getByTestId("presenter-tab").click();
        await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "presenter");
        await expect(mini(page)).toHaveCount(0);
        const right = await page
          .locator('.slot[data-side="right"] [data-testid="fader"]')
          .evaluateAll((all) => all.map((f) => f.getBoundingClientRect().height));
        expect(right.length).toBeGreaterThan(0);
        for (const h of right) expect(withMini.travel).toContain(h);
        // The recorder: the decision with the viewport, the rail and the picture, then the tab's.
        const fits = await until(
          async () =>
            pageEvents(await hubEvents()).filter((e) => e.ev === "presenter" && e.what === "mini_fit" && e.t >= since && e.vw === size.width && e.vh === size.height),
          (f) => f.some((e) => e.hide === "open") && f.some((e) => e.w !== undefined),
          "the miniature's mini_fit records",
          15000,
        );
        // The box mounted once: a new size restyles it, its frame is not mounted again (the tab's own record came after it).
        const miniFrames = pageEvents(await hubEvents()).filter((e) => e.ev === "presenter" && e.what === "mini_frame" && e.t >= since);
        expect(miniFrames.length, "one mini_frame since the open").toBe(1);
        // The last shown decision (a first measure can come before the column settles).
        const shownFit = fits.filter((e) => e.w !== undefined).at(-1)!;
        expect(Math.abs(shownFit.w - area.width)).toBeLessThan(0.6);
        expect(Math.abs(shownFit.h - area.height)).toBeLessThan(0.6);
        expect(shownFit.rows).toBe(withMini.rows.split(" ").length);
        expect(shownFit.rail_w).toBeCloseTo(railBox.width, 0);
        expect(shownFit.h).toBeGreaterThanOrEqual(40);
        expect(shownFit.hide).toBeUndefined();
      });
    });
  }

  // #90's DECIDED of 2026-11-09 (the live check of 1.38.2 at 1024 x 700):
  // rows shrunk for the miniature cut STAGE's second line (`STOP · open`) in
  // half (the grid squeezes the line's box under its 12 px and its own clip
  // cuts the letters). A row shorter than its second line needs hides that
  // line whole: every visible rail row's second line is either whole inside
  // its button or not drawn, and the row keeps its name and AUTO mark.
  // 1024 x 640 too: shorter rows still (CI's 1024 x 700 rows are about 33 px
  // with every extra tab in the column; a harness without some of them has
  // taller rows there), and the iPad's 1194 x 834, whose 44 px rows draw it.
  // Drawn exactly while the row's content box has the 28 px it needs.
  for (const size of [
    { width: 1194, height: 834 },
    { width: 1024, height: 700 },
    { width: 1024, height: 640 },
  ]) {
    test.describe(`at ${size.width}×${size.height}, the rail's second lines`, () => {
      test.use({ viewport: size });

      test("are whole inside their row or not drawn, never cut", async ({ page }) => {
        await restore();
        const live = await LiveClient.open();
        try {
          live.setHub("stage_aut", true);
          await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on");
          await openSurface(page);
          await expect(mini(page)).toBeVisible();
          await expect(page.getByTestId("stage-mics")).toHaveAttribute("data-aut", /run|held/);
          const rows = (await heights(page)).rows.split(" ").map((t) => parseFloat(t));
          // 44 px rows on the iPad's screen; shorter ones give the miniature their room.
          if (size.height === 834) expect(Math.min(...rows)).toBe(44);
          else expect(Math.max(...rows)).toBeLessThan(44);
          const looks = await page.locator('[data-testid="rail"] .rule-btn').evaluateAll((all) =>
            all.map((b) => {
              const box = b.getBoundingClientRect();
              const style = getComputedStyle(b);
              const content =
                b.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
              const sub = b.querySelector(".btn-sub") as HTMLElement;
              const subBox = sub.getBoundingClientRect();
              const label = (b.querySelector(".btn-text") as HTMLElement).getBoundingClientRect();
              const badge = b.querySelector(".aut-badge")?.getBoundingClientRect();
              return {
                id: b.getAttribute("data-testid"),
                text: sub.textContent,
                drawn: getComputedStyle(sub).display !== "none" && getComputedStyle(sub).visibility !== "hidden",
                // Whole: its box holds its line (no clip of its own cuts it) and lies inside the button.
                whole: sub.scrollHeight <= sub.clientHeight && subBox.top >= box.top && subBox.bottom <= box.bottom,
                sub: [subBox.top, subBox.bottom, sub.clientHeight, sub.scrollHeight],
                box: [box.top, box.bottom],
                content,
                label: label.height > 0 && label.top >= box.top - 1 && label.bottom <= box.bottom + 1,
                badge: badge ? badge.height > 0 && badge.top >= box.top && badge.bottom <= box.bottom : null,
              };
            }),
          );
          test.info().annotations.push({ type: `rule rows ${size.width}x${size.height}`, description: `${JSON.stringify(rows)} ${JSON.stringify(looks)}` });
          expect(looks.length).toBeGreaterThan(0);
          expect(looks.find((l) => l.id === "stage-mics")?.text).not.toBe("");
          if (size.height === 834) expect(looks.every((l) => l.drawn && l.whole)).toBe(true);
          if (size.height === 640) expect(looks.some((l) => !l.drawn)).toBe(true);
          for (const l of looks) {
            expect(!l.drawn || l.whole, `${l.id}'s second line ${JSON.stringify(l)}`).toBe(true);
            expect(l.drawn, `${l.id}'s second line drawn exactly with room for it ${JSON.stringify(l)}`).toBe(l.content >= 28 && l.text !== "");
            expect(l.label, `${l.id}'s name inside its row`).toBe(true);
            if (l.badge !== null) expect(l.badge, `${l.id}'s AUTO mark inside its row`).toBe(true);
          }
        } finally {
          live.setHub("stage_aut", false);
          await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
          live.close();
        }
      });
    });
  }

  // 520 x 1000 is a phone by its width (the 520 px query, inclusive) whose
  // open column has room for it (the rail 600 px of room, 475 needed): only
  // the phone's rule keeps it out there.
  for (const size of [
    { width: 844, height: 390 },
    { width: 520, height: 1000 },
  ]) {
    test.describe(`on a phone, ${size.width}×${size.height}`, () => {
      test.use({ viewport: size });

      test("is never in the column behind ☰", async ({ page }) => {
        await restore();
        await openSurface(page);
        await page.getByTestId("thin-burger").dispatchEvent("pointerdown");
        await expect(page.getByTestId("page")).toHaveAttribute("data-column-open", "true");
        await frames(page, 4);
        await expect(mini(page)).toHaveCount(0);
        await expect(page.getByTestId("mini-frame")).toHaveCount(0);
      });
    });
  }

  // One px wider is a tablet: the same rail has room, and it is there.
  test.describe("just past a phone, 521×1000", () => {
    test.use({ viewport: { width: 521, height: 1000 } });

    test("is in the column", async ({ page }) => {
      await restore();
      await openSurface(page);
      await expect(mini(page)).toBeVisible();
      await expect(miniFrame(page).locator("#fake-stage")).toHaveText("worship");
    });
  });
});
