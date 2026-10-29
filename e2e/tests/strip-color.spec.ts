import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  hubEvents,
  hubStatus,
  openDetail,
  openSurface,
  ready,
  resetColors,
  strip,
  token,
  track,
  until,
} from "./support/live";

// The strips' colours (#164; the owner's approved mockup
// docs/mockups/strip-color-v1.html): a strip's colour is chosen in its
// channel detail's COLOR (NONE and twelve calm colours), kept by the hub
// (`colors.json`), so every tablet shows it at once, and never taken from
// or written to Live. It shows only as the name button's 3 px edge and the
// detail's chip; a strip with none shows NONE's calm grey.

const HAND2 = track("Hand2 #");
/** NONE's calm grey and the palette's TEAL and PINK, as computed colours. */
const NONE = "rgb(115, 115, 138)";
const TEAL = "rgb(75, 179, 166)";
const PINK = "rgb(226, 127, 168)";

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

/** Errors, warnings and page errors of a second page (the fixture guards the first). */
function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
  return problems;
}

/** The 3 px edge at the name button's left: its colour and width. */
async function edgeOf(page: Page, name: string, instance = "band"): Promise<{ color: string; width: string }> {
  return strip(page, name, instance)
    .getByTestId("mute")
    .evaluate((el) => {
      const after = getComputedStyle(el, "::after");
      return { color: after.backgroundColor, width: after.width };
    });
}

/** The fader's travel of a strip (px). */
async function travel(page: Page, name: string): Promise<number> {
  return (await strip(page, name).getByTestId("fader").boundingBox())!.height;
}

/** A key of the detail's COLOR by its palette key (`none`, `red`, …). */
function key(detail: Locator, id: string): Locator {
  return detail.locator(`[data-testid="colour-key"][data-colour="${id}"]`);
}

/** Back to the mix from an open detail. */
async function closeDetail(page: Page) {
  await page.getByTestId("detail-exit").click();
  await expect(page.getByTestId("detail")).toHaveCount(0);
}

test.describe("Strip colours chosen in SEL (#164)", () => {
  // The iPad's FOH page, where a strip's travel is 322 px.
  test.use({ viewport: { width: 1194, height: 834 } });

  test("a colour picked in the detail colours the strip's edge on every page, NONE resets it, nothing reaches Live", async ({ page, context }) => {
    await live.set("band", HAND2, "mute", false);
    const liveColour = await live.get("band", HAND2, "color");
    await resetColors();
    const other = await context.newPage();
    const bearer = await token();
    const colours = async () => (await hubStatus(bearer)).colors.strips;
    try {
      await openSurface(page);
      await ready(strip(page, "Hand2 #").getByTestId("mute"));
      // Every strip starts as NONE: Live's colour is not copied.
      await expect.poll(() => edgeOf(page, "Hand2 #")).toEqual({ color: NONE, width: "3px" });
      const before = await travel(page, "Hand2 #");
      expect(Math.abs(before - 322), `a fader's travel is 322 px: ${before}`).toBeLessThanOrEqual(1);

      // A second tablet.
      await other.setViewportSize({ width: 1194, height: 834 });
      const otherConsole = watchConsole(other);
      await openSurface(other);
      await ready(strip(other, "Hand2 #").getByTestId("mute"));
      await expect.poll(() => edgeOf(other, "Hand2 #")).toEqual({ color: NONE, width: "3px" });

      const since = Date.now();
      const detail = await openDetail(page, "Hand2 #");
      const section = detail.getByTestId("detail-colour");
      await expect(section.locator("h2")).toHaveText("COLOR");
      await expect(section.getByTestId("colour-now")).toHaveText("NONE");
      // NONE and the twelve, in the mockup's order; NONE chosen.
      const keys = section.getByTestId("colour-key");
      await expect(keys).toHaveCount(13);
      expect(await keys.evaluateAll((all) => all.map((k) => k.getAttribute("data-colour")))).toEqual([
        "none", "red", "orange", "amber", "lime", "green", "teal", "cyan", "blue", "violet", "purple", "pink", "sand",
      ]);
      await expect(key(detail, "none")).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, "teal")).toHaveAttribute("aria-pressed", "false");
      // Every key a tap target of its own (Pointer Events, its touches its own).
      expect(
        await key(detail, "teal").evaluate((el) => {
          const e = new Event("touchstart", { bubbles: true, cancelable: true });
          el.dispatchEvent(e);
          return e.defaultPrevented;
        }),
      ).toBe(true);
      // The chip of NONE: the grey.
      await expect(detail.getByTestId("detail-name")).toHaveCSS("background-color", NONE);

      // One tap: TEAL, marked like the selected tab, its name beside the title.
      await key(detail, "teal").click();
      await expect(key(detail, "teal")).toHaveAttribute("aria-pressed", "true");
      await expect(key(detail, "none")).toHaveAttribute("aria-pressed", "false");
      await expect(section.getByTestId("colour-now")).toHaveText("TEAL");
      await expect(key(detail, "teal")).toHaveCSS("box-shadow", /inset/);
      await expect(detail.getByTestId("detail-name")).toHaveCSS("background-color", TEAL);
      // The other tablet at once, the strip's edge only.
      await expect.poll(() => edgeOf(other, "Hand2 #")).toEqual({ color: TEAL, width: "3px" });
      expect((await edgeOf(other, "Hand2 #", "master")).color, "the master's Hand2 # keeps its own").toBe(NONE);
      await closeDetail(page);
      await expect.poll(() => edgeOf(page, "Hand2 #")).toEqual({ color: TEAL, width: "3px" });
      expect(await travel(page, "Hand2 #"), "the travel unchanged").toBeCloseTo(before, 1);
      // The hub keeps it: the status counts one colour.
      await until(colours, (n) => n === 1, "the hub keeping one colour");

      // Another colour, then NONE: back to the neutral grey everywhere.
      const again = await openDetail(page, "Hand2 #");
      await expect(key(again, "teal")).toHaveAttribute("aria-pressed", "true");
      await key(again, "pink").click();
      await expect.poll(() => edgeOf(other, "Hand2 #")).toEqual({ color: PINK, width: "3px" });
      await key(again, "none").click();
      await expect(again.getByTestId("colour-now")).toHaveText("NONE");
      await expect(key(again, "none")).toHaveAttribute("aria-pressed", "true");
      await expect.poll(() => edgeOf(other, "Hand2 #")).toEqual({ color: NONE, width: "3px" });
      await closeDetail(page);
      await expect.poll(() => edgeOf(page, "Hand2 #")).toEqual({ color: NONE, width: "3px" });
      await until(colours, (n) => n === 0, "the hub keeping no colour");

      // Nothing reached Live: no write recorded, Live's colour as it was.
      expect(await live.get("band", HAND2, "color"), "Live's track colour unchanged").toBe(liveColour);
      const records = await hubEvents();
      const sets = records.filter((r: any) => r.ev === "set" && r.ts >= since);
      expect(sets, JSON.stringify(sets)).toEqual([]);
      const chosen = records
        .filter((r: any) => r.ev === "colors" && r.ts >= since && r.what === "set")
        .map((r: any) => [r.strip.label, r.color]);
      expect(chosen).toEqual([
        ["Hand2 #", "teal"],
        ["Hand2 #", "pink"],
        ["Hand2 #", null],
      ]);
      expect(await travel(page, "Hand2 #"), "the travel unchanged").toBeCloseTo(before, 1);
      expect(otherConsole, "the second page's console").toEqual([]);
    } finally {
      await other.close();
      await resetColors();
    }
  });
});
