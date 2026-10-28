import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, type Locator, type Page, type WebSocketRoute } from "@playwright/test";

// The E2E world: the hub (E2E_BASE_URL) with two SimLive hosts (band, master)
// started by e2e/harness/harness.py, whose control API (E2E_HARNESS_URL) can
// stall, rename, restart a host, restart the hub and swap the layout. The
// engineer PIN comes from the job (E2E_PIN, masked). The layout is the import
// tool's synthetic fixture (tools/import-tosc/fixtures/expected-layout.json).

export const BASE = process.env.E2E_BASE_URL || "http://127.0.0.1:8480";
export const HARNESS = process.env.E2E_HARNESS_URL || "http://127.0.0.1:39190";
// The pages' way to the hub (#43): the harness's impair proxy, which a test can
// stall, drop and block (`impair`). The test's own `LiveClient` and API calls
// use BASE, the hub itself, so they never lose their link.
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

/**
 * The impair proxy between the pages and the hub (#43, `e2e/harness/impair.py`):
 * `stall` holds both directions for `ms`, `drop` resets every connection,
 * `block` holds new connections until it is lifted, `rate` lets the pages'
 * bytes to the hub through at most `bytesPerS` a second (a slow link; 0 lifts it).
 */
export const impair = {
  stall: (ms: number) => harness("/link/stall", { ms }),
  drop: () => harness("/link/drop"),
  block: (on: boolean) => harness("/link/block", { on }),
  rate: (bytesPerS: number) => harness("/link/rate", { bytes_per_s: bytesPerS }),
  /** The hub's bytes to the pages at most this fast (#71 PR J; 0 lifts it). */
  downRate: (bytesPerS: number) => harness("/link/down_rate", { bytes_per_s: bytesPerS }),
};

/** The fake Companion's state (#52): `connections`, `devices`, `presses`, `down`, `failing`. */
async function companionState(): Promise<any> {
  const response = await fetch(`${HARNESS}/companion`);
  if (!response.ok) throw new Error(`harness /companion: ${response.status}`);
  return response.json();
}

/**
 * The harness's fake Companion (#52, `e2e/harness/fake_companion.py`): its
 * presses as it got them (`key`, `pressed`, `at` epoch ms), and its faults:
 * `down` (every connection closed, new ones closed at once), `up`, `fail`
 * (presses answered ERROR), `clear` (the presses forgotten). `reset` puts it
 * all back (up, not failing, nothing recorded or held, and the hub's
 * `[companion]` if a test took it away) and resolves once the hub's Stream
 * Deck is registered at the fake again: the hub reconnects with a backoff of
 * at most 2 s.
 */
export const companion = {
  state: companionState,
  down: () => harness("/companion/down"),
  up: () => harness("/companion/up"),
  fail: (on: boolean) => harness("/companion/fail", { on }),
  clear: () => harness("/companion/clear"),
  reset: async () => {
    await harness("/companion/reset");
    await until(
      async () => (await companionState()).devices.length,
      (n) => n >= 1,
      "the hub's Stream Deck registered at the fake Companion again",
      10_000,
    );
  },
};

/**
 * The harness's fake AbleSet (#90 B3b, `e2e/harness/fake_ableset.py`), the
 * hub's `[ableset]` upstream: `state()` (`port`, `down`, `sockets`, `presses`,
 * `hits`: each request it got, `link` true for the hub's own read of its index
 * every 5 s), `down` (AbleSet stopped: a connect is refused), `up`, `clear`
 * (the hits forgotten).
 */
export const fakeAbleSet = {
  state: async (): Promise<any> => {
    const response = await fetch(`${HARNESS}/ableset`);
    if (!response.ok) throw new Error(`harness /ableset: ${response.status}`);
    return response.json();
  },
  down: () => harness("/ableset/down"),
  up: () => harness("/ableset/up"),
  clear: () => harness("/ableset/clear"),
};

/** The fake AbleSet's hits of pages: the hub's own link reads left out. */
export function pageHits(hits: any[]): any[] {
  return hits.filter((hit) => !hit.link);
}

