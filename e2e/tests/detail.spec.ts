import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  clipped,
  dispatchPointer,
  frames,
  harness,
  openDetail,
  openSurface,
  panning,
  ready,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

// The channel detail (#71 PR D; spec F27, D17; the approved mockup
// docs/mockups/channel-detail-v2.html): a hold of 500 ms on a strip's ☰ (at
// the end of its readout line) opens that channel over the whole screen: its
// name, group and instance, ← SPÄŤ NA MIX, the mute, Live's dB and a tall
// fader on the left, the pan (which left the strip) with Live's display
// string and STRED on the right. A shorter touch only shows the hint. It
// stays open through a new layout while its strip is in it, and TechAlert
// flashes over it.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
const HAND2 = track("Hand2 #");
/** `--mute` (a muted detail's mute) and the detail's dark mute. */
const RED = "rgb(230, 57, 70)";
const DARK = "rgb(24, 24, 38)";

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

/** The texts of the detail that must fit their boxes (#9). */
function texts(detail: Locator): [string, Locator][] {
  return [
    ["exit", detail.getByTestId("detail-exit")],
    ["name", detail.getByTestId("detail-name")],
    ["group and instance", detail.getByTestId("detail-sub")],
    ["mute", detail.getByTestId("strip-label")],
    ["dB", detail.getByTestId("db")],
    ["pan display", detail.getByTestId("pan-display")],
    ["STRED", detail.getByTestId("detail-centre")],
  ];
}

/** Every text of the detail inside its box. */
async function textsFit(detail: Locator) {
  for (const [name, el] of texts(detail)) {
    expect(await clipped(el), name).toEqual([]);
  }
}

