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
//
// PR F (the approved mockup docs/mockups/proq-zoom-v1.html): two fingers
// zoom and pan the picture on the page only, 1× to 4×; a first finger
// reaches the editor once it waited 120 ms or moved 6 px, a quick lift is a
// tap (PR G: its down at once, its up 40 ms later on a frame). Two fingers
// are dispatched pointer events (Playwright has no two-finger input in
// either engine).
//
// The owner's ruling of 2026-10-11 (#71): the whole picture takes no edits.
// Under 2× one finger never reaches the editor: a tap zooms to 3× on its
// point (`zoomIn`), a drag does nothing, and a hint over the picture says
// so. Every test that touches the editor zooms in first.
//
// PR G: the editor takes the shape of the device that opens it. The open
// carries the screen's picture area, and the hub resizes the editor to its
// aspect at Pro-Q's own pixel count (the sim's `resize` record, the frames
// at that size) before it answers; a changed viewport sends `eq_area` and
// the editor follows; the close puts the editor's own size back before the
// guard's tap.

/** The editor's own picture (Pro-Q 4 at 100 %, the simulated backend's): the close puts it back. */
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

/** The picture's size as the canvas last drew it. */
async function pictureSize(canvas: Locator): Promise<{ width: number; height: number }> {
  return { width: Number(await canvas.getAttribute("data-width")), height: Number(await canvas.getAttribute("data-height")) };
}

/**
 * How far the picture's aspect lies from the screen's area's (a ratio, 0
 * for the same shape): the hub gives the editor the area's aspect, its sides
 * rounded to whole pixels (PR G).
 */
async function shapeGap(screen: Locator): Promise<number> {
  const area = (await screen.getByTestId("eq-area").boundingBox())!;
  const { width, height } = await pictureSize(screen.getByTestId("eq-canvas"));
  return Math.abs(width / height / (area.width / area.height) - 1);
}

/** Waits until the picture drawn has the screen's area's aspect (PR G: a resize settled and its frames arrived). */
async function shaped(screen: Locator) {
  await expect.poll(() => shapeGap(screen), { message: "the picture has the area's shape", timeout: 10_000 }).toBeLessThan(0.004);
}

/** Waits for the screen open with frames drawn at the area's aspect; the screen. */
async function openScreen(page: Page, path: string): Promise<Locator> {
  const screen = page.getByTestId("eq-screen");
  await expect(screen).toHaveAttribute("data-path", path);
  await expect(screen).toHaveAttribute("data-state", "open", { timeout: 10_000 });
  await expect(screen).not.toHaveAttribute("data-session", "");
  const canvas = screen.getByTestId("eq-canvas");
  await expect.poll(async () => Number(await canvas.getAttribute("data-frames")), { message: "frames drawn" }).toBeGreaterThan(2);
  // The editor took the area's shape before the open answered (PR G).
  await shaped(screen);
  // Each open starts with the whole picture (PR F).
  await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "false");
  await expect(screen.getByTestId("eq-zoom")).toBeHidden();
  return screen;
}

/**
 * Where the picture lies on the page: the canvas's box with the picture
 * fitted whole and centred (`object-fit: contain`, the page's
 * `behave::eq::fit`). `toPage` gives a picture pixel's page point (whole
 * px, as a pointer event's), `toPicture` a page point's picture pixel. The
 * box is the transformed one (PR F: the page's zoom scales and moves the
 * canvas), so this is where the browser draws the picture.
 */
async function pictureMap(canvas: Locator) {
  const box = (await canvas.boundingBox())!;
  const { width, height } = await pictureSize(canvas);
  const scale = Math.min(box.width / width, box.height / height);
  const left = box.x + (box.width - width * scale) / 2;
  const top = box.y + (box.height - height * scale) / 2;
  return {
    width,
    height,
    scale,
    left,
    top,
    toPage: (p: { x: number; y: number }) => ({ x: Math.round(left + p.x * scale), y: Math.round(top + p.y * scale) }),
    toPicture: (p: { x: number; y: number }) => ({ x: (p.x - left) / scale, y: (p.y - top) / scale }),
  };
}

type At = { x: number; y: number };

/**
 * Two fingers dispatched on the screen's area (PR F): down `from` px left
 * and right of `at` (page px), two frames (the page starts its pinch at the
 * first frame both are down), then spread or closed to `to` px in 8 frames
 * (one move each a frame, as the page follows them), held 2 frames,
 * lifted. The zoom then is the one before times `to / from`.
 */
async function pinch(area: Locator, at: At, from: number, to: number, ids = [31, 32]) {
  await area.evaluate(
    async (el, { at, from, to, ids }) => {
      const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));
      const fire = (type: string, id: number, x: number, primary: boolean) =>
        el.dispatchEvent(
          new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: primary, clientX: x, clientY: at.y, bubbles: true, cancelable: true }),
        );
      fire("pointerdown", ids[0], at.x - from, true);
      fire("pointerdown", ids[1], at.x + from, false);
      await frame();
      await frame();
      const steps = 8;
      for (let i = 1; i <= steps; i++) {
        const half = Math.round(from + ((to - from) * i) / steps);
        fire("pointermove", ids[0], at.x - half, true);
        fire("pointermove", ids[1], at.x + half, false);
        await frame();
      }
      await frame();
      await frame();
      fire("pointerup", ids[0], at.x - to, true);
      fire("pointerup", ids[1], at.x + to, false);
      await frame();
      await frame();
    },
    { at, from, to, ids },
  );
}

