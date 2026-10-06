import type { Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import {
  centre,
  companion,
  deckKey,
  dispatchPointer,
  frames,
  harness,
  hubEvents,
  impair,
  openDeck,
  openSurface,
  pageEvents,
  selectPage,
  until,
} from "./support/live";

// The Stream Deck tab (#52) against the harness's fake Companion
// (e2e/harness/fake_companion.py: 8 × 4 keys, each a small PNG; a press is
// answered OK and then the key's new state). The fake records every press it
// got, so a test reads what reached "Companion" and what never did. Every
// test leaves no key held: a page closed with a key held is released by the
// hub after the next test began.

/** The fake's presses of `key` since `since` (epoch ms): each one's `pressed`. */
async function pressesOf(key: number, since: number): Promise<boolean[]> {
  const state = await companion.state();
  return state.presses.filter((p: any) => p.key === key && p.at >= since).map((p: any) => p.pressed);
}

/** The fake's presses of `keys` since `since`, in the order it got them: `[key, pressed]`. */
async function pressLog(keys: number[], since: number): Promise<[number, boolean][]> {
  const state = await companion.state();
  return state.presses.filter((p: any) => keys.includes(p.key) && p.at >= since).map((p: any) => [p.key, p.pressed]);
}

/** The hub's records of kind `ev` since `since`, once `ok` holds for them (the log is written a little later). */
function hubRecords(ev: string, since: number, ok: (records: any[]) => boolean, what: string): Promise<any[]> {
  return until(async () => (await hubEvents()).filter((r: any) => r.ev === ev && r.ts >= since), ok, what);
}

/** The page's `deck` events since `since` (its flight recorder, through the hub's `trace` records), once `ok` holds. */
function deckEvents(since: number, ok: (events: any[]) => boolean, what: string): Promise<any[]> {
  return until(
    async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "deck" && e.t >= since),
    ok,
    what,
    15_000,
  );
}

/**
 * Records every key that flashes red from now on (`data-failed` true for
 * 400 ms, too short for a poll to see): `flashedKeys(page)` reads them.
 */
async function watchFlashes(page: Page) {
  await page.evaluate(() => {
    const flashed = new Set<number>();
    (window as any).deckFlashed = flashed;
    new MutationObserver((records) => {
      for (const r of records) {
        const el = r.target as Element;
        if (el.getAttribute("data-testid") !== "deck-key") continue;
        if (r.oldValue === "true" || el.getAttribute("data-failed") === "true") flashed.add(Number(el.getAttribute("data-key")));
      }
    }).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["data-failed"], attributeOldValue: true });
  });
}

/** The keys that flashed since `watchFlashes`, in key order. */
const flashedKeys = (page: Page): Promise<number[]> =>
  page.evaluate(() => [...((window as any).deckFlashed as Set<number>)].sort((a, b) => a - b));

/** A tap of `key`: down, 100 ms, up (dispatched pointer events of one finger; a first finger is primary). */
const tap = (page: Page, key: number, pointerId = 21, primary = true) =>
  dispatchPointer(deckKey(page, key), [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }], pointerId, primary);

/**
 * A finger's down on `key` dispatched in the page (`pointerId`, `primary`),
 * after the dropout counter turned red when `afterDropout` (waited for in
 * the page, frame by frame), then its state once the page's effects ran,
 * all without a round trip: `data-held`, `data-pressed`, `data-failed`, the
 * counter's `data-active` and the page clock at the read.
 */
async function downAndRead(key: Locator, pointerId: number, primary: boolean, afterDropout = false) {
  const { x, y } = await centre(key);
  return key.evaluate(
    async (el, { clientX, clientY, id, isPrimary, waitDropout }) => {
      const frame = () => new Promise((done) => requestAnimationFrame(done));
      const counter = document.querySelector('[data-testid="dropouts"]');
      if (!counter) throw new Error("no dropout counter");
      const deadline = performance.now() + 3000;
      while (waitDropout && counter.getAttribute("data-active") !== "true") {
        if (performance.now() > deadline) throw new Error("the dropout counter did not turn red within 3 s");
        await frame();
      }
      el.dispatchEvent(
        new PointerEvent("pointerdown", {
          pointerId: id,
          pointerType: "touch",
          isPrimary,
          clientX,
          clientY,
          bubbles: true,
          cancelable: true,
        }),
      );
      // Leptos' effects run in microtasks: a macrotask later the page has
      // drawn what the down did, long before any answer can come.
      await new Promise((done) => setTimeout(done, 0));
      return {
        held: el.getAttribute("data-held"),
        pressed: el.getAttribute("data-pressed"),
        failed: el.getAttribute("data-failed"),
        dropout: counter.getAttribute("data-active"),
        at: performance.timeOrigin + performance.now(),
      };
    },
    { clientX: x, clientY: y, id: pointerId, isPrimary: primary, waitDropout: afterDropout },
  );
}

