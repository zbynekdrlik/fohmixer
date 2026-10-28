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
  resetLayout,
  setIdleAfter,
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
const ON_TRACK = "Pro-Q 4 · on track";
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

/**
 * A first finger on the zoomed picture is taken now (#71 PR J, PR Q): the
 * picture shows every input the page sent (`data-caught-up`), and the page's
 * whole rule for a first finger holds (`data-takes-finger`: also no quick
 * tap's up still to go and no dropout, `behave::eq::takes_finger`, the rule
 * its pointer down decides with). A first finger before that is refused, so
 * a test waits for it before any tap or drag that must reach the editor.
 */
async function caughtUp(screen: Locator) {
  const canvas = screen.getByTestId("eq-canvas");
  await expect(canvas).toHaveAttribute("data-caught-up", "true");
  await expect(canvas).toHaveAttribute("data-takes-finger", "true");
}

/** How long after a tap's zoom a first finger reaches nothing (ms, `behave::eq::ZOOM_SETTLE_MS`). */
const ZOOM_SETTLE_MS = 500;

/**
 * A real tap on the whole picture at its pixel `at` (the owner's ruling of
 * 2026-10-11): the view zooms to 3× with that point in the area's middle,
 * and the editor gets nothing. The picture's map once the zoom is drawn,
 * its settle is over and a first finger is taken (a finger then edits).
 */
async function zoomIn(screen: Locator, at: At) {
  const canvas = screen.getByTestId("eq-canvas");
  const point = (await pictureMap(canvas)).toPage(at);
  await screen.page().mouse.click(point.x, point.y);
  await expect(screen.getByTestId("eq-factor")).toHaveText("3.0×");
  await expect(screen.getByTestId("eq-hint")).toHaveAttribute("data-shown", "false");
  await frames(screen.page(), 2);
  await screen.page().waitForTimeout(ZOOM_SETTLE_MS + 50);
  await caughtUp(screen);
  return pictureMap(canvas);
}

/**
 * Where a step's touches start: the backend's record count, and the
 * runner's clock then (the page stamps its flight recorder's events on the
 * same clock, #43), so a touch wait that fails can say what the page
 * recorded since.
 */
type TouchMark = { at: number; since: number };

/** A mark of the backend's records now (`touchesAfter`, `touchesUntil`). */
async function touchMark(): Promise<TouchMark> {
  const since = Date.now();
  return { at: (await simEq.records()).length, since };
}

/** The backend's touches after `mark`. */
async function touchesAfter(mark: TouchMark): Promise<any[]> {
  return (await simEq.records()).slice(mark.at).filter((r) => r.op === "touch");
}

/**
 * What the page recorded since `since` that keeps a finger from the editor
 * (#71 PR Q): its refused first fingers (`eq_touch` `stale`: the rule of
 * `data-takes-finger` said no; `uncovered`: off the part drawn last) and
 * the link's dropouts that ended after it.
 */
async function refusalsSince(since: number): Promise<string> {
  try {
    const found = pageEvents(await hubEvents()).filter(
      (e: any) => (e.ev === "eq_touch" && (e.what === "stale" || e.what === "uncovered") && e.t >= since) || (e.ev === "dropout" && e.t + e.ms >= since),
    );
    return JSON.stringify(found);
  } catch (error) {
    return `(the event log could not be read: ${(error as Error).message})`;
  }
}

/**
 * Waits until the backend's touches after `mark` hold what `ok` wants. One
 * that never comes fails with what the page recorded since the mark: its
 * refusals and the link's dropouts (#71 PR Q: run 38176426488's refused tap
 * failed with only "last: []"). The page uploads such an event within about
 * 2 s (`UPLOAD_MS`), well inside a wait's 8 s. Nothing is tried again: a
 * refused touch still fails its test.
 */
