import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import {
  HUB_SOCKET,
  LiveClient,
  dispatchPointer,
  dispatchSnapped,
  frames,
  hostLine,
  hubEvents,
  openDetail,
  openSurface,
  pageEvents,
  ready,
  selectPage,
  strip,
  track,
  until,
} from "./support/live";

// A guarded mute acts by a hold (#162, the owner's request of 2026-10-29;
// it replaced spec F12's double tap within 500 ms, which nobody knew was
// needed): a press fills the name button over 1 s (RAIL_HOLD_MS, as the rail
// buttons), the line under the name says what the hold will do, at full
// what the lift does, and the lift mutes or unmutes, the only write. A lift
// before full shows the `hold` hint and writes nothing; a slide past 10 px,
// a hidden page and the next primary finger elsewhere (#124) end the press
// with nothing written. The channel detail's MUTE holds the same way. An
// unguarded mute stays one tap. The fixture's master Hand1 # (others) is in
// the TouchOSC project's `double_click_mute`, its band namesake is not.

const HAND1 = track("Hand1 #");
const HAND2 = track("Hand2 #");

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
  await live.set("master", HAND1, "mute", false);
});
test.afterEach(async () => {
  await live.set("master", HAND1, "mute", false);
  live.close();
});

/** The page clock (the flight recorder's `t`) now. */
function pageNow(page: Page): Promise<number> {
  return page.evaluate(() => performance.timeOrigin + performance.now());
}

/** The guarded mute's steps the page recorded since `since` (`rail` events of the button `mute`), once uploaded. */
async function muteSteps(since: number, ok: (steps: string[]) => boolean): Promise<string[]> {
  return until(
    async () =>
      pageEvents(await hubEvents())
        .filter((e: any) => e.ev === "rail" && e.button === "mute" && e.t >= since)
        .map((e: any) => e.what),
    ok,
    "the mute's steps uploaded",
    15_000,
  );
}

