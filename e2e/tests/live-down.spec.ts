import { readFileSync } from "node:fs";
import type { Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { FIXTURE_LAYOUT, FOH_LIVES, HUB_SOCKET, MBC_MARKERS, frames, harness, hostLine, hubStatus, openSurface, selectPage, token, until } from "./support/live";

// A Live that is down, said in words (#178; the owner's approved
// docs/mockups/live-down-v2.html): no card or overlay anywhere. A strip of a
// Live that is not running (its host stopped) or has no set (its host on an
// empty set: none of the layout's names resolve) is dimmed with NOT RUNNING /
// NO SET under its name and `—` on its readout, never red; the column's foot
// says each Live in words with a one-line hint. I5's red stays for a single
// bad binding of a Live whose set is loaded. It clears by itself when Live
// comes back.

const IPAD = { width: 1194, height: 834 };
const UP = 15_000;

/** Every strip of `instance` on the page carries `live` (none: no attribute).
 *
 * #192: the page re-creates a Live's strips when it recomposes its lines (a
 * Live coming back, a resize), so the strips are read again on every poll,
 * all in one task inside the page: a count followed by waits on each index
 * never waited on a strip added after the count, and `locator.evaluateAll`
 * queries and reads in two round trips, between which the page can replace
 * them. */
async function stripsSay(page: Page, instance: string, live: string | null) {
  const wrong = () =>
    page.evaluate(
      ({ instance, live }) => {
        const strips = [...document.querySelectorAll(`[data-testid="strip"][data-instance="${instance}"]`)];
        if (strips.length === 0) return ["no strip"];
        return strips
          .filter((s) => s.getAttribute("data-live") !== live)
          .map((s) => `${s.getAttribute("data-track")}: ${s.getAttribute("data-live")}`);
      },
      { instance, live },
    );
  await expect.poll(wrong, { timeout: UP, message: `every ${instance} strip says ${live ?? "nothing"}` }).toEqual([]);
}

/** The controls drawn red (I5) inside `scope`'s matches. */
async function redIn(page: Page, scope: string): Promise<number> {
  return page.locator(`${scope} [data-binding="unresolved"], ${scope}[data-binding="unresolved"]`).count();
}

/** The column's room: the foot's height, the rail's rows and the stage
 * miniature's presence (#178: the foot never takes the rail's room, so the
 * miniature never goes because a Live is down). */
async function room(page: Page) {
  const foot = (await page.getByTestId("column-foot").boundingBox())!.height;
  const rows = await page.locator(".rail-main").evaluate((e) => getComputedStyle(e).gridTemplateRows.split(" "));
  return { foot, rows, mini: await page.getByTestId("stage-mini").count() };
}

async function roomKept(page: Page) {
  const now = await room(page);
  expect(now.foot, "the foot keeps its two rows").toBeLessThanOrEqual(34);
  for (const row of now.rows) expect(row, "every rail row keeps 44 px").toBe("44px");
  expect(now.mini, "the stage miniature stays").toBe(1);
}

const foot = (page: Page, instance: string) => page.locator(`[data-testid="badge"][data-instance="${instance}"]`);

/** The hosts a test stopped or moved, started again on the test site. */
const changed = new Set<string>();

async function host(name: string, how: "stop" | "start", site?: string) {
  changed.add(name);
  await harness(`/host/${name}/${how}`, site ? { site } : {});
}

test.describe("A Live that is down (#178)", () => {
  test.afterEach(async () => {
    for (const name of changed) await harness(`/host/${name}/start`);
    changed.clear();
    // The hub has both Lives back before the next spec opens a page (one
    // token: every login hashes the PIN).
    const bearer = await token();
    await until(
      () => hubStatus(bearer),
      (s) => s.instances.filter((i: any) => FOH_LIVES.includes(i.name)).every((i: any) => i.online && i.set_name === "Test Site"),
      "both Lives online on the test site",
      15_000,
    );
  });

  test("Lives not running: the strips and the foot say so in words, nothing red, and it clears by itself", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(foot(page, "band").getByTestId("badge-word")).toHaveText("on");
    await expect(page.getByTestId("strip-hint")).toHaveCount(0);
    await roomKept(page);
    const masterZone = page.locator('[data-testid="strip"][data-instance="master"] .strip-zone').first();
    const travel = (await masterZone.boundingBox())!.height;

    await host("band", "stop");
    await host("master", "stop");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, "off");
      await expect(foot(page, instance)).toHaveAttribute("data-live", "off");
      await expect(foot(page, instance).getByTestId("badge-word")).toHaveText("not running");
    }
    const first = page.locator('[data-testid="strip"][data-instance="band"]').first();
    await expect(first.getByTestId("strip-why")).toHaveText("NOT RUNNING");
    await expect(first.getByTestId("strip-down-db")).toHaveText("—");
    await expect(first.getByTestId("strip-down-db")).toBeVisible();
    await expect(first.getByTestId("db")).toBeHidden();
    // What to do, dim on each down strip (no pill), never in the foot.
    const hint = first.getByTestId("strip-hint");
    await expect(hint).toHaveText("Start Live on the FOH PC. The faders come back by themselves.");
    await expect(page.getByTestId("strip-hint")).toHaveCount(await page.locator('[data-testid="strip"][data-live]').count());
    const look = await hint.evaluate((e) => {
      const cs = getComputedStyle(e);
      const zone = e.parentElement!.getBoundingClientRect();
      const box = e.getBoundingClientRect();
      return {
        color: cs.color,
        background: cs.backgroundColor,
        border: cs.borderTopWidth,
        opacity: cs.opacity,
        inside: box.left >= zone.left - 0.5 && box.right <= zone.right + 0.5 && box.top >= zone.top && box.bottom <= zone.bottom,
      };
    });
    expect(look).toEqual({ color: "rgb(154, 154, 180)", background: "rgba(0, 0, 0, 0)", border: "0px", opacity: "1", inside: true });
    await expect(page.getByTestId("column-foot").getByTestId("strip-hint")).toHaveCount(0);
    await roomKept(page);
    // Not red: no control, no TechAlert frame, no status light, no badge dot.
    expect(await redIn(page, "body")).toBe(0);
    await expect(page.locator(".status:visible")).toHaveCount(0);
    await expect(page.locator(".badge-dot")).toHaveCount(0);
    // The word is in the marks line's place: no height moved.
    expect((await masterZone.boundingBox())!.height).toBe(travel);
    const why = await first.getByTestId("strip-why").evaluate((e) => getComputedStyle(e).color);
    expect(why).toBe("rgb(245, 166, 35)");
    // Nothing covers the left side: an extra tab opens as usual.
    const tab = page.getByTestId("presenter-tab");
    await tab.click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "presenter");
    await expect(page.getByTestId("stage-mini")).toHaveCount(0);
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-extra", "none");

    await host("band", "start");
    await host("master", "start");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, null);
      await expect(foot(page, instance)).toHaveAttribute("data-live", "on", { timeout: UP });
    }
    await expect(page.getByTestId("strip-hint")).toHaveCount(0);
    await expect(page.getByTestId("strip-why")).toHaveCount(0);
    await roomKept(page);
    await expect(first.getByTestId("db")).toBeVisible();
  });

  // #184 PR D: a hub on the PC says `script_missing` when Live runs there
  // while its FohMixer script has not answered for 60 s. No Live runs on the
  // runner, so the hub's word is put on the stopped hosts' `instance`
  // messages through the routed socket; the page's words and room are what
  // this checks (the hub's watch is its own Rust tests).
  test("Live running without its script: NO SCRIPT on its strips, `no script` in the foot, the room kept (#184)", async ({ page }) => {
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      const server = ws.connectToServer();
      server.onMessage((message) => {
        if (typeof message !== "string") return ws.send(message);
        const msg = JSON.parse(message);
        if (msg.type === "instance" && msg.online === false) ws.send(JSON.stringify({ ...msg, script_missing: true }));
        else ws.send(message);
      });
    });
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });

    await host("band", "stop");
    await host("master", "stop");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, "off");
      await expect(foot(page, instance)).toHaveAttribute("data-live", "off");
      await expect(foot(page, instance).getByTestId("badge-word")).toHaveText("no script");
    }
    const first = page.locator('[data-testid="strip"][data-instance="band"]').first();
    await expect(first.getByTestId("strip-why")).toHaveText("NO SCRIPT");
    await expect(first.getByTestId("strip-hint")).toHaveText("Live runs without the FohMixer script. Run the install again.");
    expect(await redIn(page, "body")).toBe(0);
    await roomKept(page);

    await host("band", "start");
    await host("master", "start");
    for (const instance of ["band", "master"]) {
      await stripsSay(page, instance, null);
      await expect(foot(page, instance).getByTestId("badge-word")).toHaveText("on", { timeout: UP });
    }
    await expect(page.getByTestId("strip-hint")).toHaveCount(0);
  });

  test("a Live with no set: NO SET on its strips, its bindings not red, the other Live untouched", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "master")).toHaveAttribute("data-live", "on", { timeout: UP });
    // The fixture's master set has bindings that do not resolve (I5, red),
    // and so may the band's: the test needs the master's to be some.
    const masterRed = await redIn(page, '[data-testid="strip"][data-instance="master"]');
    expect(masterRed).toBeGreaterThan(0);
    const bandRed = await redIn(page, '[data-testid="strip"][data-instance="band"]');

    await host("band", "start", "empty");
    await stripsSay(page, "band", "noset");
    await expect(foot(page, "band")).toHaveAttribute("data-live", "noset");
    await expect(foot(page, "band").getByTestId("badge-word")).toHaveText("no set");
    await expect(page.locator('[data-testid="strip"][data-instance="band"]').first().getByTestId("strip-why")).toHaveText("NO SET");
    await expect(page.locator('[data-testid="strip"][data-instance="band"]').first().getByTestId("strip-hint")).toHaveText("Open the set in Live.");
    await roomKept(page);
    // The band Live answers, yet none of its strips is red (it was all red,
    // #178's report); the master Live's own bindings keep I5 as they were.
    expect(await redIn(page, '[data-testid="strip"][data-instance="band"]')).toBe(0);
    await expect(page.locator('.alert-wash[data-binding="unresolved"]')).toHaveCount(0);
    await stripsSay(page, "master", null);
    expect(await redIn(page, '[data-testid="strip"][data-instance="master"]')).toBe(masterRed);
    await expect(foot(page, "master").getByTestId("badge-word")).toHaveText("on");

    await host("band", "start");
    await stripsSay(page, "band", null);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(page.getByTestId("strip-hint")).toHaveCount(0);
    // A bad binding of the loaded set is red again (I5).
    await until(
      () => redIn(page, '[data-testid="strip"][data-instance="band"]'),
      (n) => n === bandRed,
      "the band's own red bindings back",
      UP,
    );
  });

  // The 1.38.0 check on the PC (#178 reopened): the hint ran over the fader
  // scale's numbers ("Start Live on6", "themselves.12"). A down strip's hint
  // never meets a visible scale label of its own strip, on the iPad and on a
  // phone on its side, both Lives not running and the band's set missing.
  test("a down strip's hint never runs over its fader scale's numbers (iPad, phone on its side)", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });

    /** Per down strip on screen: its hint inside the zone, and the visible
     * texts of the zone it meets (the scale's labels, the group's name, the
     * LUFS reading; their text: none is right). Read in one task inside the
     * page (#192): `evaluateAll`'s two round trips can read strips the page
     * has just replaced. */
    const clashes = () =>
      page.evaluate(() =>
        [...document.querySelectorAll('[data-testid="strip"][data-live]')]
          .filter((s) => s.getBoundingClientRect().width > 0)
          .map((s) => {
            const hint = s.querySelector('[data-testid="strip-hint"]');
            const zone = s.querySelector(".strip-zone")!.getBoundingClientRect();
            if (!hint) return { track: s.getAttribute("data-track"), hint: false, inside: false, meets: [] as string[] };
            const h = hint.getBoundingClientRect();
            const meets = [...s.querySelectorAll(".scale .tick, .group-side, .lufs, .lufs-cap")]
              .filter((t) => {
                const cs = getComputedStyle(t);
                const r = t.getBoundingClientRect();
                const shown = cs.visibility !== "hidden" && cs.display !== "none" && r.width > 0 && r.height > 0;
                const crosses = r.left < h.right && r.right > h.left && r.top < h.bottom && r.bottom > h.top;
                return shown && crosses;
              })
              .map((t) => `${t.className}: ${t.textContent ?? ""}`);
            const inside = h.left >= zone.left - 0.5 && h.right <= zone.right + 0.5 && h.top >= zone.top && h.bottom <= zone.bottom;
            return { track: s.getAttribute("data-track"), hint: true, inside, meets };
          }),
      );

    // Both sizes measured before any assert, so a failure shows each.
    const check = async (live: string) => {
      const found: Record<string, Awaited<ReturnType<typeof clashes>>> = {};
      for (const size of [IPAD, { width: 844, height: 390 }]) {
        await page.setViewportSize(size);
        await stripsSay(page, "band", live);
        // The page sets --strip-w and its narrow flags in its own resize handling.
        await frames(page, 2);
        found[`${live} at ${size.width}x${size.height}`] = await clashes();
      }
      await page.setViewportSize(IPAD);
      for (const [where, seen] of Object.entries(found)) {
        expect(seen.length, `down strips on screen, ${where}`).toBeGreaterThan(0);
        const bad = seen.filter((s) => !s.hint || !s.inside || s.meets.length > 0);
        expect(bad, where).toEqual([]);
      }
    };

    /** The scale labels of `instance`'s strips that are not shown (a Live
     * that is on keeps every one). #192: read in one task inside the page.
     * Through `evaluateAll` (a query, then a read: two round trips) a strip
     * the page replaced in between was read removed, and a removed label's
     * computed `visibility` is `""` in both engines: three replaced master
     * strips read as 27 hidden labels. Waited for (`expect.poll`), as the
     * strips may still be recomposing after the resize back to the iPad.
     * No label at all reads -1, so the same read proves there are some. */
    const hiddenTicks = (instance: string) =>
      page.evaluate((instance) => {
        const ticks = [...document.querySelectorAll(`[data-testid="strip"][data-instance="${instance}"] .scale .tick`)];
        return ticks.length === 0 ? -1 : ticks.filter((t) => getComputedStyle(t).visibility !== "visible").length;
      }, instance);

    await host("band", "stop");
    await host("master", "stop");
    await stripsSay(page, "master", "off");
    await check("off");

    await host("master", "start");
    await host("band", "start", "empty");
    await stripsSay(page, "master", null);
    await check("noset");
    // Only a down strip loses its numbers: the master Live is on.
    await expect.poll(() => hiddenTicks("master"), { timeout: UP, message: "master's scale labels all shown" }).toBe(0);

    await host("band", "start");
    await stripsSay(page, "band", null);
    await expect.poll(() => hiddenTicks("band"), { timeout: UP, message: "band's scale labels all shown" }).toBe(0);
  });

  test("every Live on: one line per Live beside the counter, the rail keeps its room", async ({ page }) => {
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await expect(foot(page, "band")).toHaveAttribute("data-live", "on", { timeout: UP });
    await expect(foot(page, "master")).toHaveAttribute("data-live", "on");
    await roomKept(page);
    // One line per Live, stacked beside the counter where the dots sat.
    const counter = (await page.getByTestId("dropouts").boundingBox())!;
    const lines = (await page.locator(".column .badges").boundingBox())!;
    expect(Math.abs(lines.y + lines.height / 2 - (counter.y + counter.height / 2))).toBeLessThan(2);
    const band = (await foot(page, "band").boundingBox())!;
    const master = (await foot(page, "master").boundingBox())!;
    expect(master.y).toBeGreaterThanOrEqual(band.y + band.height - 0.5);
    expect(lines.x).toBeGreaterThan(counter.x + counter.width);
  });
});

