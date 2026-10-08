import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { clipped, harness, hubSubscriptions, openSurface, selectPage, strip, until } from "./support/live";

// Pages, the pager, the rail, the rows of sections (the redesign, #21; spec
// §4.2) on both sides of the control column (#63): the imported synthetic
// layout (schema 2), only the controls on screen subscribed.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
const layout = () => JSON.parse(readFileSync(LAYOUT, "utf-8"));

/** `#RRGGBB` as a computed `rgb(r, g, b)`. */
function rgb(color: string): string {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(color);
  if (!m) throw new Error(`not a layout colour: ${color}`);
  return `rgb(${parseInt(m[1], 16)}, ${parseInt(m[2], 16)}, ${parseInt(m[3], 16)})`;
}

/** The subscriptions the page holds (its `data-subs`). */
async function pageSubs(page: import("@playwright/test").Page): Promise<number> {
  return Number(await page.getByTestId("surface").getAttribute("data-subs"));
}

/** The hub holds the page's subscriptions plus STAGE AUT's own (live_set is_playing). */
async function expectHubToHold(page: import("@playwright/test").Page, subs: number) {
  await expect(page.getByTestId("surface")).toHaveAttribute("data-subs", String(subs));
  await until(hubSubscriptions, (n) => n === subs + 1, `the hub to hold ${subs + 1} subscriptions`);
}

/** The ids of the groups on screen, in document order. */
async function groups(page: import("@playwright/test").Page): Promise<string[]> {
  return page.locator('[data-testid="group"]').evaluateAll((els) => els.map((e) => e.getAttribute("data-group") || ""));
}

test.describe("Pages and tabs", () => {
  test("the tabs follow the layout and FOH is shown first", async ({ page }) => {
    await openSurface(page);
    const root = page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]');
    // No Conf tab (#58): TouchOSC's settings text is read, not shown.
    await expect(root).toHaveText(["Cue", "FOH"]);
    await expect(root.nth(1)).toHaveAttribute("data-selected", "true");
    await expect(root.nth(0)).toHaveAttribute("data-selected", "false");
    const pager = page.locator('[data-testid="tabbar"][data-level="1"] [data-testid="tab"]');
    await expect(pager).toHaveText(["STAGE", "OTHERS"]);
    await expect(pager.nth(0)).toHaveAttribute("data-selected", "true");
    // A page without a pager has no second tab bar.
    await selectPage(page, "cue");
    await expect(page.locator('[data-testid="tabbar"][data-level="1"]')).toHaveCount(0);
    await selectPage(page, "foh");
  });

  test("every tab shows its whole title", async ({ page }) => {
    await openSurface(page);
    for (const level of ["0", "1"]) {
      const tabs = page.locator(`[data-testid="tabbar"][data-level="${level}"] [data-testid="tab"]`);
      const count = await tabs.count();
      expect(count).toBeGreaterThan(1);
      for (let i = 0; i < count; i++) {
        const tab = tabs.nth(i);
        expect(await clipped(tab), `level ${level} tab ${await tab.getAttribute("data-page")}`).toEqual([]);
      }
    }
  });

  test("the nested pager switches its sub-pages, keeps the fixed groups and remembers the choice", async ({ page }) => {
    await openSurface(page);
    await expect(strip(page, "Keys 1")).toBeVisible();
    await expect(strip(page, "Hand1 #", "master")).toHaveCount(0);
    await expect(strip(page, "B-Main repro #")).toBeVisible();
    await selectPage(page, "others");
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await expect(strip(page, "Keys 1")).toHaveCount(0);
    await expect(strip(page, "B-Main repro #")).toBeVisible();
    // Another page and back: the pager still shows OTHERS; so does a reload.
    await selectPage(page, "cue");
    await selectPage(page, "foh");
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await page.reload();
    await expect(strip(page, "Hand1 #", "master")).toBeVisible();
    await selectPage(page, "stage");
  });

  test("the global controls are on every page, at the foot of the rail", async ({ page }) => {
    await openSurface(page);
    for (const id of ["cue", "foh"]) {
      await selectPage(page, id);
      const rail = page.getByTestId("rail");
      await expect(rail.getByTestId("alert-toggle")).toBeVisible();
      // REFRESH ALL is gone (#58): the hub keeps every binding current.
      await expect(rail.getByTestId("refresh")).toHaveCount(0);
    }
    await selectPage(page, "cue");
    await expect(page.locator('[data-testid="param-toggle"][data-label="Vox 1 TU"]')).toBeVisible();
    await selectPage(page, "foh");
  });
});