test.describe("The channel detail", () => {
  test("a short touch on ☰ shows the hint and opens nothing; strips show no pan", async ({ page }) => {
    await openSurface(page);
    const s = strip(page, "Hand2 #");
    const menu = s.getByTestId("strip-menu");
    await expect(menu).toHaveAttribute("aria-label", "Detail kanála (podrž)");
    await expect(menu).toHaveText("☰");
    // ☰ ends the readout line: under the name button, after Live's dB.
    await expect(s.getByTestId("db")).not.toHaveText("");
    const mute = (await s.getByTestId("mute").boundingBox())!;
    const box = (await menu.boundingBox())!;
    const db = (await s.getByTestId("db").boundingBox())!;
    expect(box.y, "☰ under the name button").toBeGreaterThanOrEqual(mute.y + mute.height - 0.5);
    expect(box.x, "☰ after the dB").toBeGreaterThanOrEqual(db.x + db.width - 0.5);
    expect(await clipped(s.getByTestId("db")), "the dB beside ☰").toEqual([]);
    // No strip draws a pan any more (D17: it is the detail's).
    await expect(page.getByTestId("strip").first()).toBeVisible();
    await expect(page.getByTestId("pan")).toHaveCount(0);
    // A short touch: the hint (the stylesheet's text over the name button).
    await dispatchPointer(menu, [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await expect(menu).toHaveAttribute("data-hint", "true");
    await expect(menu).toHaveAttribute("data-holding", "false");
    const hint = await menu.evaluate((el) => getComputedStyle(el, "::after").content);
    expect(hint).toContain("na detail");
    // Nothing opens, not after the hold's time either, and the hint goes.
    await page.waitForTimeout(700);
    await expect(page.getByTestId("detail")).toHaveCount(0);
    await expect(menu).toHaveAttribute("data-hint", "false", { timeout: 2000 });
  });

  test("a hold fills ☰ and opens that channel's detail with its name, group and instance; ← SPÄŤ NA MIX returns to the page", async ({ page }) => {
    await openSurface(page);
    const s = strip(page, "Hand2 #", "master");
    const menu = s.getByTestId("strip-menu");
    await ready(s.getByTestId("fader"));
    // The page's subscriptions (`data-subs`): the detail adds its pan's.
    const surface = page.getByTestId("surface");
    const subs = Number(await surface.getAttribute("data-subs"));
    // A finger down: the button fills; past 500 ms the detail opens while
    // the finger still holds.
    await dispatchPointer(menu, [{ type: "pointerdown" }]);
    await expect(menu).toHaveAttribute("data-holding", "true");
    const detail = page.getByTestId("detail");
    await expect(detail).toBeVisible({ timeout: 2000 });
    await expect(menu).toHaveAttribute("data-holding", "false");
    await dispatchPointer(menu, [{ type: "pointerup" }]);
    await expect(menu).toHaveAttribute("data-hint", "false");
    await expect(detail).toHaveAttribute("data-track", "Hand2 #");
    await expect(detail).toHaveAttribute("data-instance", "master");
    await expect(surface).toHaveAttribute("data-subs", String(subs + 1));
    await expect(detail.getByTestId("detail-name")).toHaveText("Hand2");
    await expect(detail.getByTestId("detail-sub")).toHaveText("HANDS · master");
    // Over the whole screen, the column under it.
    const viewport = page.viewportSize()!;
    const area = (await detail.boundingBox())!;
    expect([area.x, area.y, area.width, area.height]).toEqual([0, 0, viewport.width, viewport.height]);
    const column = (await page.getByTestId("column").boundingBox())!;
    const hit = await page.evaluate(([x, y]) => document.elementFromPoint(x, y)?.closest('[data-testid="detail"]') !== null, [
      column.x + column.width / 2,
      column.y + column.height / 2,
    ]);
    expect(hit, "the detail over the column").toBe(true);
    // Its parts: the mute, Live's dB, the fader, the pan, STRED, all bound.
    for (const id of ["mute", "fader", "pan", "detail-centre"]) {
      await ready(detail.getByTestId(id));
    }
    await expect(detail.getByTestId("db")).not.toHaveText("");
    await expect(detail.getByTestId("status")).toHaveAttribute("data-state", "bound");
    await expect(detail.getByTestId("strip-label")).toHaveText("MUTE");
    await expect(detail.getByTestId("pan-display")).not.toHaveText("");
    await textsFit(detail);
    // Back to the mix: the pan's subscription goes with it.
    await detail.getByTestId("detail-exit").click();
    await expect(detail).toHaveCount(0);
    await expect(s.getByTestId("fader")).toBeVisible();
    await expect(surface).toHaveAttribute("data-subs", String(subs));
  });

  test("its fader drag reaches the track's volume and its mute toggles the track's mute", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.5);
    await live.set("band", HAND2, "mute", false);
    try {
      await openSurface(page);
      const detail = await openDetail(page, "Hand2 #");
      const fader = detail.getByTestId("fader");
      await ready(fader);
      await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
      // Taller than the strip's.
      const tall = (await fader.boundingBox())!.height;
      const strips = (await strip(page, "Hand2 #").getByTestId("fader").boundingBox())!.height;
      expect(tall, "the detail's fader travel (px)").toBeGreaterThan(strips);
      const { x, y } = await centre(fader);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 10; i++) await page.mouse.move(x, y - 8 * i);
      await page.mouse.up();
      await until(() => live.get("band", volume(HAND2), "value"), (v) => v > 0.55, "the volume to rise");
      // The strip under it follows Live too.
      await until(() => shown(strip(page, "Hand2 #").getByTestId("fader")), (v) => v > 0.55, "the strip's fader");
      // The mute: dark while the channel sounds, red while muted.
      const mute = detail.getByTestId("mute");
      await ready(mute);
      await expect(mute).toHaveAttribute("data-muted", "false");
      await expect(mute).toHaveCSS("background-color", DARK);
      await mute.click();
      await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted");
      await expect(mute).toHaveAttribute("data-muted", "true");
      await expect(mute).toHaveCSS("background-color", RED);
      await expect(strip(page, "Hand2 #").getByTestId("mute")).toHaveAttribute("data-muted", "true");
      await mute.click();
      await until(() => live.get("band", HAND2, "mute"), (v) => v === false, "audible again");
      await expect(mute).toHaveCSS("background-color", DARK);
    } finally {
      await live.set("band", volume(HAND2), "value", 0.5);
      await live.set("band", HAND2, "mute", false);
    }
  });

  test("its pan drag and STRED reach the track's panning; it shows Live's display string", async ({ page }) => {
    const PAN = panning(HAND2);
    const before = await live.get("band", PAN, "value");
    await live.set("band", PAN, "value", 0);
    try {
      await openSurface(page);
      const detail = await openDetail(page, "Hand2 #");
      const pan = detail.getByTestId("pan");
      const display = detail.getByTestId("pan-display");
      await ready(pan);
      await until(() => shown(pan), (v) => Math.abs(v) < 1e-3, "the pan centred");
      await expect(display).toHaveText("C");
      // A drag right: the first move anchors, the next four move 48 px over
      // the dot's travel, the pan's width less its 48 px dot (measured, not
      // the strip's 12 px).
      const width = (await pan.boundingBox())!.width;
      const dot = (await pan.locator(".pan-dot").boundingBox())!.width;
      expect(dot, "the detail's dot").toBe(48);
      const { x, y } = await centre(pan);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 5; i++) await page.mouse.move(x + 12 * i, y);
      await frames(page);
      await page.mouse.up();
      const want = (2 * 48) / (width - dot);
      const right = await until(() => live.get("band", PAN, "value"), (v) => Math.abs(v - want) < 0.01, `panning ${want.toFixed(3)}`);
      expect(right).toBeGreaterThan(0.05);
      await expect(display).toHaveText(/^\d+R$/);
      await expect(pan.locator(".pan-dot")).toHaveCSS("background-color", "rgb(52, 193, 220)");
      // STRED: a tap centres it.
      const centreButton = detail.getByTestId("detail-centre");
      await ready(centreButton);
      await centreButton.click();
      await until(() => live.get("band", PAN, "value"), (v) => v === 0, "centred");
      await expect(display).toHaveText("C");
      await expect(pan.locator(".pan-dot")).toHaveCSS("background-color", "rgb(100, 100, 100)");
      // Live's value from elsewhere: the dot reaches the pan's right end.
      await live.set("band", PAN, "value", 1);
      await expect(display).toHaveText("50R");
      await until(() => shown(pan), (v) => v === 1, "the pan at the right");
      await frames(page);
      const end = (await pan.locator(".pan-dot").boundingBox())!;
      const box = (await pan.boundingBox())!;
      expect(Math.abs(end.x + end.width - (box.x + box.width)), "the dot at the right end (px)").toBeLessThan(3);
    } finally {
      await live.set("band", PAN, "value", before);
    }
  });

  test("a new layout keeps it open while its strip is in it; one without the strip closes it", async ({ page }) => {
    const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
    const foh = changed.pages.find((p: any) => p.id === "foh");
    const group = foh.rows.flatMap((r: any) => r.sections).find((s: any) => s.kind === "group" && s.id === "foh-2");
    group.title = "DETAIL TEST";
    await openSurface(page);
    const detail = await openDetail(page, "Hand2 #");
    await expect(detail.getByTestId("detail-sub")).toHaveText("band");
    try {
      await harness("/hub/layout", { layout: changed });
      // The new layout on screen: the detail names the group's new title.
      await expect(detail.getByTestId("detail-sub")).toHaveText("DETAIL TEST · band", { timeout: 10_000 });
      await expect(detail).toHaveAttribute("data-track", "Hand2 #");
      await ready(detail.getByTestId("fader"));
      // The strip gone from the layout: the detail closes.
      group.controls = group.controls.filter((c: any) => !(c.kind === "strip" && c.binding.anchor.name === "Hand2 #"));
      await harness("/hub/layout", { layout: changed });
      await expect(detail).toHaveCount(0, { timeout: 10_000 });
      await expect(strip(page, "Hand2 #")).toHaveCount(0);
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(strip(page, "Hand2 #")).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("group-title").filter({ hasText: "DETAIL TEST" })).toHaveCount(0, { timeout: 10_000 });
  });

  test("TechAlert's wash flashes over it", async ({ page }) => {
    const alert = track("TechAlert #");
    await live.set("band", alert, "mute", true);
    try {
      await openSurface(page);
      await openDetail(page, "Hand2 #");
      const overlay = page.getByTestId("alert");
      await live.set("band", alert, "mute", false);
      await expect(overlay).toHaveAttribute("data-active", "true");
      // Sampled in the page every 15 ms for 900 ms (the blink is 300 ms):
      // while the wash shows, it is the element a touch would hit (its
      // touches turned on for the probe only: it takes none) at the
      // detail's centre, its exit and its pan's dot; never the detail.
      const samples = await overlay.evaluate(async (wash: HTMLElement) => {
        const detail = document.querySelector('[data-testid="detail"]')!;
        const at = (el: Element | null) => {
          const r = el!.getBoundingClientRect();
          return [r.left + r.width / 2, r.top + r.height / 2];
        };
        const points = [
          [innerWidth / 2, innerHeight / 2],
          at(detail.querySelector('[data-testid="detail-exit"]')),
          at(detail.querySelector(".pan-dot")),
        ];
        let over = 0;
        let under = 0;
        const end = performance.now() + 900;
        while (performance.now() < end) {
          const on = wash.getAttribute("data-visible") === "true" && getComputedStyle(wash).visibility === "visible";
          if (on) {
            wash.style.pointerEvents = "auto";
            const hits = points.map(([x, y]) => document.elementFromPoint(x, y) === wash);
            wash.style.pointerEvents = "";
            if (hits.every(Boolean)) over += 1;
            else under += 1;
          }
          await new Promise((done) => setTimeout(done, 15));
        }
        return { over, under, wide: wash.getBoundingClientRect().width === innerWidth };
      });
      expect(samples.under, "samples with the wash under the detail").toBe(0);
      expect(samples.over, "samples with the wash over the detail").toBeGreaterThan(0);
      expect(samples.wide, "the wash over the whole screen").toBe(true);
    } finally {
      await live.set("band", alert, "mute", true);
    }
  });
});

