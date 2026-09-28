import { test, expect } from "./support/fixtures";
import { openSurface, selectPage } from "./support/live";

test.describe("Version label (version-on-dashboard)", () => {
  test("landing page shows the backend version as v<semver>", async ({ page }) => {
    const api = await (await page.request.get("/api/version")).json();
    await page.goto("/");
    const label = page.getByTestId("version").first();
    await expect(label).toBeVisible();
    const text = ((await label.textContent()) ?? "").trim();
    expect(text).toMatch(/^v\d+\.\d+\.\d+(-dev\.\d+)?$/);
    expect(text).toBe(`v${api.version}`);
  });

  test("the surface shows it over the tab bar and on the Conf page", async ({ page }) => {
    const api = await (await page.request.get("/api/version")).json();
    await openSurface(page);
    await expect(page.getByTestId("stage").getByTestId("version")).toHaveText(`v${api.version}`);
    await selectPage(page, "conf");
    await expect(page.getByTestId("conf-version")).toContainText(api.version);
  });
});
