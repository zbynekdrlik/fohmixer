import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  LiveClient,
  centre,
  clipped,
  frames,
  harness,
  hubEvents,
  hubStatus,
  impair,
  openDetail,
  openSurface,
  pageEvents,
  simEq,
  until,
} from "./support/live";

// The Pro-Q 4 screen (#71 PR E; spec F28, D17; the approved mockup
// docs/mockups/channel-detail-v2.html, its EQ card and EQ screen). The
// channel detail's middle lists the strip's Pro-Q 4 instances as cards; a
// card opens the editor over the whole surface: the hub opens it in Live
// (`is_editor_open`), takes its window and sends its picture as binary
// frames, and a finger on the picture becomes touch input at the picture's
// pixels. The harness's hub runs the simulated window backend (`[eq]
// backend = "sim"`): its picture is 1349 × 809 like Pro-Q 4 at 100 %, and
// `simEq` reads what it did (each take, touch and release, in order).
//
// The fixture's Hand2 # holds two: `Pro-Q 4` on the track, and `De-ess`
// inside the rack `Vocal FX`, chain `Main` (its `Pro-C 2` in chain `Air` is
// no Pro-Q 4 and is not listed). The return B-Main repro # holds none.

/** The picture's size (Pro-Q 4 at 100 %, the simulated backend's). */
const WIDTH = 1349;
const HEIGHT = 809;
/** The close guard's tap: the top bar's empty middle (`plugwin::inert_spot`). */
const INERT = { x: 546, y: 15 };
/** Where the test's drag starts and ends on the picture (a band and its move). */
const FROM = { x: 431, y: 321 };
const TO = { x: 511, y: 281 };
/** The drag's moves (one each two animation frames). */
const DRAG_STEPS = 8;
const ON_TRACK = "Pro-Q 4 · na tracku";
const IN_RACK = "Pro-Q 4 · Vocal FX › Main · De-ess";

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

/** `text` as a pattern matching exactly it. */
function exactly(text: string): RegExp {
  return new RegExp(`^${text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`);
}

/** The card whose place reads `where`. */
function card(detail: Locator, where: string): Locator {
  return detail.getByTestId("eq-card").filter({ has: detail.page().getByTestId("eq-card-where").filter({ hasText: exactly(where) }) });
}

/** The cards of Hand2 #'s detail, once listed; the path of each. */
async function hand2Cards(page: Page): Promise<{ detail: Locator; onTrack: Locator; inRack: Locator; trackPath: string; rackPath: string }> {
  const detail = await openDetail(page, "Hand2 #");
  await expect(detail.getByTestId("eq-card")).toHaveCount(2);
  const onTrack = card(detail, ON_TRACK);
  const inRack = card(detail, IN_RACK);
  await expect(onTrack).toHaveCount(1);
  await expect(inRack).toHaveCount(1);
  return {
    detail,
    onTrack,
    inRack,
    trackPath: (await onTrack.getAttribute("data-path"))!,
    rackPath: (await inRack.getAttribute("data-path"))!,
  };
}

/** Waits until Live's editor at `path` (band) is open or closed. */
async function editorOpen(path: string, open: boolean) {
  await until(() => live.get("band", path, "is_editor_open"), (v) => v === open, `the editor ${open ? "open" : "closed"} in Live`);
}

/** Waits for the screen open with frames drawn; the screen. */
async function openScreen(page: Page, path: string): Promise<Locator> {
  const screen = page.getByTestId("eq-screen");
  await expect(screen).toHaveAttribute("data-path", path);
  await expect(screen).toHaveAttribute("data-state", "open", { timeout: 10_000 });
  await expect(screen).not.toHaveAttribute("data-session", "");
  const canvas = screen.getByTestId("eq-canvas");
  await expect.poll(async () => Number(await canvas.getAttribute("data-frames")), { message: "frames drawn" }).toBeGreaterThan(2);
  await expect(canvas).toHaveAttribute("data-width", String(WIDTH));
  await expect(canvas).toHaveAttribute("data-height", String(HEIGHT));
  return screen;
}

/**
 * Where the picture lies on the page: the canvas's box with the picture
 * fitted whole and centred (`object-fit: contain`, the page's
 * `behave::eq::fit`). `toPage` gives a picture pixel's page point (whole
 * px, as a pointer event's), `toPicture` a page point's picture pixel.
 */
