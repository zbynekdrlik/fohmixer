import { test, expect, type Page } from "./support/fixtures";
import { PIN, harness, openSurface } from "./support/live";

// The engineer login (S4 design note §6, spec X8): a PIN pad, the token kept on
// the device, and a refused token back to the login once.

async function typePin(page: Page, pin: string) {
  for (const digit of pin) {
    await page.locator(`[data-testid="pin-key"][data-key="${digit}"]`).click();
  }
}

test.describe("The engineer login", () => {
  test("the right PIN opens the surface and the device keeps the token", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("login")).toBeVisible();
    await expect(page).toHaveURL(/\/login$/);
    await typePin(page, PIN);
    await expect(page.getByTestId("stage")).toBeVisible();
    await expect(page).toHaveURL(/:\d+\/$/);
    expect(await page.evaluate(() => localStorage.getItem("fohmixer_token"))).toBeTruthy();
    // A reload keeps the device logged in: no login during a service.
    await page.reload();
    await expect(page.getByTestId("stage")).toBeVisible();
    await expect(page.getByTestId("login")).toHaveCount(0);
  });

  test("the keypad's clear and backspace edit the PIN", async ({ page }) => {
    await page.goto("/");
    await typePin(page, "12");
    await expect(page.locator(".pin-dot.filled")).toHaveCount(2);
    await page.locator('[data-testid="pin-key"][data-key="⌫"]').click();
    await expect(page.locator(".pin-dot.filled")).toHaveCount(1);
    await page.locator('[data-testid="pin-key"][data-key="CLR"]').click();
    await expect(page.locator(".pin-dot.filled")).toHaveCount(0);
    await expect(page.getByTestId("login-error")).toHaveCount(0);
  });
});

test.describe("A wrong PIN", () => {
  // The hub answers a wrong PIN with 401, which the browser reports as a
  // failed resource load.
  test.use({ allowedConsole: [/^Failed to load resource: /] });

  test("shows the error and clears the dots", async ({ page }) => {
    await page.goto("/");
    const wrong = String((Number(PIN) + 1) % 10000).padStart(4, "0");
    await typePin(page, wrong);
    await expect(page.getByTestId("login-error")).toHaveText("Wrong PIN");
    await expect(page.locator(".pin-dot.filled")).toHaveCount(0);
    await expect(page.getByTestId("stage")).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("fohmixer_token"))).toBeNull();
  });
});

test.describe("A token the hub no longer accepts", () => {
  // The hub restarts with a new signing secret. While it is down the page's
  // reconnect requests fail, and its token check then gets 401: the browser
  // reports both as failed resource loads (and a socket that could not
  // connect, should one attempt land in the gap).
  test.use({
    allowedConsole: [[/^Failed to load resource: /, /^WebSocket connection to '.*' failed/], { scope: "test" }],
  });

  test("sends the app back to the login once, with no reload", async ({ page }) => {
    await openSurface(page);
    let loads = 0;
    page.on("load", () => (loads += 1));
    await harness("/hub/restart", { rotate_secret: true });
    await expect(page.getByTestId("login")).toBeVisible({ timeout: 15_000 });
    await expect(page).toHaveURL(/\/login$/);
    expect(await page.evaluate(() => localStorage.getItem("fohmixer_token"))).toBeNull();
    // It stays on the login: no reload loop.
    await page.waitForTimeout(3000);
    await expect(page.getByTestId("login")).toBeVisible();
    expect(loads).toBe(0);
    // A new login works against the restarted hub.
    await typePin(page, PIN);
    await expect(page.getByTestId("stage")).toBeVisible();
  });
});
