import { test, expect } from "./support/fixtures";

test.describe("The app loads", () => {
  test("the root renders the app with its title", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app")).toBeVisible();
    await expect(page).toHaveTitle("fohmixer");
    await expect(page.getByTestId("status")).toHaveText("fohmixer — čaká na pripojenie k Abletonu");
    // The loading shell is gone once the app has mounted.
    await expect(page.locator("#app-shell")).toHaveCount(0);
  });

  test("a deep link renders the same app", async ({ page }) => {
    const response = await page.goto("/foo/bar");
    expect(response?.status()).toBe(200);
    await expect(page.getByTestId("app")).toBeVisible();
    await expect(page.getByTestId("version")).toBeVisible();
    await expect(page).toHaveTitle("fohmixer");
  });

  test("a missing file is a 404, not the app", async ({ page }) => {
    const response = await page.request.get("/missing.js");
    expect(response.status()).toBe(404);
  });
});

// The app's own download can fail: the WiFi drops while a tablet loads the
// page, or the page is left before the WASM module arrived. The loading shell
// then says so and offers a reload; the console gets no uncaught error.

/** The WASM module Trunk emits (`fohmixer-ui-<hash>_bg.wasm`, an unpadded hex u64). */
const APP_WASM = /\/fohmixer-ui-[0-9a-f]{1,16}_bg\.wasm$/;

test.describe("A failed app download", () => {
  // The WASM download is aborted on purpose and the browser reports the
  // failed request (Chromium: net::ERR_CONNECTION_FAILED; WebKit words it its
  // own way), so exactly that resource-load line is declared.
  test.use({ allowedConsole: [/^Failed to load resource: /] });

  test("the shell shows the network hint and a retry that loads the app, with no uncaught error", async ({ page }) => {
    let blocked = true;
    await page.route(APP_WASM, (route) => (blocked ? route.abort("connectionfailed") : route.continue()));
    await page.goto("/");

    const failed = page.getByTestId("load-error");
    await expect(failed).toBeVisible({ timeout: 10_000 });
    await expect(failed).toContainText("same network as the Ableton PC");
    await expect(page.locator(".shell-spinner")).toBeHidden();

    blocked = false;
    await failed.getByRole("button", { name: "Try Again" }).click();
    await expect(page.getByTestId("app")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("load-error")).toHaveCount(0);
  });
});
