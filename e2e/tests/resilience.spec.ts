import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { LiveClient, centre, frames, impair, openSurface, ready, shown, strip, track, until, volume } from "./support/live";

// The page's resilience on a bad link (#43, PR B; design note §1 L1-L4, §4,
// §6.2 tests 1-4). The pages reach the hub through the harness's impair proxy
// (`impair`: stall, drop, block); the test's own LiveClient talks to the hub
// directly, so it reads Live whatever the page's link does.

const TARGET = volume(track("Hand2 #"));
/** Live's volume before each test, put back after it. */
const START = 0.5;
/** How close the fader's `data-value` (4 decimals) and Live's float32 value are. */
const SAME = 1e-3;

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("band", TARGET, "value", START);
});
test.afterEach(async () => {
  // Never leave the link held for the next test.
  await impair.block(false);
  await live.set("band", TARGET, "value", START);
  live.close();
});

const liveValue = () => live.get("band", TARGET, "value");

/** The surface, its Hand2 # fader ready at START. */
async function surfaceAtStart(page: Page): Promise<Locator> {
  await openSurface(page);
  const fader = strip(page, "Hand2 #").getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - START) < SAME, "the fader at 0.5");
  return fader;
}

/** The page loses its link: new connections are held, the open ones reset. */
async function linkDown(page: Page) {
  await impair.block(true);
  await impair.drop();
  await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
}

/** A mouse drag of `steps` × 10 px up the fader; the finger stays down when `hold`. */
async function dragUp(page: Page, fader: Locator, steps: number, hold = false): Promise<number> {
  const { x, y } = await centre(fader);
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= steps; i++) await page.mouse.move(x, y - 10 * i);
  if (!hold) await page.mouse.up();
  await frames(page);
  return shown(fader);
}

test.describe("The control link's resilience (L1-L4)", () => {
  // A dropped link resets the page's socket: WebKit reports the reset of an
  // open socket as "WebSocket connection to '…' failed: Error receiving data:
  // Connection reset by peer" (Chromium says nothing; local probe, #43).
  test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

  test("a fader released while the link is down reaches Live after the link returns, within 2 s (L1)", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    const released = await dragUp(page, fader, 5);
    expect(released, "the fader moved while the link was down").toBeGreaterThan(START + 0.02);
    expect(await liveValue(), "nothing reached Live yet").toBe(START);
    await impair.block(false);
    await until(liveValue, (v) => Math.abs(v - released) < SAME, "Live at the released value", 2000);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
  });

  test("a fader touched while the socket reconnects moves, and its held value reaches Live (L2)", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    // The fader keeps Live's last value (stale) and takes the touch.
    await expect(fader).toHaveAttribute("aria-disabled", "false");
    const held = await dragUp(page, fader, 5, true);
    try {
      expect(held, "the touched fader moved").toBeGreaterThan(START + 0.02);
      // The link returns under the still finger: the held value is sent
      // after the hello, though the finger never moves again.
      await impair.block(false);
      await until(liveValue, (v) => Math.abs(v - held) < SAME, "Live at the held value", 2000);
    } finally {
      await page.mouse.up();
    }
    await frames(page);
    expect(Math.abs((await shown(fader)) - held)).toBeLessThan(SAME);
  });

  test("a release inside a 1.5 s stall never shows Live's value from before the stall (L3)", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    // Every frame from here on: the fader's value and intent state.
    await fader.evaluate((el) => {
      const w = window as any;
      w.__samples = [];
      w.__sampling = true;
      const step = () => {
        w.__samples.push([performance.now(), el.getAttribute("data-value"), el.getAttribute("data-intent")]);
        if (w.__sampling) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
    await impair.stall(1500);
    // A quick drag and release inside the stall (its pointer events timed in
    // the page: real mouse moves would eat into the stall).
    const { x, y } = await centre(fader);
    const releasedAt = await fader.evaluate(
      async (el, [clientX, clientY]) => {
        const fire = (type: string, dy: number) =>
          el.dispatchEvent(
            new PointerEvent(type, {
              pointerId: 31,
              pointerType: "touch",
              isPrimary: true,
              clientX,
              clientY: clientY - dy,
              bubbles: true,
              cancelable: true,
            }),
          );
        const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));
        fire("pointerdown", 0);
        for (let i = 1; i <= 5; i++) {
          await wait(16);
          fire("pointermove", 12 * i);
        }
        await wait(16);
        fire("pointerup", 60);
        return performance.now();
      },
      [x, y],
    );
    await frames(page);
    const released = await shown(fader);
    expect(released, "the release moved the fader").toBeGreaterThan(START + 0.02);
    // The stall ends, Live applies the release, and the hold after its ack
    // (1 s with touch shaping) runs out.
    await until(liveValue, (v) => Math.abs(v - released) < SAME, "Live at the released value", 5000);
    await page.waitForTimeout(1500);
    const samples: [number, string | null, string | null][] = await page.evaluate(() => {
      const w = window as any;
      w.__sampling = false;
      return w.__samples;
    });
    const after = samples.filter(([at]) => at > releasedAt);
    expect(after.length, "frames drawn through the stall and the hold").toBeGreaterThan(20);
    const off = after.filter(([, value]) => Math.abs(Number(value) - released) > SAME);
    expect(off, "every frame after the release shows the released value, never Live's 0.5 from before the stall").toEqual([]);
    expect(
      after.some(([, , state]) => state === "unconfirmed"),
      "unconfirmed (amber) while the stall held the ack past 1 s",
    ).toBe(true);
  });

  test("a release dropped for 5 s is not applied after the link returns: drawn not sent, and the next touch starts from the cap (L4)", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    const released = await dragUp(page, fader, 5);
    expect(released, "the fader moved while the link was down").toBeGreaterThan(START + 0.02);
    await page.waitForTimeout(5000);
    await impair.block(false);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 5000 });
    await expect(fader).toHaveAttribute("data-intent", "not_sent");
    // Not applied: Live keeps its value.
    await page.waitForTimeout(1500);
    expect(await liveValue(), "the 5 s old release never reached Live").toBe(START);
    // Drawn not sent: the cap at the release, outlined red, a ghost at Live's
    // value, and no text at all.
    expect(Math.abs((await shown(fader)) - released)).toBeLessThan(SAME);
    const cap = fader.locator(".fader-cap");
    await expect(cap).toHaveCSS("outline-style", "solid");
    await expect(cap).toHaveCSS("outline-color", "rgb(255, 59, 48)");
    await expect(fader.locator(".fader-ghost")).toBeVisible();
    expect(Math.abs(Number(await fader.getAttribute("data-ghost")) - START)).toBeLessThan(SAME);
    expect((await fader.textContent())?.trim(), "no text on the fader").toBe("");
    // The next touch starts from the cap, not from Live's value.
    const next = await dragUp(page, fader, 2);
    expect(next, "moved on from the cap").toBeGreaterThan(released);
    await until(liveValue, (v) => Math.abs(v - next) < SAME, "Live at the new touch's value");
    await expect(fader).not.toHaveAttribute("data-intent", "not_sent");
    await expect(fader.locator(".fader-ghost")).toBeHidden();
  });
});