/**
 * The site's kind of frame (#178's failed live check of 2026-11-12): the
 * strips of a tags group (MARKERS), STAGE, the talkback toggle and
 * TechAlert named by `+CTRL` ids, REVERB and AUTOTUNE, all made by Tuner
 * markers in Live, never by a track's name.
 */
function markerFrame(): any {
  const frame = JSON.parse(readFileSync(FIXTURE_LAYOUT, "utf-8"));
  const foh = frame.pages.find((p: any) => p.id === "foh");
  const ctrl = (id: string) => ({ instance: "band", anchor: { kind: "ctrl", id } });
  foh.rows[1].sections.push({ kind: "group", id: "markers", title: "MARKERS", tags: "MARKERS" });
  foh.rail.find((c: any) => c.kind === "stage").binding = ctrl("stage");
  foh.rail.find((c: any) => c.label === "ZVUKAR").targets = [{ binding: ctrl("talkback"), prop: "mute", on: false, off: true }];
  const at = foh.rail.findIndex((c: any) => c.kind === "param_toggle" && c.label === "AUTOTUNE");
  foh.rail.splice(at, 1, { kind: "autotune", label: "AUTOTUNE" });
  foh.rail.splice(foh.rail.findIndex((c: any) => c.kind === "stage") + 1, 0, { kind: "reverb", label: "REVERB" });
  frame.global.find((c: any) => c.kind === "alert").binding = ctrl("alert");
  return frame;
}

