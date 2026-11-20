import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  BASE,
  HARNESS,
  LiveClient,
  centre,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  strip,
  token,
  track,
  until,
  volume,
} from "./support/live";

// The scene automation (#207; the owner's approved mockup
// docs/mockups/scenes-v3.html): Companion tells the hub the service's scene
// (`POST /api/scene` with the automation token the harness stored), the
// rules set per fader in its channel detail's SCENES move the fader (a glide
// to its dB over its time, a mute or an unmute), the column foot names the
// last scene and the mixer's edge flashes green once. The engineer's hand
// wins: a drag of a fader under a glide stops it there. Scene names are
// invented (intro, talk).

const HAND2 = track("Hand2 #");
const UNITY = 0.85;

/** The FOH hub's automation token (the harness stored it at its start). */
async function automationToken(): Promise<string> {
  const answer = await (await fetch(`${HARNESS}/hub/automation`)).json();
  if (!answer.token) throw new Error("the harness has no automation token (no hub?)");
  return answer.token;
}

/** `POST /api/scene` as Companion's scene button sends it: the status and the body. */
async function scene(name: string, bearer: string): Promise<{ status: number; body: any }> {
  const response = await fetch(`${BASE}/api/scene`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${bearer}` },
    body: JSON.stringify({ scene: name }),
  });
  return { status: response.status, body: await response.json() };
}

/** The harness's empty scenes.json again, served when this returns. */
async function resetScenes() {
  await fetch(`${HARNESS}/hub/scenes/reset`, { method: "POST", body: "{}" });
  const bearer = await token();
  await until(
    async () => (await hubStatus(bearer)).scenes,
    (s: any) => s !== undefined && s.known === 0 && s.strips === 0,
    "the hub serving the empty scenes.json",
  );
}

/** The SCENES row of `name` in an open detail. */
function row(detail: Locator, name: string): Locator {
  return detail.locator(`[data-testid="scene-row"][data-scene="${name}"]`);
}

/** Back to the mix from an open detail. */
async function closeDetail(page: Page) {
  await page.getByTestId("detail-exit").click();
  await expect(page.getByTestId("detail")).toHaveCount(0);
}

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", volume(HAND2), "value", UNITY);
  await live.set("band", HAND2, "mute", false);
  await resetScenes();
});
test.afterEach(async () => {
  await live.set("band", volume(HAND2), "value", UNITY);
  await live.set("band", HAND2, "mute", false);
  live.close();
  await resetScenes();
});

test.describe("The scene automation (#207)", () => {
  // The iPad's FOH page.
  test.use({ viewport: { width: 1194, height: 834 } });

  test("a rule set in SCENES glides the fader when Companion sends its scene; the foot names it and the edge flashes once", async ({
    page,
  }) => {
    const auto = await automationToken();
    const bearer = await token();
    await openSurface(page);
    await ready(strip(page, "Hand2 #").getByTestId("fader"));

    // A scene no rule names: learned, nothing moves, no flash; the foot
    // names it with the page's local time.
    const first = await scene("intro", auto);
    expect(first).toEqual({ status: 200, body: { scene: "intro", moved: 0 } });
    const line = page.getByTestId("column-foot").getByTestId("scene-line");
    await expect(line).toHaveText(/^scene intro · \d\d:\d\d$/);
    await expect(page.getByTestId("scene-flash")).toHaveCount(0);
    // The line costs the stage miniature's room, never the rail's rows.
    const rows = await page.locator(".rail-main").evaluate((e) => getComputedStyle(e).gridTemplateRows.split(" "));
    for (const r of rows) expect(r, "every rail row keeps 44 px").toBe("44px");
    await expect(page.getByTestId("stage-mini"), "the stage miniature stays").toHaveCount(1);
    const lineBox = (await line.boundingBox())!;
    expect(lineBox.height, "one calm line").toBeLessThanOrEqual(14);
    console.log(`scene line in the foot (${test.info().project.name}): ${lineBox.height} px`);

    // SCENES in the detail: after LAYOUTS, before MUTE ALARM, empty.
    const detail = await openDetail(page, "Hand2 #");
    const section = detail.getByTestId("detail-scenes");
    await expect(section.locator("h2")).toHaveText("SCENES");
    await expect(section.getByTestId("scenes-empty")).toHaveText("No scene moves this fader.");
    const order = await detail.getByTestId("detail-right").evaluate((box) =>
      [...box.querySelectorAll('[data-testid="detail-scenes"], [data-testid="detail-alarm"]')].map((e) =>
        e.getAttribute("data-testid"),
      ),
    );
    expect(order).toEqual(["detail-scenes", "detail-alarm"]);
    // Its keys own their touches.
    const add = section.getByTestId("scene-add");
    expect(
      await add.evaluate((el) => {
        const e = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(e);
        return e.defaultPrevented;
      }),
    ).toBe(true);

    // + ADD SCENE: the sheet offers the scene heard, marked NOW; one tap
    // adds a row at the fader's dB (0 dB) and 2 s.
    await add.click();
    const sheet = page.getByTestId("scene-sheet");
    const pick = sheet.locator('[data-testid="scene-pick"][data-scene="intro"]');
    await expect(pick).toContainText("NOW");
    await pick.click();
    await expect(sheet).toHaveCount(0);
    const intro = row(detail, "intro");
    await expect(intro.getByTestId("scene-db")).toHaveText("0.0 dB");
    await expect(intro.getByTestId("scene-time")).toHaveText("2 s");
    await expect(intro.getByTestId("scene-mute")).toHaveText("MUTE —");
    await expect(intro).toHaveAttribute("data-last", "true");
    await expect(section.getByTestId("scenes-count")).toHaveText("1");

    // −10 dB in whole steps, 1 s, and the chip to UNMUTE (— → MUTE → UNMUTE).
    for (let db = 1; db <= 10; db++) {
      await intro.getByTestId("scene-db-down").click();
      await expect(intro.getByTestId("scene-db")).toHaveText(`−${db}.0 dB`);
    }
    await intro.getByTestId("scene-time-down").click();
    await expect(intro.getByTestId("scene-time")).toHaveText("1 s");
    await intro.getByTestId("scene-mute").click();
    await expect(intro.getByTestId("scene-mute")).toHaveText("MUTE");
    await intro.getByTestId("scene-mute").click();
    await expect(intro.getByTestId("scene-mute")).toHaveText("UNMUTE");
    // The hub keeps it.
    await until(
      async () => (await hubStatus(bearer)).scenes,
      (s: any) => s.strips === 1,
      "the hub keeping the rule",
    );
    await closeDetail(page);

    // Live muted: the scene unmutes it and glides 0 → −10 dB over 1 s.
    await live.set("band", HAND2, "mute", true);
    const since = Date.now();
    const second = await scene("intro", auto);
    expect(second).toEqual({ status: 200, body: { scene: "intro", moved: 1 } });
    const flash = page.getByTestId("scene-flash");
    await expect(flash).toHaveCount(1);
    expect(await flash.evaluate((el) => getComputedStyle(el).animationName)).toBe("scene-flash");
    expect(await flash.evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
    const records = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "scene_glide" && r.ts >= since),
      (rs: any[]) => rs.some((r) => r.what === "stop" && r.why === "done"),
      "the glide done",
    );
    expect(records[0]).toMatchObject({ what: "start", label: "Hand2 #", scene: "intro", to_db: -10, ms: 1000 });
    const end: number = await live.get("band", volume(HAND2), "value");
    expect(end).toBeLessThan(UNITY);
    expect(end).toBeGreaterThan(0.4);
    expect(await live.get("band", HAND2, "mute")).toBe(false);
    // The strip shows Live's new level.
    await until(
      async () => Number(await strip(page, "Hand2 #").getByTestId("fader").getAttribute("data-value")),
      (v) => Math.abs(v - end) < 1e-3,
      "the strip's fader at Live's level",
    );
  });

  test("a drag of a fader under a glide stops the glide there: the engineer's hand wins", async ({ page }) => {
    const auto = await automationToken();
    const bearer = await token();
    await scene("talk", auto);
    // The rule as a page sends it: Hand2 # to −40 dB over 10 s.
    const answer = await live.ask({
      type: "set_scene_rule",
      strip: { instance: "band", label: "Hand2 #" },
      scene: "talk",
      rule: { db: -40, mute: null, time_ms: 10000 },
    });
    expect(answer.type).toBe("result");
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    const since = Date.now();
    expect((await scene("talk", auto)).body.moved).toBe(1);
    // The glide runs: Live goes down.
    await until(() => live.get("band", volume(HAND2), "value"), (v) => v < UNITY - 0.02, "the glide under way");
    // A finger takes the cap and pushes it up.
    const cap = strip(page, "Hand2 #").getByTestId("fader-cap");
    const c = await centre(cap);
    await page.mouse.move(c.x, c.y);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(c.x, c.y - 6 * i);
    await page.mouse.up();
    const stops = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "scene_glide" && r.ts >= since && r.what === "stop"),
      (rs: any[]) => rs.length > 0,
      "the glide stopped",
    );
    expect(stops[0]).toMatchObject({ why: "hand", label: "Hand2 #", scene: "talk" });
    // Nothing more from the rule: Live keeps the hand's level.
    const held: number = await live.get("band", volume(HAND2), "value");
    await page.waitForTimeout(800);
    expect(await live.get("band", volume(HAND2), "value")).toBe(held);
    expect((await hubStatus(bearer)).scenes.glides).toBe(0);
  });

  test("a wrong token is refused and moves nothing", async () => {
    const answer = await scene("intro", "not-the-token-0123456789abcdef0123456789");
    expect(answer.status).toBe(401);
    expect(answer.body.code).toBe("UNAUTHORIZED");
    expect((await hubStatus(await token())).scenes.known).toBe(0);
  });
});