/**
 * The hub's simulated window backend (#71 PR E, `[eq] backend = "sim"` in the
 * harness's config): `records()` reads what it did, in order (`GET /sim/eq`:
 * `{op: "take" | "press_and_hold" | "touch" | "resize" | "release" | "close", window, phase?, x?, y?, quiet?}`, a
 * touch's point in the picture's pixels), `clear()` forgets them.
 */
export const simEq = {
  records: async (): Promise<any[]> => {
    const response = await fetch(`${HARNESS}/sim/eq`);
    if (!response.ok) throw new Error(`harness /sim/eq: ${response.status}`);
    return (await response.json()).records;
  },
  clear: () => harness("/sim/eq/clear"),
};

/** A Stream Deck key of the page by its number (#52). */
export function deckKey(page: Page, key: number): Locator {
  return page.locator(`[data-testid="deck-key"][data-key="${key}"]`);
}

/**
 * Opens the Stream Deck tab and waits for its `keys` keys, each with
 * Companion's image, and for Companion online (a press then goes) (#52).
 */
export async function openDeck(page: Page, keys = 32) {
  const tab = page.getByTestId("deck-tab");
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
  await expect(page.getByTestId("deck-key")).toHaveCount(keys);
  await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(keys);
  await expect(tab).toHaveAttribute("data-offline", "false");
}

/**
 * Waits until the hub knows the clock of a page opened since `since` (#52):
 * a `ping` record of it with an `offset_ms`. Until then the hub's late gate
 * counts every down on time, so a test of a late press waits for it
 * (`openSurface` no longer waits a second for a refresh, #58).
 */
export async function clockKnown(since: number) {
  await until(
    async () => (await hubEvents()).some((r: any) => r.ev === "ping" && r.ts >= since && typeof r.offset_ms === "number"),
    (known) => known,
    "the hub's clock offset of the page",
  );
}

/** Every record of the hub's event log (#43), oldest first. */
export async function hubEvents(): Promise<any[]> {
  const response = await fetch(`${HARNESS}/hub/events`);
  if (!response.ok) throw new Error(`harness /hub/events: ${response.status}`);
  return (await response.json()).events;
}

/**
 * The page events of the hub's `trace` records, each once, in record order:
 * a batch resent after a lost socket can be logged twice, and the forensics
 * timeline reads it once too (#43 PR E).
 */
export function pageEvents(records: any[]): any[] {
  const seen = new Set<string>();
  const events: any[] = [];
  for (const r of records) {
    if (r.ev !== "trace") continue;
    for (const e of r.events) {
      const id = JSON.stringify(e);
      if (!seen.has(id)) {
        seen.add(id);
        events.push(e);
      }
    }
  }
  return events;
}

/** The layout the hub serves now (`GET /api/layout`). */
export async function servedLayout(): Promise<any> {
  const response = await fetch(`${BASE}/api/layout`, { headers: { authorization: `Bearer ${await token()}` } });
  if (!response.ok) throw new Error(`layout: ${response.status}`);
  return (await response.json()).layout;
}

/** The harness's layout: the import tool's synthetic fixture. */
const FIXTURE_LAYOUT = join(__dirname, "..", "..", "..", "tools", "import-tosc", "fixtures", "expected-layout.json");

/**
 * The fixture layout with touch-idle after `seconds` (#51: its
 * `config.idle_after_s`), served by the hub when this returns. A test that
 * sets it puts the fixture back with `resetLayout` in its `finally`.
 */
export async function setIdleAfter(seconds: number) {
  const layout = JSON.parse(readFileSync(FIXTURE_LAYOUT, "utf-8"));
  layout.config = { ...layout.config, idle_after_s: seconds };
  await harness("/hub/layout", { layout });
  await until(servedLayout, (l) => l.config?.idle_after_s === seconds, "the hub serving the touch-idle layout");
}

/** The fixture layout back, served by the hub when this returns (no touch-idle). */
export async function resetLayout() {
  await harness("/hub/layout/reset");
  await until(servedLayout, (l) => l.config?.idle_after_s === undefined, "the hub serving the fixture layout again");
}

