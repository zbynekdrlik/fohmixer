import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  dispatchPointer,
  dispatchSnapped,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  seen,
  selectPage,
  until,
} from "./support/live";

// The linked mute (#127, the approved mockup docs/mockups/paired-mute-v3.html,
// variant A): a strip whose Tuner carries `+ID:<id>` mutes its own track and
// every track (or return) whose Tuner carries `+MUTE:<id>`, in one batch. Its
// name button shows a chain and the count on its marks line under the name
// (#151, mockup v8; a muted strip's chain without its count): grey while
// all play, red while all are muted, amber "m/N" while m of them are muted by
// hand in Live. A tap mutes all, or unmutes all when all are muted; a
// guarded one (+MG) does it by a 1 s hold, at the lift (#162).

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/** The fixture with a MARKERS group (the strips of that tag group) in the FOH page's second row. */
function withMarkers(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  foh.rows[1].sections.push({ kind: "group", id: "markers", title: "MARKERS", tags: "MARKERS" });
  return frame;
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the Tuner markers gone", 10_000);
}

/** SimLive's band set: the strip's own track (Hand2 #), a partner track (Hand3 #) and a partner return (A-Reverb #). */
const OWN = "live_set tracks 1";
const TRACK = "live_set tracks 2";
const RETURN = "live_set return_tracks 0";
const LINKED = [OWN, TRACK, RETURN];
const KEYS = LINKED.map((t) => `band|${t}|mute`).sort();

/** The chain's colours (link.css): grey, red, amber. */
const GREY = "rgb(214, 214, 226)";
const RED = "rgb(255, 90, 103)";
const AMBER = "rgb(245, 166, 35)";
/** A strip's background (style.css `--panel`), and a muted strip's (`--panel-mute`). */
const PANEL = "rgb(26, 26, 40)";
const PANEL_MUTE = "rgb(18, 18, 27)";

/** The hub's write batches since `since` (hub ms) that carry any of the linked mutes. */
async function linkedBatches(since: number): Promise<any[]> {
  return (await hubEvents()).filter(
    (r: any) => r.ev === "batch" && r.ts >= since && r.sent.some((item: any) => KEYS.includes(item.key)),
  );
}

/** Live's mutes of the three tracks. */
async function mutes(live: LiveClient): Promise<boolean[]> {
  return Promise.all(LINKED.map((t) => live.get("band", t, "mute")));
}

/** Removes every Tuner a test here set, the layout reset, the markers gone. */
async function cleanUp(lines: string[]) {
  for (const line of lines) await hostLine("band", line);
  await harness("/hub/layout/reset");
  await markersGone();
}

