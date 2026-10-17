import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  HUB_SOCKET,
  LiveClient,
  RELOAD_KEY,
  countLoads,
  dispatchSnapped,
  hubEvents,
  openSurface,
  pageEvents,
  ready,
  token,
  track,
  until,
  withoutHubValuesAfterTheFirst,
} from "./support/live";

// The hub connection (S4 design note §4, §6; #8 review): the handshake
// reload is bounded across page loads, and a reconnect keeps the hub's
// values, stale, until the hub sends them again (#104 M4). Playwright stands between the page
// and the hub's socket (`routeWebSocket`).

/**
 * Opens the surface logged in, without waiting for a connection; with
 * `reloadedAt`, the browser remembers a handshake reload at that time.
 */
async function openLoggedIn(page: Page, reloadedAt?: number) {
  const seeded = await token();
  await page.addInitScript(
    ([t, key, at]) => {
      try {
        if (!sessionStorage.getItem("e2e-seeded")) {
          localStorage.setItem("fohmixer_token", t as string);
          if (at !== null) localStorage.setItem(key as string, String(at));
          sessionStorage.setItem("e2e-seeded", "1");
        }
      } catch {
        // storage blocked: the test then fails on the login page
      }
    },
    [seeded, RELOAD_KEY, reloadedAt ?? null],
  );
  await page.goto("/");
  await expect(page.getByTestId("stage")).toBeVisible();
}

/** One state of the STAGE button, recorded in the page at the page clock `t`. */
type Look = { t: number; failed: boolean; holding: string | null; full: string | null };

/**
 * Records the button's look at every change of its `class`, `data-holding`
 * or `data-full` from now on (a `MutationObserver`, PR #133's review, M1): a
 * flash is observed, never sampled, so nothing depends on when a read lands.
 */
async function watchLook(button: Locator) {
  await button.evaluate((el) => {
    const w = window as any;
    w.stageLooks = [];
    const note = () =>
      w.stageLooks.push({
        t: performance.now(),
        failed: el.classList.contains("failed"),
        holding: el.getAttribute("data-holding"),
        full: el.getAttribute("data-full"),
      });
    note();
    w.stageWatch = new MutationObserver(note);
    w.stageWatch.observe(el, { attributes: true, attributeFilter: ["class", "data-holding", "data-full"] });
  });
}

/** The looks `watchLook` recorded, its observer stopped. */
async function watchedLooks(page: Page): Promise<Look[]> {
  return page.evaluate(() => {
    const w = window as any;
    w.stageWatch.disconnect();
    return w.stageLooks;
  });
}

/** The page clock at the start of each `failed` flash in `looks`. */
function flashStarts(looks: Look[]): number[] {
  return looks.filter((look, i) => look.failed && !(i > 0 && looks[i - 1].failed)).map((look) => look.t);
}

/** The page's `rail` steps since `since` (the page clock), once `ok` holds. */
async function railSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the rail steps uploaded",
    15_000,
  );
}

test.describe("The protocol handshake", () => {
  // Every socket is closed at once with the hub's reload code (4001).
  test.beforeEach(async ({ page }) => {
    await page.routeWebSocket(HUB_SOCKET, (ws) => ws.close({ code: 4001, reason: "reload" }));
  });

  test("a hub that keeps asking for a reload gets one, then the page keeps itself", async ({ page }) => {
    const loads = countLoads(page);
    await openLoggedIn(page);
    // The first close reloads the page; the reloaded page, asked again, keeps
    // itself and reconnects: the reload's time survives the load.
    await expect.poll(loads, { timeout: 10_000 }).toBe(2);
    await page.waitForTimeout(4000);
    expect(loads()).toBe(2);
    expect(Number(await page.evaluate((key) => localStorage.getItem(key), RELOAD_KEY))).toBeGreaterThan(
      Date.now() - 60_000,
    );
  });

  test("after a reload less than a minute ago (another page load) the page does not reload", async ({ page }) => {
    const loads = countLoads(page);
    await openLoggedIn(page, Date.now() - 10_000);
    await page.waitForTimeout(4000);
    expect(loads()).toBe(1);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
  });
});