async function pictureMap(canvas: Locator) {
  const box = (await canvas.boundingBox())!;
  const scale = Math.min(box.width / WIDTH, box.height / HEIGHT);
  const left = box.x + (box.width - WIDTH * scale) / 2;
  const top = box.y + (box.height - HEIGHT * scale) / 2;
  return {
    toPage: (p: { x: number; y: number }) => ({ x: Math.round(left + p.x * scale), y: Math.round(top + p.y * scale) }),
    toPicture: (p: { x: number; y: number }) => ({ x: (p.x - left) / scale, y: (p.y - top) / scale }),
  };
}

/** A recorded touch's point within 1 px of `at` (the page's mapping and the hub's rounding). */
function near(record: any, at: { x: number; y: number }): boolean {
  return Math.abs(record.x - at.x) <= 1 && Math.abs(record.y - at.y) <= 1;
}

/** The simulated backend's records as `[op, phase, x, y]`. */
function steps(records: any[]): (string | number | null)[][] {
  return records.map((r) => [r.op, r.phase ?? null, r.x ?? null, r.y ?? null]);
}

/** The close guard and the release, as the simulated backend records them. */
const GUARD_AND_RELEASE = [
  ["touch", "down", INERT.x, INERT.y],
  ["touch", "up", INERT.x, INERT.y],
  ["release", null, null, null],
];

/**
 * Waits for an editor's close at the simulated backend after its first
 * `before` records (the release comes after Live's `is_editor_open` turned
 * false) and wants exactly the guard's tap on the inert spot, then the
 * release, all on one window; the records' count after it.
 */
async function closedWithGuard(before: number): Promise<number> {
  const all = await until(simEq.records, (r) => r.some((x, i) => i >= before && x.op === "release"), "the editor's release");
  const close = all.slice(before);
  expect(steps(close), "the guard's tap, then the release").toEqual(GUARD_AND_RELEASE);
  expect(close.every((r) => r.window === close[0].window), "one window").toBe(true);
  return all.length;
}

/** Every text of a card inside its box (#9). */
async function cardFits(c: Locator) {
  for (const id of ["eq-card-where", "eq-open"]) {
    expect(await clipped(c.getByTestId(id)), id).toEqual([]);
  }
}

/**
 * The page hidden (`document.hidden`, `visibilityState`) or shown again, with
 * the `visibilitychange` a browser fires (it bubbles to the window), as
 * deck.spec.ts does.
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
 * Records every text a card's note shows, as it appears (`window.eqNotes`):
 * a note can last only until the hub's next word about its editor.
 */
async function recordNotes(page: Page) {
  await page.evaluate(() => {
    const seen: string[] = [];
    (window as any).eqNotes = seen;
    const look = () =>
      document.querySelectorAll('[data-testid="eq-card-note"]').forEach((note) => {
        const text = note.textContent ?? "";
        if (!seen.includes(text)) seen.push(text);
      });
    new MutationObserver(look).observe(document.body, { childList: true, subtree: true, characterData: true });
  });
}

/** The notes `recordNotes` saw. */
async function notesSeen(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).eqNotes as string[]);
}

/** A page's console errors and warnings (a second page has no console guard). */
function watchConsole(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
  return problems;
}

