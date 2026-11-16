import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { hostLine, hubEvents, hubStatus, openDetail, openSurface, pageEvents, token, until } from "./support/live";

// Listen (#210 phase 2; the owner's approved mockup docs/mockups/listen-v1.html):
// the channel detail's 🎧 key beside the chip starts listening to that
// channel (before its fader), a slim volume shows beside it, a tap again
// stops; another channel's detail is followed while listening; the column
// foot says `listening <name>`. The sound comes from SimLive's `FohMixer
// Listen` device (sim/listendev.py: a sine), streamed to the hub
// (`[listen]`, the harness's --listen-port) and from the hub's /ws/audio
// to the page, which decodes the Opus with WebCodecs and plays it. One
// track at a time on a hub: the newest start takes it (`taken`).

/**
 * Band Hand2 # is band track 1; band A-Reverb # is return 0. `label`: what
 * the strip shows (`strip_label`: a return's name without its letter).
 */
const HAND2 = { name: "Hand2 #", where: "track 1", label: "Hand2" };
const REVERB = { name: "A-Reverb #", where: "return 0", label: "Reverb" };

type Stats = {
  running: boolean;
  limiter: boolean;
  packets: number;
  decoded: number;
  played: number;
  level: number;
  gain: number;
  error: string | null;
};

async function stats(page: Page): Promise<Stats> {
  return page.evaluate(() => (window as unknown as { __fohmixerListen: () => Stats }).__fohmixerListen());
}

function key(detail: Locator): Locator {
  return detail.getByTestId("listen-key");
}

function footLine(page: Page): Locator {
  return page.getByTestId("listen-line");
}

/** The hub's track listened to now (none: nothing). */
async function hubTarget(bearer: string): Promise<{ track: string; listeners: number } | null> {
  return (await hubStatus(bearer)).listen?.target ?? null;
}

