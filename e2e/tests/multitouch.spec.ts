import { test, expect, type Page } from "./support/fixtures";
import type { Locator } from "@playwright/test";
import {
  LiveClient,
  type PointerStep,
  centre,
  dbForm,
  dispatchPointer,
  frames,
  harness,
  hubEvents,
  impair,
  openDetail,
  openSurface,
  pageEvents,
  panning,
  ready,
  ret,
  shown,
  strip,
  track,
  until,
  volume,
} from "./support/live";

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

/**
 * A value of Live once it holds still: three equal reads 100 ms apart
 * (within 5 s). A drag's last sends reach Live some frames after the last
 * move; on a slow WebKit runner a fixed wait read fader A 0.027 short of
 * its last send (#17's CI), and the "A stays where it was lifted" check then
 * compared against a value A had not reached yet.
 */
async function settled(page: Page, read: () => Promise<number>): Promise<number> {
  let last = await read();
  let same = 0;
  for (let i = 0; i < 50; i++) {
    await page.waitForTimeout(100);
    const now = await read();
    same = now === last ? same + 1 : 0;
    last = now;
    if (same >= 2) return now;
  }
  throw new Error(`the value never held still (last ${last})`);
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
    // Both moved at once, each its own way (read once the sends settled).
    await page.waitForTimeout(300);
    const a1 = await settled(page, () => live.get("band", A.target, "value"));
    const b1 = await settled(page, () => live.get("band", B.target, "value"));
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
    // The fader shows a move at its next animation frame. Polling for the
    // first value above 0.55 could catch a frame drawn between two moves
    // (WebKit on the runner: 0.05 below the finger's last value).
    await frames(page);
    const finger = await shown(fader);
    expect(finger).toBeGreaterThan(0.55);
    // Live confirmed the finger's value before it restarts: an unconfirmed
    // write of a held fader would be sent again once Live is back (#43, L4).
    await until(() => live.get("band", B.target, "value"), (v) => Math.abs(v - finger) < 0.001, "Live at the finger's value");
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
    await expect(strip(page, B.name).getByTestId("db")).toHaveText(dbForm(await live.display("band", B.target, 0.8)));
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
    // The test's own client went down with the hub too.
    live.close();
    live = await LiveClient.open();
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

test.describe("A lost pointer capture", () => {
  test("ends the touch: what the finger moved is sent, later moves do nothing", async ({ page }) => {
    await live.set("band", B.target, "value", 0.5);
    await openSurface(page);
    const fader = strip(page, B.name).getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    await fader.evaluate((el) =>
      el.addEventListener("pointerdown", (e) => el.setAttribute("data-e2e-pointer", String(e.pointerId))),
    );
    const { x, y } = await centre(fader);
    await page.mouse.move(x, y);
    await page.mouse.down();
    for (let i = 1; i <= 3; i++) await page.mouse.move(x, y - 8 * i);
    // The capture goes away without a pointerup (another element took it).
    await fader.evaluate((el) => el.releasePointerCapture(Number(el.getAttribute("data-e2e-pointer"))));
    await page.mouse.move(x, y - 8 * 3 - 1);
    await until(() => live.get("band", B.target, "value"), (v) => v > 0.5, "the finger's moves in Live");
    await page.waitForTimeout(300);
    const ended = await live.get("band", B.target, "value");
    // The mouse is still down over the fader: its moves are no touch now.
    for (let i = 4; i <= 9; i++) await page.mouse.move(x, y - 8 * i);
    await page.waitForTimeout(300);
    expect(await live.get("band", B.target, "value")).toBe(ended);
    await page.mouse.up();
  });
});

// A touch whose end the browser never delivered (#124): at an event a reverb
// fader on the iPad took no touch until the page was reloaded. A finger's
// down was recorded, a second finger began an iOS system gesture, and that
// finger's up, cancel and lost capture never came: the fader kept refusing
// every later touch while it "held" the lost pointer. A dispatched pointer is
// no active pointer, so the fader's `setPointerCapture` fails and no capture
// is ever lost: exactly the state of a touch the system took.

/** The lost finger's pointer and the next finger's. */
const LOST = 901;
const NEXT = 902;

/** The page's clock (`performance.timeOrigin + performance.now()`, the `t` of its records). */
const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

/** The page hidden or shown, as iOS does around a system gesture (deck.spec's `setHidden`). */
async function setHidden(page: Page, hidden: boolean) {
  await page.evaluate((on) => {
    const doc = document as any;
    if (on) {
      Object.defineProperty(doc, "hidden", { configurable: true, get: () => true });
      Object.defineProperty(doc, "visibilityState", { configurable: true, get: () => "hidden" });
    } else {
      delete doc.hidden;
      delete doc.visibilityState;
    }
    document.dispatchEvent(new Event("visibilitychange", { bubbles: true }));
  }, hidden);
}

/**
 * What followed the lost touch on the iPad: the link dropped (a 1006), the
 * page hidden and shown again, and a new socket. The link is held from the
 * drop until the page is shown, so none of the page's reports is under way
 * when the drop resets the connections (WebKit logs a request reset in
 * flight as an error, deck.spec); the stuck finger lives through all of it.
 */
async function droppedHiddenAndBack(page: Page) {
  const surface = page.getByTestId("surface");
  await impair.block(true);
  await impair.drop();
  await expect(surface).toHaveAttribute("data-connected", "false");
  await setHidden(page, true);
  await setHidden(page, false);
  await impair.block(false);
  await expect(surface).toHaveAttribute("data-connected", "true", { timeout: 15_000 });
}

/** A drag of the next finger: down, the anchor, two moves `px` apart along `axis`, its lift. */
function nextDrag(axis: "dx" | "dy", px: number): PointerStep[] {
  const at = (n: number) => (axis === "dx" ? { dx: n } : { dy: n });
  return [
    { type: "pointerdown" },
    { type: "pointermove", ...at(0) },
    { wait: 20 },
    { type: "pointermove", ...at(px) },
    { wait: 20 },
    { type: "pointermove", ...at(2 * px) },
    { wait: 20 },
    { type: "pointerup", ...at(2 * px) },
  ];
}

/** The page's `touch` records of `key` from page time `from` on, in page time order. */
async function touchRecords(key: string, from: number): Promise<any[]> {
  return pageEvents(await hubEvents())
    .filter((e: any) => e.ev === "touch" && Array.isArray(e.keys) && e.keys.includes(key) && e.t >= from)
    .sort((a: any, b: any) => a.t - b.t);
}

/**
 * The touches the page recorded for `key` from page time `from` on, as
 * `[what, pointer]` in page time order, once the lift of `lifted` (the next
 * finger by default) is in the event log (a hidden page's recorder uploads
 * a little later).
 */
async function touchesOf(key: string, from: number, lifted = NEXT): Promise<[string, number][]> {
  return until(
    async () => (await touchRecords(key, from)).map((e: any): [string, number] => [e.what, e.pointer]),
    (touches) => touches.some(([what, pointer]) => what === "up" && pointer === lifted),
    `pointer ${lifted}'s lift in the event log`,
    15_000,
  );
}

/**
 * Fader A at 0.5, then a finger's drag on it (its first move anchors) whose
 * end never comes; Live at the finger's value. The fader, that value and
 * the page time before the finger.
 */
async function stuckFader(page: Page): Promise<{ fader: Locator; held: number; from: number }> {
  await live.set("band", A.target, "value", 0.5);
  await openSurface(page);
  const fader = strip(page, A.name).getByTestId("fader");
  await ready(fader);
  await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "fader A at 0.5");
  const from = await pageNow(page);
  await dispatchPointer(fader, [{ type: "pointerdown" }, { type: "pointermove", dy: 0 }, { type: "pointermove", dy: 40 }], LOST);
  await until(() => live.get("band", A.target, "value"), (v) => v > 0.51, "the lost finger's drag in Live");
  const held = await settled(page, () => live.get("band", A.target, "value"));
  await expect(fader, "the strip lit under the lost finger").toHaveAttribute("data-held", "true");
  return { fader, held, from };
}

