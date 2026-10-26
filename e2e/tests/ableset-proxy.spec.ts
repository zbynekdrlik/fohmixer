import { test, expect } from "./support/fixtures";
import { BASE, fakeAbleSet, harness, hubEvents, openSurface, pageHits, token, until } from "./support/live";
import { HTTPS } from "./support/pwa";

// AbleSet's own page under its own name behind the hub (#90 B3b).
//
// The e2e job's harness runs the fake AbleSet (e2e/harness/fake_ableset.py)
// and gives the hub `[ableset]`: the name `ableset.e2e.test` (a sibling of
// `foh.e2e.test`, so the two are one site) with its own certificate of the
// per-run test CA, mapped to 127.0.0.1 in /etc/hosts, and the fake as its
// upstream. A browser opens AbleSet's name top-level over the LAN path
// (no Access: the LAN is never authenticated). The fake records each request
// it gets (`fakeAbleSet`); the hub's own read of its index every 5 s is left
// out (`pageHits`). The tab inside fohmixer's page is PR C's.

const ABLESET = new URL(process.env.E2E_ABLESET_URL || "https://ableset.e2e.test:8443");
const PARENT = new URL(HTTPS);
// The headers a proxy sets (the hub's `access::PROXY_HEADERS`): the hub adds
// none of them on the way to AbleSet.
const PROXY_HEADER = /^(cf-|x-forwarded-|forwarded$|x-real-ip$|true-client-ip$|cdn-loop$|via$)/;

/** `/api/status` `ableset` with a token taken once (a login hashes the PIN). */
async function ablesetStatus(bearer: string): Promise<any> {
  const response = await fetch(`${BASE}/api/status`, { headers: { authorization: `Bearer ${bearer}` } });
  if (!response.ok) throw new Error(`status: ${response.status}`);
  return (await response.json()).ableset;
}

test.describe("AbleSet's name over HTTPS (the LAN path)", () => {
  test("AbleSet's page, its assets, its live socket and a press go through the hub, which adds nothing", async ({
    page,
    context,
  }) => {
    await fakeAbleSet.clear();
    const cookie = { domain: ABLESET.hostname, path: "/", secure: true };
    await context.addCookies([
      { ...cookie, name: "CF_Authorization", value: "t" },
      { ...cookie, name: "keep", value: "1" },
    ]);
    const answer = await page.goto(`${ABLESET.origin}/`);
    expect(answer?.status()).toBe(200);
    // Only AbleSet's own and fohmixer's pages may frame it: the hub replaces
    // the fake's own X-Frame-Options and frame-ancestors with its own.
    const headers = answer!.headers();
    expect(headers["x-frame-options"]).toBeUndefined();
    expect(headers["content-security-policy"]).toBe(
      `frame-ancestors 'self' https://${PARENT.hostname} ${PARENT.origin}`,
    );
    expect(headers["x-content-type-options"]).toBe("nosniff");
    const body = page.locator("body");
    // Its absolute-path script and stylesheet came through the hub.
    await expect(body).toHaveAttribute("data-app", "loaded");
    const outline = await page.locator("#fake-button").evaluate((b) => getComputedStyle(b).outlineColor);
    expect(outline).toBe("rgb(1, 2, 3)");
    // Its socket: the WebSocket tunnel to AbleSet's Socket.IO-like echo.
    await expect(body).toHaveAttribute("data-ws", "echo");
    await page.locator("#fake-button").click();
    await expect(body).toHaveAttribute("data-pressed", "ok");
    const hits = pageHits((await fakeAbleSet.state()).hits);
    const seen = hits.map((h) => `${h.method} ${h.path.split("?")[0]}`);
    for (const wanted of ["GET /", "GET /static/app.js", "GET /static/app.css", "GET /socket.io/", "POST /api/press"]) {
      expect(seen, wanted).toContain(wanted);
    }
    for (const hit of hits) {
      expect(hit.host, hit.path).toBe(ABLESET.host);
      expect(hit.headers.filter((h: string) => PROXY_HEADER.test(h)), hit.path).toEqual([]);
      expect(hit.cookies, hit.path).not.toContain("CF_Authorization");
    }
    // The Access cookie is the hub's to check, never AbleSet's to see.
    expect(hits.find((h) => h.path === "/").cookies).toEqual(["keep"]);
    const socket = hits.find((h) => h.path.startsWith("/socket.io/"));
    expect(socket.browser_id).toBe(await body.getAttribute("data-browser-id"));
  });

  test("/api/status counts AbleSet's open sockets beside its link, and every page is told of AbleSet's page", async ({
    page,
  }) => {
    const told: any[] = [];
    page.on("websocket", (ws) =>
      ws.on("framereceived", (frame) => {
        if (typeof frame.payload === "string" && frame.payload.includes('"type":"ableset"')) {
          told.push(JSON.parse(frame.payload));
        }
      }),
    );
    await openSurface(page);
    await expect.poll(() => told.length).toBeGreaterThan(0);
    const { port } = await fakeAbleSet.state();
    expect(told[0]).toEqual({
      type: "ableset",
      title: "AbleSet",
      name: ABLESET.hostname,
      parent: PARENT.hostname,
      online: true,
      lan_port: port,
      // The test's browser socket comes over the LAN (#90 PR C review M2).
      lan: true,
    });
    await page.goto(`${ABLESET.origin}/`);
    await expect(page.locator("body")).toHaveAttribute("data-ws", "echo");
    const bearer = await token();
    const open = await until(() => ablesetStatus(bearer), (s) => s.sockets === 1, "AbleSet's socket counted");
    expect(open).toMatchObject({
      name: ABLESET.hostname,
      upstream: `http://127.0.0.1:${port}`,
      online: true,
      last_error: null,
    });
    expect(open.cert.serving).toBe(true);
    expect(open.cert.cert_names).toEqual([ABLESET.hostname]);
    await page.goto("about:blank");
    await until(() => ablesetStatus(bearer), (s) => s.sockets === 0, "AbleSet's socket closed");
  });
});