test.describe("Listen to a fader in the browser (#210)", () => {
  test.beforeEach(async () => {
    // A Listen device in a rack on each track the tests listen to, each
    // playing its own tone.
    for (const [t, hz] of [
      [HAND2, 440],
      [REVERB, 660],
    ] as const) {
      expect(await hostLine("band", `listendev ${t.where} add rack`)).toBe("LISTENDEV 1");
      expect(await hostLine("band", `tone ${t.where} ${hz}`)).toBe(`TONE ${hz}`);
    }
    const bearer = await token();
    await until(
      async () => (await hubStatus(bearer)).listen?.devices?.length ?? 0,
      (n) => n >= 2,
      "the hub hears both Listen devices",
    );
  });

  test.afterEach(async ({ page }) => {
    // The page's socket goes with the page; nothing listens afterwards.
    await page.goto("about:blank");
    for (const t of [HAND2, REVERB]) {
      await hostLine("band", `listendev ${t.where} remove`);
    }
    const bearer = await token();
    await until(() => hubTarget(bearer), (t) => t === null, "nothing listened to");
  });

  test("the key plays the channel's sound, the foot says so, a tap again stops", async ({ page }) => {
    const bearer = await token();
    const since = Date.now();
    await openSurface(page);
    await expect(footLine(page)).toHaveCount(0);
    let detail = await openDetail(page, HAND2.name);
    // Calm while idle: no volume, nothing lit.
    await expect(key(detail)).toHaveAttribute("aria-pressed", "false");
    await expect(key(detail)).toHaveAttribute("data-state", "off");
    await expect(detail.getByTestId("listen-volume")).toHaveCount(0);
    const idle = await key(detail).evaluate((el) => getComputedStyle(el).opacity);
    expect(Number(idle)).toBeLessThan(1);

    await key(detail).click();
    await expect(key(detail)).toHaveAttribute("aria-pressed", "true");
    await expect(key(detail)).toHaveAttribute("data-state", "listening");
    await expect(detail.getByTestId("listen-volume")).toBeVisible();
    await expect(footLine(page)).toHaveText(`listening ${HAND2.label}`);
    await until(() => hubTarget(bearer), (t) => t?.track === HAND2.name && t.listeners === 1, "the hub sends Hand2 #");

    // The page decodes and plays the device's tone (0.5 at 440 Hz: about -6 dBFS).
    const playing = await until(
      () => stats(page),
      (s) => s.decoded > 25 && s.played > 25 && s.level > -12 && s.level < 0,
      "the page plays the tone",
      15000,
    );
    expect(playing.error).toBeNull();
    expect(playing.running).toBe(true);
    expect(playing.limiter).toBe(true);

    // Back to the mix: still listening, the foot says what.
    await detail.getByTestId("detail-exit").click();
    await expect(page.getByTestId("detail")).toHaveCount(0);
    await expect(footLine(page)).toHaveText(`listening ${HAND2.label}`);
    const before = (await stats(page)).played;
    await until(() => stats(page), (s) => s.played > before + 10, "it plays on behind the mix");

    // Stop from the detail.
    detail = await openDetail(page, HAND2.name);
    await expect(key(detail)).toHaveAttribute("aria-pressed", "true");
    await key(detail).click();
    await expect(key(detail)).toHaveAttribute("aria-pressed", "false");
    await expect(detail.getByTestId("listen-volume")).toHaveCount(0);
    await expect(footLine(page)).toHaveCount(0);
    await until(() => hubTarget(bearer), (t) => t === null, "the hub stopped sending");

    // The page's flight recorder: the tap, the open, the hub's word, the stop.
    const steps = await until(
      async () =>
        pageEvents(await hubEvents()).filter((e) => e.ev === "listen" && e.t >= since),
      (es) => es.some((e) => e.what === "tap" && e.phase === "off"),
      "the page's listen steps",
      15000,
    );
    expect(steps.map((e) => `${e.what}:${e.phase}`)).toEqual([
      "tap:opening",
      "open:opening",
      "status:listening",
      // The detail opened again on the same channel: followed, nothing sent.
      "follow:listening",
      "tap:off",
    ]);
    expect(steps[0]).toMatchObject({ instance: "band", track: HAND2.name });
  });

  test("another channel's detail is followed while listening", async ({ page }) => {
    const bearer = await token();
    await openSurface(page);
    let detail = await openDetail(page, HAND2.name);
    await key(detail).click();
    await until(() => hubTarget(bearer), (t) => t?.track === HAND2.name, "Hand2 # heard");
    await detail.getByTestId("detail-exit").click();
    detail = await openDetail(page, REVERB.name);
    // The key lit for the new channel: it is followed, not stopped.
    await expect(key(detail)).toHaveAttribute("aria-pressed", "true");
    await expect(key(detail)).toHaveAttribute("data-state", "listening");
    await until(() => hubTarget(bearer), (t) => t?.track === REVERB.name && t.listeners === 1, "A-Reverb # heard");
    await expect(footLine(page)).toHaveText(`listening ${REVERB.label}`);
    const before = (await stats(page)).played;
    await until(() => stats(page), (s) => s.played > before + 10 && s.level > -20, "the return's tone plays");
    await key(detail).click();
    await expect(footLine(page)).toHaveCount(0);
  });

  test("the volume follows a finger and only changes the page's level", async ({ page }) => {
    await openSurface(page);
    const detail = await openDetail(page, HAND2.name);
    await key(detail).click();
    const volume = detail.getByTestId("listen-volume");
    await expect(volume).toHaveAttribute("data-value", "70");
    const box = await volume.boundingBox();
    if (!box) throw new Error("no volume box");
    // A real drag from its middle to a quarter, then past its left end.
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 4, box.y + box.height / 2, { steps: 5 });
    // The page reads whole px: about a quarter, the gain on its dB law
    // (-40 dB at the left end, +12 dB at the other).
    await until(
      async () => Number(await volume.getAttribute("data-value")),
      (v) => v >= 23 && v <= 27,
      "the volume at about a quarter",
    );
    const place = Number(await volume.getAttribute("data-value")) / 100;
    const want = Math.pow(10, (-40 + 52 * place) / 20);
    const quarter = (await stats(page)).gain;
    expect(quarter / want).toBeGreaterThan(0.96);
    expect(quarter / want).toBeLessThan(1.04);
    await page.mouse.move(box.x - 20, box.y + box.height / 2, { steps: 3 });
    await page.mouse.up();
    await expect(volume).toHaveAttribute("data-value", "0");
    expect((await stats(page)).gain).toBe(0);
    // The touch target owns its touches (no loupe, no scroll).
    const prevented = await volume.evaluate((el) => {
      const ev = new Event("touchstart", { bubbles: true, cancelable: true });
      el.dispatchEvent(ev);
      return ev.defaultPrevented;
    });
    expect(prevented).toBe(true);
    await key(detail).click();
  });

  test("the newest start takes the hub's one track: the other page is told taken", async ({ page, context }) => {
    const bearer = await token();
    await openSurface(page);
    const first = await openDetail(page, HAND2.name);
    await key(first).click();
    await until(() => hubTarget(bearer), (t) => t?.track === HAND2.name, "Hand2 # heard");

    const other = await context.newPage();
    const problems: string[] = [];
    other.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") problems.push(`[${msg.type()}] ${msg.text()}`);
    });
    other.on("pageerror", (error) => problems.push(`[pageerror] ${error.message}`));
    await openSurface(other);
    const second = await openDetail(other, REVERB.name);
    await key(second).click();
    await until(() => hubTarget(bearer), (t) => t?.track === REVERB.name, "A-Reverb # taken");

    await expect(key(first)).toHaveAttribute("aria-pressed", "false");
    await expect(key(first)).toHaveAttribute("data-state", "taken");
    await expect(footLine(page)).toHaveText(`${HAND2.label}: taken by another page`);
    await expect(key(second)).toHaveAttribute("data-state", "listening");
    // A tap on the taken page starts it again and takes it back.
    await key(first).click();
    await expect(key(first)).toHaveAttribute("data-state", "listening");
    await expect(key(second)).toHaveAttribute("data-state", "taken");
    await until(() => hubTarget(bearer), (t) => t?.track === HAND2.name, "Hand2 # taken back");
    await other.close();
    expect(problems).toEqual([]);
  });

  test("a channel with no Listen device says no source", async ({ page }) => {
    await openSurface(page);
    // Band's B-Main repro # carries no Listen device. (Master's tracks
    // would not do: both SimLive hosts load the same set, and the hub picks
    // a device by its set's and track's names.)
    const detail = await openDetail(page, "B-Main repro #");
    await key(detail).click();
    await expect(key(detail)).toHaveAttribute("data-state", "no_source", { timeout: 8000 });
    await expect(key(detail)).toHaveAttribute("aria-pressed", "true");
    await expect(footLine(page)).toHaveText("Main: no source");
    expect((await stats(page)).decoded).toBe(0);
    await key(detail).click();
    await expect(footLine(page)).toHaveCount(0);
  });
});
