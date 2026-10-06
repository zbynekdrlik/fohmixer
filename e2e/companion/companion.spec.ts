import type { Locator, Page, WebSocketRoute } from "@playwright/test";
import { expect, test } from "../tests/support/fixtures";
import { HUB_SOCKET, deckKey, dispatchPointer, hubEvents, openDeck, openSurface, until } from "../tests/support/live";

// The Stream Deck tab against the real Companion 5.0.7 (#52, ci.yml
// `companion`): Companion's official container, seeded with the synthetic
// e2e/companion/test.companionconfig (key 0 "Light A" toggles light_a, key 1
// "Scene 1" sets scene_1 short on a release and long after a 1 s hold, key 2
// "Hold C" sets hold_c down and up), the hub registered on its Satellite
// API. Companion's own state is read through its HTTP API, never through
// the hub. Both projects run against one Companion, so each test starts from
// a state its check changes, and the `afterEach` puts back every variable a
// test remembered, also after a failure: one project's failure never spends
// the other's starting state.

const COMPANION = process.env.COMPANION_URL || "http://127.0.0.1:8000";
/** Every key image Companion sends the hub, which asks `BITMAP_FORMAT=webp`. */
const WEBP = "data:image/webp;base64,";

/** A custom variable of Companion, as Companion's HTTP API reads it. */
async function variable(name: string): Promise<string> {
  const response = await fetch(`${COMPANION}/api/custom-variable/${name}/value`);
  if (!response.ok) throw new Error(`Companion's ${name}: ${response.status}`);
  return (await response.text()).trim();
}

/** Sets a custom variable of Companion through its HTTP API, and reads it back. */
async function setVariable(name: string, value: string) {
  const response = await fetch(`${COMPANION}/api/custom-variable/${name}/value?value=${encodeURIComponent(value)}`, {
    method: "POST",
  });
  if (!response.ok) throw new Error(`setting Companion's ${name}: ${response.status}`);
  await until(() => variable(name), (v) => v === value, `Companion's ${name} put back to ${value}`, 2000);
}

/** The variables the running test changes, with their values before it. */
const changed = new Map<string, string>();

/** Companion's `name` now, remembered so the `afterEach` puts it back. */
async function remember(name: string): Promise<string> {
  const value = await variable(name);
  if (!changed.has(name)) changed.set(name, value);
  return value;
}

/** A tap of `key`: down, 100 ms, up (dispatched pointer events of one finger). */
const tap = (page: Page, key: number) =>
  dispatchPointer(deckKey(page, key), [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);

/** A key as the page shows it: Companion's pressed state, whether its image is `src`, and whether it is webp. */
const look = (key: Locator, src: string) =>
  key.evaluate(
    (el, { first, prefix }) => {
      const img = el.querySelector("img");
      return {
        pressed: el.getAttribute("data-pressed"),
        first: img?.getAttribute("src") === first,
        webp: !!img?.src.startsWith(prefix),
      };
    },
    { first: src, prefix: WEBP },
  );

test.describe("The Stream Deck tab against Companion 5.0.7", () => {
  test.afterEach(async () => {
    const remembered = [...changed];
    changed.clear();
    for (const [name, value] of remembered) {
      if ((await variable(name)) !== value) await setVariable(name, value);
    }
  });

  test("shows Companion's 32 keys, each a webp image that renders", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const images = page.locator('[data-testid="deck-key"] img');
    await expect(images).toHaveCount(32);
    // Every key's image, the empty keys' too, is Companion's webp data URL...
    const notWebp = await images.evaluateAll(
      (els, prefix) =>
        els.flatMap((e) =>
          (e as HTMLImageElement).src.startsWith(prefix) ? [] : [e.closest("[data-key]")?.getAttribute("data-key")],
        ),
      WEBP,
    );
    expect(notWebp).toEqual([]);
    // ...and the browser draws it: the e2e job's fake Companion sends PNGs, so
    // this is the only proof that WebKit (the iPad) renders Companion's webp.
    await expect
      .poll(() =>
        images.evaluateAll((els) =>
          els.flatMap((e) => {
            const img = e as HTMLImageElement;
            return img.complete && img.naturalWidth > 0 ? [] : [e.closest("[data-key]")?.getAttribute("data-key")];
          }),
        ),
      )
      .toEqual([]);
  });

  test("a tap runs Light A's action and Companion's new image reaches the page", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const key = deckKey(page, 0);
    await expect(key).toHaveAttribute("data-pressed", "false");
    const before = await remember("light_a");
    const toggled = before === "on" ? "off" : "on";
    const src = (await key.locator("img").getAttribute("src")) ?? "";
    expect(src.startsWith(WEBP)).toBe(true);
    await tap(page, 0);
    await until(() => variable("light_a"), (v) => v === toggled, "Light A toggled in Companion");
    // Companion draws the key pressed, then released with light_a's new
    // value: the released image is another one.
    await until(
      () => look(key, src),
      (l) => l.pressed === "false" && !l.first && l.webp,
      "Light A's new released image on the page",
    );
    // The second tap puts light_a back, and the page shows the first image
    // again (Companion draws a state the same way every time).
    await tap(page, 0);
    await until(() => variable("light_a"), (v) => v === before, "Light A back in Companion");
    await until(() => look(key, src), (l) => l.pressed === "false" && l.first, "Light A's first image back on the page");
  });

  test("a 1.5 s hold runs Scene 1's duration action and a short tap does not", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    // idle (put back after every test): never short, so the short tap's
    // result is a change. Companion runs either the release group or the 1 s
    // group on a release, so short also says the 1 s group did not run.
    expect(await remember("scene_1")).not.toBe("short");
    await tap(page, 1);
    await until(() => variable("scene_1"), (v) => v === "short", "the release action of a short tap");
    await dispatchPointer(deckKey(page, 1), [{ type: "pointerdown" }, { wait: 1500 }, { type: "pointerup" }]);
    await until(() => variable("scene_1"), (v) => v === "long", "the 1 s duration action of a 1.5 s hold");
  });

  test("closing the page's socket mid-hold leaves Hold C released in Companion", async ({ page }) => {
    const sockets: WebSocketRoute[] = [];
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      ws.connectToServer();
      sockets.push(ws);
    });
    await openSurface(page);
    await openDeck(page);
    expect(await remember("hold_c")).toBe("up");
    const since = Date.now();
    await dispatchPointer(deckKey(page, 2), [{ type: "pointerdown" }]);
    await until(() => variable("hold_c"), (v) => v === "down", "Hold C held in Companion");
    await expect(deckKey(page, 2)).toHaveAttribute("data-held", "true");
    // The page's socket closes under the finger (Playwright closes the hub's
    // side with the same code): the hub releases the key itself (`detach`).
    expect(sockets).toHaveLength(1);
    await sockets[0].close({ code: 4000, reason: "gone" });
    await until(() => variable("hold_c"), (v) => v === "up", "Hold C released by the hub", 5000);
    const releases = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "deck_release" && r.ts >= since),
      (r) => r.length >= 1,
      "the hub's release in its event log",
    );
    expect(releases.map((r: any) => [r.key, r.reason])).toEqual([[2, "detach"]]);
    await expect(deckKey(page, 2)).toHaveAttribute("data-held", "false");
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    // The finger lifts after the reconnect: the page forgot the hold, so no finger stays down.
    await dispatchPointer(deckKey(page, 2), [{ type: "pointerup" }]);
  });
});