/** The page hidden or shown, as an iPad's lock or app switch does (deck.spec's `setHidden`). */
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
 * The page's socket through a route that can hold the hub's messages
 * (`connection.spec.ts`' `routeWebSocket`): while held they wait in order,
 * `release` sends them on. Registered before the page opens.
 */
async function holdableLink(page: Page) {
  let holding = false;
  const queue: { ws: WebSocketRoute; message: Parameters<WebSocketRoute["send"]>[0] }[] = [];
  await page.routeWebSocket(HUB_SOCKET, (ws) => {
    const server = ws.connectToServer();
    server.onMessage((message) => {
      if (holding) queue.push({ ws, message });
      else ws.send(message);
    });
  });
  return {
    hold: () => {
      holding = true;
    },
    held: () => queue.map((q) => String(q.message)),
    release: () => {
      holding = false;
      for (const q of queue.splice(0)) q.ws.send(q.message);
    },
  };
}

/** Whether a hub message carries an error item for master Hand1 #'s mute (a `values`/`subbed` item: `sub`, `error`). */
function muteError(message: string): boolean {
  let parsed: unknown;
  try {
    parsed = JSON.parse(message);
  } catch {
    return false;
  }
  const items: unknown[] = [];
  const walk = (v: unknown) => {
    if (Array.isArray(v)) v.forEach(walk);
    else if (v && typeof v === "object") {
      items.push(v);
      Object.values(v).forEach(walk);
    }
  };
  walk(parsed);
  return items.some((item: any) => typeof item.sub === "string" && /^master\|.*Hand1 #.*\|mute\|(true|false)$/.test(item.sub) && !!item.error);
}

type Look = { t: number; failed: boolean; holding: string | null; full: string | null; binding: string | null };

/** Records every change of the mute's look on the page clock (connection.spec's `watchLook`): a flash lasts 400 ms. */
async function watchLook(mute: Locator) {
  await mute.evaluate((el) => {
    const w = window as any;
    w.muteLooks = [];
    const note = () =>
      w.muteLooks.push({
        t: performance.now(),
        failed: el.classList.contains("failed"),
        holding: el.getAttribute("data-holding"),
        full: el.getAttribute("data-full"),
        binding: el.getAttribute("data-binding"),
      });
    note();
    w.muteWatch = new MutationObserver(note);
    w.muteWatch.observe(el, { attributes: true, attributeFilter: ["class", "data-holding", "data-full", "data-binding"] });
  });
}

/** The looks `watchLook` recorded, its observer stopped. */
async function watchedLooks(page: Page): Promise<Look[]> {
  return page.evaluate(() => {
    const w = window as any;
    w.muteWatch.disconnect();
    return w.muteLooks;
  });
}

/** The page clock at the start of each `failed` flash. */
function flashStarts(looks: Look[]): number[] {
  return looks.filter((look, i) => look.failed && !(i > 0 && looks[i - 1].failed)).map((look) => look.t);
}

/** The guarded Hand1 # name button on the others page, ready. */
async function guardedMute(page: Page): Promise<Locator> {
  await openSurface(page);
  await selectPage(page, "others");
  const mute = strip(page, "Hand1 #", "master").getByTestId("mute");
  await ready(mute);
  await expect(mute).toHaveAttribute("data-guarded", "true");
  return mute;
}

test.describe("A guarded mute (#162)", () => {
  test("a lift before 1 s shows the hint and writes nothing; a hold fills, says what the lift does and mutes, then unmutes, at the lift", async ({ page }) => {
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    const line = mute.getByTestId("mute-hold");
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);

    // A tap (a lift at 200 ms): the hint in the marks line's place,
    // nothing written, the hint gone after its 1.2 s (the recorder's steps,
    // below, show no write either).
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 200 }, { type: "pointerup" }]);
    await expect(line).toHaveText("hold");
    await expect(mute.getByTestId("strip-marks")).toBeHidden();
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);
    await expect(mute.getByTestId("strip-marks")).toBeAttached();
    expect(await live.get("master", HAND1, "mute")).toBe(false);

    // A 1 s hold: filling with its line before the full mark, full with the
    // lift's line after it (read in the page's timer order), nothing written
    // until the lift.
    const on = await dispatchSnapped(mute, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(on.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "hold… mute" });
    expect(on.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
    expect(await mute.locator(".hold").evaluate((el) => getComputedStyle(el).transitionDuration)).toBe("1s");
    expect(await live.get("master", HAND1, "mute"), "nothing before the lift").toBe(false);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === true, "muted at the lift");
    await expect(mute).toHaveAttribute("data-muted", "true");
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(line).toHaveCount(0);
    // The band's track of the same name is another instance's (spec F2, X4).
    expect(await live.get("band", HAND1, "mute")).toBe(false);

    // The same hold unmutes.
    const off = await dispatchSnapped(mute, [
      { type: "pointerdown" },
      { wait: 300 },
      { snap: true },
      { wait: 1000 },
      { snap: true },
    ]);
    expect(off.snaps[0]).toMatchObject({ holding: "true", full: "false", sub: "hold… unmute" });
    expect(off.snaps[1]).toMatchObject({ holding: "true", full: "true", sub: "lift: unmute" });
    expect(await live.get("master", HAND1, "mute")).toBe(true);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === false, "unmuted at the lift");
    await expect(mute).toHaveAttribute("data-muted", "false");

    // The flight recorder: the tap's hint, then each hold's press, full mark
    // and lift; no write but the two lifts.
    expect(await muteSteps(since, (s) => s.filter((w) => w === "hold").length >= 2)).toEqual([
      "press",
      "hint",
      "press",
      "full",
      "hold",
      "press",
      "full",
      "hold",
    ]);
  });

  test("a slide, a hidden page or the next primary finger elsewhere ends a press, full or not, with nothing written", async ({ page }) => {
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    // Another control: the same strip's fader.
    const other = strip(page, "Hand1 #", "master").getByTestId("fader");
    await ready(other);

    // A slide past 10 px after the full mark: nothing, no hint.
    await dispatchPointer(mute, [
      { type: "pointerdown" },
      { type: "pointermove", dx: 0, dy: 0 },
      { wait: 1200 },
      { type: "pointermove", dx: 0, dy: -11 },
      { type: "pointerup", dy: -11 },
    ]);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    // A hidden page ends a press; its later lift writes nothing.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 300 }]);
    await expect(mute).toHaveAttribute("data-holding", "true");
    await setHidden(page, true);
    await setHidden(page, false);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await dispatchPointer(mute, [{ wait: 1000 }, { type: "pointerup" }]);
    // A full press whose end never came (a dispatched pointer captures
    // nothing, #124's state) ends at the next primary finger on another
    // control.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 1200 }], 21);
    await expect(mute).toHaveAttribute("data-full", "true");
    await expect(mute.getByTestId("mute-hold")).toHaveText("lift: mute");
    await dispatchPointer(other.locator(".fader-cap"), [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 22);
    await expect(mute).toHaveAttribute("data-full", "false");
    await expect(mute).toHaveAttribute("data-holding", "false");

    expect(await muteSteps(since, (s) => s.filter((w) => w === "lost").length >= 2)).toEqual([
      "press",
      "full",
      "slid",
      "press",
      "lost",
      "press",
      "full",
      "lost",
    ]);
    expect(await live.get("master", HAND1, "mute"), "nothing written").toBe(false);
  });

  test("the channel detail's MUTE holds the same way, its line in the label's place", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "others");
    const detail = await openDetail(page, "Hand1 #", "master");
    const mute = detail.getByTestId("mute");
    await ready(mute);
    await expect(mute).toHaveAttribute("data-guarded", "true");
    const since = await pageNow(page);
    const label = mute.getByTestId("strip-label");
    await expect(label).toBeVisible();
    // A tap: the hint in the label's place, nothing written.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await expect(mute.getByTestId("mute-hold")).toHaveText("hold");
    await expect(label).toBeHidden();
    await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    await expect(label).toBeVisible();
    // A hold to full, then the lift mutes.
    const held = await dispatchSnapped(mute, [{ type: "pointerdown" }, { wait: 1300 }, { snap: true }]);
    expect(held.snaps[0]).toMatchObject({ holding: "true", full: "true", sub: "lift: mute" });
    expect(await live.get("master", HAND1, "mute")).toBe(false);
    await dispatchPointer(mute, [{ type: "pointerup" }]);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === true, "muted at the detail's lift");
    await expect(mute).toHaveAttribute("data-muted", "true");
    expect(await muteSteps(since, (s) => s.includes("hold"))).toEqual(["press", "hint", "press", "full", "hold"]);
  });

  test("a cancel or a lost capture ends a press with nothing written; a second finger is no press, and the first goes on", async ({ page }) => {
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    const line = mute.getByTestId("mute-hold");

    // A cancel after the full mark: nothing written, no hint.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 1200 }, { type: "pointercancel" }]);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await expect(mute).toHaveAttribute("data-full", "false");
    await expect(line).toHaveCount(0);
    // A lost capture before it, then the same pointer's lift: nothing.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 300 }, { type: "lostpointercapture" }]);
    await expect(mute).toHaveAttribute("data-holding", "false");
    await dispatchPointer(mute, [{ wait: 1000 }, { type: "pointerup" }]);
    await expect(line).toHaveCount(0);
    expect(await live.get("master", HAND1, "mute"), "nothing written").toBe(false);

    // A first finger holds; a second (not primary) presses, lifts and
    // cancels on the same button: no press, and the first one fills, goes
    // full and mutes at its own lift.
    await dispatchPointer(mute, [{ type: "pointerdown" }], 21);
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 50 }, { type: "pointerup" }], 22, false);
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 50 }, { type: "pointercancel" }], 23, false);
    await expect(mute).toHaveAttribute("data-holding", "true");
    await expect(mute).toHaveAttribute("data-full", "true");
    await expect(line).toHaveText("lift: mute");
    expect(await live.get("master", HAND1, "mute"), "nothing before the first finger's lift").toBe(false);
    await dispatchPointer(mute, [{ type: "pointerup" }], 21);
    await until(() => live.get("master", HAND1, "mute"), (v) => v === true, "muted at the first finger's lift");

    expect(await muteSteps(since, (s) => s.includes("hold"))).toEqual(["press", "full", "cancel", "press", "cancel", "press", "full", "hold"]);
  });

  test("a press whose track loses its value before the full mark is refused there with a flash; a down on a mute not ready is no press", async ({ page }) => {
    // The track renamed in Live under a held finger: the hub turns the
    // binding unresolved, so at the full mark the lift could not write.
    // The hub's answer is held until the down went, then let through at
    // once, so it reaches the page before the 1 s mark (checked below).
    const link = await holdableLink(page);
    const mute = await guardedMute(page);
    const since = await pageNow(page);
    const line = mute.getByTestId("mute-hold");
    try {
      link.hold();
      expect(await hostLine("master", 'rename "Hand1 #" "Hand1 X"')).toBe("RENAMED 1");
      await until(
        async () => link.held().some(muteError),
        (yes) => yes,
        "the hub's error for the renamed track's mute, held",
        5_000,
      );
      await watchLook(mute);
      const press = await dispatchSnapped(mute, [{ type: "pointerdown" }]);
      link.release();
      await expect(mute).toHaveAttribute("data-binding", "unresolved");
      await page.waitForTimeout(1500);
      const looks = await watchedLooks(page);
      const lost = looks.find((look) => look.binding === "unresolved");
      expect(lost, "the binding turned unresolved").toBeDefined();
      expect(lost!.t - press.start, "the error reached the page before the full mark (ms)").toBeLessThan(1000);
      const flashes = flashStarts(looks).map((t) => t - press.start);
      expect(flashes, "one refusal flash, at the full mark").toHaveLength(1);
      expect(flashes[0]).toBeGreaterThanOrEqual(1000);
      expect(looks.map((look) => look.full), "never full").not.toContain("true");
      // Its lift does nothing.
      await dispatchPointer(mute, [{ type: "pointerup" }]);
      await expect(line).toHaveCount(0);
      // A down on the mute while it is not ready (unresolved): no press,
      // no fill, no line, and its lift writes nothing.
      await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 300 }]);
      await expect(mute).toHaveAttribute("data-holding", "false");
      await dispatchPointer(mute, [{ wait: 1000 }, { type: "pointerup" }]);
      await expect(line).toHaveCount(0);
    } finally {
      link.release();
      await hostLine("master", 'rename "Hand1 X" "Hand1 #"');
    }
    await ready(mute);
    expect(await live.get("master", HAND1, "mute"), "nothing written").toBe(false);
    // A tap once ready again: its hint, so the steps before it are all
    // that was recorded.
    await dispatchPointer(mute, [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await expect(line).toHaveText("hold");
    expect(await muteSteps(since, (s) => s.includes("hint"))).toEqual(["press", "refused", "press", "hint"]);
  });

  test("an unguarded mute stays one tap on its down", async ({ page }) => {
    await live.set("band", HAND2, "mute", false);
    try {
      await openSurface(page);
      const mute = strip(page, "Hand2 #").getByTestId("mute");
      await ready(mute);
      await expect(mute).toHaveAttribute("data-guarded", "false");
      await expect(mute.locator(".hold")).toHaveCount(0);
      // The down writes, before any lift.
      await dispatchPointer(mute, [{ type: "pointerdown" }]);
      await until(() => live.get("band", HAND2, "mute"), (v) => v === true, "muted on the down");
      await dispatchPointer(mute, [{ type: "pointerup" }]);
      expect(await mute.getAttribute("data-holding"), "no hold state").toBeNull();
      await expect(mute.getByTestId("mute-hold")).toHaveCount(0);
    } finally {
      await live.set("band", HAND2, "mute", false);
    }
  });
});