/** The Tuners of `markerFrame` (generic labels: a public repo), per Live. */
const TUNERS: Record<string, [number, string][]> = {
  band: [
    [0, "+CTRL:talkback"],
    [1, '"Hand two" +G:MARKERS'],
    [2, '"Rev ST" +G:MARKERS +REVERB'],
    [3, "+AUTOTUNE"],
    [6, "+CTRL:stage"],
    [7, "+CTRL:alert"],
  ],
  master: [[1, '"Master two" +G:MARKERS']],
};
const MARKER_TRACKS = Object.values(TUNERS).flat().length;

// The PC's morning (#178, the owner's report and the failed live check of
// 2026-11-12): the PC rebooted, the hub started while both Lives were still
// down and has never heard them since. The page still draws every strip it
// drew before, dimmed with NOT RUNNING and `—`, and nothing is red: no
// control, no rail button, no TechAlert frame round the page.
test.describe("A hub started while both Lives are down (#178)", () => {
  test.afterEach(async () => {
    // Both Lives back on the test site (their Tuners gone with the restart),
    // the fixture served again, and the hub's markers read empty from both
    // Lives before the next spec opens a page.
    for (const name of ["band", "master"]) await harness(`/host/${name}/start`);
    await harness("/hub/layout/reset");
    const bearer = await token();
    await until(
      () => hubStatus(bearer),
      (s) => s.instances.filter((i: any) => FOH_LIVES.includes(i.name)).every((i: any) => i.online && i.set_name === "Test Site") && s.layout.markers.found === MBC_MARKERS,
      "both Lives online on the test site and no Tuner marker left",
      20_000,
    );
  });

  test("its strips are drawn NOT RUNNING, nothing is red, the foot says so", async ({ page }) => {
    await harness("/hub/layout", { layout: markerFrame() });
    for (const [instance, tuners] of Object.entries(TUNERS)) {
      for (const [track, name] of tuners) expect(await hostLine(instance, `tuner track ${track} set '${name}'`)).toBe("TUNER 1");
    }
    // The hub has composed every marker (each +CTRL id on its one Tuner).
    const bearer = await token();
    await until(
      () => hubStatus(bearer),
      (s) => s.layout.markers.found === MARKER_TRACKS + MBC_MARKERS && s.layout.markers.ctrls.every((c: any) => c.tuners === 1),
      "every Tuner marker composed",
      15_000,
    );
    // Both Lives quit, then the PC's reboot: the hub starts with no Live.
    await harness("/host/band/stop");
    await harness("/host/master/stop");
    await harness("/hub/restart", { rotate_secret: false });
    await until(
      () => hubStatus(),
      (s) => s.instances.filter((i: any) => FOH_LIVES.includes(i.name)).every((i: any) => !i.online),
      "the restarted hub with both Lives down",
      15_000,
    );

    // The iPad switched on.
    await page.setViewportSize(IPAD);
    await openSurface(page);
    await selectPage(page, "foh");
    for (const instance of ["band", "master"]) {
      await expect(foot(page, instance)).toHaveAttribute("data-live", "off", { timeout: UP });
      await expect(foot(page, instance).getByTestId("badge-word")).toHaveText("not running");
    }
    // Every strip the markers made is there, dimmed with its word and `—`.
    const markers = page.locator('.slot[data-group="markers"] [data-testid="strip"]');
    await expect(markers).toHaveCount(3, { timeout: UP });
    for (const [label, instance] of [
      ["Hand two", "band"],
      ["Rev ST", "band"],
      ["Master two", "master"],
    ]) {
      const strip = page.locator(`.slot[data-group="markers"] [data-testid="strip"][data-label="${label}"]`);
      await expect(strip).toHaveAttribute("data-instance", instance);
      await expect(strip).toHaveAttribute("data-live", "off");
      await expect(strip.getByTestId("strip-why")).toHaveText("NOT RUNNING");
      await expect(strip.getByTestId("strip-down-db")).toHaveText("—");
    }
    await stripsSay(page, "band", "off");
    await stripsSay(page, "master", "off");
    // Nothing red: the hub's answers for every binding are in (a second
    // after the strips), no control is unresolved, no rail button and no
    // TechAlert frame round the page.
    await page.waitForTimeout(1000);
    expect(await redIn(page, "body")).toBe(0);
    // The rail buttons of a down Live wait, dimmed (the DECIDED calm look).
    const rail = [
      page.getByTestId("stage-mics"),
      page.getByTestId("reverb"),
      page.getByTestId("autotune"),
      page.locator('[data-testid="param-toggle"][data-label="ZVUKAR"]'),
      page.getByTestId("rail").getByTestId("alert-toggle"),
    ];
    for (const button of rail) {
      await expect(button).toHaveCount(1);
      await expect(button).toHaveAttribute("data-binding", "waiting");
      await expect(button).toHaveAttribute("aria-disabled", "true");
    }
    const wash = page.locator(".alert-wash");
    await expect(wash).toHaveCount(1);
    await expect(wash).not.toHaveAttribute("data-binding", "unresolved");
  });
});