/** A quick tap dispatched on the area at a page point: its down and its up in one task, well inside the 120 ms hold (PR G: the page sends its down at once and its up 40 ms later). */
async function quickTap(area: Locator, at: At, id = 61) {
  await area.evaluate(
    (el, { at, id }) => {
      for (const type of ["pointerdown", "pointerup"]) {
        el.dispatchEvent(
          new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: true, clientX: at.x, clientY: at.y, bubbles: true, cancelable: true }),
        );
      }
    },
    { at, id },
  );
}

/** How long after a tap's zoom a first finger reaches nothing (ms, `behave::eq::ZOOM_SETTLE_MS`). */
const ZOOM_SETTLE_MS = 500;

/**
 * A real tap on the whole picture at its pixel `at` (the owner's ruling of
 * 2026-10-11): the view zooms to 3× with that point in the area's middle,
 * and the editor gets nothing. The picture's map once the zoom is drawn
 * and its settle is over (a finger then edits).
 */
async function zoomIn(screen: Locator, at: At) {
  const canvas = screen.getByTestId("eq-canvas");
  const point = (await pictureMap(canvas)).toPage(at);
  await screen.page().mouse.click(point.x, point.y);
  await expect(screen.getByTestId("eq-factor")).toHaveText("3,0×");
  await expect(screen.getByTestId("eq-hint")).toHaveAttribute("data-shown", "false");
  await frames(screen.page(), 2);
  await screen.page().waitForTimeout(ZOOM_SETTLE_MS + 50);
  return pictureMap(canvas);
}

/** The backend's touches after its first `before` records. */
async function touchesAfter(before: number): Promise<any[]> {
  return (await simEq.records()).slice(before).filter((r) => r.op === "touch");
}

/** A finger's touches as the backend got them: a down and an up within 1 px of `from` and `to`, only updates between. */
function wentFromTo(touches: any[], from: At, to: At) {
  expect(touches[0].phase, JSON.stringify(touches)).toBe("down");
  expect(near(touches[0], from), `the down ${JSON.stringify(touches[0])} at ${JSON.stringify(from)}`).toBe(true);
  const up = touches[touches.length - 1];
  expect(up.phase, JSON.stringify(touches)).toBe("up");
  expect(near(up, to), `the up ${JSON.stringify(up)} at ${JSON.stringify(to)}`).toBe(true);
  for (const m of touches.slice(1, -1)) expect(m.phase).toBe("update");
}

/** The canvas's box is the area's: the whole picture, no zoom. */
async function drawnWhole(screen: Locator) {
  const area = (await screen.getByTestId("eq-area").boundingBox())!;
  await expect
    .poll(async () => {
      const box = (await screen.getByTestId("eq-canvas").boundingBox())!;
      return Math.max(Math.abs(box.x - area.x), Math.abs(box.y - area.y), Math.abs(box.width - area.width), Math.abs(box.height - area.height));
    }, { message: "the canvas's box is the area's" })
    .toBeLessThan(0.5);
}

/**
 * How far the overview's shape lies from the picture's (a ratio, 0 for the
 * same; #74 review: an upright picture was drawn squashed into a fixed
 * landscape box): its box inside its border against the picture's aspect,
 * which the page bends no further than 0.4 to 2.5.
 */
async function overviewGap(screen: Locator): Promise<number> {
  const { width, height } = await pictureSize(screen.getByTestId("eq-canvas"));
  const want = Math.min(2.5, Math.max(0.4, width / height));
  const shape = await screen.getByTestId("eq-overview").evaluate((el: HTMLElement) => {
    const box = el.getBoundingClientRect();
    return (box.width - 2 * el.clientLeft) / (box.height - 2 * el.clientTop);
  });
  return Math.abs(shape / want - 1);
}

/** The bar's zoom group shown, the overview in the picture's shape, and every part of the bar inside it and the screen, its texts whole. */
async function zoomBarFits(screen: Locator) {
  await expect(screen.getByTestId("eq-zoom")).toBeVisible();
  await expect.poll(() => overviewGap(screen), { message: "the overview in the picture's shape" }).toBeLessThan(0.02);
  const bar = screen.getByTestId("eq-bar");
  const barBox = (await bar.boundingBox())!;
  const width = screen.page().viewportSize()!.width;
  expect(await bar.evaluate((el) => el.scrollWidth - el.clientWidth), "the bar's overflow").toBeLessThanOrEqual(0);
  for (const id of ["eq-exit", "eq-overview", "eq-factor", "eq-whole", "eq-help"]) {
    const box = (await screen.getByTestId(id).boundingBox())!;
    const inside = box.x >= barBox.x - 0.5 && box.x + box.width <= Math.min(barBox.x + barBox.width, width) + 0.5 && box.y >= barBox.y - 0.5 && box.y + box.height <= barBox.y + barBox.height + 0.5;
    expect(inside, `${id} ${JSON.stringify(box)} inside the bar ${JSON.stringify(barBox)}`).toBe(true);
  }
  for (const id of ["eq-exit", "eq-factor", "eq-whole"]) {
    expect(await clipped(screen.getByTestId(id)), id).toEqual([]);
  }
}