test.describe("The detail's MUTE hold line (#162)", () => {
  /** A guarded mute's hold lines (behave::mute: hold_text, full_text, HOLD_HINT). */
  const HOLD_LINES = ["hold… mute", "hold… unmute", "lift: mute", "lift: unmute", "hold"];
  /** A linked mute's chain counts (#127; the widest a mixed two-digit total). */
  const LINKS = [null, "3", "1/3", "10/12"];

  test("each hold line is whole inside the MUTE and clear of a link's chain, on the tablet and both phones", async ({ page }) => {
    await openSurface(page);
    await selectPage(page, "others");
    const detail = await openDetail(page, "Hand1 #", "master");
    const mute = detail.getByTestId("mute");
    await ready(mute);
    for (const [width, height] of [
      [1194, 834],
      [844, 390],
      [390, 844],
    ]) {
      await page.setViewportSize({ width, height });
      await frames(page);
      // A copy of the real MUTE beside it (the same classes, attributes
      // and column), given each line and each chain as MuteView and
      // LinkMark draw them; the component's own element is left alone.
      const problems = await mute.evaluate(
        async (el, { lines, links }) => {
          await document.fonts.ready;
          const out: string[] = [];
          const copy = el.cloneNode(true) as HTMLElement;
          copy.removeAttribute("data-testid");
          copy.querySelectorAll(".mute-hold, .link-mark").forEach((n) => n.remove());
          el.parentElement!.insertBefore(copy, el);
          try {
            for (const link of links) {
              for (const text of lines) {
                copy.querySelectorAll(".mute-hold, .link-mark").forEach((n) => n.remove());
                const fill = copy.querySelector(".hold");
                let chain: HTMLElement | null = null;
                if (link) {
                  chain = document.createElement("span");
                  chain.className = "link-mark";
                  chain.dataset.state = "mixed";
                  chain.innerHTML = `<span class="link-n">${link}</span>`;
                  copy.insertBefore(chain, fill);
                }
                const said = document.createElement("span");
                said.className = "mute-hold";
                said.textContent = text;
                copy.insertBefore(said, fill);
                copy.setAttribute("data-says", "true");
                const name = `${text} beside ${link ?? "no chain"}`;
                const box = copy.getBoundingClientRect();
                const range = document.createRange();
                range.selectNodeContents(said);
                const t = range.getBoundingClientRect();
                const inside = (r: DOMRect) =>
                  r.left >= box.left - 0.5 && r.right <= box.right + 0.5 && r.top >= box.top - 0.5 && r.bottom <= box.bottom + 0.5;
                if (getComputedStyle(said).display === "none" || t.width === 0) out.push(`${name}: not drawn`);
                if (said.scrollWidth > said.clientWidth) out.push(`${name}: cut (${said.scrollWidth} > ${said.clientWidth})`);
                if (!inside(t)) out.push(`${name}: text outside the MUTE`);
                if (getComputedStyle(copy.querySelector(".strip-label")!).display !== "none") out.push(`${name}: the label still shows`);
                if (chain) {
                  const c = chain.getBoundingClientRect();
                  if (!inside(c)) out.push(`${name}: the chain outside the MUTE`);
                  if (t.left < c.right && t.right > c.left && t.top < c.bottom && t.bottom > c.top) out.push(`${name}: over the chain`);
                }
              }
            }
          } finally {
            copy.remove();
          }
          return out;
        },
        { lines: HOLD_LINES, links: LINKS },
      );
      expect(problems, `${width}x${height}`).toEqual([]);
    }
  });
});
