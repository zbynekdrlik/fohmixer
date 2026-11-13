import { test, expect, type Page } from "./support/fixtures";
import { BASE, PIN, openSurface } from "./support/live";

// The MBC login (#184): a second PIN whose login reaches only the Live set
// mbc. On the harness's FOH hub (band and master, no mbc) such a login sees
// nothing of the FOH side; its foot says who is logged in and LOG OUT goes
// back to the login. The FOH engineer's page is unchanged: no line.

const MBC_PIN = process.env.E2E_MBC_PIN || "";

async function typePin(page: Page, pin: string) {
  for (const digit of pin) {
    await page.locator(`[data-testid="pin-key"][data-key="${digit}"]`).click();
  }
}

async function logInAsMbc(page: Page) {
  expect(MBC_PIN, "E2E_MBC_PIN is set by the job").not.toBe("");
  await page.goto("/");
  await expect(page.getByTestId("login")).toBeVisible();
  await typePin(page, MBC_PIN);
  await expect(page.getByTestId("login-line")).toHaveText("logged in: MBC");
}

test.describe("The MBC login", () => {
  test("an MBC login shows only MBC things", async ({ page }) => {
    await logInAsMbc(page);
    await expect(page.getByTestId("stage")).toBeVisible();
    // Give the hub's attach messages time to arrive: none of them is FOH's.
    await page.waitForTimeout(1500);
    await expect(page.getByTestId("strip")).toHaveCount(0);
    await expect(page.locator('[data-testid="rail"] .btn')).toHaveCount(0);
    for (const id of ["deck-tab", "ableset-tab", "iem-row", "presenter-tab", "badge"]) {
      await expect(page.getByTestId(id), id).toHaveCount(0);
    }
    await expect(page.getByTestId("logout")).toHaveText("LOG OUT");
    // The hub itself refuses the FOH side to this token.
    const token = await page.evaluate(() => localStorage.getItem("fohmixer_token"));
    const status = await fetch(`${BASE}/api/status`, { headers: { authorization: `Bearer ${token}` } });
    expect(status.status).toBe(200);
    expect((await status.json()).instances).toEqual([]);
    const songmap = await fetch(`${BASE}/api/songmap?instance=band`, {
      headers: { authorization: `Bearer ${token}` },
    });
    expect(songmap.status).toBe(403);
    expect((await songmap.json()).code).toBe("ROLE");
  });

  test("LOG OUT returns to the login", async ({ page }) => {
    await logInAsMbc(page);
    await page.getByTestId("logout").click();
    await expect(page.getByTestId("login")).toBeVisible();
    await expect(page).toHaveURL(/\/login$/);
    expect(await page.evaluate(() => localStorage.getItem("fohmixer_token"))).toBeNull();
    // A reload stays on the login: the token is gone.
    await page.reload();
    await expect(page.getByTestId("login")).toBeVisible();
  });
});

test.describe("The engineer's page", () => {
  test("has no login line and no LOG OUT", async ({ page }) => {
    await openSurface(page);
    await expect(page.getByTestId("strip").first()).toBeVisible();
    await expect(page.getByTestId("column-foot")).toBeVisible();
    await expect(page.getByTestId("login-line")).toHaveCount(0);
    await expect(page.getByTestId("logout")).toHaveCount(0);
    expect(PIN).not.toBe("");
  });
});
