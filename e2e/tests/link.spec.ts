import type { Page, WebSocketRoute } from "@playwright/test";
import { test, expect } from "./support/fixtures";
import { HUB_SOCKET, LiveClient, centre, hubEvents, openSurface, ready, shown, strip, track, until, volume } from "./support/live";

// The control link of protocol 2 (#43, PR A): a control's write is a `set`
// acked by the hub, every hop of a move lands in the hub's event log
// (`set` → `batch` → `applied` → `ack`), the page pings every 100 ms while
// visible, and a dropout (≥ 300 ms with nothing from the hub while a pong
// is due, or a lost socket) is reported to the event log as a `trace`.

const HAND2 = track("Hand2 #");
const KEY = `band|${volume(HAND2)}|value`;

/** The page's hub socket, recorded both ways and able to hold the hub's messages. */
type Link = {
  sent: { at: number; msg: any }[];
  received: { at: number; msg: any }[];
  hold: boolean;
  held: (string | Buffer)[];
  sockets: WebSocketRoute[];
  release(): void;
};

async function recordLink(page: Page): Promise<Link> {
  const link: Link = {
    sent: [],
    received: [],
    hold: false,
    held: [],
    sockets: [],
    release() {
      this.hold = false;
      const socket = this.sockets[this.sockets.length - 1];
      for (const message of this.held.splice(0)) socket.send(message);
    },
  };
  await page.routeWebSocket(HUB_SOCKET, (ws) => {
    link.sockets.push(ws);
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      link.sent.push({ at: Date.now(), msg: JSON.parse(String(message)) });
      server.send(message);
    });
    server.onMessage((message) => {
      link.received.push({ at: Date.now(), msg: JSON.parse(String(message)) });
      if (link.hold) link.held.push(message);
      else ws.send(message);
    });
  });
  return link;
}

/** The page's messages of one type. */
const sentOf = (link: Link, type: string) => link.sent.filter((f) => f.msg.type === type).map((f) => f.msg);

/** The dropouts the page reported, in order. */
const dropouts = (link: Link) => sentOf(link, "trace").flatMap((t) => t.events.filter((e: any) => e.ev === "dropout"));

let live: LiveClient;
test.beforeEach(async () => {
  live = await LiveClient.open();
});
test.afterEach(async () => {
  live.close();
});

