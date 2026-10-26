import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  openSurface,
  ready,
  resetGroups,
  selectPage,
  setGroups,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

// A group's name on its side (#127, the owner's variant B of
// docs/mockups/group-captions-v1.html; the review rulings 6224257660): a run
// of a group's cells that starts with a strip shows no title over it, and its
// first strip names the group in its scale column, between the +6 and −6 dB
// labels: its title, or an untitled group's shared Live set (BAND, MASTER),
// in the colour of that set (muted when the strips differ). A run that starts
// with something else (a parameter fader, a button, a block) keeps its title
// over it only when there is one to show; a line keeps the title's room only
// when such a title shows on it, and gives it to its faders otherwise. Both
// projects at the iPad's size, the imported synthetic layout.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

test.use({ viewport: { width: 1194, height: 834 } });

/** The FOH page's groups on its first sub-page, row by row. */
function fohRows(): any[][] {
  const foh = JSON.parse(readFileSync(LAYOUT, "utf-8")).pages.find((p: any) => p.id === "foh");
  return foh.rows.map((row: any) =>
    row.sections.flatMap((s: any) => (s.kind === "pager" ? s.pages[0].sections : [s])),
  );
}

/** Whether a group's run starts with a strip (the fixture's groups are never cut at this size). */
const startsWithStrip = (group: any) => group.controls[0]?.kind === "strip";

/** The Live instance every strip of a group shares; null when they differ or there is none. */
function sharedInstance(group: any): string | null {
  const instances: string[] = group.controls.filter((c: any) => c.kind === "strip").map((c: any) => c.binding.instance);
  return instances.length > 0 && instances.every((i) => i === instances[0]) ? instances[0] : null;
}

/** A group's name: its title, else its shared Live set in capitals; null when it has neither. */
function groupName(group: any): string | null {
  if (group.title) return group.title;
  const shared = sharedInstance(group);
  return shared ? shared.toUpperCase() : null;
}

