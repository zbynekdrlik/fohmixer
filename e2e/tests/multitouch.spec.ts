import { test, expect, type Page } from "./support/fixtures";
import type { Locator } from "@playwright/test";
import { LiveClient, centre, harness, openSurface, ready, ret, shown, strip, track, until, volume } from "./support/live";

// Several faders at once (spec §2.5 input layer, S4 design note §3): each
// fader follows its own pointer. Chromium gets real touch input through the
// DevTools protocol; WebKit (the iPad) gets pointer events with distinct
// pointer ids, as a multi-finger touch delivers them.

const A = { name: "B-Main repro #", target: volume(ret("B-Main repro #")) };
const B = { name: "Hand2 #", target: volume(track("Hand2 #")) };

type Finger = { id: number; x: number; y: number };

/** Two fingers on two faders, moved and lifted as the test says. */
interface Touch {
  start(fingers: Finger[]): Promise<void>;
  move(fingers: Finger[]): Promise<void>;
  lift(finger: Finger, stay: Finger[]): Promise<void>;
}

/** Chromium: real touch points through the DevTools protocol. */
async function cdpTouch(page: Page): Promise<Touch> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
  const points = (fingers: Finger[]) => fingers.map((f) => ({ x: f.x, y: f.y, id: f.id }));
  return {
    start: async (fingers) => {
      await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: points(fingers) });
    },
    move: async (fingers) => {
      await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: points(fingers) });
    },
    // A point missing from the next event is lifted; with none left, it ends.
    lift: async (_finger, stay) => {
      if (stay.length > 0) {
        await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: points(stay) });
      } else {
        await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
      }
    },
  };
}

/** WebKit: pointer events with the fingers' ids, on each finger's fader. */
function pointerTouch(targets: Map<number, Locator>): Touch {
  const send = async (type: string, f: Finger) => {
    await targets.get(f.id)!.dispatchEvent(type, {
      pointerId: f.id,
      pointerType: "touch",
      isPrimary: f.id === 11,
      clientX: f.x,
      clientY: f.y,
      bubbles: true,
      cancelable: true,
    });
  };
  return {
    start: async (fingers) => {
      for (const f of fingers) await send("pointerdown", f);
    },
    move: async (fingers) => {
      for (const f of fingers) await send("pointermove", f);
    },
    lift: async (finger) => {
      await send("pointerup", finger);
    },
  };
}

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("Two fingers on two faders", () => {
  test("each fader follows its own finger, and one lifted leaves the other moving", async ({ page, browserName }) => {
    await live.set("band", A.target, "value", 0.6);
    await live.set("band", B.target, "value", 0.6);
    await openSurface(page);
    const faderA = strip(page, A.name).getByTestId("fader");
    const faderB = strip(page, B.name).getByTestId("fader");
    await ready(faderA);
    await ready(faderB);
    await until(() => shown(faderA), (v) => Math.abs(v - 0.6) < 0.001, "fader A at 0.6");
    await until(() => shown(faderB), (v) => Math.abs(v - 0.6) < 0.001, "fader B at 0.6");
    const a0 = await centre(faderA);
    const b0 = await centre(faderB);
    const touch =
      browserName === "chromium"
        ? await cdpTouch(page)
        : pointerTouch(
            new Map([
              [11, faderA],
              [12, faderB],
            ]),
          );
    const at = (step: number) => [
      { id: 11, x: a0.x, y: a0.y - 6 * step },
      { id: 12, x: b0.x, y: b0.y + 6 * step },
    ];
    await touch.start(at(0));
    for (let step = 1; step <= 8; step++) await touch.move(at(step));
    // Both moved at once, each its own way (the sends settle within frames).
    await page.waitForTimeout(300);
    const a1 = await live.get("band", A.target, "value");
    const b1 = await live.get("band", B.target, "value");
    expect(a1).toBeGreaterThan(0.63);
    expect(b1).toBeLessThan(0.57);
    // Finger 11 lifts; finger 12 keeps dragging B down.
    const [fingerA, fingerB] = at(8);
    await touch.lift(fingerA, [fingerB]);
    for (let step = 9; step <= 14; step++) {
      await touch.move([{ id: 12, x: b0.x, y: b0.y + 6 * step }]);
    }
    await until(() => live.get("band", B.target, "value"), (v) => v < b1 - 0.03, "B to keep falling");
    expect(await live.get("band", A.target, "value")).toBeCloseTo(a1, 9);
    await touch.lift({ id: 12, x: b0.x, y: b0.y + 6 * 14 }, []);
    await expect(faderA).not.toHaveClass(/failed/);
  });
});

