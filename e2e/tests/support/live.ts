import { expect, type Locator, type Page } from "@playwright/test";

// The E2E world: the hub (E2E_BASE_URL) with two SimLive hosts (band, master)
// started by e2e/harness/harness.py, whose control API (E2E_HARNESS_URL) can
// stall, rename, restart a host, restart the hub and swap the layout. The
// engineer PIN comes from the job (E2E_PIN, masked). The layout is the import
// tool's synthetic fixture (tools/import-tosc/fixtures/expected-layout.json).

export const BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8480";
export const HARNESS = process.env.E2E_HARNESS_URL || "http://127.0.0.1:39190";
export const PIN = process.env.E2E_PIN || "";

/** The LOM target of a track by name (the layout's binding form). */
export const track = (name: string) => `live_set tracks[name=${name}]`;
/** The LOM target of a return track by name. */
export const ret = (name: string) => `live_set return_tracks[name=${name}]`;
export const volume = (target: string) => `${target} mixer_device volume`;
export const panning = (target: string) => `${target} mixer_device panning`;

/** Calls the harness's control API. */
export async function harness(path: string, body: object = {}): Promise<any> {
  const response = await fetch(`${HARNESS}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const answer = await response.json();
  if (!response.ok) throw new Error(`harness ${path}: ${response.status} ${JSON.stringify(answer)}`);
  return answer;
}

/** A control line on a host's stdin (`stall <ms>`, `rename "<a>" "<b>"`): its answer. */
export async function hostLine(instance: string, line: string): Promise<string | null> {
  return (await harness(`/host/${instance}/line`, { line })).answer;
}

/** A fresh engineer token from the hub. */
export async function token(): Promise<string> {
  const response = await fetch(`${BASE}/api/auth`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ pin: PIN }),
  });
  if (!response.ok) throw new Error(`login with the E2E PIN failed: ${response.status}`);
  return (await response.json()).token;
}

/** `GET /api/status`. */
export async function hubStatus(): Promise<any> {
  const response = await fetch(`${BASE}/api/status`, { headers: { authorization: `Bearer ${await token()}` } });
  if (!response.ok) throw new Error(`status: ${response.status}`);
  return response.json();
}

/** Every client subscription the hub holds, over all instances. */
export async function hubSubscriptions(): Promise<number> {
  const status = await hubStatus();
  return status.instances.reduce((n: number, i: any) => n + i.subscriptions, 0);
}

/** How long the test's client waits for a command's answer. */
const CMD_TIMEOUT_MS = 10_000;

/**
 * A second client of the hub (the test's own): reads and writes Live the way
 * the surface does, to set up a state and to check what the surface wrote.
 */
export class LiveClient {
  private ws: WebSocket;
  private next = 0;
  private waiting = new Map<string, (msg: any) => void>();
  hub = new Map<string, unknown>();

  private constructor(ws: WebSocket) {
    this.ws = ws;
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(String(event.data));
      if (msg.type === "hub") this.hub.set(msg.key, msg.value);
      if ((msg.type === "result" || msg.type === "error") && msg.id && this.waiting.has(msg.id)) {
        this.waiting.get(msg.id)!(msg);
        this.waiting.delete(msg.id);
      }
    });
    // A closed socket (a hub restart) answers every waiting command with an
    // error instead of leaving it hanging.
    ws.addEventListener("close", () => {
      for (const [id, done] of this.waiting) done({ type: "error", id, message: "the test client's socket closed" });
      this.waiting.clear();
    });
  }

  static async open(): Promise<LiveClient> {
    const ws = new WebSocket(`${BASE.replace(/^http/, "ws")}/ws?token=${await token()}&proto=1`);
    const client = new LiveClient(ws);
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener("message", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error("the test client could not connect")), { once: true });
    });
    return client;
  }

  close() {
    this.ws.close();
  }

  /** A command batch on an instance: the script's result slots. */
  async cmd(instance: string, commands: object[]): Promise<any[]> {
    if (this.ws.readyState !== WebSocket.OPEN) throw new Error("the test client's socket is not open");
    const id = `e2e${++this.next}`;
    const answer = new Promise<any>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        reject(new Error(`no answer to ${JSON.stringify(commands)} within ${CMD_TIMEOUT_MS} ms`));
      }, CMD_TIMEOUT_MS);
      this.waiting.set(id, (msg) => {
        clearTimeout(timer);
        resolve(msg);
      });
    });
    this.ws.send(JSON.stringify({ type: "cmd", id, instance, commands }));
    const msg = await answer;
    if (msg.type === "error") throw new Error(`cmd ${JSON.stringify(commands)}: ${msg.message}`);
    return msg.data;
  }

  /** Calls `name` on a target; its data (the slot must be ok). */
  async call(instance: string, target: string, name: string, args: object = {}): Promise<any> {
    const [slot] = await this.cmd(instance, [{ target, name, args }]);
    if (!slot.ok) throw new Error(`${name} ${target}: ${JSON.stringify(slot)}`);
    return slot.data;
  }

  get(instance: string, target: string, prop: string): Promise<any> {
    return this.call(instance, target, "get_prop", { prop });
  }

  set(instance: string, target: string, prop: string, value: unknown): Promise<any> {
    return this.call(instance, target, "set_prop", { prop, value });
  }

  /** Live's display string of a parameter's value. */
  display(instance: string, target: string, value: number): Promise<string> {
    return this.call(instance, target, "str_for_value", { value });
  }

  setHub(key: string, value: unknown) {
    this.ws.send(JSON.stringify({ type: "set_hub", key, value }));
  }
}

/** Waits until `read()` satisfies `ok`; its last value. */
export async function until<T>(read: () => Promise<T>, ok: (v: T) => boolean, what: string, timeout = 8000): Promise<T> {
  const deadline = Date.now() + timeout;
  let last = await read();
  while (!ok(last)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(last)}`);
    await new Promise((r) => setTimeout(r, 50));
    last = await read();
  }
  return last;
}

