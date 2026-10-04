import type { Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, centre, frames, hubEvents, impair, openSurface, pageEvents, panning, ready, shown, strip, track, until } from "./support/live";

// The look of a pan's and a toggle's write Live has not confirmed (#43, PR C,
// following PR B's fader look, design note §4.3): they keep showing Live's
// value (P2; PR B's decision 7) and only get the fader's outline, amber once
// a release is 1 s old without its ack (`unconfirmed`), red once it was too
// old to send again after a reconnect (`not_sent`); no text, no ghost.

const HAND2 = track("Hand2 #");
const PAN = panning(HAND2);
/** `--warn` and `--danger`. */
const AMBER = "rgb(245, 166, 35)";
const RED = "rgb(255, 59, 48)";

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  // Never leave the link held for the next test.
  await impair.block(false);
  live.close();
});

/** The page loses its link: new connections are held, the open ones reset. */
async function linkDown(page: Page) {
  await impair.block(true);
  await impair.drop();
  await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
}

test.describe("The look of an open write on the pan and the toggles", () => {
  // A dropped link resets the page's socket: WebKit reports the reset of an
  // open socket as a console error (e2e.md; Chromium says nothing).
  test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

  test("a pan and a mute written while the link is down are outlined amber after 1 s and red when not sent again", async ({ page }) => {
    const panBefore = await live.get("band", PAN, "value");
    const muteBefore = await live.get("band", HAND2, "mute");
    await live.set("band", PAN, "value", 0);
    await live.set("band", HAND2, "mute", false);
    try {
      await openSurface(page);
      const s = strip(page, "Hand2 #");
      const pan = s.getByTestId("pan");
      const mute = s.getByTestId("mute");
      const dot = pan.locator(".pan-dot");
      await ready(pan);
      await ready(mute);
      await until(() => shown(pan), (v) => Math.abs(v) < 1e-3, "the pan centred");
      await expect(mute).toHaveAttribute("data-muted", "false");
      await expect(pan).toHaveAttribute("data-intent", "confirmed");
      await expect(mute).toHaveAttribute("data-intent", "confirmed");
      const label = (await mute.textContent())?.trim();

      const cutAt = await page.evaluate(() => performance.timeOrigin + performance.now());
      await linkDown(page);
      // The pan dragged right and let go, the mute tapped: both writes kept.
      const { x, y } = await centre(pan);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 4; i++) await page.mouse.move(x + 6 * i, y);
      await frames(page);
      await page.mouse.up();
      const m = await centre(mute);
      await page.mouse.click(m.x, m.y);
      // 1 s after the releases without an ack: amber outlines.
      await expect(pan).toHaveAttribute("data-intent", "unconfirmed", { timeout: 3000 });
      await expect(mute).toHaveAttribute("data-intent", "unconfirmed", { timeout: 3000 });
      await expect(dot).toHaveCSS("outline-style", "solid");
      await expect(dot).toHaveCSS("outline-color", AMBER);
      await expect(mute).toHaveCSS("outline-style", "solid");
      await expect(mute).toHaveCSS("outline-color", AMBER);
      // The toggle still shows Live's value (P2).
      await expect(mute).toHaveAttribute("data-muted", "false");

      // Back after more than 2 s: too old to send blindly, red.
      await page.waitForTimeout(1500);
      await impair.block(false);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 5000 });
      await expect(pan).toHaveAttribute("data-intent", "not_sent");
      await expect(mute).toHaveAttribute("data-intent", "not_sent");
      await expect(dot).toHaveCSS("outline-color", RED);
      await expect(mute).toHaveCSS("outline-color", RED);
      // No text was added, and Live got neither write.
      expect((await mute.textContent())?.trim()).toBe(label);
      expect((await pan.textContent())?.trim()).toBe("");
      await page.waitForTimeout(1000);
      expect(await live.get("band", PAN, "value"), "the pan's old release never reached Live").toBe(0);
      expect(await live.get("band", HAND2, "mute"), "the mute's old tap never reached Live").toBe(false);
      await expect(mute).toHaveAttribute("data-muted", "false");
      // The page's flight recorder sent the mute's tap (a `tap`: no up
      // follows) and the pan's touch up to the event log after the
      // reconnect (the contract the forensics timeline reads).
      const muteKey = `band|${HAND2}|mute`;
      const panKey = `band|${PAN}|value`;
      const touches = await until(
        async () =>
          (await hubEvents())
            .filter((r) => r.ev === "trace")
            .flatMap((r) => r.events)
            .filter((e: any) => e.ev === "touch" && e.t >= cutAt),
        (all) => all.some((e: any) => e.what === "tap" && e.keys.includes(muteKey)),
        "the mute's tap in the event log",
        10_000,
      );
      expect(touches.filter((e: any) => e.keys.includes(muteKey)).every((e: any) => e.what === "tap"), "a mute's touch is a tap only").toBe(true);
      expect(touches.some((e: any) => e.what === "down" && e.keys.includes(panKey))).toBe(true);
      expect(touches.some((e: any) => e.what === "up" && e.keys.includes(panKey))).toBe(true);
      // The page also recorded when each write turned unconfirmed (its
      // release 1 s old without an ack) and when it was not sent again:
      // only the page knows when it drew them (#43 PR E).
      // Each record once, as the timeline reads them (`pageEvents`). One
      // reconnect: a second resend would give a write a new seq.
      const intents = await until(
        async () =>
          pageEvents(await hubEvents())
            .filter((e: any) => e.ev === "intent" && e.t >= cutAt)
            .sort((a: any, b: any) => a.t - b.t),
        (all) => [panKey, muteKey].every((key) => all.some((e: any) => e.key === key && e.state === "not_sent")),
        "the writes' states in the event log",
        10_000,
      );
      for (const key of [panKey, muteKey]) {
        const mine = intents.filter((e: any) => e.key === key);
        expect(mine.map((e: any) => e.state), key).toEqual(["unconfirmed", "not_sent"]);
        expect(mine[0].seq, key).toBe(mine[1].seq);
      }
    } finally {
      await live.set("band", PAN, "value", panBefore);
      await live.set("band", HAND2, "mute", muteBefore);
    }
  });
});
