import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  dbForm,
  dispatchPointer,
  doubleTap,
  harness,
  hostLine,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  selectPage,
  until,
  value2db,
  withoutHubValuesAfterTheFirst,
} from "./support/live";

// The fader link (#130, D18; mockup reverb-link-v2): like the linked mute,
// with one leader. Rev PA's Tuner carries `+ID:REV`, Rev ST's `+FADER:REV`;
// with the link on (switched in Rev PA's detail), a drag or a double tap of
// Rev PA moves Rev ST by the same dB, in one batch, and Rev ST moved alone
// moves only itself. Podklady All is a leader with no track of its own:
// its tracks are the Tuners carrying `+FADER:PODKLADY`.

const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
/** SimLive's band set: Rev ST on Hand3 # (track 2), Rev PA on Hand4 # (track 3). */
const ST = "live_set tracks 2 mixer_device volume";
const PA = "live_set tracks 3 mixer_device volume";
/** Their tracks (the mutes). */
const ST_TRACK = "live_set tracks 2";
const PA_TRACK = "live_set tracks 3";
const KEYS = [`band|${ST}|value`, `band|${PA}|value`];
const REV = "fader_link|band|REV";
/** SimLive's band set: Drums # (track 10) and Bass # (track 11). */
const DRUMS = "live_set tracks 10 mixer_device volume";
const BASS = "live_set tracks 11 mixer_device volume";
const BT_KEYS = [`band|${DRUMS}|value`, `band|${BASS}|value`];
/** "The same dB" (the #58 review): within 0.01 dB on the page's curve. */
const SAME_DB = 0.01;

/** The fixture with the REVERBS tags group in the FOH page's second row. */
function withReverbs(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  frame.pages.find((p: any) => p.id === "foh").rows[1].sections.push({ kind: "group", id: "reverbs", title: "REVERBS", tags: "REVERBS" });
  return frame;
}

/** The fixture with Podklady All linked to band's PODKLADY instead of its two tracks by name. */
function withLinkedPodklady(): any {
  const frame = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const groups = frame.pages.flatMap((p: any) =>
    p.rows.flatMap((r: any) => r.sections.flatMap((s: any) => (s.kind === "group" ? [s] : s.pages.flatMap((sp: any) => sp.sections)))),
  );
  const fader = groups.flatMap((g: any) => g.controls ?? []).find((c: any) => c.label === "Podklady All");
  delete fader.targets;
  fader.link = { instance: "band", id: "PODKLADY" };
  return frame;
}

/** Live's value of `target` once two reads 50 ms apart agree (within 8 s). */
async function settled(live: LiveClient, target: string): Promise<number> {
  const deadline = Date.now() + 8000;
  let last = await live.get("band", target, "value");
  for (;;) {
    await new Promise((r) => setTimeout(r, 50));
    const now = await live.get("band", target, "value");
    if (now === last) return now;
    if (Date.now() > deadline) throw new Error(`${target} never settled; last ${now}`);
    last = now;
  }
}

/** The hub's write batches since `since` (hub ms) that carry any of `keys`. */
async function batches(since: number, keys: string[]): Promise<any[]> {
  return (await hubEvents()).filter((r: any) => r.ev === "batch" && r.ts >= since && r.sent.some((i: any) => keys.includes(i.key)));
}

/** Waits until the hub holds no Tuner marker: the next spec starts on the fixture. */
async function markersGone() {
  await until(async () => (await hubStatus()).layout.markers.found, (n: number) => n === 0, "the markers gone", 10_000);
}

/** A finger's drag of `dy` px up (down when negative): its first move only anchors (#43 PR F). */
const drag = (dy: number) => [
  { type: "pointerdown" as const },
  { type: "pointermove" as const, dy: 0 },
  { type: "pointermove" as const, dy: dy / 2 },
  { type: "pointermove" as const, dy },
  { type: "pointerup" as const, dy },
];