test.describe("A linked mute", () => {
  let live: LiveClient;
  let before: boolean[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
    before = await mutes(live);
  });
  test.afterEach(async () => {
    for (const [i, t] of LINKED.entries()) await live.set("band", t, "mute", before[i]);
    live.close();
  });

  test("a strip linked three ways shows the chain, and a tap mutes and unmutes all three together", async ({ page }) => {
    for (const t of LINKED) await live.set("band", t, "mute", false);
    await openSurface(page);
    const strips = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    const linked = strips.and(page.locator('[data-label="Vox 1"]'));
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      // Today's strip first: no link, no chain; its fader's travel.
      expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:MARKERS'`)).toBe("TUNER 1");
      await expect(linked).toHaveCount(1, { timeout: 10_000 });
      await ready(linked.getByTestId("fader"));
      await expect(linked.getByTestId("mute-link")).toHaveCount(0);
      const today = (await linked.getByTestId("fader").boundingBox())!.height;
      // Linked three ways: the strip's own track, a track and a return.
      expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:MARKERS +ID:VOX1'`)).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 2 set '+MUTE:VOX1'")).toBe("TUNER 1");
      expect(await hostLine("band", "tuner return 0 set '+MUTE:VOX1'")).toBe("TUNER 1");
      const mute = linked.getByTestId("mute");
      const chain = linked.getByTestId("mute-link");
      await expect(chain).toHaveText("3", { timeout: 10_000 });
      await ready(mute);
      // The partners make no strip of their own.
      await expect(strips).toHaveCount(1);
      await expect(chain).toHaveAttribute("data-state", "all");
      await expect(chain).toHaveCSS("color", GREY);
      await expect(mute).toHaveAttribute("data-muted", "false");
      await expect(mute).toHaveClass(/\blit\b/);
      // No height: the name button stays 37 px (#151) and the fader's
      // travel is the same strip's before the link.
      expect((await mute.boundingBox())!.height).toBeCloseTo(37, 1);
      await ready(linked.getByTestId("fader"));
      const travel = (await linked.getByTestId("fader").boundingBox())!.height;
      test.info().annotations.push({ type: "travel-px", description: `${travel} (unlinked ${today})` });
      expect(Math.abs(travel - today), `the linked strip's travel ${travel} against ${today}`).toBeLessThan(0.5);
      expect(travel).toBeGreaterThan(200);
      // The chain sits at the left of the marks line, under the name (#151).
      const corner = (await chain.boundingBox())!;
      const button = (await mute.boundingBox())!;
      const name = (await linked.getByTestId("strip-label").boundingBox())!;
      expect(corner.x - button.x).toBeLessThan(10);
      expect(corner.y).toBeGreaterThanOrEqual(name.y + name.height - 0.5);
      expect(corner.y + corner.height).toBeLessThanOrEqual(button.y + button.height + 0.5);

      // A tap mutes all three in Live, in one batch.
      const since = Date.now();
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === true, `${t} muted by the tap`);
      await expect(mute).toHaveAttribute("data-muted", "true");
      await expect(chain).toHaveAttribute("data-state", "muted");
      await expect(chain).toHaveText("3");
      await expect(chain).toHaveCSS("color", RED);
      await expect(mute).not.toHaveClass(/\blit\b/);
      await expect(linked).toHaveCSS("background-color", PANEL_MUTE);
      const first = await until(() => linkedBatches(since), (b) => b.length >= 1, "the tap's batch");
      expect(first.length).toBe(1);
      expect(first[0].n).toBe(3);
      expect(first[0].sent.map((item: any) => item.key).sort()).toEqual(KEYS);
      expect(first[0].sent.every((item: any) => item.value === true)).toBe(true);

      // Played again by hand in Live, then one muted by hand (outside the
      // surface): amber 1 of 3, the strip still lit.
      for (const t of LINKED) await live.set("band", t, "mute", false);
      await expect(chain).toHaveAttribute("data-state", "all");
      await expect(mute).toHaveAttribute("data-muted", "false");
      await live.set("band", TRACK, "mute", true);
      await expect(chain).toHaveText("1/3");
      await expect(chain).toHaveAttribute("data-state", "mixed");
      await expect(chain).toHaveCSS("color", AMBER);
      await expect(mute).toHaveAttribute("data-muted", "mixed");
      await expect(mute).toHaveClass(/\blit\b/);
      await expect(linked).toHaveCSS("background-color", PANEL);

      // A tap from the mixed state mutes all.
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === true, `${t} muted from mixed`);
      await expect(chain).toHaveText("3");
      await expect(chain).toHaveAttribute("data-state", "muted");
      await expect(mute).toHaveAttribute("data-muted", "true");

      // A second tap unmutes all.
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === false, `${t} unmuted by the second tap`);
      await expect(chain).toHaveAttribute("data-state", "all");
      await expect(mute).toHaveAttribute("data-muted", "false");
      await expect(linked).toHaveCSS("background-color", PANEL);

      // Every tap was one batch of the three.
      const all = await until(() => linkedBatches(since), (b) => b.length >= 3, "the three taps' batches");
      expect(all.map((b: any) => b.n)).toEqual([3, 3, 3]);
      for (const b of all) expect(b.sent.map((item: any) => item.key).sort()).toEqual(KEYS);
      expect(all.map((b: any) => b.sent[0].value)).toEqual([true, true, false]);
    } finally {
      await cleanUp(["tuner track 1 remove", "tuner track 2 remove", "tuner return 0 remove"]);
    }
    await expect(strips).toHaveCount(0, { timeout: 10_000 });
  });

  test("an id on two strips marks both and links neither; a mute naming no strip is listed with its marker", async ({ page }) => {
    for (const t of LINKED) await live.set("band", t, "mute", false);
    await openSurface(page);
    const strips = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:MARKERS +ID:VOX1'`)).toBe("TUNER 1");
      expect(await hostLine("band", `tuner track 2 set '"Vox 2" +G:MARKERS +ID:VOX1'`)).toBe("TUNER 1");
      expect(await hostLine("band", "tuner return 0 set '+MUTE:VOX1'")).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 3 set '+MUTE:NOBODY'")).toBe("TUNER 1");
      const marked = strips.and(page.locator('[data-mark="problem"]'));
      await expect(marked).toHaveCount(2, { timeout: 10_000 });
      for (const label of ["Vox 1", "Vox 2"]) {
        const s = strips.and(page.locator(`[data-label="${label}"]`));
        await expect(s).toHaveAttribute("data-mark", "problem");
        await expect(s.getByTestId("strip-mark")).toHaveText("TAG?");
        await expect(s.getByTestId("mute-link")).toHaveCount(0);
      }
      // `/api/status` lists the problems: the id on both strips, the mute
      // naming no strip with its own track's marker.
      const problems = await until(
        async () => (await hubStatus()).layout.markers.problems,
        (p: any[]) => p.length === 3,
        "the three markers' problems",
        10_000,
      );
      const listed = problems.map((r: any) => [r.kind, r.index, r.problems.map((p: any) => `${p.code} ${p.id}`)]);
      expect(listed).toEqual([
        ["track", 1, ["duplicate_id VOX1"]],
        ["track", 2, ["duplicate_id VOX1"]],
        ["track", 3, ["orphan_mute NOBODY"]],
      ]);
      expect(problems[2].name).toBe("+MUTE:NOBODY");
      // No link: Vox 1's tap mutes its own track alone, in a batch of its
      // own key only (the hub's record of the tap's batch).
      const mute = strips.and(page.locator('[data-label="Vox 1"]')).getByTestId("mute");
      await ready(mute);
      const since = Date.now();
      await mute.click();
      await until(() => live.get("band", OWN, "mute"), (v) => v === true, "Vox 1's own track muted");
      const tapped = await until(() => linkedBatches(since), (b) => b.length >= 1, "the tap's batch");
      expect(tapped.length).toBe(1);
      expect(tapped[0].n).toBe(1);
      expect(tapped[0].sent.map((item: any) => item.key)).toEqual([`band|${OWN}|mute`]);
      expect(await live.get("band", RETURN, "mute")).toBe(false);
      expect(await live.get("band", TRACK, "mute")).toBe(false);
    } finally {
      await cleanUp(["tuner track 1 remove", "tuner track 2 remove", "tuner return 0 remove", "tuner track 3 remove"]);
    }
    await expect(strips).toHaveCount(0, { timeout: 10_000 });
  });

  test("a guarded linked mute (+MG): a tap only hints and writes nothing, a 1 s hold mutes all three in one batch at its lift (#162)", async ({ page }) => {
    for (const t of LINKED) await live.set("band", t, "mute", false);
    await openSurface(page);
    const strips = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    const linked = strips.and(page.locator('[data-label="Vox 1"]'));
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:MARKERS +ID:VOX1 +MG'`)).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 2 set '+MUTE:VOX1'")).toBe("TUNER 1");
      expect(await hostLine("band", "tuner return 0 set '+MUTE:VOX1'")).toBe("TUNER 1");
      const mute = linked.getByTestId("mute");
      const chain = linked.getByTestId("mute-link");
      await expect(chain).toHaveText("3", { timeout: 10_000 });
      await ready(mute);
      await expect(mute).toHaveAttribute("data-guarded", "true");
      const since = Date.now();
      // A tap (a lift before the hold's 1 s): the hint, nothing written.
      await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
      await expect(mute.getByTestId("mute-hold")).toHaveText("hold");
      await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
      expect(await mutes(live)).toEqual([false, false, false]);
      await expect(chain).toHaveAttribute("data-state", "all");
      // A hold to full: the lift's line, still nothing written; the lift
      // mutes all three.
      const held = await dispatchSnapped(mute, [{ type: "pointerdown" }, { wait: 1300 }, { snap: true }]);
      expect(held.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
      expect(await mutes(live), "nothing before the lift").toEqual([false, false, false]);
      await dispatchPointer(mute, [{ type: "pointerup" }]);
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === true, `${t} muted at the hold's lift`);
      await expect(chain).toHaveAttribute("data-state", "muted");
      await expect(mute).toHaveAttribute("data-muted", "true");
      // The hub's only batch of the three since the tap is the lift's: the
      // tap and the press wrote nothing.
      const batches = await until(() => linkedBatches(since), (b) => b.length >= 1, "the lift's batch");
      expect(batches.length).toBe(1);
      expect(batches[0].n).toBe(3);
      expect(batches[0].sent.map((item: any) => item.key).sort()).toEqual(KEYS);
      expect(batches[0].sent.every((item: any) => item.value === true)).toBe(true);
    } finally {
      await cleanUp(["tuner track 1 remove", "tuner track 2 remove", "tuner return 0 remove"]);
    }
    await expect(strips).toHaveCount(0, { timeout: 10_000 });
  });

  test("the channel detail's MUTE carries the chain on its button, and a tap there writes the whole link", async ({ page }) => {
    for (const t of LINKED) await live.set("band", t, "mute", false);
    await openSurface(page);
    const strips = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    const linked = strips.and(page.locator('[data-label="Vox 1"]'));
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      expect(await hostLine("band", `tuner track 1 set '"Vox 1" +G:MARKERS +ID:VOX1'`)).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 2 set '+MUTE:VOX1'")).toBe("TUNER 1");
      expect(await hostLine("band", "tuner return 0 set '+MUTE:VOX1'")).toBe("TUNER 1");
      await expect(linked.getByTestId("mute-link")).toHaveText("3", { timeout: 10_000 });
      // The marker strip's track by index: SimLive's track 1 is `#2`.
      await expect(linked).toHaveAttribute("data-track", "#2");
      const detail = await openDetail(page, "#2");
      const mute = detail.getByTestId("mute");
      const pill = mute.getByTestId("mute-link");
      await ready(mute);
      // The pill on the detail's MUTE: the chain and the count, grey.
      await expect(pill).toHaveText("3");
      await expect(pill).toHaveAttribute("data-state", "all");
      await expect(pill).toHaveCSS("color", GREY);
      expect(await seen(pill)).toBe(true);
      const box = (await mute.boundingBox())!;
      const mark = (await pill.boundingBox())!;
      expect(mark.x >= box.x && mark.x + mark.width <= box.x + box.width, "the pill on the button").toBe(true);
      expect(mark.y >= box.y && mark.y + mark.height <= box.y + box.height, "the pill on the button").toBe(true);
      // A tap mutes all three in one batch; another unmutes them.
      const since = Date.now();
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === true, `${t} muted from the detail`);
      await expect(pill).toHaveAttribute("data-state", "muted");
      await expect(pill).toHaveCSS("color", RED);
      await expect(mute).toHaveAttribute("data-muted", "true");
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === false, `${t} unmuted from the detail`);
      await expect(pill).toHaveAttribute("data-state", "all");
      const batches = await until(() => linkedBatches(since), (b) => b.length >= 2, "the detail's two batches");
      expect(batches.map((b: any) => b.n)).toEqual([3, 3]);
      for (const b of batches) expect(b.sent.map((item: any) => item.key).sort()).toEqual(KEYS);
      expect(batches.map((b: any) => b.sent[0].value)).toEqual([true, false]);
    } finally {
      await cleanUp(["tuner track 1 remove", "tuner track 2 remove", "tuner return 0 remove"]);
    }
    await expect(strips).toHaveCount(0, { timeout: 10_000 });
  });

  test("an all-muted linked return with a problem keeps `!`, the bare chain and MUTED whole at 64 and 120 px, no RET", async ({ page }) => {
    for (const t of LINKED) await live.set("band", t, "mute", false);
    await openSurface(page);
    const strips = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    const hall = strips.and(page.locator('[data-label="Hall"]'));
    try {
      await harness("/hub/layout", { layout: withMarkers() });
      await selectPage(page, "foh");
      // The return's own Tuner links tracks 1 and 2; `+GX` is a typo, a
      // problem that leaves the link working.
      expect(await hostLine("band", `tuner return 0 set '"Hall" +G:MARKERS +ID:HALL +GX'`)).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 1 set '+MUTE:HALL'")).toBe("TUNER 1");
      expect(await hostLine("band", "tuner track 2 set '+MUTE:HALL'")).toBe("TUNER 1");
      const mute = hall.getByTestId("mute");
      const chain = hall.getByTestId("mute-link");
      const count = hall.getByTestId("mute-link-n");
      const flag = hall.locator(".strip-flag");
      const mark = hall.getByTestId("mute-mark");
      await expect(chain).toHaveText("3", { timeout: 10_000 });
      await expect(hall).toHaveAttribute("data-mark", "problem");
      await expect(hall).toHaveAttribute("data-kind", "return");
      await expect(hall.locator(".strip-ret")).toHaveCount(0);
      expect(await hall.textContent()).not.toMatch(/\bRET\b/i);
      await ready(mute);
      await mute.click();
      for (const t of LINKED) await until(() => live.get("band", t, "mute"), (v) => v === true, `${t} muted by the return's tap`);
      await expect(mute).toHaveAttribute("data-muted", "true");
      await expect(chain).toHaveAttribute("data-state", "muted");
      // A muted chain drops its count (MUTED says it).
      await expect(chain).toHaveClass(/\bbare\b/);
      await expect(count).toBeHidden();
      /** The strip at `px` wide, the narrow flags as the page sets them: what can be seen of its marks. */
      const at = async (px: number) => {
        await hall.evaluate((el, w) => {
          const wrap = (el.closest(".slot") ?? el.parentElement) as HTMLElement;
          wrap.style.setProperty("--strip-w", `${w}px`);
          wrap.setAttribute("data-narrow", String(w < 78));
          wrap.setAttribute("data-narrow-wide", String(w * 1.1 < 78));
        }, px);
        await expect.poll(async () => Math.round((await hall.boundingBox())!.width)).toBe(px);
        return { flag: await seen(flag), chain: await seen(chain), mute: await seen(mark) };
      };
      for (const px of [64, 120]) {
        expect(await at(px), `${px} px`).toEqual({ flag: true, chain: true, mute: true });
        const line = (await hall.getByTestId("strip-marks").boundingBox())!;
        const muted = (await mark.boundingBox())!;
        const c = (await chain.boundingBox())!;
        expect(c.x + c.width, `${px} px: the chain left of MUTED`).toBeLessThanOrEqual(muted.x + 0.5);
        expect(Math.abs(muted.x + muted.width - (line.x + line.width)), `${px} px: MUTED at the line's right end`).toBeLessThan(0.5);
        expect((await flag.boundingBox())!.x - (await mute.boundingBox())!.x, `${px} px: \`!\` first`).toBeLessThan(10);
      }
    } finally {
      await cleanUp(["tuner return 0 remove", "tuner track 1 remove", "tuner track 2 remove"]);
    }
    await expect(strips).toHaveCount(0, { timeout: 10_000 });
  });
});