/** A recorded touch's point within 1 px of `at` (the page's mapping and the hub's rounding). */
function near(record: any, at: { x: number; y: number }): boolean {
  return Math.abs(record.x - at.x) <= 1 && Math.abs(record.y - at.y) <= 1;
}

/** The simulated backend's records as `[op, phase, x, y]` (a resize: `[op, null, w, h]`). */
function steps(records: any[]): (string | number | null)[][] {
  return records.map((r) => [r.op, r.phase ?? null, r.x ?? r.w ?? null, r.y ?? r.h ?? null]);
}

/**
 * The close guard and the release, as the simulated backend records them: the
 * editor's own size back first (PR G), placed a pixel aside, then where it
 * goes, each seen to land (#74 review, round 2), then the tap.
 */
const GUARD_AND_RELEASE = [
  ["resize", null, WIDTH, HEIGHT],
  ["resize", null, WIDTH, HEIGHT],
  ["touch", "down", INERT.x, INERT.y],
  ["touch", "up", INERT.x, INERT.y],
  ["release", null, null, null],
];

/**
 * Waits for an editor's close at the simulated backend after its first
 * `before` records (the release comes after Live's `is_editor_open` turned
 * false) and wants exactly the editor's own size back, the guard's tap on
 * the inert spot, then the release, all on one window; the records' count
 * after it.
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

  test("a card opens the editor in the screen's shape; frames arrive; a drag reaches it at the picture's pixels; the exit puts its size back and taps the guard spot first", async ({ page }) => {
    const since = Date.now();
    // The page's own messages to the hub (PR G: the open carries its area).
    const opens: any[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_open"')) opens.push(JSON.parse(payload));
      });
    });
    await openSurface(page);
    const { detail, onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await simEq.clear();
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    await editorOpen(trackPath, true);
    // The open carried the screen's picture area, and the editor was
    // resized to its aspect right after its take, before any frame.
    const areaBox = (await screen.getByTestId("eq-area").boundingBox())!;
    expect(opens.length, JSON.stringify(opens)).toBe(1);
    expect(Math.abs(opens[0].area.w - areaBox.width), JSON.stringify(opens[0])).toBeLessThan(1);
    expect(Math.abs(opens[0].area.h - areaBox.height), JSON.stringify(opens[0])).toBeLessThan(1);
    const picture = await pictureSize(screen.getByTestId("eq-canvas"));
    expect(steps((await simEq.records()).slice(0, 2))).toEqual([
      ["take", null, null, null],
      ["resize", null, picture.width, picture.height],
    ]);
    // About Pro-Q's own pixel count (the room allows it at this size).
    expect(Math.abs((picture.width * picture.height) / (WIDTH * HEIGHT) - 1)).toBeLessThan(0.01);
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

    // A tap on the whole picture zooms in on the band (nothing reaches the
    // editor), then a real drag from the band to its new place: a move per
    // animation frame (the page sends at most one a frame, the newest).
    const map = await zoomIn(screen, FROM);
    expect(await touchesAfter(0), "the zoom's tap reaches no editor").toEqual([]);
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
    // again every 50 ms, so a point can repeat).
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
    await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.naturalWidth), { message: "the last picture decoded" }).toBe(picture.width);

    // The hub's records of the session and the page's steps.
    const eq = await until(
      // A close's `frames` record (its round trips since the last `rate`)
      // comes only when the link acked frames after that rate: not listed.
      async () => (await hubEvents()).filter((r: any) => r.ev === "eq" && r.path === trackPath && r.ts >= since && r.what !== "frames"),
      (all) => all.some((r: any) => r.what === "closed"),
      "the hub's eq records",
    );
    expect(eq.map((r: any) => r.what)).toEqual(["take", "open", "resize", "opened", "touch", "touch", "close", "closed"]);
    const resize = eq.find((r: any) => r.what === "resize");
    expect([resize.w, resize.h, resize.client_w, resize.client_h, resize.ok]).toEqual([picture.width, picture.height, picture.width, picture.height, true]);
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

  test("the page acks each frame and the frames keep coming; its fingers on the picture reach the flight recorder", async ({ page }) => {
    // #71 PR I: one frame in flight per page. The hub sends the next frame
    // only once the page acked the one before (or 1 s passed), so without
    // the page's acks the sim's four frames a second would come one a
    // second.
    const since = Date.now();
    const acks: any[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_frame_ack"')) acks.push(JSON.parse(payload));
      });
    });
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const session = Number(await screen.getAttribute("data-session"));
    const canvas = screen.getByTestId("eq-canvas");
    const drawn = async () => Number(await canvas.getAttribute("data-frames"));
    const first = await drawn();
    const acked = acks.length;
    await page.waitForTimeout(4_000);
    const frameCount = (await drawn()) - first;
    expect(frameCount, "frames drawn in 4 s (one a second without acks)").toBeGreaterThanOrEqual(8);
    expect(acks.length - acked, "the page's acks").toBeGreaterThanOrEqual(8);
    expect(acks.every((a) => a.session === session), JSON.stringify(acks.slice(0, 3))).toBe(true);
    // Each ack names its frame (the PR I review): the hub numbers a
    // session's frames from 1, and the page acks every one in order.
    expect(acks[0].seq, JSON.stringify(acks.slice(0, 3))).toBe(1);
    expect(acks.every((a, i) => i === 0 || a.seq === acks[i - 1].seq + 1), JSON.stringify(acks.map((a) => a.seq))).toBe(true);

    // A real drag on the picture: its down, its moves summarized and its
    // up, at the picture's pixels, with the view's zoom.
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
    const fingers = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => (e.ev === "eq_touch" || e.ev === "eq_mv") && e.t >= since),
      (all) => all.some((e: any) => e.ev === "eq_touch" && e.what === "up"),
      "the finger's events in the event log",
      15_000,
    );
    const touches = fingers.filter((e: any) => e.ev === "eq_touch");
    expect(touches.map((e: any) => e.what), JSON.stringify(fingers)).toEqual(["down", "up"]);
    expect(touches.every((e: any) => e.p === touches[0].p && e.z === 1), JSON.stringify(touches)).toBe(true);
    expect(near(touches[0], map.toPicture(from)), `the down ${JSON.stringify(touches[0])} at ${JSON.stringify(map.toPicture(from))}`).toBe(true);
    expect(near(touches[1], map.toPicture(to)), `the up ${JSON.stringify(touches[1])} at ${JSON.stringify(map.toPicture(to))}`).toBe(true);
    const moves = fingers.filter((e: any) => e.ev === "eq_mv");
    expect(moves.length, JSON.stringify(fingers)).toBeGreaterThanOrEqual(1);
    expect(moves.reduce((n: number, m: any) => n + m.n, 0), JSON.stringify(moves)).toBeGreaterThanOrEqual(DRAG_STEPS / 2);
    const last = moves[moves.length - 1];
    expect(Math.abs(last.b[0] - touches[1].x) <= 1 && Math.abs(last.b[1] - touches[1].y) <= 1, JSON.stringify(last)).toBe(true);

    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("another page sees every Pro-Q 4 locked while one is held (one on the PC's screen at a time); a closed page frees them", async ({ page, context }) => {
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
    // Since when, in the page's local time (the open took a moment: this
    // minute or the next).
    const times = await page.evaluate((t) => {
      const hhmm = (d: Date) => `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      return [hhmm(new Date(t)), hhmm(new Date(t + 60_000))];
    }, opened);
    // The held one with its holder, the other one in use: both locked.
    for (const [c, line] of [
      [onTrack, "Upravuje ho iný zvukár"],
      [inRack, "Pro-Q 4 práve používa iný zvukár"],
    ] as const) {
      await expect(c).toHaveAttribute("data-locked", "true");
      const lock = c.getByTestId("eq-card-lock");
      await expect(lock).toBeVisible();
      const text = (await lock.textContent())!;
      expect(text.startsWith("ZAMKNUTÉ"), text).toBe(true);
      expect(times.map((hm) => text.endsWith(`${line} (od ${hm})`)), text).toContain(true);
      expect(await clipped(lock)).toEqual([]);
      const open = c.getByTestId("eq-open");
      await expect(open).toHaveText("ZAMKNUTÉ");
      await expect(open).toHaveAttribute("aria-disabled", "true");
      await cardFits(c);
      // A tap on it opens nothing.
      await open.scrollIntoViewIfNeeded();
      const { x, y } = await centre(open);
      await page.mouse.click(x, y);
      await page.waitForTimeout(500);
      await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    }
    await editorOpen(rackPath, false);

    // The holder's page goes: the hub closes its editor (the guard first),
    // and both cards are free again.
    const before = (await simEq.records()).length;
    expect(holderConsole, "the holder's console").toEqual([]);
    await holder.close();
    await closedWithGuard(before);
    await editorOpen(held.trackPath, false);
    for (const c of [onTrack, inRack]) {
      await expect(c).toHaveAttribute("data-locked", "false");
      await expect(c.getByTestId("eq-card-lock")).toHaveCount(0);
      await expect(c.getByTestId("eq-open")).toHaveText("OTVORIŤ EQ NA CELÚ OBRAZOVKU");
      await expect(c.getByTestId("eq-open")).toHaveAttribute("aria-disabled", "false");
    }
    // The other one opens now.
    await inRack.getByTestId("eq-open").click();
    const screen = await openScreen(page, rackPath);
    await expect(screen.getByTestId("eq-where")).toHaveText(IN_RACK);
    await editorOpen(rackPath, true);
    const rackBefore = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(rackBefore);
    await editorOpen(rackPath, false);
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
    const before = (await simEq.records()).length;
    const map = await zoomIn(screen, FROM);
    const at = map.toPage(FROM);
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

  test("two fingers zoom the page's picture and send the editor nothing; a finger then reaches it where it shows; a quick tap goes down, then up; CELÝ EQ and a new open show it whole", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const canvas = screen.getByTestId("eq-canvas");
    const zoom = screen.getByTestId("eq-zoom");
    const areaBox = (await area.boundingBox())!;
    const middle = await centre(area);
    const whole = await pictureMap(canvas);
    const before = (await simEq.records()).length;

    // Two fingers 100 px apart about the area's middle spread to 250: 2,5×,
    // and the picture point under their midpoint stays under it.
    await pinch(area, middle, 50, 125);
    await expect(zoom).toHaveAttribute("data-shown", "true");
    await expect(screen.getByTestId("eq-factor")).toHaveText("2,5×");
    await zoomBarFits(screen);
    const map = await pictureMap(canvas);
    expect(map.scale / whole.scale, "the zoom").toBeCloseTo(2.5, 3);
    const was = whole.toPicture(middle);
    const now = map.toPicture(middle);
    expect(Math.abs(now.x - was.x) <= 1 && Math.abs(now.y - was.y) <= 1, `${JSON.stringify(now)} under the midpoint, was ${JSON.stringify(was)}`).toBe(true);
    // The overview's frame: the part of the picture in sight.
    const frame = await screen.getByTestId("eq-overview-view").evaluate((el: HTMLElement) =>
      ["left", "top", "width", "height"].map((k) => parseFloat(el.style.getPropertyValue(k)) / 100),
    );
    const span = { x: map.width * map.scale, y: map.height * map.scale };
    const seen = [
      Math.max(0, (areaBox.x - map.left) / span.x),
      Math.max(0, (areaBox.y - map.top) / span.y),
      Math.min(1, areaBox.width / span.x),
      Math.min(1, areaBox.height / span.y),
    ];
    frame.forEach((part, i) => expect(Math.abs(part - seen[i]), `the overview's frame ${frame} against ${seen}`).toBeLessThan(0.002));
    expect(seen[2], "a part of the width").toBeLessThan(1);
    // Nothing reached the editor.
    await page.waitForTimeout(300);
    expect(await touchesAfter(before), "the pinch's touches").toEqual([]);

    // One finger on the zoomed picture: a drag reaches the editor at the
    // picture's pixels under the finger.
    const from = middle;
    const to = { x: middle.x + 40, y: middle.y - 24 };
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    const drag = await until(() => touchesAfter(before), (all) => all.some((r) => r.phase === "up"), "the drag's up");
    wentFromTo(drag, map.toPicture(from), map.toPicture(to));
    expect(await clipped(screen.getByTestId("eq-factor"))).toEqual([]);

    // A quick tap: its down where it landed, then its up (PR G: the page
    // sends the up 40 ms after the down, a unit test of its finger; on the
    // way to the PC both pass the router and the window worker's steps, so
    // only their order is checked here, #74 review).
    const tapped = { x: middle.x - 30, y: middle.y + 12 };
    const tapFrom = (await simEq.records()).length;
    await quickTap(area, tapped);
    const tap = await until(() => touchesAfter(tapFrom), (all) => all.some((r) => r.phase === "up"), "the tap's up");
    wentFromTo(tap, map.toPicture(tapped), map.toPicture(tapped));

    // CELÝ EQ while a finger drags (the PR I review): the whole picture
    // again, the group gone, and the finger's contact ends with its cancel
    // where it was; the drag goes no further (it would go on at 1×, its
    // points remapped).
    const held = (await simEq.records()).length;
    const start = { x: middle.x + 20, y: middle.y + 10 };
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(start.x + 5 * i, start.y);
      await frames(page, 2);
    }
    await until(() => touchesAfter(held), (all) => all.some((r) => r.phase === "update"), "the drag's moves");
    await screen.getByTestId("eq-whole").dispatchEvent("pointerdown", { pointerId: 71, pointerType: "touch", isPrimary: false, bubbles: true, cancelable: true });
    await expect(zoom).toHaveAttribute("data-shown", "false");
    await expect(zoom).toBeHidden();
    await drawnWhole(screen);
    const cancelled = await until(() => touchesAfter(held), (all) => all.some((r) => r.phase === "cancel"), "the drag's cancel");
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(start.x + 20 + 10 * i, start.y + 5 * i);
      await frames(page, 2);
    }
    await page.mouse.up();
    await page.waitForTimeout(300);
    const after = await touchesAfter(held);
    expect(after[after.length - 1].phase, JSON.stringify(after)).toBe("cancel");
    expect(after.length, JSON.stringify(after)).toBe(cancelled.length);

    // Zoomed again, the exit closes as ever; the next open starts whole.
    await pinch(area, middle, 50, 100);
    await expect(screen.getByTestId("eq-factor")).toHaveText("2,0×");
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const again = await openScreen(page, trackPath);
    await drawnWhole(again);
    const last = (await simEq.records()).length;
    await again.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(last);
    await editorOpen(trackPath, false);
  });

  test("the whole picture takes no edits: a drag there does nothing, a tap zooms to 3× about its point and sends the editor nothing; the hint says so", async ({ page }) => {
    // The owner's ruling of 2026-10-11 (#71): on the whole picture a band
    // point is about 2 mm under a finger, so one finger edits only from 2×.
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const canvas = screen.getByTestId("eq-canvas");
    const hint = screen.getByTestId("eq-hint");
    await expect(hint).toHaveAttribute("data-shown", "true");
    await expect(hint).toBeVisible();
    await expect(hint).toHaveText("Ťukni na miesto, ktoré chceš upraviť");
    expect(await clipped(hint)).toEqual([]);
    const whole = await pictureMap(canvas);
    const before = (await simEq.records()).length;

    // A drag on the whole picture: nothing reaches the editor, nothing zooms.
    const from = whole.toPage(FROM);
    const to = whole.toPage(TO);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    await page.waitForTimeout(400);
    expect(await touchesAfter(before), "the drag's touches").toEqual([]);
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "false");
    await drawnWhole(screen);

    // A tap: 3×, the tapped point in the area's middle, and still nothing
    // at the editor.
    const tapped = whole.toPage(FROM);
    await page.mouse.click(tapped.x, tapped.y);
    await expect(screen.getByTestId("eq-factor")).toHaveText("3,0×");
    // The iPad's double tap to zoom (the PR I review): a second tap right
    // after the zoom reaches nothing (checked with the tap's touches below).
    await quickTap(area, tapped, 62);
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "true");
    await expect(hint).toHaveAttribute("data-shown", "false");
    await expect(hint).toBeHidden();
    await frames(page, 2);
    const zoomed = await pictureMap(canvas);
    expect(zoomed.scale / whole.scale, "the zoom").toBeCloseTo(3, 3);
    const was = whole.toPicture(tapped);
    const now = zoomed.toPicture(await centre(area));
    expect(Math.abs(now.x - was.x) <= 1 && Math.abs(now.y - was.y) <= 1, `${JSON.stringify(now)} in the middle, tapped ${JSON.stringify(was)}`).toBe(true);
    await page.waitForTimeout(400);
    expect(await touchesAfter(before), "the tap's touches").toEqual([]);
    // Once the settle is over, a tap edits.
    await page.waitForTimeout(ZOOM_SETTLE_MS);
    const settled = (await simEq.records()).length;
    await quickTap(area, await centre(area), 63);
    const edit = await until(() => touchesAfter(settled), (all) => all.some((r) => r.phase === "up"), "the tap after the settle");
    // Its down and its up (a keep-alive update between when the up came
    // more than 50 ms after the down at the hub).
    const phases = edit.map((r) => r.phase);
    expect([phases[0], phases[phases.length - 1]], JSON.stringify(edit)).toEqual(["down", "up"]);
    expect(phases.slice(1, -1).every((p) => p === "update"), JSON.stringify(edit)).toBe(true);
    // A drag started right after a tap's zoom (the PR I re-review) goes once
    // the settle is over, its down where it landed.
    await screen.getByTestId("eq-whole").click();
    await expect(hint).toHaveAttribute("data-shown", "true");
    const again = (await pictureMap(canvas)).toPage(FROM);
    await page.mouse.click(again.x, again.y);
    await expect(screen.getByTestId("eq-factor")).toHaveText("3,0×");
    const zoomedAgain = await pictureMap(canvas);
    const heldFrom = (await simEq.records()).length;
    const start = await centre(area);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(start.x + 8 * i, start.y);
      await frames(page, 2);
    }
    await page.waitForTimeout(ZOOM_SETTLE_MS);
    await page.mouse.up();
    const held = await until(() => touchesAfter(heldFrom), (all) => all.some((r) => r.phase === "up"), "the drag after the settle");
    expect(held[0].phase, JSON.stringify(held)).toBe("down");
    expect(near(held[0], zoomedAgain.toPicture(start)), `the down ${JSON.stringify(held[0])} at ${JSON.stringify(zoomedAgain.toPicture(start))}`).toBe(true);

    // The legend says it too.
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend-overview")).toContainText("Ťukni na miesto, ktoré chceš upraviť");
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend")).toHaveCount(0);

    // CELÝ EQ: the whole picture and its hint again.
    await screen.getByTestId("eq-whole").click();
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "false");
    await expect(hint).toHaveAttribute("data-shown", "true");
    await drawnWhole(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("the zoom's bar fits a phone on its side and upright; a turned screen draws the zoomed picture for its new size", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const canvas = screen.getByTestId("eq-canvas");
    for (const size of [
      { width: 844, height: 390 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(size);
      await frames(page, 3);
      // The editor takes the turned screen's shape (PR G).
      await shaped(screen);
      await screen.getByTestId("eq-whole").evaluate((el) =>
        el.dispatchEvent(new PointerEvent("pointerdown", { pointerId: 71, pointerType: "touch", isPrimary: true, bubbles: true, cancelable: true })),
      );
      await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "false");
      // 2.5×: one finger edits from 2× on.
      await pinch(area, await centre(area), 50, 125);
      await expect(screen.getByTestId("eq-factor"), `${size.width}x${size.height}`).toHaveText("2,5×");
      await zoomBarFits(screen);
    }
    // Turned while zoomed: the picture is drawn for the new area (the
    // editor in its shape again), and a tap lands where it shows.
    await page.setViewportSize({ width: 844, height: 390 });
    await frames(page, 3);
    await shaped(screen);
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "true");
    const map = await pictureMap(canvas);
    const at = await centre(area);
    const before = (await simEq.records()).length;
    await quickTap(area, at);
    const tap = await until(() => touchesAfter(before), (all) => all.some((r) => r.phase === "up"), "the tap's up");
    wentFromTo(tap, map.toPicture(at), map.toPicture(at));
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("a turned screen sends its new area and the editor takes its shape once; a tap lands where it shows; the close puts the editor's own size back first", async ({ page }) => {
    // The page's `eq_area` messages (PR G), watched before the page opens.
    const areas: any[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_area"')) areas.push(JSON.parse(payload));
      });
    });
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const canvas = screen.getByTestId("eq-canvas");
    const opened = await pictureSize(canvas);
    const before = (await simEq.records()).length;
    expect(areas, "no change yet").toEqual([]);
    // An upright phone: the area goes once it rested, and the editor takes
    // its aspect.
    await page.setViewportSize({ width: 390, height: 844 });
    await until(async () => areas.length, (n) => n >= 1, "the new area sent");
    await shaped(screen);
    const box = (await screen.getByTestId("eq-area").boundingBox())!;
    const sent = areas[areas.length - 1];
    expect(Math.abs(sent.w - box.width) < 1 && Math.abs(sent.h - box.height) < 1, `${JSON.stringify(sent)} ${JSON.stringify(box)}`).toBe(true);
    const upright = await pictureSize(canvas);
    expect(upright.height, JSON.stringify(upright)).toBeGreaterThan(upright.width);
    expect(opened.width, JSON.stringify(opened)).toBeGreaterThan(opened.height);
    const resizes = (await simEq.records()).slice(before).filter((r) => r.op === "resize");
    expect(resizes.map((r) => [r.w, r.h])).toEqual([[upright.width, upright.height]]);
    // Zoomed in on the middle, a tap where the picture shows reaches it
    // there.
    const area = screen.getByTestId("eq-area");
    const at = await centre(area);
    const middle = (await pictureMap(canvas)).toPicture(at);
    const map = await zoomIn(screen, middle);
    const tapFrom = (await simEq.records()).length;
    await quickTap(area, at);
    const tap = await until(() => touchesAfter(tapFrom), (all) => all.some((r) => r.phase === "up"), "the tap's up");
    wentFromTo(tap, map.toPicture(at), map.toPicture(at));
    // The close: the editor's own size back, then the guard's tap.
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("an area that changes size without a window resize (the bar's height) sends its new area, and the editor takes its shape", async ({ page }) => {
    // #74 review (M9): the page read the area's box only on a window
    // `resize` and a finger's down, so a layout change of its own (a safe
    // area, iOS laying out after its `resize`) kept a stale box: no
    // `eq_area`, and the view mapped against the old box until the next
    // touch. The bar's height stands in for such a change.
    const areas: any[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_area"')) areas.push(JSON.parse(payload));
      });
    });
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const was = (await area.boundingBox())!;
    expect(areas, "no change yet").toEqual([]);
    try {
      await screen.evaluate((el: HTMLElement) => el.style.setProperty("--bar-h", "160px"));
      await until(async () => areas.length, (n) => n >= 1, "the new area sent");
      const box = (await area.boundingBox())!;
      expect(box.height, "the area is lower").toBeLessThan(was.height - 100);
      const sent = areas[areas.length - 1];
      expect(Math.abs(sent.w - box.width) < 1 && Math.abs(sent.h - box.height) < 1, `${JSON.stringify(sent)} ${JSON.stringify(box)}`).toBe(true);
      await shaped(screen);
    } finally {
      await screen.evaluate((el: HTMLElement) => el.style.removeProperty("--bar-h"));
    }
    // Back: sent again, and the editor in that shape again before the close.
    await until(async () => areas.length, (n) => n >= 2, "the area sent back");
    await shaped(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("the cards are listed again when Live comes back, and open", async ({ page }) => {
    // What the page's socket carries, watched before the page opens: its
    // `eq_list` asks, and the band instance's states the hub sends it. A
    // new list of the same cards changes nothing on the page (the cards'
    // memo keeps an equal list), so the page's asks prove a list again.
    let lists = 0;
    const band: boolean[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_list"')) lists += 1;
      });
      ws.on("framereceived", ({ payload }) => {
        if (typeof payload !== "string") return;
        let msg: any;
        try {
          msg = JSON.parse(payload);
        } catch {
          return;
        }
        if (msg.type === "instance" && msg.name === "band") band.push(msg.online);
      });
    });
    await openSurface(page);
    const { detail, trackPath } = await hand2Cards(page);
    expect(lists, "the detail's list").toBeGreaterThanOrEqual(1);
    const asked = lists;
    const states = band.length;
    await harness("/host/band/restart");
    // The page hears its Live go offline, then come back (the hub sends
    // both, in order).
    await until(
      async () => band.slice(states),
      (seen) => seen.includes(false) && seen.lastIndexOf(true) > seen.indexOf(false),
      "the band offline, then online again",
      10_000
    );
    await until(hubStatus, (s) => s.instances[0].online, "the band's Live back", 10_000);
    await until(async () => lists, (n) => n > asked, "the cards listed again", 10_000);
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

test.describe("The Pro-Q 4 screen on a slow link", () => {
  // #71 PR J (acceptance points 1, 3 and 5): the hub's bytes to the page
  // are slowed below what the frames would take; the picture keeps up with a
  // drag at 3×, no pong waits behind a backlog of frames, and a finger on a
  // picture that does not show the page's last input yet never acts.
  /** The page's downlink (bytes/s): ~20 frames a second of the sim's ~21 KB picture would need ~420 KB/s. */
  const DOWN_RATE = 150_000;
  /** The bounds of the latency on that link, until the real iPad's numbers set them (#71). */
  const LAT_MEDIAN_MAX = 400;
  const LAT_P95_MAX = 800;

  test("a drag at 3× is numbered and measured, no pong waits behind frames, and a finger on a picture behind its last input is refused", async ({ page }) => {
    const sent: any[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"eq_input"')) sent.push(JSON.parse(payload));
      });
    });
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const stale = screen.getByTestId("eq-stale");
    await expect(stale).toHaveAttribute("data-shown", "false");
    await zoomIn(screen, FROM);
    const since = Date.now();
    await impair.downRate(DOWN_RATE);
    try {
      // A drag at 3×: every input numbered from 1, the down with the input
      // its picture showed (none yet).
      const before = (await simEq.records()).length;
      const from = await centre(area);
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      for (let i = 1; i <= 3 * DRAG_STEPS; i++) {
        await page.mouse.move(from.x + 3 * i, from.y);
        await frames(page, 2);
      }
      await page.mouse.up();
      // At once: a finger on the picture, which does not show the up yet.
      await quickTap(area, from, 71);
      await expect(stale).toHaveAttribute("data-shown", "true");
      await expect(stale).toHaveText("Obraz ešte dobieha, polož prst znova");
      const drag = await until(() => touchesAfter(before), (all) => all.some((r) => r.phase === "up"), "the drag's up");
      expect(drag[0].phase).toBe("down");
      expect(sent.map((m) => m.n), JSON.stringify(sent)).toEqual(sent.map((_, i) => i + 1));
      expect([sent[0].touch, sent[0].seen]).toEqual(["down", 0]);
      expect(sent.slice(1).every((m) => m.seen === undefined), JSON.stringify(sent.slice(0, 3))).toBe(true);
      // The refused finger reached nothing: the drag's up is the last touch,
      // and the recorder says why.
      await page.waitForTimeout(500);
      const touches = await touchesAfter(before);
      expect(touches[touches.length - 1].phase, JSON.stringify(touches)).toBe("up");
      expect(sent[sent.length - 1].touch, "nothing sent for it").toBe("up");
      const refused = await until(
        async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_touch" && e.what === "stale" && e.t >= since),
        (all) => all.length >= 1,
        "the refused finger in the recorder",
        15_000,
      );
      expect(refused[0].p).toBe(71);
      // The note goes, and a finger now edits.
      await expect(stale).toHaveAttribute("data-shown", "false", { timeout: 3_000 });
      const tapFrom = (await simEq.records()).length;
      await quickTap(area, from, 72);
      const tap = await until(() => touchesAfter(tapFrom), (all) => all.some((r) => r.phase === "up"), "the tap once caught up");
      expect(tap.map((r) => r.phase)).toEqual(["down", "up"]);
      // The picture's latency, from the page's eq_lat records: every input a
      // picture was the first to show, from its send to the draw.
      const lat = await until(
        async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_lat" && e.t >= since),
        (all) => all.flatMap((e: any) => e.ms).length >= 5,
        "the latency samples",
        20_000,
      );
      const ms = lat.flatMap((e: any) => e.ms).sort((a: number, b: number) => a - b);
      const median = ms[Math.floor(ms.length / 2)];
      const p95 = ms[Math.min(ms.length - 1, Math.floor(ms.length * 0.95))];
      console.log(`eq latency on a ${DOWN_RATE} B/s downlink: n=${ms.length} median=${median} ms p95=${p95} ms`);
      expect(median, JSON.stringify(ms)).toBeLessThan(LAT_MEDIAN_MAX);
      expect(p95, JSON.stringify(ms)).toBeLessThan(LAT_P95_MAX);
      // No pong waited behind frames: each ping's measured round trip is of
      // the ping before it or the one before that.
      const pings = (await hubEvents()).filter((r: any) => r.ev === "ping" && r.ts >= since && r.rtt_n != null);
      expect(pings.length, "the page pinged").toBeGreaterThan(10);
      const lag = Math.max(...pings.map((p: any) => p.n - p.rtt_n));
      expect(lag, JSON.stringify(pings.map((p: any) => p.n - p.rtt_n))).toBeLessThanOrEqual(2);
    } finally {
      await impair.downRate(0);
    }
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });
});