test.describe("The Pro-Q 4 screen", () => {
  test("the detail lists a track's Pro-Q 4 instances as cards; a track without one says so", async ({ page }) => {
    await openSurface(page);
    const { detail, onTrack, inRack, trackPath, rackPath } = await hand2Cards(page);
    await expect(detail.getByTestId("detail-middle").locator("h2")).toHaveText("EQ");
    // Live's paths of the two devices: on the track, and in the rack's chain.
    expect(trackPath).toMatch(/^live_set tracks \d+ devices \d+$/);
    expect(rackPath).toMatch(/^live_set tracks \d+ devices \d+ chains \d+ devices \d+$/);
    expect(await live.get("band", trackPath, "class_display_name")).toBe("Pro-Q 4");
    expect(await live.get("band", rackPath, "name")).toBe("De-ess");
    for (const c of [onTrack, inRack]) {
      await expect(c).toHaveAttribute("data-locked", "false");
      const open = c.getByTestId("eq-open");
      await expect(open).toHaveText("OTVORIŤ EQ NA CELÚ OBRAZOVKU");
      await expect(open).toHaveAttribute("aria-disabled", "false");
      await cardFits(c);
    }
    // Back to the mix and into a band strip with no device (the return
    // `B-Main repro #` on the default page; Hand1 # is a master strip on the
    // OTHERS page).
    await detail.getByTestId("detail-exit").click();
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const other = await openDetail(page, "B-Main repro #");
    await expect(other.getByTestId("eq-note")).toHaveText("Na tomto tracku nie je Pro-Q 4.");
    await expect(other.getByTestId("eq-card")).toHaveCount(0);
    expect(await clipped(other.getByTestId("eq-note"))).toEqual([]);
  });

  test("a card opens the editor; frames arrive; a drag reaches it at the picture's pixels; the exit taps the guard spot first", async ({ page }) => {
    const since = Date.now();
    await openSurface(page);
    const { detail, onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await simEq.clear();
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    await editorOpen(trackPath, true);
    // The bar: the exit, the strip's chip, where the editor sits, the lock mark.
    await expect(screen.getByTestId("eq-where")).toHaveText(ON_TRACK);
    await expect(screen.getByTestId("eq-mine")).toHaveAttribute("data-shown", "true");
    await expect(screen.getByTestId("eq-mine")).toBeVisible();
    for (const id of ["eq-exit", "eq-where", "eq-mine"]) {
      expect(await clipped(screen.getByTestId(id)), id).toEqual([]);
    }
    // `?` shows the touch legend and hides it again.
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend")).toBeVisible();
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend")).toHaveCount(0);
    // The take, before any touch.
    expect(steps(await simEq.records())[0]).toEqual(["take", null, null, null]);

    // A real drag on the picture, from a band to its new place: a move per
    // animation frame (the page sends at most one a frame, the newest).
    const canvas = screen.getByTestId("eq-canvas");
    const map = await pictureMap(canvas);
    const from = map.toPage(FROM);
    const to = map.toPage(TO);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    const drag = (
      await until(
        simEq.records,
        (all) => all.some((r) => r.op === "touch" && r.phase === "up"),
        "the drag's up at the simulated backend",
      )
    ).filter((r) => r.op === "touch");
    const editorWindow = drag[0].window;
    expect(drag.every((r) => r.window === editorWindow), "one window").toBe(true);
    expect(drag[0].phase).toBe("down");
    expect(near(drag[0], map.toPicture(from)), `the down ${JSON.stringify(drag[0])} at ${JSON.stringify(map.toPicture(from))}`).toBe(true);
    const up = drag[drag.length - 1];
    expect(up.phase).toBe("up");
    expect(near(up, map.toPicture(to)), `the up ${JSON.stringify(up)} at ${JSON.stringify(map.toPicture(to))}`).toBe(true);
    // The moves between, along the way (a resting finger's point is sent
    // again every 100 ms, so a point can repeat).
    const moves = drag.slice(1, -1);
    const points = new Set(moves.map((m) => `${m.x},${m.y}`));
    expect(points.size, `the moves' points: ${[...points].join(" ")}`).toBeGreaterThanOrEqual(DRAG_STEPS / 2);
    for (const m of moves) {
      expect(m.phase).toBe("update");
      expect(m.x).toBeGreaterThanOrEqual(FROM.x - 2);
      expect(m.x).toBeLessThanOrEqual(TO.x + 2);
      expect(m.y).toBeGreaterThanOrEqual(TO.y - 2);
      expect(m.y).toBeLessThanOrEqual(FROM.y + 2);
    }

    // Back to the channel: the guard's tap on the inert spot, then the
    // window handed back, and Live closes the editor.
    const before = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await expect(detail).toBeVisible();
    await closedWithGuard(before);
    expect((await simEq.records()).slice(before)[0].window, "the editor's window").toBe(editorWindow);
    await editorOpen(trackPath, false);

    // The card shows the editor's last picture now.
    const img = onTrack.getByTestId("eq-card-img");
    await expect(img).toBeVisible();
    await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth), { message: "the last picture decoded" }).toBe(WIDTH);

    // The hub's records of the session and the page's steps.
    const eq = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "eq" && r.path === trackPath && r.ts >= since),
      (all) => all.some((r: any) => r.what === "closed"),
      "the hub's eq records",
    );
    expect(eq.map((r: any) => r.what)).toEqual(["take", "open", "opened", "touch", "touch", "close", "closed"]);
    expect(eq.filter((r: any) => r.what === "touch").map((r: any) => r.why)).toEqual(["down", "up"]);
    expect(eq.find((r: any) => r.what === "close").why).toBe("exit");
    const pageSteps = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "detail" && e.t >= since && String(e.what).startsWith("eq_")),
      (all) => all.some((e: any) => e.what === "eq_close"),
      "the page's eq steps",
      15_000,
    );
    expect(pageSteps.map((e: any) => [e.what, e.why ?? null])).toEqual([
      ["eq_open", null],
      ["eq_close", "exit"],
    ]);
  });

  test("another page sees the editor locked and cannot open it; the other one stays free; a closed page frees it", async ({ page, context }) => {
    const holder = await context.newPage();
    const holderConsole = watchConsole(holder);
    await openSurface(holder);
    const held = await hand2Cards(holder);
    await editorOpen(held.trackPath, false);
    const opened = Date.now();
    await held.onTrack.getByTestId("eq-open").click();
    await openScreen(holder, held.trackPath);

    await openSurface(page);
    const { onTrack, inRack, rackPath } = await hand2Cards(page);
    await expect(onTrack).toHaveAttribute("data-locked", "true");
    const lock = onTrack.getByTestId("eq-card-lock");
    await expect(lock).toBeVisible();
    // Since when, in the page's local time (the open took a moment: this
    // minute or the next).
    const times = await page.evaluate((t) => {
      const hhmm = (d: Date) => `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      return [hhmm(new Date(t)), hhmm(new Date(t + 60_000))];
    }, opened);
    const text = (await lock.textContent())!;
    expect(text.startsWith("ZAMKNUTÉ"), text).toBe(true);
    expect(times.map((hm) => text.endsWith(`Upravuje ho iný zvukár (od ${hm})`)), text).toContain(true);
    expect(await clipped(lock)).toEqual([]);
    const open = onTrack.getByTestId("eq-open");
    await expect(open).toHaveText("ZAMKNUTÉ");
    await expect(open).toHaveAttribute("aria-disabled", "true");
    await cardFits(onTrack);
    // A tap on it opens nothing.
    await open.scrollIntoViewIfNeeded();
    const { x, y } = await centre(open);
    await page.mouse.click(x, y);
    await page.waitForTimeout(500);
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    // The rack's editor is free: this page opens it beside the other one.
    await expect(inRack).toHaveAttribute("data-locked", "false");
    await inRack.getByTestId("eq-open").click();
    const screen = await openScreen(page, rackPath);
    await expect(screen.getByTestId("eq-where")).toHaveText(IN_RACK);
    await editorOpen(rackPath, true);
    await editorOpen(held.trackPath, true);
    const rackBefore = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    const before = await closedWithGuard(rackBefore);
    await editorOpen(rackPath, false);

    // The holder's page goes: the hub closes its editor (the guard first).
    expect(holderConsole, "the holder's console").toEqual([]);
    await holder.close();
    await expect(onTrack).toHaveAttribute("data-locked", "false");
    await expect(onTrack.getByTestId("eq-open")).toHaveText("OTVORIŤ EQ NA CELÚ OBRAZOVKU");
    await closedWithGuard(before);
    await editorOpen(held.trackPath, false);
  });

  test("an open the hub refuses says why under its card in Slovak, never the hub's English", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    await openScreen(page, trackPath);
    const before = (await simEq.records()).length;
    await recordNotes(page);
    // Back to the channel and at once the same card again: the hub is still
    // closing it (the guard's tap, then 300 ms), so it refuses the open
    // (`closing`) until the close is done. Both downs in one evaluate, the
    // second once the screen is gone.
    await page.evaluate(async (path) => {
      const down = (selector: string) => {
        const el = document.querySelector(selector);
        if (!el) throw new Error(`no ${selector}`);
        el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 41, isPrimary: true }));
      };
      down('[data-testid="eq-exit"]');
      for (let i = 0; document.querySelector('[data-testid="eq-screen"]'); i++) {
        if (i === 100) throw new Error("the screen stayed");
        await new Promise((r) => setTimeout(r, 0));
      }
      down(`[data-testid="eq-card"][data-path="${path}"] [data-testid="eq-open"]`);
    }, trackPath);
    await expect.poll(() => notesSeen(page), { message: "the card's note" }).toContain("ešte sa zatvára, skús znova");
    // The refused screen went; the first close ends with its guard, and the
    // note goes with it (the editor can open again).
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(before);
    await editorOpen(trackPath, false);
    await expect(onTrack.getByTestId("eq-card-note")).toHaveCount(0);
    // The only note it showed: the Slovak one (never "Neotvoril sa: closing").
    expect(await notesSeen(page)).toEqual(["ešte sa zatvára, skús znova"]);
  });

  test("a page going hidden lifts its finger: the PC's contact is cancelled where it was", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const map = await pictureMap(screen.getByTestId("eq-canvas"));
    const at = map.toPage(FROM);
    const before = (await simEq.records()).length;
    const touches = async () => (await simEq.records()).slice(before).filter((r) => r.op === "touch");
    await page.mouse.move(at.x, at.y);
    await page.mouse.down();
    await until(touches, (all) => all.some((r) => r.phase === "down"), "the finger's down");
    // The finger rests and the page goes hidden (it still pings, so the
    // hub's 2 s silence would never end the contact).
    await setHidden(page, true);
    const ended = await until(touches, (all) => all.some((r) => r.phase === "cancel"), "the hidden page's cancel", 1_500);
    expect(ended[0].phase).toBe("down");
    const cancel = ended[ended.length - 1];
    expect(cancel.phase).toBe("cancel");
    expect(near(cancel, map.toPicture(at)), `the cancel ${JSON.stringify(cancel)} at ${JSON.stringify(map.toPicture(at))}`).toBe(true);
    // Between them only the resting finger's resends, at its point.
    for (const r of ended.slice(1, -1)) {
      expect(r.phase).toBe("update");
      expect(near(r, map.toPicture(at))).toBe(true);
    }
    // Shown again, the lift sends nothing; the exit closes with the guard.
    await setHidden(page, false);
    await page.mouse.up();
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
    expect((await touches()).filter((r) => r.phase === "up" && !near(r, INERT)), "no lift of the cancelled finger").toEqual([]);
  });

  test("the cards are listed again when Live comes back, and open", async ({ page }) => {
    await openSurface(page);
    const { detail, trackPath } = await hand2Cards(page);
    // The cards as listed now; a new list makes new ones.
    await detail.getByTestId("eq-card").first().evaluate((el) => el.setAttribute("data-e2e-kept", "1"));
    await harness("/host/band/restart");
    await until(hubStatus, (s) => s.instances[0].online, "the band's Live back", 10_000);
    await expect(detail.locator('[data-testid="eq-card"][data-e2e-kept]'), "listed again").toHaveCount(0, { timeout: 10_000 });
    await expect(detail.getByTestId("eq-card")).toHaveCount(2);
    const onTrack = card(detail, ON_TRACK);
    await expect(onTrack).toHaveCount(1);
    await expect(onTrack).toHaveAttribute("data-path", trackPath);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    await editorOpen(trackPath, true);
    const before = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(before);
    await editorOpen(trackPath, false);
  });
});

test.describe("The Pro-Q 4 screen and a lost link", () => {
  // The dropped link resets the page's socket: WebKit logs the reset.
  test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

  test("a lost socket closes the screen and the hub closes the editor; it opens again once back", async ({ page }) => {
    await openSurface(page);
    const { detail, onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    await openScreen(page, trackPath);
    const before = (await simEq.records()).length;
    await impair.drop();
    // The page leaves the screen for the channel; the hub, the page gone,
    // closes the editor with its guard.
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await expect(detail).toBeVisible();
    await expect(onTrack.getByTestId("eq-card-note")).toHaveCount(0);
    await closedWithGuard(before);
    await editorOpen(trackPath, false);
    // Back on the link, the cards are listed again and the editor opens.
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 10_000 });
    await expect(onTrack).toHaveAttribute("data-locked", "false");
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    await editorOpen(trackPath, true);
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await editorOpen(trackPath, false);
  });

  test("while the socket is down a card offers no open (no screen waiting for nothing); back on the link it does", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    const open = onTrack.getByTestId("eq-open");
    await expect(open).toHaveAttribute("aria-disabled", "false");
    try {
      // The link cut and held: the page cannot reconnect meanwhile.
      await impair.block(true);
      await impair.drop();
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
      await expect(open).toHaveAttribute("aria-disabled", "true");
      await expect(open).toHaveText("OTVORIŤ EQ NA CELÚ OBRAZOVKU");
      // A real tap opens nothing.
      await open.scrollIntoViewIfNeeded();
      const { x, y } = await centre(open);
      await page.mouse.click(x, y);
      await page.waitForTimeout(500);
      await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    } finally {
      await impair.block(false);
    }
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 10_000 });
    await expect(open).toHaveAttribute("aria-disabled", "false");
    await editorOpen(trackPath, false);
  });
});