/** Every group of a layout: the rows' and every pager sub-page's. */
function allGroups(layout: any): any[] {
  return layout.pages.flatMap((page: any) =>
    page.rows.flatMap((row: any) =>
      row.sections.flatMap((s: any) => (s.kind === "pager" ? s.pages.flatMap((sub: any) => sub.sections) : [s])),
    ),
  );
}

/** What a test gives a fixture group: a title, a colour. */
export type GroupFields = { title?: string; color?: string };

/**
 * The fixture layout with these groups' fields set (#127: the fixture's runs
 * that start with no strip have no title to show, so no line keeps the
 * titles' room, and its blocks have no colour; a title or a colour makes
 * one), served by the hub when this returns. Generic titles only (a public
 * repo). A test that sets it puts the fixture back with `resetGroups` in its
 * `finally`.
 */
export async function setGroups(changes: Record<string, GroupFields>) {
  const layout = JSON.parse(readFileSync(FIXTURE_LAYOUT, "utf-8"));
  for (const group of allGroups(layout)) Object.assign(group, changes[group.id] ?? {});
  await harness("/hub/layout", { layout });
  await until(
    servedLayout,
    (l) =>
      Object.entries(changes).every(([id, fields]) => {
        const group = allGroups(l).find((g) => g.id === id);
        return Object.entries(fields).every(([key, value]) => group?.[key] === value);
      }),
    "the hub serving the changed groups",
  );
}

/**
 * The fixture layout again after `setGroups`, served by the hub when this
 * returns (the fixture gives these groups no title and no colour).
 */
export async function resetGroups(ids: string[]) {
  await harness("/hub/layout/reset");
  await until(
    servedLayout,
    (l) =>
      ids.every((id) => {
        const group = allGroups(l).find((g) => g.id === id);
        return group?.title === undefined && group?.color === undefined;
      }),
    "the hub serving the fixture layout again",
  );
}

/**
 * The name on the side of the first strip of `inner`'s group's run (#127:
 * its title, or an untitled group's shared Live set): the first one among
 * the group's strips.
 */
export async function groupLabelOf(page: Page, inner: Locator): Promise<Locator> {
  const id = await inner.evaluate((e) => e.closest("[data-group]")?.getAttribute("data-group") ?? "");
  return page.locator(`.slot[data-group="${id}"] [data-testid="group-side"]`).first();
}

/** A fader's identity in the fader sectors (#148): its Live set and its label (a frame strip: its anchor's name). */
export type StripRef = { instance: string; label: string };
/**
 * One named layout of a sector (#148): a title of at most 8 characters, its
 * slots in order, each a fader or null (an empty slot, the slot model of
 * 1.24.0).
 */
export type SectorLayout = { id: string; title: string; strips: (StripRef | null)[] };
/** `sectors.json` (#148): the sectors `lu`, `ld`, `ru`, `rd` and their layouts. */
export type SectorsDoc = { rev: number; sectors: Partial<Record<"lu" | "ld" | "ru" | "rd", SectorLayout[]>> };

/** How many layouts a sectors document holds. */
function layoutCount(doc: SectorsDoc): number {
  return Object.values(doc.sectors).reduce((n, list) => n + (list?.length ?? 0), 0);
}

/**
 * Writes `doc` as the hub's `sectors.json` (#148, a hand edit) and returns
 * once the hub serves it (`/api/status` `sectors`: a newer revision with its
 * layouts and no error; the hub polls the file at the layout poll's tick).
 * The spec puts the empty document back with `resetSectors` after each test.
 */
export async function setSectors(doc: SectorsDoc) {
  const bearer = await token();
  const before = (await hubStatus(bearer)).sectors;
  await harness("/hub/sectors", { doc });
  const layouts = layoutCount(doc);
  await until(
    async () => (await hubStatus(bearer)).sectors,
    (s: any) => s.rev > before.rev && s.layouts === layouts && !s.error,
    "the hub serving the sectors document",
  );
}

/**
 * The harness's empty `sectors.json` again (#148: sectors off, the FOH page
 * as before), served by the hub when this returns. The hub keeps its undo
 * version in memory, so a test never relies on ↶ being dim at its start.
 */
