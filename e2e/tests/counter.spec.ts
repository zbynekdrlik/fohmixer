import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { hubEvents, impair, openSurface, until } from "./support/live";

// The dropout counter and the page's flight recorder (#43, PR C; design note
// §4.4, §5.2, §6.2 test 6): a small number in the top bar that rises by one
// per dropout, red while one lasts and neutral otherwise, reset to 0 by a
// tap; the page's own events (its dropouts and the reset among them) reach
// the hub's event log as `trace` records, and what the page saw while its
// link was down goes up after the reconnect. The pages reach the hub through
// the harness's impair proxy (`impair`).

/** `--danger`, the counter's colour while a dropout lasts. */
const RED = "rgb(255, 59, 48)";

/** The page's clock (`performance.timeOrigin + performance.now()`, the `t` of its events). */
const pageNow = (page: Page) => page.evaluate(() => performance.timeOrigin + performance.now());

/** One sample of the counter: when (page clock), its number, red or not, its colour. */
type Sample = { t: number; text: string; active: string; color: string };

/**
 * Records every change of the counter in the page, every animation frame
 * (a Playwright poll can miss a red phase of a few hundred ms).
 */
async function sampleCounter(counter: Locator) {
  await counter.evaluate((el: Element) => {
    const w = window as unknown as { counterSamples: Sample[] };
    w.counterSamples = [];
    let last = "";
    const step = () => {
      const text = el.textContent ?? "";
      const active = el.getAttribute("data-active") ?? "";
      if (`${text}/${active}` !== last) {
        last = `${text}/${active}`;
        w.counterSamples.push({ t: performance.timeOrigin + performance.now(), text, active, color: getComputedStyle(el).color });
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
}

const samples = (page: Page) => page.evaluate(() => (window as unknown as { counterSamples: Sample[] }).counterSamples);

/** The page's events in the hub's event log (every `trace` record's events), with the record. */
async function pageEvents(): Promise<{ ts: number; e: any }[]> {
  return (await hubEvents()).filter((r) => r.ev === "trace").flatMap((r) => r.events.map((e: any) => ({ ts: r.ts, e })));
}

test.describe("The dropout counter", () => {
  // A dropped link resets the page's socket: WebKit reports the reset of an
  // open socket as a console error (e2e.md; Chromium says nothing).
  test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

  test.afterEach(async () => {
    // Never leave the link held for the next test.
    await impair.block(false);
  });

  test("rises by one per dropout, is red only while one lasts, a tap resets it, and the event log holds the page's records", async ({ page }) => {
    await openSurface(page);
    const counter = page.getByTestId("dropouts");
    await expect(counter).toHaveText("0");
    await expect(counter).toHaveAttribute("data-active", "false");
    const neutral = await counter.evaluate((el) => getComputedStyle(el).color);
    expect(neutral, "neutral, not red").not.toBe(RED);
    // A number only: no words in the top bar.
    expect((await counter.textContent())?.trim()).toMatch(/^\d+$/);
    await sampleCounter(counter);

    // A stall of 800 ms (both directions held): one dropout, red while it
    // lasts, neutral after. (A 400 ms stall is a dropout too, but whether the
    // page's 100 ms tick sees it lasting depends on the ping's phase.)
    const stallFrom = await pageNow(page);
    await impair.stall(800);
    await expect.poll(async () => (await samples(page)).some((s) => s.active === "true"), { timeout: 3000 }).toBe(true);
    await expect(counter).toHaveAttribute("data-active", "false", { timeout: 3000 });
    await expect(counter).toHaveText("1");
    const stallTo = await pageNow(page);
    const stalled = await samples(page);
    const red = stalled.filter((s) => s.active === "true");
    expect(red.length, "red once").toBe(1);
    expect(red[0].text, "counted at its start").toBe("1");
    expect(red[0].color).toBe(RED);
    expect(red[0].t).toBeGreaterThan(stallFrom);
    expect(stalled[stalled.length - 1], "neutral after the stall").toMatchObject({ text: "1", active: "false", color: neutral });

    // The link drops (the socket reset, the reconnect held): one more, red
    // until the next hello.
    const droppedAt = await pageNow(page);
    await impair.block(true);
    await impair.drop();
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
    await expect(counter).toHaveText("2");
    await expect(counter).toHaveAttribute("data-active", "true");
    await expect(counter).toHaveCSS("color", RED);
    await page.waitForTimeout(800);
    await expect(counter, "red while the link stays down").toHaveAttribute("data-active", "true");
    const unblockedAt = await pageNow(page);
    await impair.block(false);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 5000 });
    await expect(counter).toHaveAttribute("data-active", "false");
    await expect(counter).toHaveText("2");
    await expect(counter).toHaveCSS("color", neutral);

    // A tap resets it to 0.
    const tappedFrom = await pageNow(page);
    await counter.click();
    await expect(counter).toHaveText("0");
    await expect(counter).toHaveAttribute("data-active", "false");

    // The hub's event log holds the page's records: the stall's dropout, the
    // lost socket's, the reset, and what the page saw while its link was
    // down (its socket's close), sent up after the reconnect.
    const events = await until(
      pageEvents,
      (all) => all.some(({ e }) => e.ev === "reset" && e.t >= tappedFrom),
      "the reset in the event log",
      10_000,
    );
    const stallDropout = events.find(({ e }) => e.ev === "dropout" && !e.socket_lost && e.t >= stallFrom - 100 && e.t <= stallTo);
    expect(stallDropout, "the stall's dropout").toBeTruthy();
    expect(stallDropout!.e.ms).toBeGreaterThanOrEqual(300);
    const lostDropout = events.find(({ e }) => e.ev === "dropout" && e.socket_lost && e.t >= droppedAt - 100);
    expect(lostDropout, "the lost socket's dropout").toBeTruthy();
    const reset = events.find(({ e }) => e.ev === "reset" && e.t >= tappedFrom)!;
    expect(reset.e).toMatchObject({ count: 2, active: false });
    const closed = events.find(({ e }) => e.ev === "sock" && e.what === "close" && e.t >= droppedAt && e.t <= unblockedAt);
    expect(closed, "the socket's close, seen by the page while its link was down").toBeTruthy();
    expect(closed!.ts, "sent up after the reconnect").toBeGreaterThanOrEqual(Math.floor(unblockedAt) - 50);
    // The recorder's round trips are there too: one summary a second of
    // its pongs (#43 PR D; no event per pong).
    expect(
      events.some(({ e }) => e.ev === "rtt" && e.n >= 1 && typeof e.med === "number" && e.max >= e.min && e.t >= stallFrom - 1000),
    ).toBe(true);
    expect(events.some(({ e }) => e.ev === "pong"), "no event per pong").toBe(false);
  });
});