test.describe("The page is its rows of sections around the control column", () => {
  test("the rail holds the page's function controls in the layout's order", async ({ page }) => {
    await openSurface(page);
    const kinds = await page
      .getByTestId("rail")
      .locator(".btn")
      .evaluateAll((els) => els.map((e) => e.getAttribute("data-testid")));
    const rail = layout().pages[1].rail.map((c: any) =>
      ({ stage: "stage-mics", hub_toggle: "stage-aut", solo: "solo", param_toggle: "param-toggle" })[c.kind as string],
    );
    expect(kinds).toEqual([...rail, "alert-toggle"]);
    await expect(page.getByTestId("solo").first()).toHaveText("SOLO Vocals");
  });

  test("a rail button never breaks a word, and its words stay inside it", async ({ page }) => {
    // #21 post-deploy check: "TECHALERT" broke as "TECHALER" / "T" in the
    // rail's 112 px (overflow-wrap: anywhere was the only guard); a long
    // word now shrinks the button's font instead.
    await openSurface(page);
    const broken = await page
      .getByTestId("rail")
      .locator(".btn")
      .evaluateAll((buttons) =>
        buttons.flatMap((button) => {
          const out: string[] = [];
          const box = button.getBoundingClientRect();
          const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
          for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            const text = node.textContent ?? "";
            for (const m of text.matchAll(/\S+/g)) {
              const range = document.createRange();
              range.setStart(node, m.index!);
              range.setEnd(node, m.index! + m[0].length);
              const rects = Array.from(range.getClientRects()).filter((r) => r.width > 0);
              const lines = new Set(rects.map((r) => Math.round(r.top)));
              if (lines.size > 1) out.push(`"${m[0]}" on ${lines.size} lines`);
              const r = range.getBoundingClientRect();
              if (r.left < box.left - 0.5 || r.right > box.right + 0.5) out.push(`"${m[0]}" outside its button`);
            }
          }
          return out;
        }),
      );
    expect(broken).toEqual([]);
    await expect(page.getByTestId("alert-toggle")).toHaveText("TechAlert");
  });

  test("sections show in rows in the layout's order, with their titles and colours", async ({ page }) => {
    const fixture = layout();
    const foh = fixture.pages[1];
    await openSurface(page);
    // Each row is a line of its own (a tablet's and a desktop's height), cut
    // in two by the column: its left side, then its right one.
    await expect(page.getByTestId("line")).toHaveCount(foh.rows.length);
    // Row by row: the groups (the pager's selected sub-page in its place).
    const expected: string[][] = foh.rows.map((row: any) =>
      row.sections.flatMap((s: any) => (s.kind === "pager" ? s.pages[0].sections.map((g: any) => g.id) : [s.id])),
    );
    for (let r = 0; r < expected.length; r++) {
      const ids = await page
        .locator(`[data-testid="line"][data-line="${r}"] [data-testid="group"]`)
        .evaluateAll((els) => els.map((e) => e.getAttribute("data-group")));
      expect(ids, `row ${r}`).toEqual(expected[r]);
    }
    // The column sits between the sides: every left cell before it, every
    // right cell after it, on every line.
    const column = (await page.getByTestId("column").boundingBox())!;
    const sides = await page.locator('[data-testid="line"] .slot').evaluateAll((slots) =>
      slots.map((s) => {
        const r = s.getBoundingClientRect();
        return { side: s.getAttribute("data-side"), left: r.left, right: r.right };
      }),
    );
    expect(sides.filter((s) => s.side === "left").length).toBeGreaterThan(0);
    expect(sides.filter((s) => s.side === "right").length).toBeGreaterThan(0);
    for (const s of sides) {
      if (s.side === "left") expect(s.right, "a left cell ends before the column").toBeLessThanOrEqual(column.x + 0.5);
      else expect(s.left, "a right cell starts after the column").toBeGreaterThanOrEqual(column.x + column.width - 0.5);
    }
    expect(await groups(page)).toEqual(expected.flat());
    // Titles and colour markers.
    for (const row of foh.rows) {
      for (const section of row.sections) {
        if (section.kind !== "group") continue;
        const group = page.locator(`[data-testid="group"][data-group="${section.id}"]`);
        await expect(group.getByTestId("group-title")).toHaveText(section.title ?? "");
        if (section.color) await expect(group.locator(".group-mark")).toHaveCSS("background-color", rgb(section.color));
      }
    }
    // The strips of a group in its order.
    const effects = foh.rows.flatMap((r: any) => r.sections).find((s: any) => s.title === "EFFECTS");
    const names = await page
      .locator(`.slot[data-group="${effects.id}"] [data-testid="strip"]`)
      .evaluateAll((els) => els.map((e) => `${e.getAttribute("data-instance")}:${e.getAttribute("data-track")}`));
    expect(names).toEqual(effects.controls.map((c: any) => `${c.binding.instance}:${c.binding.anchor.name}`));
    await expect(strip(page, "B-Main repro #")).toHaveAttribute("data-kind", "return");
  });

  test("every row's strips share one width, a wide strip is 1.1 of it, and nothing overflows", async ({ page }) => {
    await openSurface(page);
    const widths = await page.locator('[data-testid="strip"]:not(.wide)').evaluateAll((els) =>
      els.map((e) => e.getBoundingClientRect().width),
    );
    expect(widths.length).toBeGreaterThan(3);
    for (const w of widths) expect(Math.abs(w - widths[0])).toBeLessThan(0.6);
    expect(widths[0]).toBeGreaterThanOrEqual(64);
    expect(widths[0]).toBeLessThanOrEqual(120.5);
    const wide = await strip(page, "B-Main repro #").evaluate((e) => e.getBoundingClientRect().width);
    expect(Math.abs(wide - widths[0] * 1.1)).toBeLessThan(0.6);
    // The page never scrolls; this layout fits: no line needs arrows.
    const size = await page.evaluate(() => ({
      w: document.documentElement.scrollWidth,
      h: document.documentElement.scrollHeight,
      vw: innerWidth,
      vh: innerHeight,
    }));
    expect(size.w).toBeLessThanOrEqual(size.vw);
    expect(size.h).toBeLessThanOrEqual(size.vh);
    await expect(page.getByTestId("shift")).toHaveCount(0);
    for (const s of await page.locator('[data-testid="strip"]').all()) {
      const box = (await s.boundingBox())!;
      expect(box.x + box.width).toBeLessThanOrEqual(size.vw + 0.5);
      expect(box.y + box.height).toBeLessThanOrEqual(size.vh + 0.5);
    }
  });
});