test.describe("The channel detail on a phone", () => {
  /** The detail at `width` x `height`: on the screen, no horizontal scroll, every text whole. */
  async function fits(page: Page, detail: Locator, width: number, height: number) {
    await page.setViewportSize({ width, height });
    await frames(page);
    const area = (await detail.boundingBox())!;
    expect([area.width, area.height], `${width}x${height}`).toEqual([width, height]);
    const scroll = await page.evaluate(() => [document.scrollingElement!.scrollWidth, innerWidth]);
    expect(scroll[0], "no horizontal scroll").toBeLessThanOrEqual(scroll[1]);
    expect(await detail.evaluate((el) => el.scrollWidth <= el.clientWidth), "nothing wider than the detail").toBe(true);
    for (const id of ["detail-exit", "mute", "fader", "pan", "detail-centre"]) {
      const box = (await detail.getByTestId(id).boundingBox())!;
      expect(box.x, `${id} on screen`).toBeGreaterThanOrEqual(-0.5);
      expect(box.x + box.width, `${id} on screen`).toBeLessThanOrEqual(width + 0.5);
      expect(box.y + box.height, `${id} on screen`).toBeLessThanOrEqual(height + 0.5);
    }
    await textsFit(detail);
  }

  test("upright, the pan sits beside the fader's top; on its side, the three columns; never a horizontal scroll", async ({ page }) => {
    await openSurface(page);
    const detail = await openDetail(page, "Hand2 #");
    await ready(detail.getByTestId("pan"));
    await fits(page, detail, 390, 844);
    const left = (await detail.getByTestId("detail-left").boundingBox())!;
    const right = (await detail.getByTestId("detail-right").boundingBox())!;
    expect(right.x, "the pan's box right of the fader's").toBeGreaterThanOrEqual(left.x + left.width);
    expect(Math.abs(right.y - left.y), "beside its top").toBeLessThan(1);
    expect(right.height, "the pan's box shorter than the fader's").toBeLessThan(left.height / 2);
    expect((await detail.getByTestId("fader").boundingBox())!.height, "the fader's travel (px)").toBeGreaterThan(400);
    await fits(page, detail, 844, 390);
    const middle = (await detail.getByTestId("detail-middle").boundingBox())!;
    const side = (await detail.getByTestId("detail-right").boundingBox())!;
    expect(side.x, "the pan on the right").toBeGreaterThanOrEqual(middle.x + middle.width);
    expect((await detail.getByTestId("fader").boundingBox())!.height, "the fader's travel (px)").toBeGreaterThan(150);
  });
});
