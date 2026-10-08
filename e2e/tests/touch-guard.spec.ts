import type { Locator, Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "./support/fixtures";
import { centre, frames, harness, hubEvents, openSurface, pageEvents, ready, strip, track, until, volume } from "./support/live";

// The controls own their touches (#43 PR G, design comment 6011117148). The
// owner's retest of PR F on the FOH iPad saw, now and then, a magnifier
// (WebKit's loupe) or the whole control lifted as if dragged. CSS cannot stop
// the loupe of a tap followed by a hold: only an active `touchstart` listener
// that calls `preventDefault` does (WebKit bug 231161). So every control's
// root prevents its `touchstart` (Pointer Events stay the input path), the
// surface prevents `contextmenu`, `selectstart` and `dragstart`, no element of
// it can be dragged or selected, and the page's background (a line, the
// column) keeps its touches. The page's flight recorder records
// the system's gestures (a context menu, a selection, a drag, a pinch, a
// cancelled pointer, a capture lost while the finger is still down, a zoom),
// and the forensics timeline lists them.

const HAND2 = track("Hand2 #");
const FADER_KEY = `band|${volume(HAND2)}|value`;
const MUTE_KEY = `band|${HAND2}|mute`;

/** The page's clock (`performance.timeOrigin + performance.now()`, the `t` of its events). */
const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

/** Dispatches a cancelable, bubbling `type` event on `el`: whether something prevented it. */
async function prevented(el: Locator, type: string): Promise<boolean> {
  return el.evaluate((node: Element, name: string) => {
    const event = new Event(name, { bubbles: true, cancelable: true });
    node.dispatchEvent(event);
    return event.defaultPrevented;
  }, type);
}

/** Dispatches a pointer event of `type` by pointer `id` on `el`. */
async function pointer(el: Locator, type: string, id: number) {
  await el.evaluate(
    (node: Element, { name, pointerId }) => {
      node.dispatchEvent(new PointerEvent(name, { pointerId, pointerType: "touch", bubbles: true, cancelable: true }));
    },
    { name: type, pointerId: id },
  );
}

/** A computed style property of `el`. */
const style = (el: Locator, property: string) =>
  el.evaluate((node: Element, name: string) => getComputedStyle(node).getPropertyValue(name), property);

/** The data attributes of every `<tr>` of `cls` in a report. */
function rows(html: string, cls: string): Record<string, string>[] {
  return [...html.matchAll(/<tr\b[^>]*>/g)]
    .map((m) => m[0])
    .filter((tag) => new RegExp(`class="${cls}"`).test(tag))
    .map((tag) => Object.fromEntries([...tag.matchAll(/data-([a-z-]+)="([^"]*)"/g)].map((a) => [a[1], a[2]])));
}

/** Every tap target of the foh page and the rail's foot, as the user touches them. */
function controls(page: Page): [string, Locator][] {
  const hand2 = strip(page, "Hand2 #");
  return [
    ["fader", hand2.getByTestId("fader")],
    ["fader cap", hand2.getByTestId("fader").locator(".fader-cap")],
    ["pan", hand2.getByTestId("pan")],
    ["pan dot", hand2.getByTestId("pan").locator(".pan-dot")],
    ["mute", hand2.getByTestId("mute")],
    ["mute label", hand2.getByTestId("strip-label")],
    ["meter clip", hand2.getByTestId("clip")],
    ["solo", page.getByTestId("solo").first()],
    ["stage mics", page.getByTestId("stage-mics").first()],
    ["STAGE AUT", page.getByTestId("stage-aut").first()],
    ["param toggle", page.getByTestId("param-toggle").first()],
    ["param fader", page.getByTestId("param-fader").first().getByTestId("fader")],
    ["TechAlert", page.getByTestId("alert-toggle")],
    ["tab", page.getByTestId("tab").first()],
    ["SOLO clear", page.getByTestId("solo-clear")],
    ["dropout counter", page.getByTestId("dropouts")],
  ];
}

test("every control owns its touches: its touchstart is prevented, nothing on it can be dragged; the rest of the page keeps its touches", async ({ page }) => {
  await openSurface(page);
  await ready(strip(page, "Hand2 #").getByTestId("fader"));
  for (const [name, el] of controls(page)) {
    expect(await prevented(el, "touchstart"), `a touchstart on the ${name}`).toBe(true);
    // Not selectable is not checked here: both engines compute
    // `-webkit-user-select: none` for any child of the body's, before PR G
    // too, so only the drag tells the stylesheet apart.
    expect(await style(el, "-webkit-user-drag"), `the ${name} cannot be dragged`).toBe("none");
  }
  // Outside the controls a touch stays the browser's: a strip's dB scale, a
  // group's title and the instance it names, the rail, the column, a line,
  // the version label.
  const free: [string, Locator][] = [
    ["dB scale", strip(page, "Hand2 #").getByTestId("scale")],
    ["group title", page.getByTestId("group-title").first()],
    ["group instance", page.getByTestId("group-instance").first()],
    ["rail", page.getByTestId("rail")],
    ["column", page.getByTestId("column")],
    ["line", page.getByTestId("line").first()],
    ["version", page.getByTestId("stage").getByTestId("version")],
  ];
  for (const [name, el] of free) {
    expect(await prevented(el, "touchstart"), `a touchstart on the ${name}`).toBe(false);
  }
});

test("a line wider than the screen keeps its pinned strip; its arrows own their touches and move the rest", async ({ page }) => {
  // #63: a third row of 31 strips puts every row on one line (a tablet's and
  // a desktop's height are under three lines' 1 020 px), which does not fit:
  // the line shows its pinned strip and a window over the others, moved by
  // the column's arrows. The pinned strip, a master strip sixth in the row,
  // ends the first window (the right side) and starts the next (the left):
  // it changes side and must not be rebuilt. The group's strips differ, so
  // each shows its instance tag (#63), a part of a strip that is no control.
  const LAYOUT = join(__dirname, "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");
  const BAND = ["Hand1 #", "Hand2 #", "Hand3 #", "Hand4 #", "Vocal 1 repro#", "Vocal 2 repro#", "Vocal 3 repro#", "Keys 1", "Drums #", "Bass #"];
  const changed = JSON.parse(readFileSync(LAYOUT, "utf-8"));
  const foh = changed.pages.find((p: any) => p.id === "foh");
  const strips: any[] = [...BAND, ...BAND, ...BAND].map((name) => ({
    kind: "strip",
    binding: { instance: "band", anchor: { kind: "track", name } },
    strip_kind: "standard",
  }));
  strips.splice(5, 0, { kind: "strip", binding: { instance: "master", anchor: { kind: "track", name: "Hand1 #" } }, strip_kind: "standard", pinned: true });
  foh.rows.push({ sections: [{ kind: "group", id: "wide", title: "Wide", controls: strips }] });
  await openSurface(page);
  const group = page.locator('[data-testid="group"][data-group="wide"]');
  const cells = page.locator('.slot[data-group="wide"]');
  try {
    await harness("/hub/layout", { layout: changed });
    const shift = page.getByTestId("shift");
    await expect(shift).toHaveCount(1, { timeout: 10_000 });
    await expect(page.getByTestId("line")).toHaveCount(1);
    const pinned = cells.locator('[data-testid="strip"][data-instance="master"]');
    await expect(pinned).toBeVisible();
    const where = shift.getByTestId("shift-where");
    await expect(where).toHaveText(/^1–\d+\/\d+$/);
    const start = (await where.textContent())!;
    const next = shift.getByTestId("shift-next");
    const back = shift.getByTestId("shift-back");
    // The arrows own their touches; a line's background and a strip's tag keep theirs.
    expect(await prevented(next, "touchstart"), "a touchstart on ▶").toBe(true);
    expect(await prevented(back, "touchstart"), "a touchstart on ◀").toBe(true);
    expect(await prevented(page.getByTestId("line").first(), "touchstart"), "a touchstart on a line's background").toBe(false);
    expect(await prevented(cells.getByTestId("strip-instance").first(), "touchstart"), "a touchstart on a strip's tag").toBe(false);
    expect(await prevented(cells.getByTestId("fader").first(), "touchstart"), "a touchstart on a fader in it").toBe(true);
    // ▶ moves the window; the pinned strip stays, the same element (a
    // rebuilt strip would lose the mark, and a finger on it its touch).
    const slot = page.locator('.slot:has([data-testid="strip"][data-instance="master"][data-track="Hand1 #"])').filter({ has: pinned });
    await expect(slot).toHaveAttribute("data-side", "right");
    await pinned.evaluate((e) => e.setAttribute("data-e2e-kept", "1"));
    await next.dispatchEvent("pointerdown");
    await expect(where).not.toHaveText(start);
    await expect(slot).toHaveAttribute("data-side", "left");
    await expect(pinned).toBeVisible();
    await expect(pinned).toHaveAttribute("data-e2e-kept", "1");
    // ◀ brings the first window back.
    await back.dispatchEvent("pointerdown");
    await expect(where).toHaveText(start);
    await expect(slot).toHaveAttribute("data-side", "right");
    await expect(pinned).toHaveAttribute("data-e2e-kept", "1");
  } finally {
    await harness("/hub/layout/reset");
  }
  await expect(group).toHaveCount(0, { timeout: 10_000 });
  await expect(page.getByTestId("shift")).toHaveCount(0);
});

test("a context menu, a selection and a drag never start on the surface, and the page records them and the system's gestures", async ({ page, browserName }) => {
  await openSurface(page);
  const hand2 = strip(page, "Hand2 #");
  const fader = hand2.getByTestId("fader");
  await ready(fader);
  const from = await pageNow(page);

  // A real finger's tap on the fader's cap (WebKit's touchscreen; Chromium's
  // through the DevTools protocol) reaches the fader as Pointer Events,
  // although its touchstart is prevented, and leaves no system gesture: its
  // capture is lost only after the lift.
  const cap = await centre(fader.locator(".fader-cap"));
  const cdp = browserName === "chromium" ? await page.context().newCDPSession(page) : null;
  if (cdp) {
    await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ ...cap, id: 1 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  } else {
    await page.touchscreen.tap(cap.x, cap.y);
  }

  // On a control: prevented, recorded with the control's kind and keys.
  for (const type of ["contextmenu", "selectstart", "dragstart"]) {
    expect(await prevented(fader, type), `a ${type} on the fader`).toBe(true);
  }
  expect(await prevented(hand2.getByTestId("mute"), "contextmenu"), "a contextmenu on the mute").toBe(true);
  // Anywhere on the surface: prevented too.
  expect(await prevented(page.getByTestId("line").first(), "contextmenu"), "a contextmenu on a line").toBe(true);
  // A pinch's start and a cancelled pointer are only recorded (on a part of
  // the page that is no control: the instance its group names, #63).
  const tag = page.getByTestId("group-instance").first();
  expect(await prevented(tag, "gesturestart"), "a gesturestart").toBe(false);
  await pointer(tag, "pointercancel", 81);
  // A capture lost while the finger is still down is recorded; the one that
  // follows every lifted finger is not.
  await pointer(tag, "pointerdown", 82);
  await pointer(tag, "lostpointercapture", 82);
  await pointer(tag, "pointerup", 82);
  await pointer(tag, "lostpointercapture", 82);
  if (cdp) {
    // A zoom (WebKit's Playwright cannot pinch): to 2 and back to 1. The
    // visual viewport's resize fires once per frame with the scale of then,
    // so each scale is held for a few frames before the next.
    await cdp.send("Emulation.setPageScaleFactor", { pageScaleFactor: 2 });
    await until(() => page.evaluate(() => visualViewport!.scale), (s) => s === 2, "the page zoomed");
    await frames(page, 3);
    await cdp.send("Emulation.setPageScaleFactor", { pageScaleFactor: 1 });
    await until(() => page.evaluate(() => visualViewport!.scale), (s) => s === 1, "the page back at 1");
    await frames(page, 3);
  }
  const zooms = browserName === "chromium" ? 2 : 0;

  const lifted = (e: any) => e.ev === "touch" && e.what === "up" && e.keys.includes(FADER_KEY);
  const events = await until(
    async () => pageEvents(await hubEvents()).filter((e: any) => e.t >= from && ["sys", "zoom", "touch"].includes(e.ev)),
    (all) =>
      all.filter((e: any) => e.ev === "sys").length >= 8 &&
      all.filter((e: any) => e.ev === "zoom").length >= zooms &&
      all.some(lifted),
    "the page's tap and system-gesture records in the event log",
    10_000,
  );
  const taps = events.filter((e: any) => e.ev === "touch" && e.keys.includes(FADER_KEY)).map((e: any) => e.what);
  expect(taps, "the tap reached the fader: its down and its lift").toEqual(["down", "up"]);
  const sys = events.filter((e: any) => e.ev === "sys");
  const on = (what: string, where: string) => sys.filter((e: any) => e.what === what && e.on === where);
  for (const type of ["contextmenu", "selectstart", "dragstart"]) {
    expect(on(type, "fader"), `the fader's ${type}`).toEqual([
      expect.objectContaining({ keys: [FADER_KEY], prevented: true }),
    ]);
  }
  expect(on("contextmenu", "mute")).toEqual([expect.objectContaining({ keys: [MUTE_KEY], prevented: true })]);
  const lineMenu = on("contextmenu", "line");
  expect(lineMenu).toEqual([expect.objectContaining({ prevented: true })]);
  expect("keys" in lineMenu[0], "a line has no keys").toBe(false);
  expect(on("gesturestart", "group-instance")).toEqual([expect.objectContaining({ prevented: false })]);
  expect(on("pointercancel", "group-instance")).toEqual([expect.objectContaining({ pointer: 81, prevented: false })]);
  expect(on("lostpointercapture", "group-instance")).toEqual([expect.objectContaining({ pointer: 82, prevented: false })]);
  expect(sys.length, `only those: ${JSON.stringify(sys)}`).toBe(8);
  for (const e of sys) expect(typeof e.t, "a record's page time").toBe("number");
  const scales = events.filter((e: any) => e.ev === "zoom").map((e: any) => e.scale);
  expect(scales, "the zooms").toEqual(browserName === "chromium" ? [2, 1] : []);

  // The forensics timeline lists them; its stdout gives counts only.
  const to = (await pageNow(page)) + 1000;
  const report = await harness("/forensics/timeline", { from_ms: from - 1000, to_ms: to });
  expect(report.exit, report.stderr).toBe(0);
  // Each kind apart: nothing escaped (the surface prevented the context
  // menu, the selection and the drag), one pinch's start, one cancel, one
  // capture lost while the finger was down.
  expect(report.stdout).toMatch(/^system_events=8$/m);
  expect(report.stdout).toMatch(/^system_escaped=0$/m);
  expect(report.stdout).toMatch(/^gesturestarts=1$/m);
  expect(report.stdout).toMatch(/^pointer_cancels=1$/m);
  expect(report.stdout).toMatch(/^lost_captures=1$/m);
  expect(report.stdout).toMatch(new RegExp(`^zooms=${zooms}$`, "m"));
  expect(report.stdout).not.toContain("Hand2");
  const listed = rows(report.html, "sys");
  expect(listed.filter((r) => r.what === "contextmenu" && r.on === "fader" && r.prevented === "true")).toHaveLength(1);
  expect(listed.filter((r) => r.what === "lostpointercapture" && r.pointer === "82")).toHaveLength(1);
  expect(listed.filter((r) => r.what === "zoom").map((r) => r.scale)).toEqual(browserName === "chromium" ? ["2", "1"] : []);
});