async function touchesUntil(mark: TouchMark, ok: (all: any[]) => boolean, what: string, timeout?: number): Promise<any[]> {
  try {
    return await until(() => touchesAfter(mark), ok, what, timeout);
  } catch (error) {
    throw new Error(`${(error as Error).message}; the page's refusals and dropouts since the step began: ${await refusalsSince(mark.since)}`);
  }
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

/** The bar not overflowing, the parts `inside` each within it and the screen, and the parts `whole` with their texts whole. */
async function barFits(screen: Locator, inside: string[], whole: string[]) {
  const bar = screen.getByTestId("eq-bar");
  const barBox = (await bar.boundingBox())!;
  const width = screen.page().viewportSize()!.width;
  expect(await bar.evaluate((el) => el.scrollWidth - el.clientWidth), "the bar's overflow").toBeLessThanOrEqual(0);
  for (const id of inside) {
    const box = (await screen.getByTestId(id).boundingBox())!;
    const within = box.x >= barBox.x - 0.5 && box.x + box.width <= Math.min(barBox.x + barBox.width, width) + 0.5 && box.y >= barBox.y - 0.5 && box.y + box.height <= barBox.y + barBox.height + 0.5;
    expect(within, `${id} ${JSON.stringify(box)} inside the bar ${JSON.stringify(barBox)}`).toBe(true);
  }
  for (const id of whole) {
    expect(await clipped(screen.getByTestId(id)), id).toEqual([]);
  }
}

/** The bar's zoom group shown, the overview in the picture's shape, and every part of the bar inside it and the screen, its texts whole (#71 PR P2b: ↶ UNDO too). */
async function zoomBarFits(screen: Locator) {
  await expect(screen.getByTestId("eq-zoom")).toBeVisible();
  await expect.poll(() => overviewGap(screen), { message: "the overview in the picture's shape" }).toBeLessThan(0.02);
  await barFits(screen, ["eq-exit", "eq-overview", "eq-factor", "eq-whole", "eq-undo", "eq-help"], ["eq-exit", "eq-factor", "eq-whole", "eq-undo"]);
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
  // Press-and-hold on again (#71 PR O), then the window back.
  ["press_and_hold", null, null, null],
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
  expect(close[close.length - 2].quiet, "press-and-hold on again").toBe(false);
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

/**
 * The page's numbered inputs of its editor (`eq_input`) and its undos
 * (`eq_undo`, #71 PR P2b: one numbering), in send order, and the hub's
 * answers to the undos, read off the page's socket: watched before the page
 * opens.
 */
function watchUndo(page: Page): { sent: any[]; answers: any[] } {
  const sent: any[] = [];
  const answers: any[] = [];
  page.on("websocket", (ws) => {
    ws.on("framesent", ({ payload }) => {
      if (typeof payload === "string" && (payload.includes('"type":"eq_input"') || payload.includes('"type":"eq_undo"'))) sent.push(JSON.parse(payload));
    });
    ws.on("framereceived", ({ payload }) => {
      if (typeof payload === "string" && payload.includes('"type":"eq_undo"')) answers.push(JSON.parse(payload));
    });
  });
  return { sent, answers };
}

/** The page's undos among `watchUndo`'s sends. */
function undoAsks(sent: any[]): any[] {
  return sent.filter((m) => m.type === "eq_undo");
}

/** A real press of ↶ UNDO at its centre (`page.mouse`: a locator's click would wait out its `aria-disabled`). */
async function pressUndo(screen: Locator) {
  const at = await centre(screen.getByTestId("eq-undo"));
  await screen.page().mouse.click(at.x, at.y);
}

/** The hub's answer to the next undo after the first `before` of `answers`. */
async function answerAfter(answers: any[], before: number): Promise<any> {
  return (await until(async () => answers.slice(before), (a) => a.length >= 1, "the hub's answer to the undo"))[0];
}

/**
 * How long to wait past the button's 700 ms before a press that must be
 * tapped: the hub counts its own 700 ms from its tap on Pro-Q's ↶, which
 * comes after the press (a grab, the read, the tap), and refuses a sooner
 * undo (`too soon after the last undo`).
 */
const HUB_APART_MARGIN = 400;

/**
 * Pro-Q's undo history emptied (#71 PR P2b): the simulated Pro-Q keeps it
 * through the hub's whole run, and earlier tests' drags left changes in it,
 * so ↶ UNDO is pressed until the hub answers `nothing`.
 */
async function undoAll(screen: Locator, answers: any[]) {
  const undo = screen.getByTestId("eq-undo");
  for (let i = 0; i < 12; i++) {
    await expect(undo).toHaveAttribute("aria-disabled", "false");
    await screen.page().waitForTimeout(HUB_APART_MARGIN);
    const before = answers.length;
    await pressUndo(screen);
    if ((await answerAfter(answers, before)).result === "nothing") return;
  }
  throw new Error(`Pro-Q's undo history did not empty: ${JSON.stringify(answers)}`);
}

/**
 * Records each state of ↶ UNDO's note as it changes (`window.undoNotes`,
 * `<data-shown>|<text>`): it shows for 1.6 s, which a poll on a slow runner
 * could miss.
 */
async function recordUndoNotes(screen: Locator) {
  await screen.getByTestId("eq-undo-note").evaluate((note) => {
    const seen: string[] = [];
    (window as any).undoNotes = seen;
    const look = () => {
      const now = `${note.getAttribute("data-shown")}|${note.textContent ?? ""}`;
      if (seen[seen.length - 1] !== now) seen.push(now);
    };
    look();
    new MutationObserver(look).observe(note, { attributes: true, childList: true, subtree: true, characterData: true });
  });
}

/** The note's states `recordUndoNotes` saw. */
async function undoNotes(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).undoNotes as string[]);
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
      await expect(open).toHaveText("OPEN EQ FULL SCREEN");
      await expect(open).toHaveAttribute("aria-disabled", "false");
      await cardFits(c);
    }
    // Back to the mix and into a band strip with no device (the return
    // `B-Main repro #` on the default page; Hand1 # is a master strip on the
    // OTHERS page).
    await detail.getByTestId("detail-exit").click();
    await expect(page.getByTestId("detail")).toHaveCount(0);
    const other = await openDetail(page, "B-Main repro #");
    await expect(other.getByTestId("eq-note")).toHaveText("No Pro-Q 4 on this track.");
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
    const cleared = await touchMark();
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
    // Taken, press-and-hold off for it (#71 PR O: a still finger never
    // becomes a right click), then resized.
    const opened = (await simEq.records()).slice(0, 3);
    expect(steps(opened)).toEqual([
      ["take", null, null, null],
      ["press_and_hold", null, null, null],
      ["resize", null, picture.width, picture.height],
    ]);
    expect(opened[1].quiet).toBe(true);
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
    expect(await touchesAfter(cleared), "the zoom's tap reaches no editor").toEqual([]);
    const from = map.toPage(FROM);
    const to = map.toPage(TO);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    const drag = await touchesUntil(cleared, (all) => all.some((r) => r.phase === "up"), "the drag's up at the simulated backend");
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
    // At 3× the page asked for the part in sight (PR L), once the view
    // rested: its `view`, of the picture's size it drew.
    const views = eq.filter((r: any) => r.what === "view");
    expect(views.length, JSON.stringify(views)).toBeGreaterThanOrEqual(1);
    expect(views[views.length - 1].part.picture).toEqual([picture.width, picture.height]);
    expect(eq.filter((r: any) => r.what !== "view").map((r: any) => r.what)).toEqual(["take", "open", "resize", "opened", "touch", "touch", "close", "closed"]);
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
      [onTrack, "Another engineer is editing it"],
      [inRack, "Pro-Q 4 is in use by another engineer"],
    ] as const) {
      await expect(c).toHaveAttribute("data-locked", "true");
      const lock = c.getByTestId("eq-card-lock");
      await expect(lock).toBeVisible();
      const text = (await lock.textContent())!;
      expect(text.startsWith("LOCKED"), text).toBe(true);
      expect(times.map((hm) => text.endsWith(`${line} (since ${hm})`)), text).toContain(true);
      expect(await clipped(lock)).toEqual([]);
      const open = c.getByTestId("eq-open");
      await expect(open).toHaveText("LOCKED");
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
      await expect(c.getByTestId("eq-open")).toHaveText("OPEN EQ FULL SCREEN");
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

  test("an open the hub refuses says why under its card in its own words, never the hub's reason id", async ({ page }) => {
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
    await expect.poll(() => notesSeen(page), { message: "the card's note" }).toContain("still closing, try again");
    // The refused screen went; the first close ends with its guard, and the
    // note goes with it (the editor can open again).
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(before);
    await editorOpen(trackPath, false);
    await expect(onTrack.getByTestId("eq-card-note")).toHaveCount(0);
    // The only note it showed: the page's own words (never "Did not open: closing").
    expect(await notesSeen(page)).toEqual(["still closing, try again"]);
  });

  test("a page going hidden lifts its finger: the PC's contact is cancelled where it was", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const before = await touchMark();
    const map = await zoomIn(screen, FROM);
    const at = map.toPage(FROM);
    const touches = () => touchesAfter(before);
    await page.mouse.move(at.x, at.y);
    await page.mouse.down();
    await touchesUntil(before, (all) => all.some((r) => r.phase === "down"), "the finger's down");
    // The finger rests and the page goes hidden (it still pings, so the
    // hub's 2 s silence would never end the contact).
    await setHidden(page, true);
    const ended = await touchesUntil(before, (all) => all.some((r) => r.phase === "cancel"), "the hidden page's cancel", 1_500);
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

  test("two fingers zoom the page's picture and send the editor nothing; a finger then reaches it where it shows; a quick tap goes down, then up; FULL EQ and a new open show it whole", async ({ page }) => {
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
    const before = await touchMark();

    // Two fingers 100 px apart about the area's middle spread to 250: 2.5×,
    // and the picture point under their midpoint stays under it.
    await pinch(area, middle, 50, 125);
    await expect(zoom).toHaveAttribute("data-shown", "true");
    await expect(screen.getByTestId("eq-factor")).toHaveText("2.5×");
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
    await caughtUp(screen);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    const drag = await touchesUntil(before, (all) => all.some((r) => r.phase === "up"), "the drag's up");
    wentFromTo(drag, map.toPicture(from), map.toPicture(to));
    expect(await clipped(screen.getByTestId("eq-factor"))).toEqual([]);

    // A quick tap: its down where it landed, then its up (PR G: the page
    // sends the up 40 ms after the down, a unit test of its finger; on the
    // way to the PC both pass the router and the window worker's steps, so
    // only their order is checked here, #74 review).
    const tapped = { x: middle.x - 30, y: middle.y + 12 };
    const tapFrom = await touchMark();
    await caughtUp(screen);
    await quickTap(area, tapped);
    const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap's up");
    wentFromTo(tap, map.toPicture(tapped), map.toPicture(tapped));

    // FULL EQ while a finger drags (the PR I review): the whole picture
    // again, the group gone, and the finger's contact ends with its cancel
    // where it was; the drag goes no further (it would go on at 1×, its
    // points remapped).
    const held = await touchMark();
    const start = { x: middle.x + 20, y: middle.y + 10 };
    await caughtUp(screen);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(start.x + 5 * i, start.y);
      await frames(page, 2);
    }
    await touchesUntil(held, (all) => all.some((r) => r.phase === "update"), "the drag's moves");
    await screen.getByTestId("eq-whole").dispatchEvent("pointerdown", { pointerId: 71, pointerType: "touch", isPrimary: false, bubbles: true, cancelable: true });
    await expect(zoom).toHaveAttribute("data-shown", "false");
    await expect(zoom).toBeHidden();
    await drawnWhole(screen);
    const cancelled = await touchesUntil(held, (all) => all.some((r) => r.phase === "cancel"), "the drag's cancel");
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
    await expect(screen.getByTestId("eq-factor")).toHaveText("2.0×");
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

  test("the canvas says when a first finger is taken: not while a quick tap's up is still to go, nor while the link drops out", async ({ page }) => {
    // #71 PR Q (run 38176426488): the page refuses a first finger on the
    // zoomed picture unless its picture shows every input it sent, no quick
    // tap's up is still to go and the link does not drop out
    // (`behave::eq::takes_finger`). The canvas says that rule's answer as
    // `data-takes-finger`, which `caughtUp` waits for.
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    await zoomIn(screen, FROM);

    // A quick tap: its down goes at once, its up on a frame 40 ms later (PR
    // G). Read in the page: taken before the tap; not taken a macrotask
    // after it (the attribute is written in a microtask); taken again only
    // once its up went (the page's own sends, on its clock) and the picture
    // showed it.
    const tapFrom = await touchMark();
    const seen = await area.evaluate(
      async (el, at) => {
        const canvas = el.querySelector('[data-testid="eq-canvas"]')!;
        const read = () => canvas.getAttribute("data-takes-finger");
        const changes: { t: number; v: string | null }[] = [];
        const observer = new MutationObserver((records) => {
          const t = performance.now();
          // Each change's new value: the next change's old one, the last one's now.
          records.forEach((_, i) => changes.push({ t, v: i + 1 < records.length ? records[i + 1].oldValue : read() }));
        });
        observer.observe(canvas, { attributes: true, attributeFilter: ["data-takes-finger"], attributeOldValue: true });
        const sends: { t: number; touch: string }[] = [];
        const send = WebSocket.prototype.send;
        WebSocket.prototype.send = function (this: WebSocket, data: any) {
          if (typeof data === "string" && data.includes('"type":"eq_input"')) sends.push({ t: performance.now(), touch: JSON.parse(data).touch });
          return send.call(this, data);
        };
        try {
          const before = read();
          for (const type of ["pointerdown", "pointerup"]) {
            el.dispatchEvent(
              new PointerEvent(type, { pointerId: 64, pointerType: "touch", isPrimary: true, clientX: at.x, clientY: at.y, bubbles: true, cancelable: true }),
            );
          }
          await new Promise((r) => setTimeout(r, 0));
          const after = read();
          const end = performance.now() + 5_000;
          while (read() !== "true" && performance.now() < end) await new Promise((r) => setTimeout(r, 10));
          return { before, after, last: read(), changes, sends };
        } finally {
          observer.disconnect();
          WebSocket.prototype.send = send;
        }
      },
      await centre(area),
    );
    const story = JSON.stringify(seen);
    expect(seen.before, `taken before the tap: ${story}`).toBe("true");
    expect(seen.after, `not taken right after the tap: ${story}`).toBe("false");
    expect(seen.last, `taken again: ${story}`).toBe("true");
    expect(seen.sends.map((s) => s.touch), story).toEqual(["down", "up"]);
    const taken = seen.changes.filter((c) => c.v === "true");
    expect(taken.length, story).toBeGreaterThanOrEqual(1);
    expect(taken[0].t, `taken again only after the tap's up went: ${story}`).toBeGreaterThanOrEqual(seen.sends[1].t);
    // The editor got the tap: its down, then its up (a keep-alive update
    // between when the up came more than 50 ms after the down at the hub).
    const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap's up");
    const phases = tap.map((r) => r.phase);
    expect([phases[0], phases[phases.length - 1]], JSON.stringify(tap)).toEqual(["down", "up"]);
    expect(phases.slice(1, -1).every((p) => p === "update"), JSON.stringify(tap)).toBe(true);

    // The link stalls 800 ms (both directions held): a dropout (300 ms of
    // silence while a pong is due) lasts a few hundred ms, the counter red
    // (counter.spec.ts). The picture shows every input and no tap's up is to
    // go, so the canvas says no exactly while the counter is red: sampled
    // each animation frame in the page (a Playwright poll can miss a red
    // phase of a few hundred ms).
    await caughtUp(screen);
    await expect(page.getByTestId("dropouts")).toHaveAttribute("data-active", "false");
    await page.evaluate(() => {
      const w = window as any;
      const counter = document.querySelector('[data-testid="dropouts"]')!;
      const canvas = document.querySelector('[data-testid="eq-canvas"]')!;
      const samples: { t: number; active: string | null; takes: string | null; caught: string | null }[] = [];
      w.takenSamples = samples;
      w.takenStop = false;
      let last = "";
      const step = () => {
        const sample = {
          t: performance.now(),
          active: counter.getAttribute("data-active"),
          takes: canvas.getAttribute("data-takes-finger"),
          caught: canvas.getAttribute("data-caught-up"),
        };
        const key = `${sample.active}/${sample.takes}/${sample.caught}`;
        if (key !== last) {
          last = key;
          samples.push(sample);
        }
        if (!w.takenStop) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
    const samples = () => page.evaluate(() => (window as any).takenSamples as { t: number; active: string; takes: string; caught: string }[]);
    await impair.stall(800);
    await expect.poll(async () => (await samples()).some((s) => s.active === "true"), { message: "the dropout seen", timeout: 3_000 }).toBe(true);
    await expect
      .poll(
        async () => {
          const all = await samples();
          return all[all.length - 1].active;
        },
        { message: "the dropout over", timeout: 3_000 },
      )
      .toBe("false");
    await page.evaluate(() => {
      (window as any).takenStop = true;
    });
    const stalled = await samples();
    const told = JSON.stringify(stalled);
    for (const s of stalled) {
      expect(s.caught, told).toBe("true");
      expect(s.takes, told).toBe(s.active === "true" ? "false" : "true");
    }
    await caughtUp(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
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
    await expect(hint).toHaveText("Tap where you want to edit");
    expect(await clipped(hint)).toEqual([]);
    const whole = await pictureMap(canvas);
    const before = await touchMark();

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
    await expect(screen.getByTestId("eq-factor")).toHaveText("3.0×");
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
    const settled = await touchMark();
    await caughtUp(screen);
    await quickTap(area, await centre(area), 63);
    const edit = await touchesUntil(settled, (all) => all.some((r) => r.phase === "up"), "the tap after the settle");
    // Its down and its up (a keep-alive update between when the up came
    // more than 50 ms after the down at the hub).
    const phases = edit.map((r) => r.phase);
    expect([phases[0], phases[phases.length - 1]], JSON.stringify(edit)).toEqual(["down", "up"]);
    expect(phases.slice(1, -1).every((p) => p === "update"), JSON.stringify(edit)).toBe(true);
    // A drag started right after a tap's zoom (the PR I re-review) goes once
    // the settle is over, its down where it landed.
    await caughtUp(screen);
    await screen.getByTestId("eq-whole").click();
    await expect(hint).toHaveAttribute("data-shown", "true");
    const again = (await pictureMap(canvas)).toPage(FROM);
    await page.mouse.click(again.x, again.y);
    await expect(screen.getByTestId("eq-factor")).toHaveText("3.0×");
    const zoomedAgain = await pictureMap(canvas);
    const heldFrom = await touchMark();
    const start = await centre(area);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(start.x + 8 * i, start.y);
      await frames(page, 2);
    }
    await page.waitForTimeout(ZOOM_SETTLE_MS);
    await page.mouse.up();
    const held = await touchesUntil(heldFrom, (all) => all.some((r) => r.phase === "up"), "the drag after the settle");
    expect(held[0].phase, JSON.stringify(held)).toBe("down");
    expect(near(held[0], zoomedAgain.toPicture(start)), `the down ${JSON.stringify(held[0])} at ${JSON.stringify(zoomedAgain.toPicture(start))}`).toBe(true);

    // The legend says it too.
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend-overview")).toContainText("Tap where you want to edit");
    await screen.getByTestId("eq-help").click();
    await expect(screen.getByTestId("eq-legend")).toHaveCount(0);

    // FULL EQ: the whole picture and its hint again.
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
      await expect(screen.getByTestId("eq-factor"), `${size.width}x${size.height}`).toHaveText("2.5×");
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
    const before = await touchMark();
    await caughtUp(screen);
    await quickTap(area, at);
    const tap = await touchesUntil(before, (all) => all.some((r) => r.phase === "up"), "the tap's up");
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
    const tapFrom = await touchMark();
    await quickTap(area, at);
    const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap's up");
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

test.describe("The Pro-Q 4 screen's part in sight", () => {
  // #71 PR L: at 2× and more the hub sends only the part of the picture in
  // sight, with a margin; the page draws it at its corner, and the picture
  // under the fingers stays the whole picture's.
  test("at 3× the frames are the part in sight, smaller; the whole picture again at 1×", async ({ page }) => {
    const frames_: { x: number; y: number; width: number; height: number; bytes: number }[] = [];
    page.on("websocket", (ws) => {
      ws.on("framereceived", ({ payload }) => {
        if (typeof payload === "string" || payload.length < 28) return;
        frames_.push({
          x: payload.readUInt32BE(12),
          y: payload.readUInt32BE(16),
          width: payload.readUInt32BE(20),
          height: payload.readUInt32BE(24),
          bytes: payload.length,
        });
      });
    });
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const canvas = screen.getByTestId("eq-canvas");
    // The editor in the screen's shape (PR G): the picture's size.
    const size = await pictureSize(canvas);
    const start = frames_.length;
    const whole = await until(
      async () => frames_.slice(start).filter((f) => f.x === 0 && f.y === 0 && f.width === size.width),
      (found) => found.length >= 2,
      "whole frames",
    );
    expect([whole[0].width, whole[0].height]).toEqual([size.width, size.height]);
    // Zoomed to 3× on the picture's middle: once the view rested, parts.
    const map = await zoomIn(screen, { x: size.width / 2, y: size.height / 2 });
    const from = frames_.length;
    const parts = await until(
      async () => frames_.slice(from).filter((f) => f.x > 0 && f.y > 0),
      (found) => found.length >= 2,
      "frames of the part in sight",
    );
    expect([parts[0].width, parts[0].height], "the whole picture's size in each").toEqual([size.width, size.height]);
    const wholeBytes = Math.min(...whole.map((f) => f.bytes));
    expect(Math.max(...parts.map((f) => f.bytes)), JSON.stringify({ whole: whole.map((f) => f.bytes), parts: parts.map((f) => f.bytes) })).toBeLessThan(wholeBytes);
    // The canvas still draws the whole picture, the part at its corner
    // inside it: a tap lands where it shows.
    await expect(canvas).toHaveAttribute("data-width", String(size.width));
    const part = await until(
      async () => (await canvas.getAttribute("data-part"))!.split(",").map(Number),
      ([x, y]) => x > 0 && y > 0,
      "a part drawn",
    );
    expect(part[0] + part[2] <= size.width && part[1] + part[3] <= size.height, JSON.stringify(part)).toBe(true);
    expect(part[2] < size.width && part[3] < size.height, JSON.stringify(part)).toBe(true);
    // The part drawn is the frames' (its corner, x before y: the PR L
    // re-review) and holds the point the tap zoomed on.
    const last = frames_.filter((f) => f.x > 0 && f.y > 0).pop()!;
    expect([part[0], part[1]], JSON.stringify({ part, last })).toEqual([last.x, last.y]);
    const middle = { x: size.width / 2, y: size.height / 2 };
    expect(part[0] <= middle.x && middle.x <= part[0] + part[2] && part[1] <= middle.y && middle.y <= part[1] + part[3], JSON.stringify(part)).toBe(true);
    const area = screen.getByTestId("eq-area");
    const at = await centre(area);
    const tapFrom = await touchMark();
    await caughtUp(screen);
    await quickTap(area, at, 91);
    const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap on the part");
    wentFromTo(tap, map.toPicture(at), map.toPicture(at));
    // FULL EQ: the whole picture again.
    await caughtUp(screen);
    await screen.getByTestId("eq-whole").click();
    const back = frames_.length;
    await until(
      async () => frames_.slice(back).filter((f) => f.x === 0 && f.y === 0),
      (found) => found.length >= 2,
      "whole frames again",
    );
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("after a pan past the margin a finger on the strip not drawn yet waits; one on the part drawn reaches the editor, and one on the strip once its part landed", async ({ page }) => {
    // The PR L re-review: only a finger landing on old pixels waits (the
    // page's `lands_on`), never one on the part drawn last.
    const since = Date.now();
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const canvas = screen.getByTestId("eq-canvas");
    const area = screen.getByTestId("eq-area");
    const stale = screen.getByTestId("eq-stale");
    const size = await pictureSize(canvas);
    const map = await zoomIn(screen, { x: size.width / 2, y: size.height / 2 });
    const drawnPart = async () => (await canvas.getAttribute("data-part"))!.split(",").map(Number);
    const old = await until(drawnPart, ([x, y]) => x > 0 && y > 0, "the part in sight drawn");
    await caughtUp(screen);
    // Two fingers move the view half the area to the right (a pan at 3×):
    // the picture slides left by that many page px.
    const box = (await area.boundingBox())!;
    const shift = Math.round(box.width / 2);
    const mid = { x: Math.round(box.x + box.width * 0.75), y: Math.round(box.y + box.height / 2) };
    const panned = (p: At) => {
      const at = map.toPage(p);
      return { x: at.x - shift, y: at.y };
    };
    // On the strip the pan brought into sight (beyond the part drawn), and
    // on the part drawn (current pixels), at the picture's middle height.
    const right = old[0] + old[2];
    const strip = panned({ x: right + 40, y: size.height / 2 });
    const onPart = panned({ x: right - 60, y: size.height / 2 });
    expect(strip.x, JSON.stringify({ strip, box })).toBeLessThan(box.x + box.width - 20);
    const before = await touchMark();
    await area.evaluate(
      async (el, { mid, half, shift, taps }) => {
        const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));
        const fire = (type: string, id: number, x: number, y: number, primary: boolean) =>
          el.dispatchEvent(
            new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: primary, clientX: x, clientY: y, bubbles: true, cancelable: true }),
          );
        fire("pointerdown", 41, mid.x - half, mid.y, true);
        fire("pointerdown", 42, mid.x + half, mid.y, false);
        await frame();
        await frame();
        const steps = 8;
        for (let i = 1; i <= steps; i++) {
          const dx = Math.round((shift * i) / steps);
          fire("pointermove", 41, mid.x - half - dx, mid.y, true);
          fire("pointermove", 42, mid.x + half - dx, mid.y, false);
          await frame();
        }
        await frame();
        fire("pointerup", 41, mid.x - half - shift, mid.y, true);
        fire("pointerup", 42, mid.x + half - shift, mid.y, false);
        await frame();
        // At once (the view rests 150 ms before the hub hears of it, so the
        // strip's part is not drawn yet): a tap on the strip, then one on
        // the part drawn.
        for (const [id, at] of taps) {
          fire("pointerdown", id, at.x, at.y, true);
          fire("pointerup", id, at.x, at.y, true);
        }
      },
      { mid, half: 40, shift, taps: [[81, strip], [82, onPart]] as [number, At][] },
    );
    // The view as panned: the strip's point lies beyond the old part's
    // side, the other point inside it (the test's own geometry).
    const moved = await pictureMap(canvas);
    const stripAt = moved.toPicture(strip);
    const onPartAt = moved.toPicture(onPart);
    expect(stripAt.x, JSON.stringify({ old, stripAt })).toBeGreaterThan(right + 16);
    expect(onPartAt.x, JSON.stringify({ old, onPartAt })).toBeLessThan(right - 16);
    expect(onPartAt.x, JSON.stringify({ old, onPartAt })).toBeGreaterThan(old[0] + 16);
    for (const at of [stripAt, onPartAt]) {
      expect(at.y, JSON.stringify({ old, at })).toBeGreaterThan(old[1] + 16);
      expect(at.y, JSON.stringify({ old, at })).toBeLessThan(old[1] + old[3] - 16);
    }
    // The strip's finger waits: the note, its record, nothing to the editor.
    await expect(stale).toHaveAttribute("data-shown", "true");
    const refused = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_touch" && e.what === "uncovered" && e.t >= since),
      (all) => all.length >= 1,
      "the finger on the strip in the recorder",
      15_000,
    );
    expect(refused.map((e: any) => e.p)).toEqual([81]);
    // The finger on the part drawn reached the editor where it showed.
    const tap = await touchesUntil(before, (all) => all.some((r) => r.phase === "up"), "the tap on the part drawn");
    wentFromTo(tap, onPartAt, onPartAt);
    // The strip's part lands (the hub heard the view once it rested); a
    // finger there now reaches the editor.
    await until(
      drawnPart,
      ([x, y, w, h]) => x + 16 <= stripAt.x && stripAt.x <= x + w - 16 && y + 16 <= stripAt.y && stripAt.y <= y + h - 16,
      "the strip's part drawn",
    );
    await caughtUp(screen);
    const tapFrom = await touchMark();
    await quickTap(area, strip, 83);
    const second = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap on the strip");
    wentFromTo(second, stripAt, stripAt);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });
});

test.describe("The Pro-Q 4 screen and touch-idle (#51)", () => {
  test("an open Pro-Q 4 screen never lets the page go idle; closed, the page goes idle the layout's seconds after the exit", async ({ page }) => {
    // Touch-idle after 2 s (the fixture has none). The screen holds the
    // page awake: its first-finger rules were tuned for the live frames,
    // and it holds the PC's one Pro-Q 4 editor.
    const idle: { at: number; msg: any }[] = [];
    page.on("websocket", (ws) => {
      ws.on("framesent", ({ payload }) => {
        if (typeof payload === "string" && payload.includes('"type":"idle"')) idle.push({ at: Date.now(), msg: JSON.parse(payload) });
      });
    });
    try {
      await setIdleAfter(2);
      await openSurface(page);
      const surface = page.getByTestId("surface");
      // The page has touch-idle: untouched, it goes idle (the touches that
      // open the screen wake it).
      await expect(surface).toHaveAttribute("data-idle", "true", { timeout: 10_000 });
      const { onTrack, trackPath } = await hand2Cards(page);
      await editorOpen(trackPath, false);
      await onTrack.getByTestId("eq-open").click();
      const screen = await openScreen(page, trackPath);
      await expect(surface).toHaveAttribute("data-idle", "false");
      // Open and untouched for twice the layout's seconds: never idle, and
      // the frames keep coming.
      const canvas = screen.getByTestId("eq-canvas");
      const drawn = async () => Number(await canvas.getAttribute("data-frames"));
      const first = await drawn();
      const openedAt = Date.now();
      await page.waitForTimeout(4_000);
      expect(
        idle.filter((f) => f.at >= openedAt).map((f) => f.msg),
        "no idle while the screen is open",
      ).toEqual([]);
      await expect(surface).toHaveAttribute("data-idle", "false");
      expect((await drawn()) - first, "frames drawn in 4 s").toBeGreaterThanOrEqual(8);

      // Closed: the page goes idle the layout's seconds after the exit's
      // touch, its meters paced.
      const closing = (await simEq.records()).length;
      const exitAt = Date.now();
      await screen.getByTestId("eq-exit").click();
      await expect(page.getByTestId("eq-screen")).toHaveCount(0);
      await closedWithGuard(closing);
      await editorOpen(trackPath, false);
      await expect(surface).toHaveAttribute("data-idle", "true", { timeout: 10_000 });
      const slept = await until(async () => idle.filter((f) => f.at >= exitAt), (a) => a.length >= 1, "the idle after the exit");
      expect(slept.map((f) => f.msg)).toEqual([{ type: "idle", on: true, meter_ms: 250 }]);
      expect(slept[0].at - exitAt, "counted from the exit").toBeGreaterThanOrEqual(1_900);
    } finally {
      await resetLayout();
    }
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
      await expect(open).toHaveText("OPEN EQ FULL SCREEN");
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
  /**
   * The page's downlink (bytes/s): ~20 frames a second of the sim's ~21 KB
   * picture would need ~420 KB/s; at this rate one frame holds a pong about
   * 70 ms, under the pings' 100 ms (the PR J review: at 150 KB/s a pong
   * could wait past the next ping).
   */
  const DOWN_RATE = 300_000;
  /**
   * The bounds of the latency on that link, set from the real iPad (#71): the
   * owner's session over the site Wi-Fi measured p50 80 ms and p95 118 ms, and
   * six CI runs on this link measured medians of 87-104 ms and p95s of
   * 124-147 ms. Each bound is about twice CI's worst, so a regression that
   * doubles the latency fails while the shared runners' spread passes.
   */
  const LAT_MEDIAN_MAX = 200;
  const LAT_P95_MAX = 300;

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
      const before = await touchMark();
      const from = await centre(area);
      await caughtUp(screen);
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      for (let i = 1; i <= 3 * DRAG_STEPS; i++) {
        await page.mouse.move(from.x + 3 * i, from.y);
        await frames(page, 2);
      }
      // At once: a finger on the picture, which does not show the up yet.
      // The tap goes in the up's own task (a `MutationObserver` on the
      // canvas's `data-caught-up` turning false, as in the third test here):
      // a tap as a Playwright call of its own came 95 ms after the up in push
      // run 38411708522, when the picture already showed it (PAINT_MS 20),
      // and the page took it. The picture first shows the drag's last move,
      // so the up is what makes it old.
      await caughtUp(screen);
      await area.evaluate(
        (el, { at, id }) => {
          (window as any).__tapAfterUp = new Promise<string>((done) => {
            const canvas = el.querySelector('[data-testid="eq-canvas"]')!;
            const observer = new MutationObserver(() => {
              if (canvas.getAttribute("data-caught-up") !== "false") return;
              observer.disconnect();
              for (const type of ["pointerdown", "pointerup"]) {
                el.dispatchEvent(
                  new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: true, clientX: at.x, clientY: at.y, bubbles: true, cancelable: true }),
                );
              }
              done("tapped behind the up");
            });
            observer.observe(canvas, { attributes: true, attributeFilter: ["data-caught-up"] });
          });
        },
        { at: from, id: 71 },
      );
      await page.mouse.up();
      const tapped = await page.evaluate(() =>
        Promise.race([(window as any).__tapAfterUp, new Promise((done) => setTimeout(() => done("the up made no picture old within 3 s"), 3000))]),
      );
      expect(tapped).toBe("tapped behind the up");
      await expect(stale).toHaveAttribute("data-shown", "true");
      await expect(stale).toHaveText("Image still catching up, touch again");
      const drag = await touchesUntil(before, (all) => all.some((r) => r.phase === "up"), "the drag's up");
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
      await caughtUp(screen);
      const tapFrom = await touchMark();
      await quickTap(area, from, 72);
      const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the tap once caught up");
      const phases = tap.map((r) => r.phase);
      expect([phases[0], phases[phases.length - 1]], JSON.stringify(tap)).toEqual(["down", "up"]);
      expect(phases.slice(1, -1).every((p) => p === "update"), JSON.stringify(tap)).toBe(true);
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

  test("a link that stalls under a drag ends its contact with a cancel: no move of the stall reaches the editor after it", async ({ page }) => {
    // PR J review (point 5): the moves queued in a stalled socket came in a
    // burst once the link was back. The page's dropout counter turns active
    // after ~300 ms of silence; the page then cancels the contact and sends
    // nothing more of that finger.
    test.setTimeout(60_000);
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    await zoomIn(screen, FROM);
    const before = await touchMark();
    const from = await centre(area);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= 4; i++) {
      await page.mouse.move(from.x + 3 * i, from.y);
      await frames(page, 2);
    }
    await touchesUntil(before, (all) => all.some((r) => r.phase === "update"), "the drag's moves");
    // The note shows for 1.5 s: an observer remembers it (WebKit on the
    // runner draws slowly enough to miss it with a later look).
    await screen.getByTestId("eq-stale").evaluate((el) => {
      (window as any).staleSeen = false;
      new MutationObserver(() => {
        if (el.getAttribute("data-shown") === "true") (window as any).staleSeen = true;
      }).observe(el, { attributes: true });
    });
    await impair.stall(2_000);
    // The finger goes on moving into the stall, and stays down until the
    // page's dropout counter is surely active (~300 ms of silence).
    for (let i = 5; i <= 14; i++) {
      await page.mouse.move(from.x + 3 * i, from.y);
      await frames(page, 1);
    }
    await page.waitForTimeout(700);
    await expect.poll(() => page.evaluate(() => (window as any).staleSeen), { message: "the note showed" }).toBe(true);
    await page.mouse.up();
    // The link is back: what came in it ends with the cancel.
    const touches = await touchesUntil(before, (all) => all.some((r) => r.phase === "cancel"), "the stalled drag's cancel", 10_000);
    await page.waitForTimeout(500);
    const all = await touchesAfter(before);
    expect(all[all.length - 1].phase, JSON.stringify(all)).toBe("cancel");
    expect(all.some((r) => r.phase === "up"), "no up after it").toBe(false);
    expect(all.length, JSON.stringify(all)).toBe(touches.length);
    // The recorder says it too: the finger's cancel.
    const cancels = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_touch" && e.what === "cancel"),
      (found) => found.length >= 1,
      "the finger's cancel in the recorder",
      15_000,
    );
    expect(cancels.length).toBeGreaterThanOrEqual(1);
    await caughtUp(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("a screen left right after a drag holds no finger of the next screen back", async ({ page }) => {
    // The PR J review: the page's inputs of a screen left before its
    // picture showed them refused every finger of the next one, until a
    // reload.
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await impair.downRate(DOWN_RATE);
    try {
      await onTrack.getByTestId("eq-open").click();
      let screen = await openScreen(page, trackPath);
      await zoomIn(screen, FROM);
      const from = await centre(screen.getByTestId("eq-area"));
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      for (let i = 1; i <= DRAG_STEPS; i++) {
        await page.mouse.move(from.x + 4 * i, from.y);
        await frames(page, 2);
      }
      // The drag shown, then left the moment its up goes, while the picture
      // cannot show it yet (#71 PR O: with PAINT_MS 20 the picture catches
      // up within a frame or two, quicker than a poll of the attribute; an
      // observer in the page leaves in the same task).
      await caughtUp(screen);
      const dragFrom = (await simEq.records()).length;
      // The observer is in place before the up (a locator's evaluate
      // resolves the element first, and the up could pass it).
      await screen.getByTestId("eq-canvas").evaluate((canvas) => {
        (window as any).__leftBehind = new Promise<string>((done) => {
          const exit = document.querySelector('[data-testid="eq-exit"]')!;
          const observer = new MutationObserver(() => {
            if (canvas.getAttribute("data-caught-up") === "false") {
              observer.disconnect();
              exit.dispatchEvent(new PointerEvent("pointerdown", { pointerId: 99, pointerType: "touch", isPrimary: true, bubbles: true, cancelable: true }));
              done("left behind its up");
            }
          });
          observer.observe(canvas, { attributes: true, attributeFilter: ["data-caught-up"] });
        });
      });
      await page.mouse.up();
      expect(await page.evaluate(() => (window as any).__leftBehind)).toBe("left behind its up");
      await expect(page.getByTestId("eq-screen")).toHaveCount(0);
      // The drag's up reached the editor before the close (one socket, in
      // order); the guard's sequence follows it.
      const all = await until(simEq.records, (r) => r.some((x, i) => i >= dragFrom && x.op === "release"), "the editor's release");
      const up = all.findIndex((r, i) => i >= dragFrom && r.op === "touch" && r.phase === "up");
      expect(up, JSON.stringify(all.slice(dragFrom))).toBeGreaterThanOrEqual(dragFrom);
      await closedWithGuard(up + 1);
      await editorOpen(trackPath, false);
      // The next screen: its first tap edits.
      await onTrack.getByTestId("eq-open").click();
      screen = await openScreen(page, trackPath);
      await zoomIn(screen, FROM);
      const area = screen.getByTestId("eq-area");
      const at = await centre(area);
      const tapFrom = await touchMark();
      await quickTap(area, at, 81);
      const tap = await touchesUntil(tapFrom, (all) => all.some((r) => r.phase === "up"), "the next screen's tap");
      expect(tap[0].phase, JSON.stringify(tap)).toBe("down");
      await expect(screen.getByTestId("eq-stale")).toHaveAttribute("data-shown", "false");
      const closing = (await simEq.records()).length;
      await screen.getByTestId("eq-exit").click();
      await expect(page.getByTestId("eq-screen")).toHaveCount(0);
      await closedWithGuard(closing);
      await editorOpen(trackPath, false);
    } finally {
      await impair.downRate(0);
    }
  });
});

test.describe("The Pro-Q 4 screen's ↶ UNDO", () => {
  // #71 PR P2b (the owner approved the look, #71 comment 6120180007, the
  // mockup docs/mockups/proq-undo-v1.html): a big undo on the bar asks the
  // hub to tap Pro-Q's own ↶ (`eq_undo`, PR P2a). The hub reads the ↶ on a
  // fresh picture and taps it only when it reads lit (something to undo);
  // the simulated Pro-Q keeps an undo history (a drag makes one change, a
  // tap on its ↶ undoes one and is recorded `{op: "undo", left}`). The
  // button takes no press while a finger keeps the PC's contact, nor within
  // 700 ms of the last one; a note over the picture says the hub's answer.

  test("↶ UNDO takes back Pro-Q's last change: the hub taps Pro-Q's own ↶, the note says so, the button rests 700 ms and a second press then sends nothing; with nothing left it says so and taps nothing", async ({ page }) => {
    const since = Date.now();
    const { sent, answers } = watchUndo(page);
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const session = Number(await screen.getAttribute("data-session"));
    const undo = screen.getByTestId("eq-undo");
    const canvas = screen.getByTestId("eq-canvas");
    await expect(undo).toHaveAttribute("aria-disabled", "false");
    await expect(screen.getByTestId("eq-undo-note")).toHaveAttribute("data-shown", "false");
    await undoAll(screen, answers);
    await recordUndoNotes(screen);

    // A real drag of a band at 3×: one change in Pro-Q's history.
    const map = await zoomIn(screen, FROM);
    const dragFrom = await touchMark();
    const from = map.toPage(FROM);
    const to = map.toPage(TO);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS; i++) {
      await page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
      await frames(page, 2);
    }
    await page.mouse.up();
    await touchesUntil(dragFrom, (all) => all.some((r) => r.phase === "up"), "the drag's up");
    await caughtUp(screen);

    // The press, watched in the page on its clock: the button, its flash and
    // the canvas's rule for a first finger a macrotask after it, each change
    // of them after it, and a second press dispatched 200 ms after it.
    await screen.evaluate((el) => {
      const w = window as any;
      const button = el.querySelector('[data-testid="eq-undo"]') as HTMLElement;
      const canvas = el.querySelector('[data-testid="eq-canvas"]')!;
      const seen: any = { pressed: null, before: null, after: null, again: null, changes: [] };
      w.undoPress = seen;
      const observer = new MutationObserver((records) => {
        const t = performance.now();
        // Each change's new value: the next change's old one, the last one's now.
        records.forEach((r, i) => {
          const later = records.slice(i + 1).find((o) => o.target === r.target && o.attributeName === r.attributeName);
          seen.changes.push({ t, name: r.attributeName, v: later ? later.oldValue : (r.target as Element).getAttribute(r.attributeName!) });
        });
      });
      observer.observe(button, { attributes: true, attributeFilter: ["aria-disabled", "data-flash"], attributeOldValue: true });
      observer.observe(canvas, { attributes: true, attributeFilter: ["data-takes-finger"], attributeOldValue: true });
      const look = () => {
        const style = getComputedStyle(button);
        return {
          disabled: button.getAttribute("aria-disabled"),
          flash: button.getAttribute("data-flash"),
          takes: canvas.getAttribute("data-takes-finger"),
          border: style.borderTopColor,
          color: style.color,
          background: style.backgroundColor,
        };
      };
      button.addEventListener(
        "pointerdown",
        () => {
          seen.pressed = performance.now();
          seen.before = look();
          setTimeout(() => (seen.after = look()), 0);
          setTimeout(() => {
            seen.again = { t: performance.now(), disabled: button.getAttribute("aria-disabled") };
            button.dispatchEvent(new PointerEvent("pointerdown", { pointerId: 93, pointerType: "touch", isPrimary: true, bubbles: true, cancelable: true }));
          }, 200);
        },
        { capture: true, once: true },
      );
    });
    const mark = await touchMark();
    const asked = undoAsks(sent).length;
    const answered = answers.length;
    const notesBefore = (await undoNotes(page)).length;
    await pressUndo(screen);
    const answer = await answerAfter(answers, answered);
    expect([answer.session, answer.result], JSON.stringify(answer)).toEqual([session, "done"]);
    // The undo goes numbered as the editor's next input: every input and
    // undo of the session in one sequence from 1.
    expect(undoAsks(sent).slice(asked).map((m) => m.n), JSON.stringify(sent.slice(-3))).toEqual([answer.n]);
    expect(sent.map((m) => m.n), JSON.stringify(sent.map((m) => [m.type, m.n]))).toEqual(sent.map((_, i) => i + 1));

    // The hub tapped Pro-Q's ↶ where it read it for the picture's width (its
    // `undo` record), and the simulated Pro-Q took the drag back: none left.
    const picture = await pictureSize(canvas);
    const [record] = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "eq" && r.what === "undo" && r.ts >= since && r.n === answer.n),
      (all) => all.length >= 1,
      "the hub's undo record",
    );
    expect([record.result, record.session, record.y, record.w, record.h], JSON.stringify(record)).toEqual(["done", session, 18, picture.width, picture.height]);
    const tapped = await until(async () => (await simEq.records()).slice(mark.at), (r) => r.some((x) => x.op === "undo"), "the simulated Pro-Q's undo");
    expect(steps(tapped), JSON.stringify(tapped)).toEqual([
      ["touch", "down", record.x, 18],
      ["touch", "up", record.x, 18],
      ["undo", null, null, null],
    ]);
    expect(tapped[2].left, JSON.stringify(tapped)).toBe(0);
    expect(tapped.every((r) => r.window === tapped[0].window), "one window").toBe(true);
    await expect.poll(async () => (await undoNotes(page)).slice(notesBefore), { message: "the note" }).toContain("true|Undone: last change in Pro-Q");
    expect(await clipped(screen.getByTestId("eq-undo-note"))).toEqual([]);

    // The button: disabled and flashing a macrotask after the press, the
    // canvas taking no first finger until a picture shows the undo; then the
    // flash goes after 180 ms, the button takes a press again after 700 ms,
    // and the canvas once caught up.
    await expect(undo).toHaveAttribute("aria-disabled", "false");
    await expect(canvas).toHaveAttribute("data-takes-finger", "true");
    const press = await page.evaluate(() => (window as any).undoPress);
    const story = JSON.stringify(press);
    expect(press.before, story).toMatchObject({ disabled: "false", takes: "true", border: "rgb(255, 195, 90)", color: "rgb(255, 195, 90)", background: "rgb(42, 32, 16)" });
    expect(press.after, story).toMatchObject({ disabled: "true", flash: "true", takes: "false", border: "rgb(90, 74, 42)", color: "rgb(122, 106, 74)", background: "rgb(90, 68, 16)" });
    const changes = (name: string) => press.changes.filter((c: any) => c.name === name);
    expect(changes("aria-disabled").map((c: any) => c.v), story).toEqual(["true", "false"]);
    const enabled = changes("aria-disabled")[1].t - press.pressed;
    expect(enabled, story).toBeGreaterThanOrEqual(700);
    expect(enabled, story).toBeLessThan(1_500);
    expect(changes("data-flash").map((c: any) => c.v), story).toEqual(["true", "false"]);
    expect(changes("data-flash")[1].t - press.pressed, story).toBeGreaterThanOrEqual(180);
    const takes = changes("data-takes-finger");
    expect(takes[0]?.v, story).toBe("false");
    expect(takes[takes.length - 1].v, story).toBe("true");
    // The second press, 200 ms after the first, found it disabled and sent
    // nothing: one undo went, one tap.
    expect(press.again?.disabled, story).toBe("true");
    expect(undoAsks(sent).length - asked, JSON.stringify(undoAsks(sent))).toBe(1);
    expect(steps((await simEq.records()).slice(mark.at)), "only the first press's tap").toEqual(steps(tapped));

    // The page's flight recorder: the press with its number, the answer and
    // its wait for it.
    const [event] = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_undo" && e.n === answer.n && e.t >= since),
      (all) => all.length >= 1,
      "the page's eq_undo event",
      15_000,
    );
    expect(event.result, JSON.stringify(event)).toBe("done");
    expect(event.ms, JSON.stringify(event)).toBeGreaterThanOrEqual(0);
    expect(event.ms, JSON.stringify(event)).toBeLessThan(5_000);

    // Nothing left to undo: the hub reads Pro-Q's ↶ grey and taps nothing.
    await page.waitForTimeout(HUB_APART_MARGIN);
    const nothingFrom = await touchMark();
    const notesNow = (await undoNotes(page)).length;
    const before = answers.length;
    await pressUndo(screen);
    expect((await answerAfter(answers, before)).result).toBe("nothing");
    await expect.poll(async () => (await undoNotes(page)).slice(notesNow), { message: "the note" }).toContain("true|Nothing to undo");
    await page.waitForTimeout(300);
    expect(steps((await simEq.records()).slice(nothingFrom.at)), "no tap on the ↶").toEqual([]);

    await caughtUp(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("while a finger holds the picture ↶ UNDO takes no press and nothing goes; once it lifts a press takes the drag back", async ({ page }) => {
    const { sent, answers } = watchUndo(page);
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const undo = screen.getByTestId("eq-undo");
    await recordUndoNotes(screen);
    const map = await zoomIn(screen, FROM);
    const dragFrom = await touchMark();
    const from = map.toPage(FROM);
    const to = map.toPage(TO);
    const step = (i: number) => page.mouse.move(from.x + ((to.x - from.x) * i) / DRAG_STEPS, from.y + ((to.y - from.y) * i) / DRAG_STEPS);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    for (let i = 1; i <= DRAG_STEPS / 2; i++) {
      await step(i);
      await frames(page, 2);
    }
    await touchesUntil(dragFrom, (all) => all.some((r) => r.phase === "update"), "the drag's moves");
    await expect(undo).toHaveAttribute("aria-disabled", "true");
    // A second finger on ↶ UNDO (the mouse holds the picture): nothing goes.
    const asked = undoAsks(sent).length;
    await undo.dispatchEvent("pointerdown", { pointerId: 94, pointerType: "touch", isPrimary: false, bubbles: true, cancelable: true });
    await page.waitForTimeout(400);
    expect(undoAsks(sent).length, "no undo while the finger holds the picture").toBe(asked);
    for (let i = DRAG_STEPS / 2 + 1; i <= DRAG_STEPS; i++) {
      await step(i);
      await frames(page, 2);
    }
    await page.mouse.up();
    await touchesUntil(dragFrom, (all) => all.some((r) => r.phase === "up"), "the drag's up");
    expect((await simEq.records()).slice(dragFrom.at).filter((r) => r.op === "undo"), "no undo during the drag").toEqual([]);

    // Lifted: it takes a press, and the hub takes the drag back.
    await expect(undo).toHaveAttribute("aria-disabled", "false");
    const mark = await touchMark();
    const notesBefore = (await undoNotes(page)).length;
    const before = answers.length;
    await pressUndo(screen);
    expect((await answerAfter(answers, before)).result).toBe("done");
    expect(undoAsks(sent).length - asked, "one undo").toBe(1);
    await until(async () => (await simEq.records()).slice(mark.at), (r) => r.some((x) => x.op === "undo"), "the simulated Pro-Q's undo");
    await expect.poll(async () => (await undoNotes(page)).slice(notesBefore), { message: "the note" }).toContain("true|Undone: last change in Pro-Q");

    await caughtUp(screen);
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });

  test("↶ UNDO sits on the bar after the name, before the lock mark and ?; upright only ↶ shows; the bar fits a phone either way; the legend names it", async ({ page }) => {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const area = screen.getByTestId("eq-area");
    const undo = screen.getByTestId("eq-undo");
    const word = screen.getByTestId("eq-undo-word");
    await expect(screen.getByTestId("eq-undo-arrow")).toHaveText("↶");
    await expect(word).toHaveText("UNDO");
    await expect(word).toBeVisible();
    // Right of the name, left of the lock mark and ?.
    const box = async (locator: Locator) => (await locator.boundingBox())!;
    const name = await box(screen.locator(".eq-name"));
    const own = await box(undo);
    const mine = await box(screen.getByTestId("eq-mine"));
    const help = await box(screen.getByTestId("eq-help"));
    expect(own.x, "after the name").toBeGreaterThanOrEqual(name.x + name.width - 0.5);
    expect(own.x + own.width, "before the lock mark").toBeLessThanOrEqual(mine.x + 0.5);
    expect(mine.x + mine.width, "the lock mark before ?").toBeLessThanOrEqual(help.x + 0.5);
    // The mockup's look: amber on dark, a 2 px border, at least 132 px wide.
    const look = await undo.evaluate((el) => {
      const style = getComputedStyle(el);
      return { border: style.borderTopColor, line: style.borderTopWidth, color: style.color, background: style.backgroundColor };
    });
    expect(look).toEqual({ border: "rgb(255, 195, 90)", line: "2px", color: "rgb(255, 195, 90)", background: "rgb(42, 32, 16)" });
    expect(own.width).toBeGreaterThanOrEqual(132);
    await barFits(screen, ["eq-exit", "eq-undo", "eq-mine", "eq-help"], ["eq-exit", "eq-undo", "eq-mine"]);

    // The legend: a hold opens nothing (PR O proved it on the PC), and ↶
    // UNDO's line.
    await screen.getByTestId("eq-help").click();
    const legend = screen.getByTestId("eq-legend");
    await expect(legend.getByTestId("eq-legend-hold").locator("td")).toHaveText(["hold", "nothing: the band menu does not open"]);
    await expect(legend.getByTestId("eq-legend-undo").locator("td")).toHaveText(["↶ UNDO", "undoes the last change in Pro-Q"]);
    await expect(legend).toContainText("When zoomed, the image on the PC stays the same: only the view on this device changes.");
    await screen.getByTestId("eq-help").click();
    await expect(legend).toHaveCount(0);

    // A phone upright: only ↶, and nothing in the bar cut off or outside it;
    // zoomed too, the zoom group beside it.
    await page.setViewportSize({ width: 390, height: 844 });
    await frames(page, 3);
    await shaped(screen);
    await expect(word).toBeHidden();
    await expect(screen.getByTestId("eq-undo-arrow")).toBeVisible();
    expect((await box(undo)).width).toBeGreaterThanOrEqual(56);
    await barFits(screen, ["eq-exit", "eq-undo", "eq-help"], ["eq-exit", "eq-undo"]);
    await pinch(area, await centre(area), 50, 125);
    await expect(screen.getByTestId("eq-factor")).toHaveText("2.5×");
    await zoomBarFits(screen);

    // A phone on its side: the 40 px bar, the word back, the zoom group shown.
    await page.setViewportSize({ width: 844, height: 390 });
    await frames(page, 3);
    await shaped(screen);
    await expect(word).toBeVisible();
    expect(Math.round((await box(screen.getByTestId("eq-bar"))).height), "the low bar").toBe(40);
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "true");
    await zoomBarFits(screen);

    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  });
});

test.describe("The Pro-Q 4 screen and a finger whose end never came (#124)", () => {
  // iOS can take a finger for a system gesture and deliver no pointerup,
  // pointercancel or lostpointercapture for it (#124: a fader's touch with
  // no end on 2026-10-15). A dispatched pointer is no active pointer, so
  // its setPointerCapture fails and no capture is ever lost: the state of
  // such a touch. The screen kept it among its fingers, and every later
  // finger pinched with it. Now the next primary finger of its type, or a
  // hide, ends it (its contact cancelled at the PC, `eq_touch` `lost`), and
  // the new finger starts a gesture of its own.

  /** One finger's pointer event (a touch, `primary` as a browser marks it) dispatched on `target` (the area, a bar's button) at a page point. */
  async function fingerEvent(target: Locator, type: string, at: At, id: number, primary: boolean) {
    await target.evaluate(
      (el, { type, at, id, primary }) => {
        el.dispatchEvent(
          new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: primary, clientX: at.x, clientY: at.y, bubbles: true, cancelable: true }),
        );
      },
      { type, at, id, primary },
    );
  }

  /**
   * One finger's drag dispatched on the area (a touch, `primary` as a
   * browser marks it): down at `from`, `DRAG_STEPS` moves to `to`, one an
   * animation frame (the page sends at most one a frame), two frames, the
   * up at `to`. Its first move passes the 6 px slop, so its down goes at
   * once. Whether the screen's note "Image still catching up" (`eq-stale`)
   * showed at any frame of the drag: read in the page, as it lasts only
   * 1.5 s (e2e.md's short states).
   */
  async function fingerDrag(area: Locator, from: At, to: At, id: number, primary: boolean): Promise<boolean> {
    return area.evaluate(
      async (el, { from, to, id, primary, steps }) => {
        const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()));
        const note = document.querySelector('[data-testid="eq-stale"]');
        let noted = false;
        const look = () => {
          noted = noted || note?.getAttribute("data-shown") === "true";
        };
        const fire = (type: string, x: number, y: number) =>
          el.dispatchEvent(
            new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: primary, clientX: x, clientY: y, bubbles: true, cancelable: true }),
          );
        fire("pointerdown", from.x, from.y);
        for (let i = 1; i <= steps; i++) {
          await frame();
          look();
          fire("pointermove", Math.round(from.x + ((to.x - from.x) * i) / steps), Math.round(from.y + ((to.y - from.y) * i) / steps));
        }
        await frame();
        look();
        await frame();
        look();
        fire("pointerup", to.x, to.y);
        return noted;
      },
      { from, to, id, primary, steps: DRAG_STEPS },
    );
  }

  /** Hand2 #'s Pro-Q 4 on the track open and zoomed in on `FROM` (a finger edits): the screen, its area, the picture's map and the editor's path. */
  async function openZoomed(page: Page) {
    await openSurface(page);
    const { onTrack, trackPath } = await hand2Cards(page);
    await editorOpen(trackPath, false);
    await onTrack.getByTestId("eq-open").click();
    const screen = await openScreen(page, trackPath);
    const map = await zoomIn(screen, FROM);
    return { screen, area: screen.getByTestId("eq-area"), map, trackPath };
  }

  /** The exit: the editor closed with the guard and released, closed in Live. */
  async function exitScreen(page: Page, screen: Locator, trackPath: string) {
    const closing = (await simEq.records()).length;
    await screen.getByTestId("eq-exit").click();
    await expect(page.getByTestId("eq-screen")).toHaveCount(0);
    await closedWithGuard(closing);
    await editorOpen(trackPath, false);
  }

  /** The hub's records of the PC's contacts on `path` since `since` (`eq` `touch`: down, up, cancel; never a move), once `n` are in. */
  async function contacts(path: string, since: number, n: number): Promise<string[]> {
    const records = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "eq" && r.what === "touch" && r.path === path && r.ts >= since),
      (all) => all.length >= n,
      `the hub's ${n} contact records`,
    );
    return records.map((r: any) => r.why);
  }

  /** The page's `eq_touch` records since `since` as [what, pointer], once pointer `last`'s `end` (its up by default) is in (up to 15 s for the upload). */
  async function fingerRecords(since: number, last: number, end = "up"): Promise<(string | number)[][]> {
    const records = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "eq_touch" && e.t >= since),
      (all) => all.some((e: any) => e.what === end && e.p === last),
      `the page's eq_touch ${end} of ${last}`,
      15_000,
    );
    return records.map((e: any) => [e.what, e.p]);
  }

  /** Whether the view stayed where `shown` was: no pinch zoomed or panned it. */
  async function sameView(screen: Locator, shown: { left: number; top: number }) {
    await expect(screen.getByTestId("eq-factor")).toHaveText("3.0×");
    const now = await pictureMap(screen.getByTestId("eq-canvas"));
    expect(Math.max(Math.abs(now.left - shown.left), Math.abs(now.top - shown.top)), "the view stayed: no pinch").toBeLessThan(0.5);
  }

  test("a finger whose end never came keeps no pinch going: the next first finger drags a band alone", async ({ page }) => {
    const LOST = 901;
    const SECOND = 902;
    const NEXT = 903;
    const { screen, area, map, trackPath } = await openZoomed(page);
    const since = Date.now();
    const start = await touchMark();
    // A finger rests on the band; after its hold the PC's contact is down.
    const lostAt = map.toPage(FROM);
    await fingerEvent(area, "pointerdown", lostAt, LOST, true);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "down"), "the lost finger's contact");
    // A second finger comes and goes (the system gesture's): the page
    // pinches, so the contact is cancelled then. The first finger's end
    // never comes, and it alone kept the screen pinching.
    const other = { x: lostAt.x + 150, y: lostAt.y };
    await fingerEvent(area, "pointerdown", other, SECOND, false);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "cancel"), "the pinch's cancel of the contact");
    await fingerEvent(area, "pointerup", other, SECOND, false);
    await frames(page, 2);
    await expect(screen.getByTestId("eq-factor")).toHaveText("3.0×");
    const shown = await pictureMap(screen.getByTestId("eq-canvas"));
    await caughtUp(screen);
    // The next first finger (primary: no other finger is down) drags a
    // band: alone, so the editor gets its drag and the view stays.
    const from = { x: lostAt.x - 100, y: lostAt.y + 60 };
    const to = { x: from.x + 80, y: from.y - 40 };
    const mark = await touchMark();
    await fingerDrag(area, from, to, NEXT, true);
    const drag = await touchesUntil(mark, (all) => all.some((r) => r.phase === "up"), "the next finger's drag at the editor");
    wentFromTo(drag, shown.toPicture(from), shown.toPicture(to));
    await sameView(screen, shown);
    // The PC's contacts: the lost finger's down and cancel, the drag's down
    // and up.
    expect(await contacts(trackPath, since, 4)).toEqual(["down", "cancel", "down", "up"]);
    // The page's record: the lost finger ended at the next finger's down.
    expect(await fingerRecords(since, NEXT)).toEqual([
      ["down", LOST],
      ["down", SECOND],
      ["up", SECOND],
      ["lost", LOST],
      ["down", NEXT],
      ["up", NEXT],
    ]);
    await exitScreen(page, screen, trackPath);
  });

  test("a finger whose end never came on the editor: the next first finger ends its contact and pinches nothing; once the picture shows that cancel a finger drags a band alone", async ({ page }) => {
    const LOST = 911;
    const NEXT = 912;
    const AFTER = 913;
    const { screen, area, map, trackPath } = await openZoomed(page);
    const since = Date.now();
    const start = await touchMark();
    const lostAt = map.toPage(FROM);
    await fingerEvent(area, "pointerdown", lostAt, LOST, true);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "down"), "the lost finger's contact");
    const shown = await pictureMap(screen.getByTestId("eq-canvas"));
    // The next first finger ends the lost one: its contact is cancelled at
    // its point, so no band stays grabbed. This finger landed on a picture
    // that cannot show that cancel yet, so the page refuses it (#71 PR J:
    // an old picture never acts; the hub would drop its down): it reaches
    // nothing, and it pinches nothing either.
    const from = { x: lostAt.x - 100, y: lostAt.y + 60 };
    const to = { x: from.x + 80, y: from.y - 40 };
    // The engineer sees why: "Image still catching up, touch again".
    expect(await fingerDrag(area, from, to, NEXT, true), "the stale note under the refused finger").toBe(true);
    const ended = await touchesUntil(start, (all) => all.some((r) => r.phase === "cancel"), "the lost contact's cancel");
    expect(ended[0].phase).toBe("down");
    const cancel = ended[ended.length - 1];
    expect(near(cancel, map.toPicture(lostAt)), `the cancel ${JSON.stringify(cancel)} at ${JSON.stringify(map.toPicture(lostAt))}`).toBe(true);
    await sameView(screen, shown);
    await page.waitForTimeout(300);
    const settled = await touchesAfter(start);
    expect(settled[settled.length - 1].phase, `nothing after the cancel: ${JSON.stringify(settled)}`).toBe("cancel");
    // The finger after it drags a band, alone.
    await caughtUp(screen);
    const mark = await touchMark();
    await fingerDrag(area, from, to, AFTER, true);
    const drag = await touchesUntil(mark, (all) => all.some((r) => r.phase === "up"), "the next finger's drag at the editor");
    wentFromTo(drag, shown.toPicture(from), shown.toPicture(to));
    await sameView(screen, shown);
    expect(await contacts(trackPath, since, 4)).toEqual(["down", "cancel", "down", "up"]);
    expect(await fingerRecords(since, AFTER)).toEqual([
      ["down", LOST],
      ["lost", LOST],
      ["down", NEXT],
      ["stale", NEXT],
      ["up", NEXT],
      ["down", AFTER],
      ["up", AFTER],
    ]);
    await exitScreen(page, screen, trackPath);
  });

  test("a hide ends a finger whose end never came, recorded lost: the next finger drags a band alone, primary or not", async ({ page }) => {
    const LOST = 921;
    const NEXT = 922;
    const { screen, area, map, trackPath } = await openZoomed(page);
    const since = Date.now();
    const start = await touchMark();
    const lostAt = map.toPage(FROM);
    await fingerEvent(area, "pointerdown", lostAt, LOST, true);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "down"), "the lost finger's contact");
    const shown = await pictureMap(screen.getByTestId("eq-canvas"));
    // A hidden Home Screen app keeps no touch going.
    await setHidden(page, true);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "cancel"), "the hidden page's cancel", 1_500);
    await setHidden(page, false);
    await caughtUp(screen);
    // Not primary (the browser might still count the lost finger): the
    // hide freed the screen anyway.
    const from = { x: lostAt.x - 100, y: lostAt.y + 60 };
    const to = { x: from.x + 80, y: from.y - 40 };
    const mark = await touchMark();
    await fingerDrag(area, from, to, NEXT, false);
    const drag = await touchesUntil(mark, (all) => all.some((r) => r.phase === "up"), "the next finger's drag at the editor");
    wentFromTo(drag, shown.toPicture(from), shown.toPicture(to));
    await sameView(screen, shown);
    expect(await contacts(trackPath, since, 4)).toEqual(["down", "cancel", "down", "up"]);
    expect(await fingerRecords(since, NEXT)).toEqual([
      ["down", LOST],
      ["lost", LOST],
      ["down", NEXT],
      ["up", NEXT],
    ]);
    await exitScreen(page, screen, trackPath);
  });

  test("a finger whose end never came leaves ↶ UNDO and FULL EQ working: a primary touch on either ends it first, its contact's cancel before the undo", async ({ page }) => {
    // #124 review I1: the lost finger kept the PC's contact, so ↶ UNDO
    // took no press until a touch on the picture. A primary touch on the
    // bar proves no other touch is down, so it ends the lost finger first.
    const LOST = 931;
    const PRESS = 932;
    const LOST_AGAIN = 933;
    const WHOLE = 934;
    const { sent, answers } = watchUndo(page);
    const { screen, area, map, trackPath } = await openZoomed(page);
    const undo = screen.getByTestId("eq-undo");
    const since = Date.now();
    const start = await touchMark();
    const lostAt = map.toPage(FROM);
    await fingerEvent(area, "pointerdown", lostAt, LOST, true);
    await touchesUntil(start, (all) => all.some((r) => r.phase === "down"), "the lost finger's contact");
    await expect(undo, "the lost finger keeps the PC's contact").toHaveAttribute("aria-disabled", "true");
    // A primary touch on ↶ UNDO: the lost contact's cancel, then the undo.
    const sentFrom = sent.length;
    const before = answers.length;
    const press = await centre(undo);
    await fingerEvent(undo, "pointerdown", press, PRESS, true);
    await fingerEvent(undo, "pointerup", press, PRESS, true);
    const ended = await touchesUntil(start, (all) => all.some((r) => r.phase === "cancel"), "the lost contact's cancel");
    const cancel = ended.find((r) => r.phase === "cancel");
    expect(near(cancel, map.toPicture(lostAt)), `the cancel ${JSON.stringify(cancel)} at ${JSON.stringify(map.toPicture(lostAt))}`).toBe(true);
    const answer = await answerAfter(answers, before);
    expect(["done", "nothing"], `the hub took the undo: ${JSON.stringify(answer)}`).toContain(answer.result);
    expect(
      sent.slice(sentFrom).map((m) => (m.type === "eq_undo" ? "undo" : m.touch)),
      "on the page's socket the cancel goes before the undo",
    ).toEqual(["cancel", "undo"]);
    // FULL EQ: the same. Another lost finger once a picture shows the undo,
    // then a primary touch on FULL EQ: its contact's cancel, the whole
    // picture, and ↶ UNDO takes a press again with no touch on the
    // picture.
    await caughtUp(screen);
    const again = await touchMark();
    await fingerEvent(area, "pointerdown", lostAt, LOST_AGAIN, true);
    await touchesUntil(again, (all) => all.some((r) => r.phase === "down"), "the second lost finger's contact");
    const whole = screen.getByTestId("eq-whole");
    const wholeAt = await centre(whole);
    await fingerEvent(whole, "pointerdown", wholeAt, WHOLE, true);
    await fingerEvent(whole, "pointerup", wholeAt, WHOLE, true);
    await touchesUntil(again, (all) => all.some((r) => r.phase === "cancel"), "the second lost contact's cancel");
    await expect(screen.getByTestId("eq-zoom")).toHaveAttribute("data-shown", "false");
    await expect(undo, "↶ UNDO free again").toHaveAttribute("aria-disabled", "false");
    expect(await contacts(trackPath, since, 4)).toEqual(["down", "cancel", "down", "cancel"]);
    expect(await fingerRecords(since, LOST_AGAIN, "lost")).toEqual([
      ["down", LOST],
      ["lost", LOST],
      ["down", LOST_AGAIN],
      ["lost", LOST_AGAIN],
    ]);
    await exitScreen(page, screen, trackPath);
  });
});