test.describe("Nothing moves under a finger", () => {
  test("switching the pager's sub-page never moves the fixed groups", async ({ page }) => {
    // #21 review: a tap aimed at a bus strip must stay on it whichever
    // sub-page shows beside it.
    await openSurface(page);
    const bus = strip(page, "B-Main repro #");
    const onStage = (await bus.boundingBox())!;
    await selectPage(page, "others");
    const onOthers = (await bus.boundingBox())!;
    expect(Math.abs(onOthers.x - onStage.x)).toBeLessThan(0.5);
    expect(Math.abs(onOthers.width - onStage.width)).toBeLessThan(0.5);
    await selectPage(page, "stage");
  });

  test("a long section title never widens its section", async ({ page }) => {
    await openSurface(page);
    const changed = layout();
    const row = changed.pages[1].rows[1];
    const section = row.sections.find((s: any) => s.kind === "group" && s.controls.length === 1);
    section.title = "A VERY LONG SECTION TITLE FOR ONE STRIP";
    try {
      await harness("/hub/layout", { layout: changed });
      const group = page.locator(`[data-testid="group"][data-group="${section.id}"]`);
      await expect(group.getByTestId("group-title")).toHaveText(section.title, { timeout: 10_000 });
      // The title spans its one cell, nothing more (#63: a line over the run).
      const title = (await group.locator(".group-title").boundingBox())!;
      const one = (await page.locator(`.slot[data-group="${section.id}"]`).first().boundingBox())!;
      expect(Math.abs(title.width - one.width)).toBeLessThan(1);
      expect(Math.abs(title.x - one.x)).toBeLessThan(1);
    } finally {
      await harness("/hub/layout/reset");
    }
  });
});

test.describe("Subscriptions follow the controls on screen", () => {
  test("a page switch subscribes the new page and releases the old one", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "stage");
    const foh = await pageSubs(page);
    // Rail (stage, two solos, five toggles' targets), the STAGE sub-page's
    // and the fixed groups' strips (volume, pan, mute, meter, colour each),
    // the parameter fader and TechAlert: 55 keys.
    expect(foh).toBe(55);
    await expectHubToHold(page, foh);
    await selectPage(page, "cue");
    await expectHubToHold(page, 2);
    await selectPage(page, "foh");
    await selectPage(page, "others");
    await expectHubToHold(page, 51);
    await selectPage(page, "stage");
    await expectHubToHold(page, foh);
  });

  test("a layout change while a page is open releases the old subscriptions", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "stage");
    await expectHubToHold(page, 55);
    const changed = layout();
    for (const row of changed.pages[1].rows) {
      for (const section of row.sections) {
        if (section.kind !== "group") continue;
        section.controls = section.controls.filter(
          (c: any) => !(c.kind === "strip" && c.binding.instance === "band" && c.binding.anchor.name === "Hand2 #"),
        );
      }
    }
    try {
      await harness("/hub/layout", { layout: changed });
      await expect(strip(page, "Hand2 #")).toHaveCount(0, { timeout: 10_000 });
      await expectHubToHold(page, 50);
    } finally {
      await harness("/hub/layout/reset");
    }
    await expect(strip(page, "Hand2 #")).toBeVisible({ timeout: 10_000 });
    await expectHubToHold(page, 55);
  });
});