test.describe("The control link", () => {
  test("a fader drag reaches Live as acked sets, and every hop is in the hub's event log", async ({ page }) => {
    await live.set("band", volume(HAND2), "value", 0.5);
    const link = await recordLink(page);
    await openSurface(page);
    const fader = strip(page, "Hand2 #").getByTestId("fader");
    await ready(fader);
    await until(() => shown(fader), (v) => Math.abs(v - 0.5) < 0.001, "the fader at 0.5");
    try {
      const { x, y } = await centre(fader);
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 10; i++) await page.mouse.move(x, y - 8 * i);
      await page.mouse.up();
      // The page's writes of the volume are `set`s, sequence numbers rising.
      const sets = () => sentOf(link, "set").filter((m) => `${m.instance}|${m.target}|${m.prop}` === KEY);
      await expect.poll(() => sets().length).toBeGreaterThan(0);
      const acked = (seq: number) =>
        link.received.some((f) => f.msg.type === "ack" && f.msg.items.some((i: any) => i.key === KEY && i.seq >= seq && !i.error));
      // The last one is acked (no newer set follows it).
      await expect
        .poll(
          () => {
            const all = sets();
            return all.length > 0 && acked(all[all.length - 1].seq);
          },
          { timeout: 10_000 },
        )
        .toBe(true);
      const all = sets();
      const last = all[all.length - 1];
      for (let i = 1; i < all.length; i++) expect(all[i].seq).toBeGreaterThan(all[i - 1].seq);
      expect(last.value).toBeGreaterThan(0.55);
      expect(typeof last.t).toBe("number");
      // Live followed: its volume is the last set's value.
      await until(() => live.get("band", volume(HAND2), "value"), (v) => Math.abs(v - last.value) < 1e-9, "Live at the last value");
      // The hub's event log has the move's every hop, in order.
      const events = await until(
        hubEvents,
        (ev) => ev.some((e) => e.ev === "ack" && e.key === KEY && e.seq === last.seq),
        "the last ack in the event log",
      );
      const iSet = events.findIndex((e) => e.ev === "set" && e.key === KEY && e.seq === last.seq);
      expect(iSet, "the set").toBeGreaterThanOrEqual(0);
      expect(events[iSet].t).toBe(last.t);
      expect(typeof events[iSet].hub_ms).toBe("number");
      expect(typeof events[iSet].delay_ms, "the page pinged before it: the delay is known").toBe("number");
      const client = events[iSet].client;
      const iBatch = events.findIndex(
        (e, i) => i > iSet && e.ev === "batch" && e.instance === "band" && e.sent.some((s: any) => s.key === KEY && s.seq === last.seq),
      );
      expect(iBatch, "its batch after the set").toBeGreaterThan(iSet);
      const batch = events[iBatch].batch;
      const iApplied = events.findIndex((e, i) => i > iBatch && e.ev === "applied" && e.instance === "band" && e.batch === batch);
      expect(iApplied, "the batch's result after it").toBeGreaterThan(iBatch);
      expect(events[iApplied].errors).toBe(0);
      const iAck = events.findIndex((e, i) => i > iApplied && e.ev === "ack" && e.key === KEY && e.seq === last.seq);
      expect(iAck, "the ack after the result").toBeGreaterThan(iApplied);
      expect(events[iAck].client).toBe(client);
      expect(events[iAck].batch).toBe(batch);
    } finally {
      await live.set("band", volume(HAND2), "value", 0.5);
    }
  });

  test("the page pings every 100 ms with its clock and round trip, and the hub logs every ping", async ({ page }) => {
    const link = await recordLink(page);
    await openSurface(page);
    await page.waitForTimeout(1500);
    const now = Date.now();
    const recent = link.sent.filter((f) => f.msg.type === "ping" && f.at > now - 1000).map((f) => f.msg);
    expect(recent.length, "pings in the last second").toBeGreaterThanOrEqual(6);
    for (let i = 1; i < recent.length; i++) {
      expect(recent[i].n).toBe(recent[i - 1].n + 1);
      expect(recent[i].t).toBeGreaterThan(recent[i - 1].t);
      expect(typeof recent[i].rtt, "the previous pong's round trip").toBe("number");
    }
    const pongs = link.received.filter((f) => f.msg.type === "pong").map((f) => f.msg);
    expect(pongs.some((p) => p.n === recent[0].n && p.t === recent[0].t)).toBe(true);
    const events = await until(
      hubEvents,
      (ev) => ev.some((e) => e.ev === "ping" && e.t === recent[recent.length - 1].t),
      "the pings in the event log",
    );
    const logged = events.filter((e) => e.ev === "ping" && recent.some((p) => p.t === e.t));
    expect(logged.length).toBe(recent.length);
    expect(logged.some((e) => typeof e.offset_ms === "number")).toBe(true);
  });

  test("a held link is one dropout and a lost socket another, each reported to the event log", async ({ page }) => {
    const link = await recordLink(page);
    await openSurface(page);
    await page.waitForTimeout(500);
    const before = dropouts(link).length;
    // The hub's messages are held 800 ms (a Wi-Fi stall): the pongs due
    // stop, then arrive at once.
    link.hold = true;
    await page.waitForTimeout(800);
    link.release();
    await expect.poll(() => dropouts(link).length, { timeout: 5000 }).toBeGreaterThan(before);
    const held = dropouts(link)
      .slice(before)
      .find((d: any) => d.ms >= 600);
    expect(held, `a dropout of the hold among ${JSON.stringify(dropouts(link))}`).toBeTruthy();
    expect(held.socket_lost).toBe(false);
    expect(held.ms).toBeLessThan(3000);
    expect(held.rtts.length).toBeGreaterThan(0);
    await until(
      hubEvents,
      (ev) => ev.some((e) => e.ev === "trace" && e.events.some((d: any) => d.ev === "dropout" && d.t === held.t)),
      "the dropout in the event log",
    );
    // The socket drops: one dropout until the next hello, its socket lost.
    const sockets = link.sockets.length;
    await link.sockets[sockets - 1].close({ code: 4000, reason: "gone" });
    await expect.poll(() => link.sockets.length, { timeout: 10_000 }).toBe(sockets + 1);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await expect.poll(() => dropouts(link).filter((d: any) => d.socket_lost).length, { timeout: 5000 }).toBe(1);
    const lost = dropouts(link).find((d: any) => d.socket_lost);
    expect(lost.ms).toBeGreaterThanOrEqual(300);
    await until(
      hubEvents,
      (ev) => ev.some((e) => e.ev === "trace" && e.events.some((d: any) => d.socket_lost && d.t === lost.t)),
      "the lost socket in the event log",
    );
  });
});
