import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  frames,
  hostLine,
  impair,
  openSurface,
  panning,
  ready,
  selectPage,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

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

/**
 * A finger on `control` as dispatched pointer events (id 51), moved up by
 * `dy` px and, unless `hold`, lifted: timed inside the page, and able to
 * stay down while the real mouse does something else.
 */
async function touchUp(control: Locator, dy: number, hold = false) {
  const { x, y } = await centre(control);
  await control.evaluate(
    (el, [clientX, clientY, up, keep]) => {
      const fire = (type: string, at: number) =>
        el.dispatchEvent(
          new PointerEvent(type, {
            pointerId: 51,
            pointerType: "touch",
            isPrimary: true,
            clientX,
            clientY: at,
            bubbles: true,
            cancelable: true,
          }),
        );
      fire("pointerdown", clientY);
      fire("pointermove", clientY - up);
      if (!keep) fire("pointerup", clientY - up);
    },
    [x, y, dy, hold ? 1 : 0],
  );
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
    const meter = strip(page, "Hand2 #").getByTestId("meter");
    const status = strip(page, "Hand2 #").getByTestId("status");
    await expect(status).toHaveAttribute("data-state", "bound");
    expect(await hostLine("band", 'meter "Hand2 #" 0.8')).toBe("METER 1");
    try {
      await until(async () => Number(await meter.getAttribute("data-level")), (v) => v > 0.3, "the meter up");
      await linkDown(page);
      // A stale level is no level: the meter falls instead of freezing, and
      // the status light says Live's values are not fresh.
      await until(async () => Number(await meter.getAttribute("data-level")), (v) => v < 0.01, "the meter down", 5000);
      await expect(status).toHaveAttribute("data-state", "unbound");
    } finally {
      await hostLine("band", 'meter "Hand2 #" off');
    }
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
      const cap = el.querySelector(".fader-cap") as Element;
      const ghost = el.querySelector(".fader-ghost");
      w.__samples = [];
      w.__sampling = true;
      const step = () => {
        w.__samples.push([
          performance.now(),
          el.getAttribute("data-value"),
          el.getAttribute("data-intent"),
          getComputedStyle(cap).outlineColor,
          ghost ? getComputedStyle(ghost).display : "none",
        ]);
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
    // The stall runs on 1.5 s from the release (a running stall is
    // extended): the ack is held well past the 1 s that makes it unconfirmed.
    await impair.stall(1500);
    await frames(page);
    const released = await shown(fader);
    expect(released, "the release moved the fader").toBeGreaterThan(START + 0.02);
    // The stall ends, Live applies the release, and the hold after its ack
    // (1 s with touch shaping) runs out.
    await until(liveValue, (v) => Math.abs(v - released) < SAME, "Live at the released value", 5000);
    await page.waitForTimeout(1500);
    const samples: [number, string | null, string | null, string, string][] = await page.evaluate(() => {
      const w = window as any;
      w.__sampling = false;
      return w.__samples;
    });
    const after = samples.filter(([at]) => at > releasedAt);
    expect(after.length, "frames drawn through the stall and the hold").toBeGreaterThan(20);
    const off = after.filter(([, value]) => Math.abs(Number(value) - released) > SAME);
    expect(off, "every frame after the release shows the released value, never Live's 0.5 from before the stall").toEqual([]);
    // Unconfirmed while the stall held the ack past 1 s: the cap outlined
    // amber (--warn) and the ghost line shown; neither once it is confirmed.
    const amber = after.filter(([, , state]) => state === "unconfirmed");
    expect(amber.length, "unconfirmed while the stall held the ack past 1 s").toBeGreaterThan(0);
    for (const [, , , outline, ghost] of amber) {
      expect(outline).toBe("rgb(245, 166, 35)");
      expect(ghost).toBe("block");
    }
    const [, , lastState, , lastGhost] = after[after.length - 1];
    expect(lastState).toBe("confirmed");
    expect(lastGhost).toBe("none");
  });

  test("a fader rebuilt while the link is down shows its write, takes the next touch and goes on from the cap", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    const released = await dragUp(page, fader, 5);
    expect(released, "the fader moved while the link was down").toBeGreaterThan(START + 0.02);
    // A page switch rebuilds the strip while the link is down and the write waits.
    await selectPage(page, "cue");
    await selectPage(page, "foh");
    const rebuilt = strip(page, "Hand2 #").getByTestId("fader");
    await frames(page);
    expect(Math.abs((await shown(rebuilt)) - released), "the rebuilt cap shows the write, not 0").toBeLessThan(SAME);
    // Still down: the rebuilt fader keeps Live's last value (stale) and takes
    // the next touch (L2), which goes on from the cap.
    await expect(rebuilt).toHaveAttribute("aria-disabled", "false");
    const next = await dragUp(page, rebuilt, 2);
    expect(next, "moved on from the cap").toBeGreaterThan(released);
    await impair.block(false);
    await until(liveValue, (v) => Math.abs(v - next) < SAME, "Live at the new touch's value", 3000);
  });

  test("a not-sent release that Live already holds is confirmed by Live's value", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    // A drag past the top: the release is exactly 1.0, Live's top.
    await touchUp(fader, 4000);
    await frames(page);
    expect(await shown(fader)).toBe(1);
    const releasedAt = Date.now();
    // Live gets that value without this page (as when the hub applied the
    // release and its ack was lost with the socket).
    await live.set("band", TARGET, "value", 1.0);
    await page.waitForTimeout(Math.max(0, 2500 - (Date.now() - releasedAt)));
    await impair.block(false);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    // Too old to send again, but Live's fresh value is the release: confirmed, not red.
    await expect(fader).toHaveAttribute("data-intent", "confirmed");
  });

  test("a fader taken away under a finger counts as released: its old write is not sent after the link returns", async ({ page }) => {
    const fader = await surfaceAtStart(page);
    await linkDown(page);
    // A finger holds the fader up (never lifted) while the mouse switches the page.
    await touchUp(fader, 40, true);
    await frames(page);
    expect(await shown(fader), "the held fader moved").toBeGreaterThan(START + 0.02);
    await selectPage(page, "cue");
    const goneAt = Date.now();
    // Back after 2.5 s: a released write that old is not sent (L4); a held one would be.
    await page.waitForTimeout(Math.max(0, 2500 - (Date.now() - goneAt)));
    await impair.block(false);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await page.waitForTimeout(1500);
    expect(await liveValue(), "the taken-away fader's write never reached Live").toBe(START);
    await selectPage(page, "foh");
    await expect(strip(page, "Hand2 #").getByTestId("fader")).toHaveAttribute("data-intent", "not_sent");
  });

  test("a pan touched again while its write waits is held: its value goes when the link returns", async ({ page }) => {
    const PAN = panning(track("Hand2 #"));
    const before = await live.get("band", PAN, "value");
    await live.set("band", PAN, "value", 0);
    try {
      await openSurface(page);
      const pan = strip(page, "Hand2 #").getByTestId("pan");
      await ready(pan);
      await until(() => shown(pan), (v) => Math.abs(v) < SAME, "the pan centred");
      await linkDown(page);
      const { x, y } = await centre(pan);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 4; i++) await page.mouse.move(x + 6 * i, y);
      // Read under the finger: 100 ms after a release the pan shows Live's
      // (stale) value again (L3's rule is the fader's; a slow WebKit frame
      // read after the release saw 0).
      await frames(page);
      const moved = await shown(pan);
      await page.mouse.up();
      expect(moved, "the pan moved while the link was down").toBeGreaterThan(0.05);
      // Touched again and held still for 2.5 s: its write is held again, so
      // its first release no longer counts (L4).
      await page.mouse.down();
      try {
        await page.waitForTimeout(2500);
        await impair.block(false);
        await until(() => live.get("band", PAN, "value"), (v) => Math.abs(v - moved) < SAME, "Live at the held pan's value", 3000);
      } finally {
        await page.mouse.up();
      }
    } finally {
      await live.set("band", PAN, "value", before);
    }
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