export async function resetSectors() {
  const bearer = await token();
  const before = (await hubStatus(bearer)).sectors;
  await harness("/hub/sectors/reset");
  await until(
    async () => (await hubStatus(bearer)).sectors,
    (s: any) => s.layouts === 0 && !s.error && (before.layouts === 0 || s.rev > before.rev),
    "the hub serving the empty sectors document",
  );
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

/** `GET /api/status` (with `bearer`, a token taken once: every login hashes the PIN). */
export async function hubStatus(bearer?: string): Promise<any> {
  const response = await fetch(`${BASE}/api/status`, { headers: { authorization: `Bearer ${bearer ?? (await token())}` } });
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
  /** The hub's last REVERB state (#130: `ServerMsg::Reverb`, told to every client). */
  reverb: any = null;
  /** The hub's last AUTOTUNE state (#104: `ServerMsg::Autotune`, told to every client). */
  autotune: any = null;
  /** The hub's last fader sectors (#148: `ServerMsg::Sectors`, `{doc, undo}`, told to every client). */
  sectors: any = null;

  private constructor(ws: WebSocket) {
    this.ws = ws;
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(String(event.data));
      if (msg.type === "hub") this.hub.set(msg.key, msg.value);
      if (msg.type === "reverb") this.reverb = msg;
      if (msg.type === "autotune") this.autotune = msg;
      if (msg.type === "sectors") this.sectors = msg;
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
    const ws = new WebSocket(`${BASE.replace(/^http/, "ws")}/ws?token=${await token()}&proto=2`);
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

  /**
   * A request the hub answers by its `id` (a `cmd`, #148's `sector_tick`,
   * `sector_orders`, `sector_undo`): `msg` with a fresh `id`, and the hub's
   * `result` or `error` for it (an error is returned, not thrown).
   */
  async ask(msg: Record<string, unknown>): Promise<any> {
    if (this.ws.readyState !== WebSocket.OPEN) throw new Error("the test client's socket is not open");
    const id = `e2e${++this.next}`;
    const answer = new Promise<any>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        reject(new Error(`no answer to ${JSON.stringify(msg)} within ${CMD_TIMEOUT_MS} ms`));
      }, CMD_TIMEOUT_MS);
      this.waiting.set(id, (reply) => {
        clearTimeout(timer);
        resolve(reply);
      });
    });
    this.ws.send(JSON.stringify({ ...msg, id }));
    return answer;
  }

  /** A command batch on an instance: the script's result slots. */
  async cmd(instance: string, commands: object[]): Promise<any[]> {
    const msg = await this.ask({ type: "cmd", instance, commands });
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

/** Opens the surface logged in (the token seeded once per tab), connected to the hub. */
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
}

/**
 * A Live volume's dB on the strips' law, spelled as the page's
 * `behave::fader::value2db` (TouchOSC's `value2db`; SimLive's `volume_to_db`
 * is the same curve): above 1 (or no number) 0 dB, the line from 0.4, the
 * parabola from 0.15, the power curve below it, −∞ at −70 dB and under. The
 * backing-tracks fader's tests (#58, `params.spec.ts`) check it against the
 * page's own `data-db` of the same volumes.
 */
export function value2db(v: number): number {
  if (v > 1 || Number.isNaN(v)) return 0;
  if (v >= 0.4) return 40 * v - 34;
  if (v >= 0.15) return -((399.751894 * v - 201.871345) ** 2 + 12630.61132) / 799.503788;
  const gamma = 7504 / 5567;
  const db = 118.426374 * v ** (1 / gamma) - 70;
  return db <= -70 ? -Infinity : db;
}

/**
 * Live's display string in the strip readout's form (#21, TouchOSC's): one
 * decimal rounded half away from zero, no unit, a real minus, "−∞" for
 * silence.
 */
export function dbForm(display: string): string {
  const n = display.trim().replace(/dB$/, "").trim();
  if (/^-inf$/i.test(n)) return "−∞";
  const x = Number(n);
  const tenths = (Math.sign(x) * Math.round(Math.abs(x) * 10)) / 10;
  if (tenths === 0) return "0.0";
  return tenths < 0 ? `−${(-tenths).toFixed(1)}` : tenths.toFixed(1);
}