/**
 * The page hidden (`document.hidden`, `visibilityState`) or shown again, with
 * the `visibilitychange` a browser fires: it bubbles to the window, where the
 * deck listens.
 */
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
 * The page's socket refuses a `deck_press` while `window.deckRefuse` names
 * its kind (`"down"` or `"up"`): its `send` throws, so the store has not sent
 * it. Every other message goes as before. An init script: in place before
 * the app's first socket.
 */
async function refuseDeckSends(page: Page) {
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (this: WebSocket, data: any) {
      const refuse = (window as any).deckRefuse;
      if (refuse && typeof data === "string" && data.includes('"deck_press"')) {
        const msg = JSON.parse(data);
        if (msg.type === "deck_press" && (msg.down ? "down" : "up") === refuse) {
          throw new DOMException("refused by the test", "InvalidStateError");
        }
      }
      return send.call(this, data);
    };
  });
}

/**
 * A first finger's down on `key` while the page's socket reads as closing
 * (the hub closed it, its close event not in yet): `readyState` is
 * overridden for that one dispatched event only, so no other page code ever
 * sees it.
 */
async function downWhileClosing(key: Locator) {
  const { x, y } = await centre(key);
  await key.evaluate(
    (el, { clientX, clientY }) => {
      const proto = WebSocket.prototype;
      const real = Object.getOwnPropertyDescriptor(proto, "readyState");
      if (!real) throw new Error("WebSocket.prototype has no readyState accessor");
      Object.defineProperty(proto, "readyState", { configurable: true, get: () => WebSocket.CLOSING });
      try {
        el.dispatchEvent(
          new PointerEvent("pointerdown", {
            pointerId: 21,
            pointerType: "touch",
            isPrimary: true,
            clientX,
            clientY,
            bubbles: true,
            cancelable: true,
          }),
        );
      } finally {
        Object.defineProperty(proto, "readyState", real);
      }
    },
    { clientX: x, clientY: y },
  );
}

/** The page ids of the lit tabs of the tab bar of `level` (0: the layout's pages, 1: the pager's). */
const litTabs = (page: Page, level: number): Promise<(string | null)[]> =>
  page
    .locator(`[data-testid="tabbar"][data-level="${level}"] [data-testid="tab"]`)
    .evaluateAll((els) => els.filter((e) => e.getAttribute("data-selected") === "true").map((e) => e.getAttribute("data-page")));