/** A colour token of the page (`--band`, …) as a computed colour. */
function token(page: Page, name: string): Promise<string> {
  return page.evaluate((property) => {
    const probe = document.createElement("i");
    probe.style.color = `var(${property})`;
    document.body.append(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  }, name);
}

/** A line of the page by its number. */
const line = (page: Page, n: number) => page.locator(`[data-testid="line"][data-line="${n}"]`);

/** The room a line keeps at its top for its groups' titles (px). */
const titleRoom = (l: Locator) => l.evaluate((e) => parseFloat(getComputedStyle(e).paddingTop));

/** A strip's fader travel (px). */
async function travel(s: Locator): Promise<number> {
  await ready(s.getByTestId("fader"));
  return (await s.getByTestId("fader").boundingBox())!.height;
}

/** The side label of `s` and the parts of its info column (#151), as page boxes. */
async function sideGeometry(s: Locator) {
  return s.evaluate((el) => {
    const box = (e: Element) => {
      const r = e.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
    };
    const col = el.querySelector('[data-testid="strip-col"]')!;
    const label = col.querySelector('[data-testid="group-side"]') as HTMLElement;
    return {
      label: box(label),
      col: box(col),
      lufs: box(col.querySelector('[data-testid="lufs"]')!),
      overflow: [label.scrollWidth - label.clientWidth, label.scrollHeight - label.clientHeight],
    };
  });
}

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("A group's name on its side (#127)", () => {
  test("no line keeps room for a title it does not show: every fixture line gives it to its faders", async ({ page }) => {
    const rows = fohRows();
    // The premise: the runs that start with no strip (the backing-tracks
    // fader's, the cue page's block) have no title and share no Live set.
    const bare = rows.flat().filter((g) => !startsWithStrip(g));
    expect(bare.length).toBeGreaterThan(0);
    for (const group of bare) expect(groupName(group), group.id).toBeNull();
    await openSurface(page);
    await expect(page.getByTestId("line")).toHaveCount(rows.length);
    for (let r = 0; r < rows.length; r++) {
      await expect(line(page, r), `row ${r}`).toHaveAttribute("data-captions", "false");
      expect(await titleRoom(line(page, r)), `row ${r}`).toBe(0);
      for (const group of rows[r]) {
        // Every run keeps its anchor (the tests find a strip's group by it),
        // with no title over it.
        const anchor = line(page, r).locator(`[data-testid="group"][data-group="${group.id}"]`);
        await expect(anchor, `${group.id}: one run`).toHaveCount(1);
        await expect(anchor.locator(".group-title"), `${group.id}: no title`).toHaveCount(0);
      }
      // A strip fills its line from the top.
      const s = line(page, r).getByTestId("strip").first();
      const box = (await s.boundingBox())!;
      const lineBox = (await line(page, r).boundingBox())!;
      expect(Math.abs(box.y - lineBox.y), `row ${r}: the strip at the top`).toBeLessThan(0.5);
      expect(Math.abs(box.height - lineBox.height), `row ${r}: the strip as high as its line`).toBeLessThan(0.5);
    }
    // The cue page's untitled block: no title, no room, no side label.
    await selectPage(page, "cue");
    await expect(line(page, 0)).toHaveAttribute("data-captions", "false");
    await expect(line(page, 0).locator(".group-title")).toHaveCount(0);
    await expect(page.getByTestId("group-side")).toHaveCount(0);
    expect(await titleRoom(line(page, 0))).toBe(0);
    await selectPage(page, "foh");
  });

  test("a title over a run that starts with no strip keeps its line's room: the faders' measured gain", async ({ page }) => {
    // Generic titles on the fixture's backing-tracks fader group (the FOH
    // page's lower line) and the cue page's block.
    const groups = { "foh-4": { title: "BACKING" }, "cue-1": { title: "CUE" } };
    try {
      await setGroups(groups);
      await openSurface(page);
      const upper = line(page, 0);
      const lower = line(page, 1);
      const caption = lower.locator('[data-testid="group"][data-group="foh-4"] .group-title');
      await expect(caption).toBeVisible({ timeout: 10_000 });
      await expect(caption.getByTestId("group-title")).toHaveText("BACKING");
      await expect(lower).toHaveAttribute("data-captions", "true");
      await expect(upper).toHaveAttribute("data-captions", "false");
      expect(await titleRoom(upper)).toBe(0);
      const room = await titleRoom(lower);
      expect(room, "the title's room").toBeGreaterThan(10);
      const upperBox = (await upper.boundingBox())!;
      const lowerBox = (await lower.boundingBox())!;
      expect(Math.abs(upperBox.height - lowerBox.height), "the premise: two lines as high").toBeLessThan(0.5);
      // A strip of each: the upper fills its line, the lower starts under
      // the title's room.
      const top = strip(page, "Hand2 #");
      const bottom = strip(page, "Hand4 #", "master");
      const topBox = (await top.boundingBox())!;
      const bottomBox = (await bottom.boundingBox())!;
      expect(Math.abs(topBox.y - upperBox.y)).toBeLessThan(0.5);
      expect(Math.abs(topBox.height - upperBox.height)).toBeLessThan(0.5);
      expect(Math.abs(bottomBox.y - (lowerBox.y + room))).toBeLessThan(0.5);
      // The fader's travel: the upper strip's is longer than the lower's
      // (what every line left before #127) by exactly the title's room;
      // nothing else in the strip changed.
      const now = await travel(top);
      const before = await travel(bottom);
      console.log(
        `group-side travel (${test.info().project.name}): ${before} px with the title's room, ${now} px without (+${now - before} px, the room ${room} px)`,
      );
      expect(Math.abs(now - before - room), `${now} px against ${before} px`).toBeLessThan(0.5);
      const head = async (s: Locator) => (await s.locator(".strip-head").boundingBox())!.height;
      expect(Math.abs((await head(top)) - (await head(bottom)))).toBeLessThan(0.5);
      for (const s of [top, bottom]) await expect(s.getByTestId("strip-label")).toHaveCSS("font-size", "17px");
      // The cue page's titled block: its title, and the room over it.
      await selectPage(page, "cue");
      const cue = line(page, 0);
      await expect(cue).toHaveAttribute("data-captions", "true");
      await expect(cue.locator('[data-testid="group"] .group-title')).toBeVisible();
      await expect(cue.getByTestId("group-title")).toHaveText("CUE");
      await expect(page.getByTestId("group-side"), "a block carries no side label").toHaveCount(0);
      const cueRoom = await titleRoom(cue);
      expect(cueRoom).toBe(room);
      const block = (await cue.locator(".block").boundingBox())!;
      expect(Math.abs(block.y - ((await cue.boundingBox())!.y + cueRoom)), "the block under the title's room").toBeLessThan(0.5);
      await selectPage(page, "foh");
    } finally {
      await resetGroups(Object.keys(groups));
    }
  });

  test("an untitled coloured block shows its group's colour as its top edge, at no height", async ({ page }) => {
    // #127 re-review m1: with no title to show, the block's group keeps no
    // title over it, so the block wears the colour itself, as a strip does.
    const COLOUR = "#5480E4";
    try {
      await setGroups({ "cue-1": { color: COLOUR } });
      await openSurface(page);
      await selectPage(page, "cue");
      const cue = line(page, 0);
      const block = cue.locator(".block");
      await expect(block).toBeVisible({ timeout: 10_000 });
      await expect(cue).toHaveAttribute("data-captions", "false");
      await expect(cue.locator(".group-title")).toHaveCount(0);
      await expect(page.getByTestId("group-side")).toHaveCount(0);
      const edge = await block.evaluate((el) => {
        const before = getComputedStyle(el, "::before");
        return { color: before.borderTopColor, width: before.borderTopWidth, style: before.borderTopStyle, touch: before.pointerEvents };
      });
      expect(edge).toEqual({ color: "rgb(84, 128, 228)", width: "3px", style: "solid", touch: "none" });
      // At no height: the block starts at its line's top, as without it.
      const blockBox = (await block.boundingBox())!;
      expect(Math.abs(blockBox.y - (await cue.boundingBox())!.y)).toBeLessThan(0.5);
      await selectPage(page, "foh");
    } finally {
      await resetGroups(["cue-1"]);
    }
  });

  test("each run's first strip names its group in grey, in its info column", async ({ page }) => {
    // #151 (mockup v8): one colour per strip, the track's own, so the
    // group's name is calm grey whatever Live set its strips share.
    const groups = fohRows().flat().filter(startsWithStrip);
    const named = groups.filter((g) => groupName(g) !== null);
    // The premise: titled band, master and mixed groups, and untitled band
    // and master ones.
    expect(named.filter((g) => g.title).map(sharedInstance).sort()).toEqual(["band", "master", null].sort());
    expect(named.filter((g) => !g.title).map(sharedInstance).sort()).toEqual(["band", "master"]);
    await openSurface(page);
    const grey = await token(page, "--faint");
    expect(grey).toBe("rgb(92, 92, 120)");
    await expect(page.getByTestId("group-side")).toHaveCount(named.length);
    for (const group of named) {
      const first = group.controls[0];
      const s = strip(page, first.binding.anchor.name, first.binding.instance);
      const label = s.getByTestId("group-side");
      await expect(label).toHaveText(groupName(group)!);
      await expect(page.locator(`.slot[data-group="${group.id}"]`).getByTestId("group-side"), `${group.id}: one label`).toHaveCount(1);
      const shared = sharedInstance(group);
      if (shared) await expect(label).toHaveAttribute("data-instance", shared);
      else await expect(label).not.toHaveAttribute("data-instance");
      await expect(label).toHaveCSS("color", grey);
      await expect(label).toHaveCSS("pointer-events", "none");
      // Its strips' own instance tags: only where the strips differ.
      const strips = group.controls.filter((c: any) => c.kind === "strip").length;
      await expect(page.locator(`.slot[data-group="${group.id}"]`).getByTestId("strip-instance")).toHaveCount(shared ? 0 : strips);
      // In the info column: 40 px under its top, 96 px long, under the
      // LUFS row, the whole name shown.
      const geo = await sideGeometry(s);
      const { label: box, col } = geo;
      expect(box.left, `${group.id}: in the column`).toBeGreaterThanOrEqual(col.left - 0.5);
      expect(box.right).toBeLessThanOrEqual(col.right + 0.5);
      expect(Math.abs(box.top - col.top - 40), `${group.id}: 40 px under the column's top`).toBeLessThan(0.5);
      expect(Math.abs(box.bottom - box.top - 96), `${group.id}: 96 px long`).toBeLessThan(0.5);
      expect(box.top, `${group.id}: under the LUFS row`).toBeGreaterThanOrEqual(geo.lufs.bottom - 0.5);
      expect(geo.overflow, `${group.id}: the whole name`).toEqual([0, 0]);
    }
  });

  test("a finger on the side label drags the fader under it", async ({ page }) => {
    const HAND4 = volume(track("Hand4 #"));
    const before = await live.get("master", HAND4, "value");
    try {
      await live.set("master", HAND4, "value", 0.5);
      await openSurface(page);
      const s = strip(page, "Hand4 #", "master");
      const fader = s.getByTestId("fader");
      await ready(fader);
      await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
      const box = (await s.getByTestId("group-side").boundingBox())!;
      const x = Math.round(box.x + box.width / 2);
      const y = Math.round(box.y + box.height / 2);
      // The label takes no touch: the point is the fader's.
      expect(await fader.evaluate((f, [px, py]) => f.contains(document.elementFromPoint(px, py)), [x, y])).toBe(true);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 10; i++) await page.mouse.move(x, y + 8 * i);
      await page.mouse.up();
      await until(() => live.get("master", HAND4, "value"), (v) => v < 0.45, "the volume to fall");
    } finally {
      await live.set("master", HAND4, "value", before);
    }
  });
});
