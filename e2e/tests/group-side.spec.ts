import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, openSurface, ready, selectPage, shown, strip, track, until, volume } from "./support/live";

// A group's name on its side (#127, the owner's variant B of
// docs/mockups/group-captions-v1.html): a run of a group's cells that
// starts with a strip shows no title over it, and its first strip names a
// titled group in its scale column, between the +6 and −6 dB labels, in the
// colour of the Live set the group's strips share (muted when they differ).
// A line of such runs gives the title's room to its faders; a run that
// starts with something else (a parameter fader, a button, a block) keeps
// its title over it, and its line the room. Both projects at the iPad's
// size, the imported synthetic layout.

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

/** The Live instance every strip of a group shares; null when they differ. */
function sharedInstance(group: any): string | null {
  const instances: string[] = group.controls.filter((c: any) => c.kind === "strip").map((c: any) => c.binding.instance);
  return instances.length > 0 && instances.every((i) => i === instances[0]) ? instances[0] : null;
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

/** The side label of `s` and the parts of its scale column, as page boxes. */
async function sideGeometry(s: Locator) {
  return s.evaluate((el) => {
    const box = (e: Element) => {
      const r = e.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
    };
    const scale = el.querySelector('[data-testid="scale"]')!;
    const label = scale.querySelector('[data-testid="group-side"]') as HTMLElement;
    return {
      label: box(label),
      scale: box(scale),
      ticks: [...scale.querySelectorAll(".tick")].map((t) => ({ db: t.getAttribute("data-db"), ...box(t) })),
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
  test("a line of strip runs gives the title's room to its faders; a run that starts with no strip keeps its title", async ({ page }) => {
    const rows = fohRows();
    // The premise: the upper row's runs all start with a strip, the lower
    // row holds one that does not (the backing-tracks fader's group).
    expect(rows[0].every(startsWithStrip)).toBe(true);
    expect(rows[1].some((g) => !startsWithStrip(g))).toBe(true);
    await openSurface(page);
    await expect(page.getByTestId("line")).toHaveCount(rows.length);
    for (let r = 0; r < rows.length; r++) {
      const kept = rows[r].some((g) => !startsWithStrip(g));
      await expect(line(page, r), `row ${r}`).toHaveAttribute("data-captions", String(kept));
      for (const group of rows[r]) {
        // Every run keeps its anchor (the tests find a strip's group by it).
        const anchor = line(page, r).locator(`[data-testid="group"][data-group="${group.id}"]`);
        await expect(anchor, `${group.id}: one run`).toHaveCount(1);
        if (startsWithStrip(group)) {
          await expect(anchor.locator(".group-title"), `${group.id}: no title over its strips`).toHaveCount(0);
          await expect(anchor.getByTestId("group-instance")).toHaveCount(0);
        } else {
          await expect(anchor.locator(".group-title"), `${group.id}: its title`).toBeVisible();
          await expect(anchor.getByTestId("group-title")).toHaveText(group.title ?? "");
        }
      }
    }
    // The upper line keeps no room, the lower one the title's.
    expect(await titleRoom(line(page, 0))).toBe(0);
    const room = await titleRoom(line(page, 1));
    expect(room, "the title's room").toBeGreaterThan(10);
    const upper = (await line(page, 0).boundingBox())!;
    const lower = (await line(page, 1).boundingBox())!;
    expect(Math.abs(upper.height - lower.height), "the premise: two lines as high").toBeLessThan(0.5);
    // A strip of each: the upper fills its line from the top, the lower
    // starts under the title's room.
    const top = strip(page, "Hand2 #");
    const bottom = strip(page, "Hand4 #", "master");
    const topBox = (await top.boundingBox())!;
    const bottomBox = (await bottom.boundingBox())!;
    expect(Math.abs(topBox.y - upper.y)).toBeLessThan(0.5);
    expect(Math.abs(topBox.height - upper.height)).toBeLessThan(0.5);
    expect(Math.abs(bottomBox.y - (lower.y + room))).toBeLessThan(0.5);
    // The fader's travel: the upper strip's is longer than what the lower
    // line (as every line before #127) leaves by exactly the title's room;
    // nothing else in the strip changed.
    await ready(top.getByTestId("fader"));
    await ready(bottom.getByTestId("fader"));
    const travel = async (s: Locator) => (await s.getByTestId("fader").boundingBox())!.height;
    const now = await travel(top);
    const before = await travel(bottom);
    console.log(`group-side travel (${test.info().project.name}): ${before} px with the title's room, ${now} px without (+${now - before} px, the room ${room} px)`);
    expect(Math.abs(now - before - room), `${now} px against ${before} px`).toBeLessThan(0.5);
    const head = async (s: Locator) => (await s.locator(".strip-head").boundingBox())!.height;
    expect(Math.abs((await head(top)) - (await head(bottom)))).toBeLessThan(0.5);
    for (const s of [top, bottom]) await expect(s.getByTestId("strip-label")).toHaveCSS("font-size", "17px");
  });

  test("each run's first strip names a titled group in the colour of the Live set its strips share", async ({ page }) => {
    const labelled = fohRows()
      .flat()
      .filter((g) => startsWithStrip(g) && g.title);
    // The premise: a band, a master and a mixed group.
    expect(labelled.map(sharedInstance).sort()).toEqual(["band", "master", null].sort());
    await openSurface(page);
    const colours: Record<string, string> = {
      band: await token(page, "--band"),
      master: await token(page, "--master"),
      mixed: await token(page, "--muted"),
    };
    expect(new Set(Object.values(colours)).size, "three colours").toBe(3);
    await expect(page.getByTestId("group-side")).toHaveCount(labelled.length);
    for (const group of labelled) {
      const first = group.controls[0];
      const s = strip(page, first.binding.anchor.name, first.binding.instance);
      const label = s.getByTestId("group-side");
      await expect(label).toHaveText(group.title);
      await expect(page.locator(`.slot[data-group="${group.id}"]`).getByTestId("group-side"), `${group.id}: one label`).toHaveCount(1);
      const shared = sharedInstance(group);
      if (shared) await expect(label).toHaveAttribute("data-instance", shared);
      else await expect(label).not.toHaveAttribute("data-instance");
      await expect(label).toHaveCSS("color", colours[shared ?? "mixed"]);
      await expect(label).toHaveCSS("pointer-events", "none");
      // Its strips' own instance tags: only where the strips differ.
      const strips = group.controls.filter((c: any) => c.kind === "strip").length;
      await expect(page.locator(`.slot[data-group="${group.id}"]`).getByTestId("strip-instance")).toHaveCount(shared ? 0 : strips);
      // Inside the scale column, between the +6 and −6 dB labels, over no
      // tick label, the whole title shown.
      const geo = await sideGeometry(s);
      const { label: box, scale } = geo;
      expect(box.left, `${group.id}: in the scale`).toBeGreaterThanOrEqual(scale.left - 0.5);
      expect(box.right).toBeLessThanOrEqual(scale.right + 0.5);
      expect(box.top).toBeGreaterThanOrEqual(scale.top - 0.5);
      expect(box.bottom).toBeLessThanOrEqual(scale.bottom + 0.5);
      const plus6 = geo.ticks.find((t) => t.db === "6")!;
      const minus6 = geo.ticks.find((t) => t.db === "-6")!;
      expect(box.top, `${group.id}: under +6`).toBeGreaterThanOrEqual(plus6.bottom);
      expect(box.bottom, `${group.id}: over −6`).toBeLessThanOrEqual(minus6.top);
      expect(box.bottom - box.top, "a real band").toBeGreaterThan(40);
      const over = geo.ticks.filter((t) => box.left < t.right && t.left < box.right && box.top < t.bottom && t.top < box.bottom);
      expect(over.map((t) => t.db), `${group.id}: over a tick label`).toEqual([]);
      expect(geo.overflow, `${group.id}: the whole title`).toEqual([0, 0]);
    }
    // An untitled group's strips name nothing.
    for (const group of fohRows().flat().filter((g) => startsWithStrip(g) && !g.title)) {
      await expect(page.locator(`.slot[data-group="${group.id}"]`).getByTestId("group-side"), group.id).toHaveCount(0);
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

  test("the cue page's block keeps its group's title and its line the room", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "cue");
    const cue = line(page, 0);
    await expect(cue).toHaveAttribute("data-captions", "true");
    const anchor = cue.locator('[data-testid="group"]');
    await expect(anchor).toHaveCount(1);
    await expect(anchor.locator(".group-title")).toBeVisible();
    await expect(page.getByTestId("group-side")).toHaveCount(0);
    const room = await titleRoom(cue);
    expect(room).toBeGreaterThan(10);
    const block = (await cue.locator(".block").boundingBox())!;
    expect(Math.abs(block.y - ((await cue.boundingBox())!.y + room)), "the block under the title's room").toBeLessThan(0.5);
    await selectPage(page, "foh");
  });
});
