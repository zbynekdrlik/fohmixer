import type { Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { HUB_SOCKET, LiveClient, RELOAD_KEY, countLoads, dispatchPointer, openSurface, ready, token, until } from "./support/live";

// The hub connection (S4 design note §4, §6; #8 review): the handshake
// reload is bounded across page loads, and a reconnect forgets the hub's
// values until the hub sends them again. Playwright stands between the page
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
  test("forgets the hub's values until the hub sends them again", async ({ page }) => {
    // STAGE AUT on (#104, D18: the merged STAGE button carries it).
    const live = await LiveClient.open();
    try {
      live.setHub("stage_aut", true);
      await until(async () => live.hub.get("stage_aut"), (v) => v === true, "STAGE AUT on");
      const sockets: WebSocketRoute[] = [];
      await page.routeWebSocket(HUB_SOCKET, (ws) => {
        sockets.push(ws);
        const server = ws.connectToServer();
        if (sockets.length === 1) return;
        // Every later socket: all but the hub values (as if the hub had not
        // sent STAGE AUT yet).
        server.onMessage((message) => {
          if (JSON.parse(String(message)).type !== "hub") ws.send(message);
        });
      });
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
      // yet: the button shows no AUT, however long the hub stays silent
      // about it, and a 3 s hold turns nothing (I8).
      await expect(stage).toHaveAttribute("data-aut", "off");
      await page.waitForTimeout(1000);
      await expect(stage).toHaveAttribute("data-aut", "off");
      await dispatchPointer(stage, [{ type: "pointerdown" }, { wait: 3200 }, { type: "pointerup" }]);
      await page.waitForTimeout(300);
      expect(live.hub.get("stage_aut")).toBe(true);
    } finally {
      live.setHub("stage_aut", false);
      await until(async () => live.hub.get("stage_aut"), (v) => v === false, "STAGE AUT off");
      live.close();
    }
  });
});