test.describe("Another site's request on AbleSet's name", () => {
  // WebKit's console line for the refused image's 403 (the test provokes it;
  // Chromium's ORB drops the JSON answer to an image without one).
  test.use({ allowedConsole: [/^Failed to load resource: the server responded with a status of 403/] });

  test("is refused by the hub before AbleSet, while the same image from fohmixer's own name loads", async ({ page }) => {
    // A fresh hub, so the refusal's record below is the first of its kind in
    // its window whatever an earlier test refused: the hub keeps one record
    // of a client's fault of one kind per 10 s (`ableset::FAULT_RECORDS`), and
    // a cross-site 403 within 10 s before this one would leave none (the
    // #90 B3b review). No page is open across the restart.
    await harness("/hub/restart", { rotate_secret: false });
    await fakeAbleSet.clear();
    const since = Date.now();
    const load = (src: string) =>
      page.evaluate(
        (url) =>
          new Promise<string>((resolve) => {
            const image = new Image();
            image.onload = () => resolve("load");
            image.onerror = () => resolve("error");
            image.src = url;
          }),
        src,
      );
    // Another site: the hub's own plain listener by address (fohmixer's page
    // policy allows AbleSet's images, so only the hub can refuse it).
    await page.goto(`${BASE}/api/version`);
    expect(await load(`${ABLESET.origin}/static/pixel.gif?from=cross-site`)).toBe("error");
    // The hub's own record of the refusal: a browser may hide the 403 itself
    // (Chromium's ORB blocks a JSON answer to an image before the page sees it).
    const refused = await until(
      async () =>
        (await hubEvents()).filter((r) => r.ev === "ableset_proxy" && r.ts >= since && r.status === 403),
      (records) => records.length > 0,
      "the hub's record of the refused image",
    );
    expect(refused[0]).toMatchObject({ method: "GET", path: "/static/pixel.gif" });
    expect(refused[0].why).toBe('a cross-site request: mode "no-cors", dest "image"');
    // The same image from fohmixer's own name (the same site) reaches AbleSet.
    await page.goto(`${HTTPS}/api/version`);
    expect(await load(`${ABLESET.origin}/static/pixel.gif?from=same-site`)).toBe("load");
    const paths = pageHits((await fakeAbleSet.state()).hits).map((h) => h.path);
    expect(paths).toEqual(["/static/pixel.gif?from=same-site"]);
  });
});

test.describe("AbleSet down", () => {
  // The hub's 502 page while the fake is stopped: a failed document load in
  // the console (the test stops AbleSet on purpose).
  test.use({ allowedConsole: [/^Failed to load resource: the server responded with a status of 502/] });

  test("a top-level load gets the hub's page, the link goes down, and the page loads AbleSet once it is back", async ({
    page,
  }) => {
    // The link reads AbleSet's index every 5 s: down after two failed reads
    // (up to ~10 s), up at the next good one; the hub's page tries again every 5 s.
    const bearer = await token();
    const since = Date.now();
    await fakeAbleSet.down();
    try {
      const answer = await page.goto(`${ABLESET.origin}/`);
      expect(answer?.status()).toBe(502);
      await expect(page.locator("body")).toHaveText("AbleSet on the PC is not answering. Retrying in 5 s.");
      const down = await until(() => ablesetStatus(bearer), (s) => s.online === false, "AbleSet's link down", 15_000);
      expect(down.last_error).toContain("AbleSet does not answer");
    } finally {
      await fakeAbleSet.up();
      // The link is up again before the next spec, also after a failure
      // above (the #90 B3b review).
      await until(() => ablesetStatus(bearer), (s) => s.online === true, "AbleSet's link up", 10_000);
    }
    const body = page.locator("body");
    await expect(body).toHaveAttribute("data-app", "loaded", { timeout: 10_000 });
    await expect(body).toHaveAttribute("data-ws", "echo");
    const links = (await hubEvents()).filter((r) => r.ev === "ableset_link" && r.ts >= since);
    expect(links.map((r) => r.up)).toEqual([false, true]);
  });
});