/** A strip of the REVERBS group by its label. */
const strip = (page: Page, label: string): Locator =>
  page.locator('.slot[data-group="reverbs"] [data-testid="strip"]').and(page.locator(`[data-label="${label}"]`));

/** The change of `to` from `from` in dB on the page's curve. */
const change = (from: number, to: number) => value2db(to) - value2db(from);

test.describe("A fader link", () => {
  let live: LiveClient;
  let before: number[];
  let mutes: boolean[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
    before = [await live.get("band", ST, "value"), await live.get("band", PA, "value")];
    mutes = [await live.get("band", ST_TRACK, "mute"), await live.get("band", PA_TRACK, "mute")];
  });
  test.afterEach(async () => {
    live.setHub(REV, true); // the default (Q3): put back what the test switched
    await live.set("band", ST, "value", before[0]);
    await live.set("band", PA, "value", before[1]);
    await live.set("band", ST_TRACK, "mute", mutes[0]);
    await live.set("band", PA_TRACK, "mute", mutes[1]);
    await hostLine("band", "tuner track 2 remove");
    await hostLine("band", "tuner track 3 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    live.close();
  });

  /** Rev PA leads Rev ST in the REVERBS group (the link on by default): both strips ready. */
  async function linked(page: Page): Promise<{ pa: Locator; st: Locator }> {
    await openSurface(page);
    await harness("/hub/layout", { layout: withReverbs() });
    await selectPage(page, "foh");
    expect(await hostLine("band", `tuner track 3 set '"Rev PA" +G:REVERBS +ID:REV'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 2 set '"Rev ST" +G:REVERBS +FADER:REV'`)).toBe("TUNER 1");
    await expect(strip(page, "Rev ST").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "follow", { timeout: 10_000 });
    const pa = strip(page, "Rev PA").getByTestId("fader");
    const st = strip(page, "Rev ST").getByTestId("fader");
    await ready(pa);
    await ready(st);
    return { pa, st };
  }

  test("a leader shown nowhere leads nothing; placed, its link starts on, both marked; the switch is the leader's, the partner's detail names it", async ({ page }) => {
    await live.set("band", ST, "value", 0.36);
    await live.set("band", PA, "value", 0.6);
    await openSurface(page);
    await harness("/hub/layout", { layout: withReverbs() });
    await selectPage(page, "foh");

    // Rev PA without a group (#130 review M1): shown nowhere, it leads
    // nothing, so Rev ST follows no one and its note never waits on a
    // switch the hub does not keep.
    expect(await hostLine("band", `tuner track 3 set '"Rev PA" +ID:REV'`)).toBe("TUNER 1");
    expect(await hostLine("band", `tuner track 2 set '"Rev ST" +G:REVERBS +FADER:REV'`)).toBe("TUNER 1");
    await until(
      async () => (await hubStatus()).layout.markers.problems,
      (p: any[]) => p.length === 1 && p[0].index === 3 && p[0].problems.some((x: any) => x.code === "no_group"),
      "Rev PA shown nowhere, Rev ST no orphan",
      10_000,
    );
    await expect(strip(page, "Rev ST")).toHaveCount(1, { timeout: 10_000 });
    await expect(strip(page, "Rev PA")).toHaveCount(0);
    const status = await hubStatus();
    expect(status.layout.markers.links).toEqual([]);
    expect(status.fader_links).toEqual([]);
    await expect(page.getByTestId("fader-link-mark")).toHaveCount(0);
    await expect(strip(page, "Rev ST")).toHaveAttribute("data-flink", "");
    const alone = await openDetail(page, "#3");
    await expect(alone.getByTestId("fader-follow")).toHaveCount(0);
    await expect(alone.getByTestId("fader-link")).toHaveCount(0);
    await alone.getByTestId("detail-exit").click();
    await ready(strip(page, "Rev ST").getByTestId("fader"));
    const unmarked = (await strip(page, "Rev ST").getByTestId("fader").boundingBox())!.height;

    // Placed: a new link starts on (Q3), the leader's mark filled, the
    // partner's outlined, and no height (the same travel).
    expect(await hostLine("band", `tuner track 3 set '"Rev PA" +G:REVERBS +ID:REV'`)).toBe("TUNER 1");
    await expect(strip(page, "Rev PA")).toHaveCount(1, { timeout: 10_000 });
    const pa = strip(page, "Rev PA").getByTestId("fader");
    await ready(pa);
    await expect(strip(page, "Rev PA").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "lead");
    await expect(strip(page, "Rev ST").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "follow");
    await expect(strip(page, "Rev PA").getByTestId("fader-link-mark")).not.toHaveClass(/\bfollow\b/);
    await expect(strip(page, "Rev ST").getByTestId("fader-link-mark")).toHaveClass(/\bfollow\b/);
    await expect(strip(page, "Rev PA")).toHaveAttribute("data-flink", "lead");
    await expect(strip(page, "Rev ST")).toHaveAttribute("data-flink", "follow");
    await until(async () => (await hubStatus()).fader_links, (l: any[]) => l.length === 1, "the leader's switch");
    expect((await hubStatus()).fader_links).toEqual([{ instance: "band", id: "REV", on: true }]);
    expect((await hubStatus()).layout.markers.links).toEqual([{ instance: "band", id: "REV", leader: "strip", partners: 1 }]);
    const travel = (await pa.boundingBox())!.height;
    const marked = (await strip(page, "Rev ST").getByTestId("fader").boundingBox())!.height;
    console.log(`fader-link travel (${test.info().project.name}): ${travel} px marked, ${unmarked} px unmarked`);
    test.info().annotations.push({ type: "travel-px", description: `${travel} (unmarked ${unmarked})` });
    expect(marked, "no height").toBeCloseTo(unmarked, 1);
    expect(travel, "no height").toBeCloseTo(unmarked, 1);

    // The partner's detail names its leader and has no switch.
    const partner = await openDetail(page, "#3");
    await expect(partner.getByTestId("fader-link")).toHaveCount(0);
    await expect(partner.getByTestId("fader-follow")).toContainText("IDE S Rev PA");
    await expect(partner.getByTestId("fader-follow")).toContainText("spojenie zapnuté");
    await expect(partner.getByTestId("fader-follow")).toHaveAttribute("data-on", "true");
    await partner.getByTestId("detail-exit").click();

    // Switched off in the leader's detail: the marks go, the leader moves alone.
    const detail = await openDetail(page, "#4");
    const link = detail.getByTestId("fader-link");
    await expect(link).toHaveText(/SPOJENÝ S Rev ST/i);
    await expect(link).toHaveAttribute("data-on", "true");
    await expect(link).toHaveClass(/\bon\b/);
    expect(
      await link.evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);
    await link.click();
    await expect(link).toHaveAttribute("data-on", "false");
    await until(async () => (await hubStatus()).fader_links[0].on, (on: boolean) => on === false, "the switch off");
    await detail.getByTestId("detail-exit").click();
    await expect(page.getByTestId("fader-link-mark")).toHaveCount(0);
    await expect(strip(page, "Rev PA")).toHaveAttribute("data-flink", "");
    const off = await openDetail(page, "#3");
    await expect(off.getByTestId("fader-follow")).toContainText("spojenie vypnuté · zapína sa v detaile Rev PA");
    await expect(off.getByTestId("fader-follow")).toHaveAttribute("data-on", "false");
    await off.getByTestId("detail-exit").click();
    const since = Date.now();
    await dispatchPointer(pa, drag(20));
    await until(() => settled(live, PA), (v) => v > 0.6, "Rev PA moved");
    expect(await live.get("band", ST, "value")).toBe(0.36);
    const unlinked = await until(() => batches(since, KEYS), (b) => b.length > 0, "the unlinked drag's batches");
    for (const b of unlinked) expect(b.sent.map((i: any) => i.key)).toEqual([KEYS[1]]);

    // On again (a page's switch, here the test's client): both marks back.
    live.setHub(REV, true);
    const mark = strip(page, "Rev PA").getByTestId("fader-link-mark");
    await expect(mark).toHaveAttribute("data-role", "lead");
    await expect(strip(page, "Rev ST").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "follow");

    // Beside a crowded mute chain the mark is the chain alone (the app's
    // `flink_compact` over the chain's own count): Rev ST also mutes with
    // Rev PA (`+MUTE:REV`, the mockup's Vox 1 and Vox 1 S); muted by hand in
    // Live the chain reads 1/2 and the mark is compact, unmuted it is whole.
    await live.set("band", ST_TRACK, "mute", false);
    await live.set("band", PA_TRACK, "mute", false);
    expect(await hostLine("band", `tuner track 2 set '"Rev ST" +G:REVERBS +FADER:REV +MUTE:REV'`)).toBe("TUNER 1");
    const chain = strip(page, "Rev PA").getByTestId("mute-link");
    await expect(chain).toHaveText("2", { timeout: 10_000 });
    await expect(mark).toHaveAttribute("data-role", "lead");
    await expect(mark).not.toHaveClass(/\bcompact\b/);
    await live.set("band", ST_TRACK, "mute", true);
    await expect(chain).toHaveText("1/2");
    await expect(mark).toHaveClass(/\bcompact\b/);
    await live.set("band", ST_TRACK, "mute", false);
    await expect(chain).toHaveText("2");
    await expect(mark).not.toHaveClass(/\bcompact\b/);
  });

  test("the leader moves its partner by the same dB in one batch, on a drag and a double tap; the partner alone moves alone; one at −∞ stays", async ({ page }) => {
    await live.set("band", ST, "value", 0.36);
    await live.set("band", PA, "value", 0.6);
    const { pa, st } = await linked(page);

    // A drag of the leader: the partner by the same dB, every batch both.
    let since = Date.now();
    await dispatchPointer(pa, drag(40));
    const paAt = await until(() => settled(live, PA), (v) => v > 0.6, "Rev PA moved");
    const stAt = await settled(live, ST);
    expect(stAt, "Rev ST moved").toBeGreaterThan(0.36);
    expect(Math.abs(change(0.36, stAt) - change(0.6, paAt)), `Rev PA ${paAt}, Rev ST ${stAt}`).toBeLessThan(SAME_DB);
    const linkedBatches = await until(() => batches(since, KEYS), (b) => b.length > 0, "the drag's batches");
    for (const b of linkedBatches) expect(b.sent.map((i: any) => i.key).sort()).toEqual(KEYS);

    // The partner alone: the leader stays, its batches carry Rev ST alone.
    since = Date.now();
    await dispatchPointer(st, drag(-20));
    const stDown = await until(() => settled(live, ST), (v) => v < stAt, "Rev ST down");
    expect(await live.get("band", PA, "value")).toBe(paAt);
    const alone = await until(() => batches(since, KEYS), (b) => b.length > 0, "the partner's batches");
    for (const b of alone) expect(b.sent.map((i: any) => i.key)).toEqual([KEYS[0]]);

    // A double tap of the leader: Rev PA to 0 dB, Rev ST by the same change.
    since = Date.now();
    await doubleTap(pa);
    await until(() => settled(live, PA), (v) => Math.abs(v - 0.85) < 1e-6, "Rev PA at 0 dB", 8000);
    const stEnd = await settled(live, ST);
    expect(Math.abs(change(stDown, stEnd) - change(paAt, 0.85)), `Rev ST ${stDown} to ${stEnd}`).toBeLessThan(SAME_DB);
    const tapped = await until(() => batches(since, KEYS), (b) => b.length > 0, "the double tap's batches");
    for (const b of tapped) expect(b.sent.map((i: any) => i.key).sort()).toEqual(KEYS);

    // A partner at −∞ stays there (the page shows it first: the down reads it).
    await live.set("band", ST, "value", 0);
    await expect(strip(page, "Rev ST").getByTestId("db")).toHaveText(dbForm(await live.display("band", ST, 0)));
    await dispatchPointer(pa, drag(-20));
    await until(() => settled(live, PA), (v) => v < 0.85, "Rev PA down");
    expect(await live.get("band", ST, "value")).toBe(0);

    // A leader at −∞ moves alone (the main session's ruling on #130: the
    // same dB is undefined from there; a move up threw the partner to
    // +6 dB). Both values shown on the strips first: the down reads them.
    await live.set("band", ST, "value", 0.36);
    await live.set("band", PA, "value", 0);
    await expect(strip(page, "Rev ST").getByTestId("db")).toHaveText(dbForm(await live.display("band", ST, 0.36)));
    await expect(strip(page, "Rev PA").getByTestId("db")).toHaveText(dbForm(await live.display("band", PA, 0)));
    since = Date.now();
    await dispatchPointer(pa, drag(40));
    await until(() => settled(live, PA), (v) => v > 0, "Rev PA up from −∞");
    expect(await live.get("band", ST, "value"), "Rev ST unchanged").toBe(0.36);
    const lone = await until(() => batches(since, KEYS), (b) => b.length > 0, "the leader's batches");
    for (const b of lone) expect(b.sent.map((i: any) => i.key)).toEqual([KEYS[1]]);
  });

  test("after a reconnect the leader's switch shows its value, stale; a tap before the hub tells it again is refused and sends nothing", async ({ page }) => {
    // The design's C table (review I1): the switch acts only on a value the
    // hub sent on this connection (`behave::rail::hold_value` over
    // `HubValues::fresh_flag`), as STAGE AUT's hold does (connection.spec.ts).
    const sockets = await withoutHubValuesAfterTheFirst(page);
    await linked(page);
    // The socket drops (a Wi-Fi blip): the page reconnects, and the hub's
    // values do not come on the new socket.
    await sockets[0].close({ code: 4000, reason: "gone" });
    await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    // Shown stale: the marks and the switch as they were.
    await expect(strip(page, "Rev PA").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "lead");
    await expect(strip(page, "Rev ST").getByTestId("fader-link-mark")).toHaveAttribute("data-role", "follow");
    const detail = await openDetail(page, "#4");
    const link = detail.getByTestId("fader-link");
    await expect(link).toHaveAttribute("data-on", "true");
    await expect(link).toHaveAttribute("aria-disabled", "false");
    // A tap: refused with a flash (400 ms: observed, not sampled), nothing sent.
    await link.evaluate((el) => {
      (window as any).linkFlashes = 0;
      new MutationObserver(() => {
        if (el.classList.contains("failed")) (window as any).linkFlashes += 1;
      }).observe(el, { attributes: true, attributeFilter: ["class"] });
    });
    const since = Date.now();
    await link.click();
    await expect.poll(() => page.evaluate(() => (window as any).linkFlashes), { message: "the refusal's flash" }).toBeGreaterThan(0);
    await expect(link).toHaveAttribute("data-on", "true");
    // Nothing reached the hub: the switch the test's client sends after the
    // tap is the only one the event log holds since it (the log is read up
    // to that record, so a set_hub of the tap would be in it).
    live.setHub(REV, true);
    const switched = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "fader_link" && r.ts >= since),
      (r: any[]) => r.length > 0,
      "the test's own switch recorded",
    );
    expect(switched.map((r: any) => r.on)).toEqual([true]);
    expect((await hubStatus()).fader_links).toEqual([{ instance: "band", id: "REV", on: true }]);
  });
});