/** Opens the surface logged in (the token seeded once per tab), after the automatic refresh. */
export async function openSurface(page: Page) {
  const seeded = await token();
  await page.addInitScript((t) => {
    try {
      if (!sessionStorage.getItem("e2e-seeded")) {
        localStorage.setItem("fohmixer_token", t);
        sessionStorage.setItem("e2e-seeded", "1");
      }
    } catch {
      // storage blocked: the test then fails on the login page
    }
  }, seeded);
  await page.goto("/");
  await expect(page.getByTestId("stage")).toBeVisible();
  await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
  // The automatic refresh (spec F6) resubscribes everything a second after
  // load; tests act after it.
  await expect(page.getByTestId("surface")).not.toHaveAttribute("data-refreshes", "0", { timeout: 5000 });
}

/** A strip by its track name and instance. */
export function strip(page: Page, name: string, instance = "band"): Locator {
  return page.locator(`[data-testid="strip"][data-track="${name}"][data-instance="${instance}"]`);
}

/** Selects a tab by its page id. */
export async function selectPage(page: Page, id: string) {
  const tab = page.locator(`[data-testid="tab"][data-page="${id}"]`);
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
}

/** Waits until a control shows Live's value (I8: then it takes input). */
export async function ready(control: Locator) {
  await expect(control).toHaveAttribute("aria-disabled", "false");
}

/** The centre of an element on the page. */
export async function centre(control: Locator): Promise<{ x: number; y: number }> {
  const box = await control.boundingBox();
  if (!box) throw new Error("the control is not on screen");
  return { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };
}

/**
 * Two taps on a control, `gapMs` from the first release to the second press,
 * timed inside the page: the fader's double tap wants its releases 50–250 ms
 * apart, and one real click takes ~160 ms in WebKit on the CI runner, so two
 * real clicks cannot hit that window in both engines. The taps are the same
 * pointer events a finger sends (the WebKit multi-touch path sends them too).
 */
export async function doubleTap(control: Locator, gapMs = 100) {
  const { x, y } = await centre(control);
  await control.evaluate(
    async (el, [clientX, clientY, gap]) => {
      const fire = (type: string) =>
        el.dispatchEvent(
          new PointerEvent(type, {
            pointerId: 21,
            pointerType: "touch",
            isPrimary: true,
            clientX,
            clientY,
            bubbles: true,
            cancelable: true,
          }),
        );
      const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));
      fire("pointerdown");
      await wait(20);
      fire("pointerup");
      await wait(gap);
      fire("pointerdown");
      await wait(20);
      fire("pointerup");
    },
    [x, y, gapMs],
  );
}

/**
 * A toggle's double tap (TouchOSC's 200 ms window between the two presses):
 * the two presses as one double click. Two separate clicks with a pause can
 * miss the window in WebKit, where one click takes ~160 ms on the CI runner.
 */
export async function toggleDoubleTap(page: Page, control: Locator) {
  const { x, y } = await centre(control);
  await page.mouse.dblclick(x, y);
}

/** The value a fader or pan shows (its `data-value`, Live's units). */
/**
 * Waits `n` animation frames in the page. The surface draws a control (its
 * `data-value` too) in its animation frame, so a pointer move shows at the
 * next frame: read a control after its last move only once this returns.
 */
export async function frames(page: Page, n = 2): Promise<void> {
  await page.evaluate(
    (count) =>
      new Promise<void>((resolve) => {
        const step = (left: number) => {
          if (left === 0) resolve();
          else requestAnimationFrame(() => step(left - 1));
        };
        step(count);
      }),
    n,
  );
}

export async function shown(control: Locator): Promise<number> {
  return Number(await control.getAttribute("data-value"));
}

/**
 * How an element's text is cut off by its box ([] when the whole text fits):
 * the box's scroll size against its client size, and the laid-out text's
 * rectangle against the box's (both in the page, so the stage's scale and
 * any rotation count).
 */
export async function clipped(el: Locator): Promise<string[]> {
  return el.evaluate((node: Element) => {
    const box = node as HTMLElement;
    const out: string[] = [];
    if (box.scrollWidth > box.clientWidth) out.push(`scrollWidth ${box.scrollWidth} > clientWidth ${box.clientWidth}`);
    if (box.scrollHeight > box.clientHeight) out.push(`scrollHeight ${box.scrollHeight} > clientHeight ${box.clientHeight}`);
    const range = document.createRange();
    range.selectNodeContents(box);
    const t = range.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    const slack = 0.5;
    if (t.left < b.left - slack || t.right > b.right + slack || t.top < b.top - slack || t.bottom > b.bottom + slack) {
      const r = (x: DOMRect) => `(${x.left.toFixed(1)}, ${x.top.toFixed(1)}, ${x.width.toFixed(1)} x ${x.height.toFixed(1)})`;
      out.push(`text ${r(t)} outside the box ${r(b)}`);
    }
    return out;
  });
}

/** The laid-out text's width and height (page px). */
export async function textSize(el: Locator): Promise<{ w: number; h: number }> {
  return el.evaluate((node: Element) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    const t = range.getBoundingClientRect();
    return { w: t.width, h: t.height };
  });
}