/** The hub's `set` records of `key` the page sent from page time `from` on. */
async function setsOf(key: string, from: number): Promise<any[]> {
  return (await hubEvents()).filter((e: any) => e.ev === "set" && e.key === key && e.t >= from);
}

test.describe("A touch whose end never came (#124)", () => {
  // The drop resets the page's socket: WebKit reports the reset of an open
  // socket as "WebSocket connection to '…' failed: Error receiving data:
  // Connection reset by peer" (Chromium says nothing; resilience.spec).
  test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

  test.afterEach(async () => {
    // Never leave the link held for the next test.
    await impair.block(false);
  });

  test("a fader held by a lost finger takes the next primary finger", async ({ page }) => {
    const key = `band|${A.target}|value`;
    const before = await live.get("band", A.target, "value");
    await live.set("band", A.target, "value", 0.5);
    try {
      await openSurface(page);
      const fader = strip(page, A.name).getByTestId("fader");
      await ready(fader);
      await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "fader A at 0.5");
      const from = await pageNow(page);
      // The finger drags fader A up (its first move anchors) and its end
      // never comes.
      await dispatchPointer(fader, [{ type: "pointerdown" }, { type: "pointermove", dy: 0 }, { type: "pointermove", dy: 40 }], LOST);
      await until(() => live.get("band", A.target, "value"), (v) => v > 0.51, "the lost finger's drag in Live");
      const held = await settled(page, () => live.get("band", A.target, "value"));
      await expect(fader, "the strip lit under the lost finger").toHaveAttribute("data-held", "true");
      await droppedHiddenAndBack(page);
      await ready(fader);
      // A new finger on the same fader: the first one down, so primary.
      const nextFrom = await pageNow(page);
      await dispatchPointer(fader, nextDrag("dy", 20), NEXT);
      await until(() => live.get("band", A.target, "value"), (v) => v > held + 0.01, "the next finger's drag in Live");
      await expect(fader).toHaveAttribute("data-held", "false");
      expect((await setsOf(key, nextFrom)).length, "the next finger's sets at the hub").toBeGreaterThan(0);
      // The page's record: the lost touch ended when the next finger came.
      expect(await touchesOf(key, from)).toEqual([
        ["down", LOST],
        ["lost", LOST],
        ["down", NEXT],
        ["up", NEXT],
      ]);
    } finally {
      await live.set("band", A.target, "value", before);
    }
  });

  test("a pan held by a lost finger takes the next primary finger", async ({ page }) => {
    const target = panning(track("Hand2 #"));
    const key = `band|${target}|value`;
    const before = await live.get("band", target, "value");
    await live.set("band", target, "value", 0);
    try {
      await openSurface(page);
      // The pan is the channel detail's (#71).
      const pan = (await openDetail(page, "Hand2 #")).getByTestId("pan");
      await ready(pan);
      await until(() => shown(pan), (v) => Math.abs(v) < 0.001, "the pan centred");
      const from = await pageNow(page);
      await dispatchPointer(pan, [{ type: "pointerdown" }, { type: "pointermove", dx: 0 }, { type: "pointermove", dx: 30 }], LOST);
      await until(() => live.get("band", target, "value"), (v) => v > 0.01, "the lost finger's drag in Live");
      const held = await settled(page, () => live.get("band", target, "value"));
      await droppedHiddenAndBack(page);
      await ready(pan);
      const nextFrom = await pageNow(page);
      await dispatchPointer(pan, nextDrag("dx", 15), NEXT);
      await until(() => live.get("band", target, "value"), (v) => v > held + 0.01, "the next finger's drag in Live");
      expect((await setsOf(key, nextFrom)).length, "the next finger's sets at the hub").toBeGreaterThan(0);
      expect(await touchesOf(key, from)).toEqual([
        ["down", LOST],
        ["lost", LOST],
        ["down", NEXT],
        ["up", NEXT],
      ]);
    } finally {
      await live.set("band", target, "value", before);
    }
  });
  test("with no drop and no hide, the next primary finger takes it from Live's value, not the frozen cap", async ({ page }) => {
    const key = `band|${A.target}|value`;
    const before = await live.get("band", A.target, "value");
    try {
      const { fader, held, from } = await stuckFader(page);
      // Live's value moves elsewhere while the cap stays frozen under the
      // lost finger (the channel detail's fader, Live's own window): the
      // strip's dB readout follows Live, the cap does not.
      const moved = held - 0.25;
      await live.set("band", A.target, "value", moved);
      await expect(strip(page, A.name).getByTestId("db")).toHaveText(dbForm(await live.display("band", A.target, moved)));
      expect(Math.abs((await shown(fader)) - held), "the cap frozen at the lost finger's value").toBeLessThan(0.001);
      // Past the hold after the lost finger's last ack (1 s with touch
      // shaping): the touch was lost long ago, as on the iPad.
      await page.waitForTimeout(1200);
      const nextFrom = await pageNow(page);
      await dispatchPointer(fader, nextDrag("dy", 10), NEXT);
      await until(() => live.get("band", A.target, "value"), (v) => v > moved + 0.005, "the next finger's drag in Live");
      await expect(fader).toHaveAttribute("data-held", "false");
      // No jump: the drag goes on from Live's value; no set comes near the
      // frozen cap, 0.25 above it (the finger's 20 px are at most ~0.09 on
      // the desktop's 225 px travel, both moves in one frame).
      const sets = (await setsOf(key, nextFrom)).sort((a: any, b: any) => a.t - b.t);
      expect(sets.length, "the next finger's sets at the hub").toBeGreaterThan(0);
      expect(sets[0].value - moved, "the first set near Live's value").toBeLessThan(0.15);
      for (const set of sets) expect(set.value, "no set near the frozen cap").toBeLessThan(held - 0.1);
      expect(await touchesOf(key, from)).toEqual([
        ["down", LOST],
        ["lost", LOST],
        ["down", NEXT],
        ["up", NEXT],
      ]);
      const down = (await touchRecords(key, from)).find((e: any) => e.what === "down" && e.pointer === NEXT);
      expect(down.local, "the next finger's touch starts from Live's value").toBe(false);
    } finally {
      await live.set("band", A.target, "value", before);
    }
  });

  test("a second finger on a fader a lost finger holds is still refused", async ({ page }) => {
    const key = `band|${A.target}|value`;
    const before = await live.get("band", A.target, "value");
    const SECOND = 904;
    try {
      const { fader, held, from } = await stuckFader(page);
      // Not primary: another finger is down (the browser still counts the
      // first one), so the fader keeps it.
      await dispatchPointer(fader, nextDrag("dy", 20), SECOND, false);
      await frames(page);
      await page.waitForTimeout(600);
      expect(await live.get("band", A.target, "value"), "the second finger moved nothing").toBe(held);
      await expect(fader).toHaveAttribute("data-held", "true");
      // The first finger's own lift still ends its touch.
      await dispatchPointer(fader, [{ type: "pointerup", dy: 40 }], LOST);
      await expect(fader).toHaveAttribute("data-held", "false");
      expect(await touchesOf(key, from, LOST)).toEqual([
        ["down", LOST],
        ["up", LOST],
      ]);
    } finally {
      await live.set("band", A.target, "value", before);
    }
  });

  test("a hide ends a lost finger's touch: the next finger takes the fader, primary or not", async ({ page }) => {
    const key = `band|${A.target}|value`;
    const before = await live.get("band", A.target, "value");
    const OTHER = 903;
    try {
      const { fader, held, from } = await stuckFader(page);
      // A hidden Home Screen app keeps no touch going: the hide ends it,
      // whatever the next finger's isPrimary says.
      await setHidden(page, true);
      await expect(fader).toHaveAttribute("data-held", "false");
      await setHidden(page, false);
      await dispatchPointer(fader, nextDrag("dy", 20), OTHER, false);
      await until(() => live.get("band", A.target, "value"), (v) => v > held + 0.01, "the next finger's drag in Live");
      expect(await touchesOf(key, from, OTHER)).toEqual([
        ["down", LOST],
        ["lost", LOST],
        ["down", OTHER],
        ["up", OTHER],
      ]);
    } finally {
      await live.set("band", A.target, "value", before);
    }
  });

  test("a pulse toggle whose press never ended goes off at a hide, and at the next first finger", async ({ page }) => {
    // ZVUKAR pulses Hand4 #'s mute: on (false) while pressed, off (true) at
    // the release. A pulse left on at an event is worse than a stuck fader.
    const hand4 = track("Hand4 #");
    const key = `band|${hand4}|mute`;
    const before = await live.get("band", hand4, "mute");
    const mute = () => live.get("band", hand4, "mute");
    await live.set("band", hand4, "mute", true);
    try {
      await openSurface(page);
      const zvukar = page.locator('[data-testid="param-toggle"][data-label="ZVUKAR"]');
      await ready(zvukar);
      await expect(zvukar).toHaveAttribute("data-state", "off");
      const from = await pageNow(page);
      // A press whose release never comes, then a hide: the pulse ends.
      await dispatchPointer(zvukar, [{ type: "pointerdown" }], 911);
      await until(mute, (v) => v === false, "on while pressed");
      await setHidden(page, true);
      await until(mute, (v) => v === true, "off at the hide");
      await setHidden(page, false);
      // Another press whose release never comes; the next first finger
      // ends it (no double tap: 300 ms apart).
      await page.waitForTimeout(300);
      await dispatchPointer(zvukar, [{ type: "pointerdown" }], 912);
      await until(mute, (v) => v === false, "on while pressed again");
      await page.waitForTimeout(300);
      await dispatchPointer(zvukar, [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 913);
      await until(mute, (v) => v === true, "off at the next first finger");
      await page.waitForTimeout(500);
      expect(await mute(), "no pulse left on").toBe(true);
      expect(await touchesOf(key, from, 913)).toEqual([
        ["down", 911],
        ["lost", 911],
        ["down", 912],
        ["lost", 912],
        ["down", 913],
        ["up", 913],
      ]);
    } finally {
      await live.set("band", hand4, "mute", before);
    }
  });
});