/** A strip by its track name and instance. */
export function strip(page: Page, name: string, instance = "band"): Locator {
  return page.locator(`[data-testid="strip"][data-track="${name}"][data-instance="${instance}"]`);
}

/**
 * The title of the group that holds `inner` (#63: a line lists each group's
 * title before its run of cells, and a cell's slot names its group; the
 * first title when the column cuts the group).
 */
export async function groupOf(page: Page, inner: Locator): Promise<Locator> {
  const id = await inner.evaluate((e) => e.closest("[data-group]")?.getAttribute("data-group") ?? "");
  return page.locator(`[data-testid="group"][data-group="${id}"]`).first();
}

/** Selects a tab by its page id. */
export async function selectPage(page: Page, id: string) {
  const tab = page.locator(`[data-testid="tab"][data-page="${id}"]`);
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
}

/**
 * Opens the channel detail of `name`'s strip (#71, F27): a hold on its SEL
 * key (#151; it was ☰),
 * a touch held 600 ms (the hold is 500 ms) dispatched in the page as
 * `dispatchPointer` times it; the detail, once it shows that strip and its
 * opening guard is over (`data-guard`: for 400 ms its parts take no touch,
 * so a real mouse or finger on them would reach nothing).
 */
export async function openDetail(page: Page, name: string, instance = "band"): Promise<Locator> {
  const menu = strip(page, name, instance).getByTestId("detail-key");
  await expect(menu).toBeVisible();
  await dispatchPointer(menu, [{ type: "pointerdown" }, { wait: 600 }, { type: "pointerup" }]);
  const detail = page.getByTestId("detail");
  await expect(detail).toHaveAttribute("data-track", name);
  await expect(detail).toHaveAttribute("data-instance", instance);
  await expect(detail).toHaveAttribute("data-guard", "false");
  return detail;
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
 * One step of `dispatchPointer`: a pointer event `dy` px above the control's
 * centre and `dx` px right of it (a pan's drag, #124), a wait, or a snapshot
 * of the control's look (`dispatchSnapped`).
 */
export type PointerStep =
  | { type: "pointerdown" | "pointermove" | "pointerup" | "pointercancel"; dx?: number; dy?: number }
  | { wait: number }
  | { snap: true };

/**
 * A rail button's look at a `{ snap: true }` step, read in the page (PR
 * #133's review, M1): `at` ms after the sequence started, its `data-holding`
 * and `data-full`, its second line (`.btn-sub`) and whether it flashes
 * `failed`.
 */
export type ButtonSnap = { at: number; holding: string | null; full: string | null; sub: string; failed: boolean };

/**
 * `dispatchPointer`, which also gives the page clock (`performance.now()`)
 * when the sequence started and the control's look at each `{ snap: true }`
 * step. A snap is read in the page's own timer order (PR #133's review, M1): one
 * 300 ms after a down always comes before a check the down set for 1020 ms,
 * one 1300 ms after it always after, however slow the runner (a read of its
 * own after the call returns lands one Playwright round trip later).
 */
export async function dispatchSnapped(
  control: Locator,
  steps: PointerStep[],
  pointerId = 21,
  primary = true,
): Promise<{ start: number; end: number; snaps: ButtonSnap[] }> {
  const { x, y } = await centre(control);
  return control.evaluate(
    async (el, { clientX, clientY, list, id, isPrimary }) => {
      const start = performance.now();
      const snaps: { at: number; holding: string | null; full: string | null; sub: string; failed: boolean }[] = [];
      for (const step of list) {
        if ("wait" in step) {
          await new Promise((done) => setTimeout(done, step.wait));
          continue;
        }
        if ("snap" in step) {
          snaps.push({
            at: performance.now() - start,
            holding: el.getAttribute("data-holding"),
            full: el.getAttribute("data-full"),
            sub: el.querySelector(".btn-sub")?.textContent ?? "",
            failed: el.classList.contains("failed"),
          });
          continue;
        }
        el.dispatchEvent(
          new PointerEvent(step.type, {
            pointerId: id,
            pointerType: "touch",
            isPrimary,
            clientX: clientX + (step.dx ?? 0),
            clientY: clientY - (step.dy ?? 0),
            bubbles: true,
            cancelable: true,
          }),
        );
      }
      return { start, end: performance.now(), snaps };
    },
    { clientX: x, clientY: y, list: steps, id: pointerId, isPrimary: primary },
  );
}

/**
 * Pointer events of one finger (a touch, `pointerId`) dispatched on `control`
 * inside the page, at its centre or `dy` px above it (and `dx` right of it),
 * with the waits timed in the page: exact timing in both engines (one real
 * click takes ~160 ms in WebKit on the CI runner), and a finger that can stay
 * down while the real mouse does something else. The page clock
 * (`performance.now()`) after the last step. `primary`: the finger is the
 * first one down (`isPrimary`, as a browser marks a touch while no other is
 * down); a second finger passes false (a dispatched event's own default is
 * false).
 */
export async function dispatchPointer(control: Locator, steps: PointerStep[], pointerId = 21, primary = true): Promise<number> {
  return (await dispatchSnapped(control, steps, pointerId, primary)).end;
}

/**
 * Two taps on a control, `gapMs` from the first release to the second press,
 * timed inside the page: the fader's double tap wants its releases 50–250 ms
 * apart, which two real clicks cannot hit in both engines. The taps are the
 * same pointer events a finger sends (the WebKit multi-touch path sends them too).
 */
export async function doubleTap(control: Locator, gapMs = 100) {
  await dispatchPointer(control, [
    { type: "pointerdown" },
    { wait: 20 },
    { type: "pointerup" },
    { wait: gapMs },
    { type: "pointerdown" },
    { wait: 20 },
    { type: "pointerup" },
  ]);
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

/**
 * Whether `el` can be seen (#127): drawn (not `display: none`, not hidden,
 * a box of its own) and inside every box that clips it (an `overflow` other
 * than `visible`) up to its strip, so a mark that the name button's corner
 * wrapped out of its line is not seen.
 */
export async function seen(el: Locator): Promise<boolean> {
  return el.evaluate((node: Element) => {
    const b = node.getBoundingClientRect();
    const cs = getComputedStyle(node);
    if (cs.display === "none" || cs.visibility !== "visible" || b.width === 0 || b.height === 0) return false;
    const strip = node.closest('[data-testid="strip"]');
    for (let a = node.parentElement; a && a !== strip?.parentElement; a = a.parentElement) {
      if (getComputedStyle(a).overflow === "visible") continue;
      const c = a.getBoundingClientRect();
      if (b.left < c.left - 0.5 || b.right > c.right + 0.5 || b.top < c.top - 0.5 || b.bottom > c.bottom + 0.5) return false;
    }
    return true;
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

/** The page's hub socket (`page.routeWebSocket(HUB_SOCKET, …)`). */
export const HUB_SOCKET = /\/ws\?/;

/**
 * Routes the page's sockets to the hub: the first one as it is, every later
 * one with all but the hub values (as if the hub had not sent STAGE AUT or a
 * fader link's switch yet: `connection.spec.ts`, `fader-link.spec.ts`). The
 * routes, in order.
 */
export async function withoutHubValuesAfterTheFirst(page: Page): Promise<WebSocketRoute[]> {
  const sockets: WebSocketRoute[] = [];
  await page.routeWebSocket(HUB_SOCKET, (ws) => {
    sockets.push(ws);
    const server = ws.connectToServer();
    if (sockets.length === 1) return;
    server.onMessage((message) => {
      if (JSON.parse(String(message)).type !== "hub") ws.send(message);
    });
  });
  return sockets;
}

/** Where the page keeps its last handshake reload (wall clock ms). */
export const RELOAD_KEY = "fohmixer_proto_reload_at";

/** Every page load of `page`, counted. */
export function countLoads(page: Page): () => number {
  let loads = 0;
  page.on("load", () => (loads += 1));
  return () => loads;
}