test.describe("A host restart in the middle of a drag", () => {
  test("the fader keeps the finger, then shows Live's value after the reconnect", async ({ page }) => {
    await live.set("band", B.target, "value", 0.5);
    await openSurface(page);
    const fader = strip(page, B.name).getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    const { x, y } = await centre(fader);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 5; i++) await page.mouse.move(x, y - 8 * i);
    const finger = await until(() => shown(fader), (v) => v > 0.55, "the finger's value");
    const restart = harness("/host/band/restart");
    // While Live is away the fader keeps the finger's value.
    await expect(fader).toHaveAttribute("aria-disabled", "true");
    expect(Math.abs((await shown(fader)) - finger)).toBeLessThan(0.001);
    await restart;
    await expect(fader).toHaveAttribute("aria-disabled", "false", { timeout: 10_000 });
    await page.mouse.up();
    // The restarted host has its fixture's value (Hand2 # at 0.8) and the
    // release sent nothing: the finger's stale value never reached Live.
    const now = await live.get("band", B.target, "value");
    expect(now).toBeCloseTo(0.8, 9);
    expect(Math.abs(now - finger)).toBeGreaterThan(0.01);
    // After the release and its hold the fader shows that value.
    await until(() => shown(fader), (v) => Math.abs(v - 0.8) < 0.001, "the fader at Live's value", 5000);
    await expect(strip(page, B.name).getByTestId("db")).toHaveText(await live.display("band", B.target, 0.8));
  });
});

test.describe("A hub restart in the middle of a drag", () => {
  // While the hub is down the page's reconnect requests fail: the browser
  // reports them as failed resource loads (and a socket that could not
  // connect, should one attempt land in the gap).
  test.use({
    allowedConsole: [[/^Failed to load resource: /, /^WebSocket connection to '.*' failed/], { scope: "test" }],
  });

  test("the fader stays under the finger and writes again after the reconnect (I4)", async ({ page }) => {
    await live.set("band", B.target, "value", 0.5);
    await openSurface(page);
    const surface = page.getByTestId("surface");
    const fader = strip(page, B.name).getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    // Marks the element: a rebuilt stage would lose the mark.
    await fader.evaluate((el) => el.setAttribute("data-e2e-kept", "1"));
    const { x, y } = await centre(fader);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 3; i++) await page.mouse.move(x, y - 8 * i);
    await until(() => live.get("band", B.target, "value"), (v) => v > 0.51, "the first moves in Live");
    const restart = harness("/hub/restart", { rotate_secret: false });
    await expect(surface).toHaveAttribute("data-connected", "false");
    await restart;
    await expect(surface).toHaveAttribute("data-connected", "true", { timeout: 15_000 });
    await ready(fader);
    // The same layout again rebuilt nothing: the same element, still held.
    await expect(fader).toHaveAttribute("data-e2e-kept", "1");
    const before = await live.get("band", B.target, "value");
    for (let i = 4; i <= 9; i++) await page.mouse.move(x, y - 8 * i);
    await until(
      () => live.get("band", B.target, "value"),
      (v) => v > before + 0.01,
      "the finger to drive the fader again",
    );
    await page.mouse.up();
    await expect(fader).not.toHaveClass(/failed/, { timeout: 2000 });
  });
});