test.describe("A reconnect", () => {
  test("keeps the hub's values, stale, until the hub sends them again; a hold meanwhile is refused", async ({ page }) => {
    // STAGE AUT on (#104, D18: the merged STAGE button carries it).
    const live = await LiveClient.open();
    try {
      live.setHub("stage_aut", true);
      await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on");
      const sockets = await withoutHubValuesAfterTheFirst(page);
      await openSurface(page);
      const stage = page.getByTestId("stage-mics");
      await ready(stage);
      await expect(stage).toHaveAttribute("data-aut", "run");
      // The socket drops (a Wi-Fi blip): the page reconnects. (Playwright
      // closes the hub's side with the same code, which a browser socket
      // takes only as 1000 or 3000-4999; 4001 is the hub's reload code.)
      await sockets[0].close({ code: 4000, reason: "gone" });
      await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
      // Connected again, but STAGE AUT has no value from this connection
      // yet: the button keeps showing the last one, stale (#104 M4, as
      // Live's values, L2), however long the hub stays silent about it.
      await expect(stage).toHaveAttribute("data-aut", "run");
      await page.waitForTimeout(1000);
      await expect(stage).toHaveAttribute("data-aut", "run");
      // A hold meanwhile cannot act (I8): no fill, no hold's line, and at
      // 1 s (the owner's ruling on #104) it is refused, recorded so; the
      // flag stays.
      const since = await page.evaluate(() => performance.timeOrigin + performance.now());
      // One finger's sequence timed in the page: the look at 0.4 s and the
      // lift at 1.5 s, both read in the page's timer order around the 1 s
      // mark, and the refusal's 400 ms flash observed (PR #133's review, M1).
      await watchLook(stage);
      const press = await dispatchSnapped(stage, [
        { type: "pointerdown" },
        { wait: 400 },
        { snap: true },
        { wait: 1100 },
        { snap: true },
        { type: "pointerup" },
        { wait: 100 },
      ]);
      const looks = await watchedLooks(page);
      expect(press.snaps[0].holding).toBe("false");
      expect(press.snaps[0].sub).not.toMatch(/drž|pusť/);
      const flashes = flashStarts(looks).map((t) => t - press.start);
      expect(flashes, "one refusal flash, at the 1 s mark, before the lift").toHaveLength(1);
      expect(flashes[0]).toBeGreaterThanOrEqual(1000);
      expect(flashes[0]).toBeLessThan(press.snaps[1].at);
      expect(looks.map((look) => look.full), "never full").not.toContain("true");
      expect(looks.map((look) => look.holding), "never a fill").not.toContain("true");
      await page.waitForTimeout(300);
      expect(live.hub.get("stage_aut")).toBe(true);
      expect(await railSteps(since, (s) => s.includes("refused"))).toEqual(["press", "refused"]);
    } finally {
      live.setHub("stage_aut", false);
      await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
      live.close();
    }
  });

  test("a full hold whose connection drops before its lift is refused at the lift; the flag and the mics stay (#104)", async ({ page }) => {
    // The lift checks again whether the hold can act (PR #133's review, M2): a
    // value kept from the full mark would switch STAGE AUT after a
    // reconnect whose hub has not told it yet.
    const live = await LiveClient.open();
    const mics = track("Mics Stage #");
    try {
      live.setHub("stage_aut", false);
      await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
      await live.set("band", mics, "mute", false);
      const sockets = await withoutHubValuesAfterTheFirst(page);
      await openSurface(page);
      const stage = page.getByTestId("stage-mics");
      await ready(stage);
      await expect(stage).toHaveAttribute("data-aut", "off");
      const since = await page.evaluate(() => performance.timeOrigin + performance.now());
      await watchLook(stage);
      // Held past the 1 s mark: full, nothing sent (read in the page's
      // timer order).
      const press = await dispatchSnapped(stage, [{ type: "pointerdown" }, { wait: 1300 }, { snap: true }]);
      expect(press.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: "pusť — zapne AUT", failed: false });
      expect(live.hub.get("stage_aut")).toBe(false);
      // The finger stays down; the socket drops and the page reconnects,
      // and the new socket has no STAGE AUT from the hub: the full press can
      // no longer act, so it shows no fill.
      await sockets[0].close({ code: 4000, reason: "gone" });
      await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
      await expect(stage).toHaveAttribute("data-full", "false");
      // The lift: refused, one flash after it, nothing sent.
      const lift = await dispatchSnapped(stage, [{ type: "pointerup" }, { wait: 600 }]);
      const looks = await watchedLooks(page);
      const flashes = flashStarts(looks);
      expect(flashes, "one refusal flash, at the lift").toHaveLength(1);
      expect(flashes[0]).toBeGreaterThanOrEqual(lift.start);
      await page.waitForTimeout(300);
      expect(live.hub.get("stage_aut")).toBe(false);
      expect(await live.get("band", mics, "mute")).toBe(false);
      expect(await railSteps(since, (s) => s.includes("refused"))).toEqual(["press", "full", "refused"]);
    } finally {
      live.setHub("stage_aut", false);
      await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
      live.close();
    }
  });
});