test.describe("The Stream Deck tab", () => {
  test.afterEach(async () => {
    // Up, not failing, nothing recorded or held; resolves once the hub's
    // Stream Deck is back at the fake.
    await companion.reset();
  });

  test("shows 8 × 4 square keys in key order, and only the global controls on the rail", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const boxes = await page.getByTestId("deck-key").evaluateAll((els) =>
      els.map((e) => {
        const r = e.getBoundingClientRect();
        return { key: Number(e.getAttribute("data-key")), x: r.x, y: r.y, w: r.width, h: r.height };
      }),
    );
    expect(boxes.map((b) => b.key)).toEqual([...Array(32).keys()]);
    const viewport = page.viewportSize()!;
    for (const b of boxes) {
      expect(Math.abs(b.w - b.h), `key ${b.key} is square`).toBeLessThanOrEqual(1);
      expect(b.w, `key ${b.key} is large`).toBeGreaterThan(40);
      expect(b.x + b.w).toBeLessThanOrEqual(viewport.width);
      expect(b.y + b.h).toBeLessThanOrEqual(viewport.height);
    }
    for (let key = 1; key < 32; key++) {
      if (key % 8 > 0) {
        expect(boxes[key].x, `key ${key} right of ${key - 1}`).toBeGreaterThan(boxes[key - 1].x);
        expect(Math.abs(boxes[key].y - boxes[key - 1].y)).toBeLessThanOrEqual(1);
      }
      if (key >= 8) {
        expect(boxes[key].y, `key ${key} below ${key - 8}`).toBeGreaterThan(boxes[key - 8].y);
        expect(Math.abs(boxes[key].x - boxes[key - 8].x)).toBeLessThanOrEqual(1);
      }
    }
    await expect(page.locator('[data-testid="deck"] .rail-main > *')).toHaveCount(0);
    await expect(page.locator('[data-testid="deck"] .rail-foot > *').first()).toBeVisible();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false");
    await expect(page.getByTestId("deck-dot")).toHaveCount(0);
    // The layout's tabs are not lit while the deck is shown, and the pages'
    // tab bar still lists only them as `tab`s: the deck's tab ends the bar
    // under its own test id.
    const bar = page.locator('[data-testid="tabbar"][data-level="0"]');
    await expect(bar.locator('[data-testid="tab"][data-selected="true"]')).toHaveCount(0);
    await expect(bar.getByTestId("tab")).toHaveText(["Cue", "FOH", "Conf"]);
    await expect(bar.getByTestId("deck-tab")).toHaveCount(1);
  });

  test("the top bar keeps the version on screen beside the deck tab", async ({ page }) => {
    await openSurface(page);
    await expect(page.getByTestId("deck-tab")).toBeVisible();
    const width = page.viewportSize()!.width;
    // The widest bar first: FOH with its pager's tabs; then the deck's.
    for (const shown of ["foh", "deck"]) {
      if (shown === "deck") await openDeck(page);
      const version = page.getByTestId("version");
      await expect(version).toBeVisible();
      const box = (await version.boundingBox())!;
      expect(box.x, `the version's left edge with ${shown} shown`).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width, `the version's right edge with ${shown} shown`).toBeLessThanOrEqual(width);
    }
  });

  test("a key goes down at the touch and up at pointerup, pointercancel and on leaving the tab", async ({ page }) => {
    const since = Date.now();
    await openSurface(page);
    await openDeck(page);
    await tap(page, 3);
    await until(() => pressesOf(3, since), (p) => p.join() === "true,false", "key 3 down and up");
    await dispatchPointer(deckKey(page, 4), [{ type: "pointerdown" }, { wait: 150 }, { type: "pointercancel" }]);
    await until(() => pressesOf(4, since), (p) => p.join() === "true,false", "key 4 down, and up on a cancel");
    await dispatchPointer(deckKey(page, 5), [{ type: "pointerdown" }]);
    await until(() => pressesOf(5, since), (p) => p.join() === "true", "key 5 down");
    await expect(deckKey(page, 5)).toHaveAttribute("data-held", "true");
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await expect(page.getByTestId("deck")).toHaveCount(0);
    await until(() => pressesOf(5, since), (p) => p.join() === "true,false", "key 5 up on leaving the tab");
    // The hub's records say why each up came; so does the page's flight recorder.
    const presses = await hubRecords(
      "deck_press",
      since,
      (r) => r.filter((x) => x.down === false).length >= 3,
      "the hub's ups of keys 3, 4 and 5",
    );
    const ups = presses.filter((r) => r.down === false);
    expect(new Map(ups.map((r) => [r.key, r.why]))).toEqual(new Map([[3, "up"], [4, "cancel"], [5, "tab"]]));
    expect(ups.every((r) => r.forwarded === true && r.hold_ms > 0)).toBe(true);
    // Companion's answer to key 3's down: the deck_ok of the same client and
    // seq, at or after it, with its round trip.
    const press = presses.find((r) => r.key === 3 && r.down === true);
    expect(press, "key 3's down in the event log").toBeTruthy();
    expect([press.forwarded, press.reason, press.why]).toEqual([true, null, null]);
    const answers = await hubRecords(
      "deck_ok",
      since,
      (r) => r.some((x) => x.client === press.client && x.seq === press.seq),
      "Companion's answer to key 3's down",
    );
    const answer = answers.find((r) => r.client === press.client && r.seq === press.seq);
    expect([answer.key, answer.down, answer.ok, answer.error]).toEqual([3, true, true, null]);
    expect(answer.ts).toBeGreaterThanOrEqual(press.ts);
    expect(typeof answer.rtt_ms).toBe("number");
    expect(answer.rtt_ms).toBeGreaterThanOrEqual(0);
    const deckUps = (
      await deckEvents(since, (e) => e.filter((x) => x.d === 0).length >= 3, "the page's deck ups of keys 3, 4 and 5")
    ).filter((e) => e.d === 0);
    expect(new Map(deckUps.map((e) => [e.k, e.why]))).toEqual(new Map([[3, "up"], [4, "cancel"], [5, "tab"]]));
    expect(deckUps.every((e) => e.sent === true && typeof e.q === "number")).toBe(true);
    // The forensics timeline counts the window's three presses, numbers only.
    const report = await harness("/forensics/timeline", { from_ms: since, to_ms: Date.now() });
    expect(report.exit).toBe(0);
    const summary = new Map<string, string>(report.stdout.trim().split("\n").map((l: string) => l.split("=", 2) as [string, string]));
    expect([summary.get("deck_presses"), summary.get("deck_unsent"), summary.get("deck_forced_releases")]).toEqual(["3", "0", "0"]);
    expect(summary.get("deck_rtt_p50_ms")).toMatch(/^\d+\.\d$/);
    expect(report.html).toContain('class="deck-press"');
  });

  test("a held key is outlined at once and shows Companion's pressed state", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const key = deckKey(page, 10);
    const since = Date.now();
    // The page's link held 300 ms, under the 0.5 s a down may take to the
    // hub: no answer can come meanwhile, and the down still goes. The down
    // and the read right after it run in the page, without a round trip.
    await impair.stall(300);
    const seen = await downAndRead(key, 21, true);
    // The outline is the page's own, at once; Companion's pressed look waits
    // for its answer.
    expect([seen.held, seen.pressed, seen.failed]).toEqual(["true", "false", "false"]);
    await expect(key).toHaveAttribute("data-pressed", "true");
    await expect(key).toHaveAttribute("data-held", "true");
    // The read came before the hub even had the down, and the hub forwarded it.
    const records = await hubRecords("deck_press", since, (r) => r.some((x) => x.key === 10 && x.down === true), "key 10's down in the event log");
    const down = records.find((x) => x.key === 10 && x.down === true);
    expect(down.ts, "the hub got the down after the page showed its outline").toBeGreaterThan(seen.at);
    expect([down.forwarded, down.reason]).toEqual([true, null]);
    expect(down.delay_ms).toBeLessThanOrEqual(500);
    await dispatchPointer(key, [{ type: "pointerup" }]);
    await expect(key).toHaveAttribute("data-held", "false");
    await expect(key).toHaveAttribute("data-pressed", "false");
  });

  test("a down that would reach Companion late is refused: it flashes red and is never sent", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await watchFlashes(page);
    const since = Date.now();
    // The page's link held 2 s (under the page's 3 s of silence that closes
    // its socket): a down at once waits in the socket the whole time.
    await impair.stall(2000);
    const first = await downAndRead(deckKey(page, 10), 21, true);
    // Outlined at once; the red flash comes with the hub's answer.
    expect(first.held).toBe("true");
    // Once the dropout counter is red (nothing from the hub for 300 ms), a
    // second finger's down flashes at once and the page sends nothing.
    const second = await downAndRead(deckKey(page, 12), 22, false, true);
    expect([second.held, second.failed, second.dropout]).toEqual(["true", "true", "true"]);
    await dispatchPointer(deckKey(page, 12), [{ type: "pointerup" }], 22, false);
    // Key 10 flashes when the hub's answer comes (the hub refused the down,
    // late), or flashed at once had the page already seen the dropout.
    await expect.poll(() => flashedKeys(page), { timeout: 10_000 }).toEqual([10, 12]);
    await dispatchPointer(deckKey(page, 10), [{ type: "pointerup" }]);
    await expect(deckKey(page, 10)).toHaveAttribute("data-held", "false");
    // The link carries presses again once the dropout is over: a tap
    // reaches Companion, after where the two refused downs would have been.
    await expect(page.getByTestId("dropouts")).toHaveAttribute("data-active", "false", { timeout: 10_000 });
    await tap(page, 11, 23);
    await until(() => pressesOf(11, since), (p) => p.join() === "true,false", "key 11 down and up after the stall");
    const events = await deckEvents(since, (e) => e.some((x) => x.k === 11 && x.d === 0), "the page's deck events up to key 11's up");
    expect(events.filter((e) => e.k === 12).map((e) => [e.d, e.sent])).toEqual([[1, false]]);
    const presses = (await hubEvents()).filter((r: any) => r.ev === "deck_press" && r.ts >= since);
    expect(presses.filter((r: any) => r.key === 12)).toEqual([]);
    const down10 = events.find((e) => e.k === 10 && e.d === 1);
    const hub10 = presses.filter((r: any) => r.key === 10);
    if (down10.sent) {
      // The page sent it into the stalled link: the hub refused it, late,
      // and the finger's up after it is not held.
      expect(hub10.map((r: any) => [r.down, r.forwarded, r.reason])).toEqual([
        [true, false, "late"],
        [false, false, "not held"],
      ]);
      expect(hub10[0].delay_ms).toBeGreaterThan(500);
    } else {
      expect(hub10).toEqual([]);
    }
    // Nothing of keys 10 and 12 ever reaches Companion.
    await page.waitForTimeout(1000);
    expect(await pressesOf(10, since)).toEqual([]);
    expect(await pressesOf(12, since)).toEqual([]);
    expect(await flashedKeys(page)).toEqual([10, 12]);
    // The forensics timeline counts both red flashes.
    const report = await harness("/forensics/timeline", { from_ms: since, to_ms: Date.now() });
    expect(report.exit).toBe(0);
    const summary = new Map<string, string>(report.stdout.trim().split("\n").map((l: string) => l.split("=", 2) as [string, string]));
    // The hub's downs: key 11's, and key 10's when the page sent it.
    expect([summary.get("deck_presses"), summary.get("deck_unsent")]).toEqual([down10.sent ? "2" : "1", "2"]);
  });

  test("a re-tap of the open tab keeps a held key held; the page going hidden lifts it", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const since = Date.now();
    const deck = page.getByTestId("deck");
    await deck.evaluate((el) => el.setAttribute("data-e2e-kept", "1"));
    await dispatchPointer(deckKey(page, 21), [{ type: "pointerdown" }]);
    await until(() => pressesOf(21, since), (p) => p.join() === "true", "key 21 down");
    // The open tab tapped again: the same deck page, the key still held.
    await page.getByTestId("deck-tab").click();
    await frames(page);
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "true");
    await expect(deck).toHaveAttribute("data-e2e-kept", "1");
    await expect(deckKey(page, 21)).toHaveAttribute("data-held", "true");
    // A second finger's tap after it: an up of key 21 sent by the re-tap
    // would have reached the fake before it.
    await tap(page, 27, 22, false);
    await until(() => pressesOf(27, since), (p) => p.join() === "true,false", "key 27 down and up");
    expect(await pressesOf(21, since)).toEqual([true]);
    // The page goes hidden: every held key goes up (`hidden`).
    await setHidden(page, true);
    await until(() => pressesOf(21, since), (p) => p.join() === "true,false", "key 21 up when the page went hidden");
    await expect(deckKey(page, 21)).toHaveAttribute("data-held", "false");
    await setHidden(page, false);
    // The finger's own lift then sends nothing (a later tap is the marker).
    await dispatchPointer(deckKey(page, 21), [{ type: "pointerup" }]);
    await tap(page, 28, 23);
    const ups = await hubRecords("deck_press", since, (r) => r.some((x) => x.key === 28 && x.down === false), "key 28's up in the event log");
    expect(ups.filter((r) => r.down === false).map((r) => [r.key, r.why, r.forwarded])).toEqual([
      [27, "up", true],
      [21, "hidden", true],
      [28, "up", true],
    ]);
    const events = await deckEvents(since, (e) => e.some((x) => x.k === 28 && x.d === 0), "the page's deck events up to key 28's up");
    expect(events.filter((e) => e.k === 21).map((e) => [e.d, e.why ?? null, e.sent])).toEqual([
      [1, null, true],
      [0, "hidden", true],
    ]);
  });

  test("a hold whose end never came goes up (lost) at the next first finger, never at a second one", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const since = Date.now();
    // Finger 21 on key 22; its end never comes.
    await dispatchPointer(deckKey(page, 22), [{ type: "pointerdown" }], 21, true);
    await until(() => pressesOf(22, since), (p) => p.join() === "true", "key 22 down");
    // A second finger (not primary) taps key 23: key 22 stays held.
    await tap(page, 23, 22, false);
    await until(() => pressesOf(23, since), (p) => p.join() === "true,false", "key 23 down and up");
    await expect(deckKey(page, 22)).toHaveAttribute("data-held", "true");
    // The next first finger (primary: no other finger is down, as the
    // browser sees it) on key 24: key 22 goes up first.
    await tap(page, 24, 23, true);
    await until(() => pressesOf(24, since), (p) => p.join() === "true,false", "key 24 down and up");
    await expect(deckKey(page, 22)).toHaveAttribute("data-held", "false");
    expect(await pressLog([22, 23, 24], since)).toEqual([
      [22, true],
      [23, true],
      [23, false],
      [22, false],
      [24, true],
      [24, false],
    ]);
    const presses = await hubRecords("deck_press", since, (r) => r.length >= 6, "the six presses in the event log");
    const ups = presses.filter((r) => r.down === false);
    expect(ups.map((r) => [r.key, r.why, r.forwarded])).toEqual([
      [23, "up", true],
      [22, "lost", true],
      [24, "up", true],
    ]);
    expect(ups[1].hold_ms).toBeGreaterThan(0);
  });

  test("with Companion away the keys dim, the tab shows its dot, a press flashes red and is never sent", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await watchFlashes(page);
    const since = Date.now();
    // Key 2 is held when Companion goes away, its down answered first: the
    // fake records a press 3 ms before its OK, and a down whose OK the
    // outage swallows is answered offline and flashes (spec §4). The
    // pressed look comes after the OK, and the hub acks the page before it
    // sends the key's state.
    await dispatchPointer(deckKey(page, 2), [{ type: "pointerdown" }]);
    await until(() => pressesOf(2, since), (p) => p.join() === "true", "key 2 down");
    await expect(deckKey(page, 2)).toHaveAttribute("data-pressed", "true");
    await companion.down();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "true");
    await expect(page.getByTestId("deck-dot")).toBeVisible();
    await expect(deckKey(page, 6)).toHaveClass(/\boffline\b/);
    await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(32);
    // Its lift goes to the hub, which refuses it (`offline`): a refused up
    // never flashes.
    await dispatchPointer(deckKey(page, 2), [{ type: "pointerup" }]);
    await expect(deckKey(page, 2)).toHaveAttribute("data-held", "false");
    // A press now flashes red at once and goes nowhere: the page sends nothing.
    await tap(page, 6);
    await expect.poll(() => flashedKeys(page)).toEqual([6]);
    await companion.up();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false", { timeout: 8000 });
    await expect(page.getByTestId("deck-dot")).toHaveCount(0);
    await expect(deckKey(page, 6)).not.toHaveClass(/\boffline\b/);
    // Companion kept key 2 held through the outage: the hub releases it once
    // the link is back (`reconnect`).
    await until(() => pressesOf(2, since), (p) => p.join() === "true,false", "key 2 released after the reconnect");
    await expect(deckKey(page, 2)).toHaveAttribute("data-pressed", "false");
    const releases = await hubRecords("deck_release", since, (r) => r.length >= 1, "the hub's release of key 2");
    expect(releases.map((r) => [r.key, r.reason])).toEqual([[2, "reconnect"]]);
    // The event log is in order: what the page sent came before the release.
    const presses = (await hubEvents()).filter((r: any) => r.ev === "deck_press" && r.ts >= since);
    expect(presses.map((r: any) => [r.key, r.down, r.forwarded, r.reason])).toEqual([
      [2, true, true, null],
      [2, false, false, "offline"],
    ]);
    // The page's recorder: key 6's down not sent, and no up of it at all.
    const events = await deckEvents(since, (e) => e.some((x) => x.k === 6), "the page's event of key 6");
    expect(events.map((e) => [e.k, e.d, e.sent])).toEqual([
      [2, 1, true],
      [2, 0, true],
      [6, 1, false],
    ]);
    await page.waitForTimeout(1000);
    expect(await pressesOf(6, since)).toEqual([]);
    expect(await flashedKeys(page)).toEqual([6]);
  });

  test("Companion refusing a down flashes the key red; a refused up never does", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await watchFlashes(page);
    const since = Date.now();
    // Key 13 goes down while Companion answers OK; then Companion refuses
    // every press, the key's up too.
    await dispatchPointer(deckKey(page, 13), [{ type: "pointerdown" }]);
    await expect(deckKey(page, 13)).toHaveAttribute("data-pressed", "true");
    await companion.fail(true);
    await dispatchPointer(deckKey(page, 13), [{ type: "pointerup" }]);
    const refused = await hubRecords("deck_ok", since, (r) => r.some((x) => x.key === 13 && x.down === false), "Companion's answer to key 13's up");
    expect(refused.filter((r) => r.key === 13).map((r) => [r.down, r.ok, r.error])).toEqual([
      [true, true, null],
      [false, false, "test refusal"],
    ]);
    // A down Companion refuses flashes its key. The hub's acks reach the page
    // in the order Companion answered, so once key 9 has flashed the page has
    // already handled key 13's refused up: had that flashed, key 13 would be
    // in the set too and this poll would never read [9].
    await tap(page, 9);
    await expect.poll(() => flashedKeys(page)).toEqual([9]);
    // Companion still holds key 13 (it refused the release): a tap with
    // Companion answering OK again releases it.
    await companion.fail(false);
    await tap(page, 13);
    await until(() => pressesOf(13, since), (p) => p.join() === "true,false,true,false", "key 13 released by the next tap");
    await expect(deckKey(page, 13)).toHaveAttribute("data-pressed", "false");
  });

  test("a touchstart on a key or its image is prevented: no loupe, no callout", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    for (const target of [deckKey(page, 11), deckKey(page, 11).locator("img")]) {
      const prevented = await target.evaluate((el: Element) => {
        const event = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(event);
        return event.defaultPrevented;
      });
      expect(prevented).toBe(true);
    }
    await expect(deckKey(page, 11).locator("img")).toHaveCSS("pointer-events", "none");
  });

  test("a layout tab lights again after the deck; a re-tap or a pager tap rebuilds no fader", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await expect.poll(() => litTabs(page, 0)).toEqual([]);
    await page.locator('[data-testid="tab"][data-page="cue"]').click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-page", "cue");
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "false");
    await expect.poll(() => litTabs(page, 0)).toEqual(["cue"]);
    await openDeck(page);
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await expect(page.getByTestId("page")).toHaveAttribute("data-page", "foh");
    await expect.poll(() => litTabs(page, 0)).toEqual(["foh"]);
    await expect.poll(() => litTabs(page, 1)).toEqual(["stage"]);
    // Every fader marked: a rebuilt one would lose the mark (P1).
    const faders = page.getByTestId("fader");
    await expect(faders.first()).toBeVisible();
    const all = await faders.count();
    await faders.evaluateAll((els) => els.forEach((el) => el.setAttribute("data-e2e-kept", "1")));
    const fixed = () =>
      page.evaluate(() => {
        const outside = [...document.querySelectorAll('[data-testid="fader"]')].filter((e) => !e.closest('[data-testid="pager"]'));
        return [outside.length, outside.filter((e) => e.hasAttribute("data-e2e-kept")).length];
      });
    const [fixedFaders] = await fixed();
    expect(fixedFaders).toBeGreaterThan(0);
    // The shown page's tab again: nothing rebuilt.
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await frames(page);
    await expect.poll(() => litTabs(page, 0)).toEqual(["foh"]);
    await expect(page.locator('[data-testid="fader"][data-e2e-kept]')).toHaveCount(all);
    // The pager's other page: only the pager's faders are new.
    await selectPage(page, "others");
    await expect(page.locator('[data-testid="pager"][data-page="others"]')).toBeVisible();
    await frames(page);
    expect(await fixed()).toEqual([fixedFaders, fixedFaders]);
  });

  test("a reload opens the last mixer page, never the deck", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "cue");
    await openDeck(page);
    await page.reload();
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await expect(page.getByTestId("page")).toHaveAttribute("data-page", "cue");
    await expect(page.getByTestId("deck")).toHaveCount(0);
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "false");
    await expect(page.locator('[data-testid="tab"][data-page="cue"]')).toHaveAttribute("data-selected", "true");
  });

  test.describe("a press the page cannot send", () => {
    test("a down the socket refuses flashes and its up is never sent; an up it refuses never flashes", async ({ page }) => {
      await refuseDeckSends(page);
      await openSurface(page);
      await openDeck(page);
      await watchFlashes(page);
      const since = Date.now();
      await page.evaluate(() => ((window as any).deckRefuse = "down"));
      await tap(page, 25);
      await expect.poll(() => flashedKeys(page)).toEqual([25]);
      await page.evaluate(() => ((window as any).deckRefuse = "up"));
      await dispatchPointer(deckKey(page, 26), [{ type: "pointerdown" }]);
      await until(() => pressesOf(26, since), (p) => p.join() === "true", "key 26 down");
      await dispatchPointer(deckKey(page, 26), [{ type: "pointerup" }]);
      await expect(deckKey(page, 26)).toHaveAttribute("data-held", "false");
      await page.evaluate(() => ((window as any).deckRefuse = null));
      const events = await deckEvents(since, (e) => e.length >= 3, "the page's deck events of keys 25 and 26");
      expect(events.map((e) => [e.k, e.d, e.sent])).toEqual([
        [25, 1, false],
        [26, 1, true],
        [26, 0, false],
      ]);
      expect(await flashedKeys(page)).toEqual([25]);
      // The hub still counts key 26 held by this page (its up never came):
      // the next tap's down is taken without a second press at Companion, and
      // its up releases the key.
      await tap(page, 26);
      await until(() => pressesOf(26, since), (p) => p.join() === "true,false", "key 26 released by the next tap");
      const presses = await hubRecords("deck_press", since, (r) => r.length >= 3, "the presses of key 26 in the event log");
      expect(presses.map((r) => [r.key, r.down, r.forwarded, r.reason])).toEqual([
        [26, true, true, null],
        [26, true, false, "held"],
        [26, false, true, null],
      ]);
      expect(await pressesOf(25, since)).toEqual([]);
    });

    test("a down while the socket is closing flashes at once, spends no press number and is never sent", async ({ page }) => {
      await openSurface(page);
      await openDeck(page);
      await watchFlashes(page);
      const since = Date.now();
      await tap(page, 14);
      await until(() => pressesOf(14, since), (p) => p.join() === "true,false", "key 14 down and up");
      // Companion online, the socket closing: the page refuses the down itself.
      await downWhileClosing(deckKey(page, 15));
      await expect.poll(() => flashedKeys(page)).toEqual([15]);
      await dispatchPointer(deckKey(page, 15), [{ type: "pointerup" }]);
      await tap(page, 16);
      const presses = await hubRecords("deck_press", since, (r) => r.length >= 4, "the presses of keys 14 and 16 in the event log");
      expect(presses.map((r) => [r.key, r.down])).toEqual([
        [14, true],
        [14, false],
        [16, true],
        [16, false],
      ]);
      // Its press numbers run on from key 14's to key 16's: key 15 spent none
      // (a down the store had tried to send would have).
      expect(presses.map((r) => r.seq - presses[0].seq)).toEqual([0, 1, 2, 3]);
      const events = await deckEvents(since, (e) => e.some((x) => x.k === 16 && x.d === 0), "the page's deck events up to key 16's up");
      expect(events.filter((e) => e.k === 15).map((e) => [e.d, e.sent, e.q ?? null])).toEqual([[1, false, null]]);
      expect(await pressesOf(15, since)).toEqual([]);
      expect(await flashedKeys(page)).toEqual([15]);
    });
  });

  test.describe("the page's own socket down", () => {
    // A reset of an open socket is a console error in WebKit only (e2e.md).
    test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

    test.afterEach(async () => {
      await impair.block(false);
    });

    test("the tab and the open deck stay through a reconnect; a press meanwhile flashes and is never sent; the keys refresh", async ({
      page,
    }) => {
      await openSurface(page);
      await openDeck(page);
      await watchFlashes(page);
      const deck = page.getByTestId("deck");
      await deck.evaluate((el) => el.setAttribute("data-e2e-kept", "1"));
      const since = Date.now();
      // Key 20 is held when the link goes.
      await dispatchPointer(deckKey(page, 20), [{ type: "pointerdown" }]);
      await expect(deckKey(page, 20)).toHaveAttribute("data-pressed", "true");
      await impair.block(true);
      await impair.drop();
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
      await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "true");
      await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(32);
      // The closed socket forgets the hold; the hub releases the key
      // (`detach`), and the page keeps showing Companion's key as it last knew it.
      await expect(deckKey(page, 20)).toHaveAttribute("data-held", "false");
      await until(() => pressesOf(20, since), (p) => p.join() === "true,false", "key 20 released by the hub");
      await expect(deckKey(page, 20)).toHaveAttribute("data-pressed", "true");
      // The finger's lift sends nothing; a press flashes red.
      await dispatchPointer(deckKey(page, 20), [{ type: "pointerup" }]);
      await tap(page, 7);
      await expect.poll(() => flashedKeys(page)).toEqual([7]);
      await impair.block(false);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 8000 });
      await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false");
      // The same tab and deck page all along.
      await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "true");
      await expect(deck).toHaveAttribute("data-e2e-kept", "1");
      // The open tab told the hub again after the hello: the keys refresh
      // (key 20 as Companion has it now), and a press reaches Companion.
      await expect(deckKey(page, 20)).toHaveAttribute("data-pressed", "false");
      await tap(page, 12);
      await until(() => pressesOf(12, since), (p) => p.join() === "true,false", "key 12 after the reconnect");
      const presses = await hubRecords(
        "deck_press",
        since,
        (r) => r.some((x) => x.key === 12 && x.down === false),
        "key 12's up in the event log",
      );
      expect(presses.map((r) => [r.key, r.down])).toEqual([
        [20, true],
        [12, true],
        [12, false],
      ]);
      const releases = (await hubEvents()).filter((r: any) => r.ev === "deck_release" && r.ts >= since);
      expect(releases.map((r: any) => [r.key, r.reason])).toEqual([[20, "detach"]]);
      await page.waitForTimeout(1000);
      expect(await pressesOf(7, since)).toEqual([]);
      expect(await pressesOf(20, since)).toEqual([true, false]);
      expect(await flashedKeys(page)).toEqual([7]);
    });
  });

  test("the tab exists only when the hub has a [companion] table", async ({ page }) => {
    // Both hub restarts happen with no page open: a page's reconnect while
    // the hub is down would log a failed load.
    try {
      await harness("/hub/companion", { on: false });
      await openSurface(page);
      await expect(page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]').first()).toBeVisible();
      await expect(page.getByTestId("deck-tab")).toHaveCount(0);
    } finally {
      try {
        await page.goto("about:blank");
      } finally {
        await harness("/hub/companion", { on: true });
      }
    }
    await openSurface(page);
    await expect(page.getByTestId("deck-tab")).toBeVisible();
  });
});