test.describe("Podklady All on Tuner tags", () => {
  let live: LiveClient;
  let before: number[];
  test.beforeEach(async () => {
    live = await LiveClient.open();
    before = [await live.get("band", DRUMS, "value"), await live.get("band", BASS, "value")];
  });
  test.afterEach(async () => {
    await live.set("band", DRUMS, "value", before[0]);
    await live.set("band", BASS, "value", before[1]);
    await hostLine("band", "tuner track 10 remove");
    await hostLine("band", "tuner track 11 remove");
    await hostLine("master", "tuner track 0 remove");
    await harness("/hub/layout/reset");
    await markersGone();
    live.close();
  });

  test("its tracks are the Tuners' +FADER partners: red without, the same dB in one batch with, 0 dB on a double tap", async ({ page }) => {
    await live.set("band", DRUMS, "value", 0.85);
    await live.set("band", BASS, "value", 0.24);
    await openSurface(page);
    await harness("/hub/layout", { layout: withLinkedPodklady() });
    await selectPage(page, "foh");
    const podklady = page.locator('[data-testid="param-fader"][data-label="Podklady All"]');
    const fader = podklady.getByTestId("fader");

    // No Tuner carries its id: red, no groove, reported.
    await until(
      async () => (await hubStatus()).layout.markers.links,
      (l: any[]) => l.some((r) => r.id === "PODKLADY" && r.leader === "frame" && r.partners === 0),
      "PODKLADY alone",
    );
    await expect(fader).toHaveAttribute("data-binding", "unresolved");
    await expect(podklady.getByTestId("param-groove")).toHaveCount(0);
    await expect(podklady).toHaveClass(/\bvca\b/);

    // Tagged: two grooves, the backing-tracks fader.
    expect(await hostLine("band", "tuner track 10 set '+FADER:PODKLADY'")).toBe("TUNER 1");
    expect(await hostLine("band", "tuner track 11 set '+FADER:PODKLADY'")).toBe("TUNER 1");
    await expect(podklady.getByTestId("param-groove")).toHaveCount(2, { timeout: 10_000 });
    await expect(podklady).toHaveClass(/\bvca\b/);
    await ready(fader);
    expect((await hubStatus()).layout.markers.problems).toEqual([]);
    expect((await hubStatus()).layout.markers.links).toEqual([{ instance: "band", id: "PODKLADY", leader: "frame", partners: 2 }]);

    // A drag: both by the same dB, each frame's writes in one batch; back to the start: exact.
    const since = Date.now();
    await dispatchPointer(fader, [{ type: "pointerdown" }, { type: "pointermove", dy: 0 }, { type: "pointermove", dy: -20 }, { type: "pointermove", dy: -40 }], 31);
    const drums = await until(() => settled(live, DRUMS), (v) => v < 0.85, "Drums down");
    const bass = await settled(live, BASS);
    expect(bass, "Bass down").toBeLessThan(0.24);
    expect(Math.abs(change(0.24, bass) - change(0.85, drums)), `Drums ${drums}, Bass ${bass}`).toBeLessThan(SAME_DB);
    const both = await until(() => batches(since, BT_KEYS), (b) => b.length > 0, "the drag's batches");
    for (const b of both) expect(b.sent.map((i: any) => i.key).sort()).toEqual(BT_KEYS);
    await dispatchPointer(fader, [{ type: "pointermove", dy: 0 }, { type: "pointerup", dy: 0 }], 31);
    await until(() => settled(live, DRUMS), (v) => v === 0.85, "Drums back exactly");
    expect(await settled(live, BASS)).toBe(0.24);

    // A double tap: every track exactly at 0 dB.
    await doubleTap(fader);
    await until(() => settled(live, BASS), (v) => Math.abs(v - 0.85) < 1e-6, "Bass at 0 dB", 8000);
    expect(Math.abs((await settled(live, DRUMS)) - 0.85)).toBeLessThan(1e-6);

    // One partner left: still the backing-tracks fader; the other set's tag is an orphan.
    await hostLine("band", "tuner track 11 remove");
    expect(await hostLine("master", "tuner track 0 set '+FADER:PODKLADY'")).toBe("TUNER 1");
    await expect(podklady.getByTestId("param-groove")).toHaveCount(1, { timeout: 10_000 });
    await expect(podklady).toHaveClass(/\bvca\b/);
    await until(
      async () => (await hubStatus()).layout.markers.problems,
      (p: any[]) => p.some((r) => r.instance === "master" && r.problems.some((x: any) => x.code === "orphan_fader")),
      "the orphan reported",
    );
  });
});
