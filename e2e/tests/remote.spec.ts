import { createPrivateKey, createSign, randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import http from "node:http";
import { test, expect } from "./support/fixtures";
import { BASE, centre, frames, openSurface, token } from "./support/live";
import {
  HTTPS,
  recordWakeLockWrites,
  releaseWakeLocks,
  removeWakeLock,
  stubWakeLock,
  wakeLockRequests,
  wakeLockWrites,
} from "./support/pwa";

// Remote access (#17): one name for the LAN and the Cloudflare tunnel.
//
// The e2e job gives the hub the public name `foh.e2e.test` with a certificate
// of a per-run test CA both browsers trust, maps the name to 127.0.0.1 in
// /etc/hosts (as the installer does on the Ableton PC; Chromium's
// --host-resolver-rules would not reach WebKit), and an [access] section whose
// key set the harness serves from a per-run RSA key (E2E_ACCESS_KEY), which
// signs the assertions here the way Cloudflare Access signs them.
//
// A request "from the internet" is one with the tunnel's forwarded header
// (`cf-connecting-ip`): cloudflared connects from the PC itself, so the header,
// not the address, is what makes it an internet request.

const PUBLIC = new URL(HTTPS);
const KEY_FILE = process.env.E2E_ACCESS_KEY || "";
const TEAM = process.env.E2E_ACCESS_TEAM || "fohmixer-e2e.cloudflareaccess.com";
const AUD = process.env.E2E_ACCESS_AUD || "fohmixer-e2e-aud";
const KID = "e2e-kid";
const TUNNEL = { "cf-connecting-ip": "203.0.113.7" };

function b64url(data: string | Buffer): string {
  return Buffer.from(data).toString("base64url");
}

/** An Access assertion for the test application, `claims` overriding. */
function accessJwt(claims: Record<string, unknown> = {}): string {
  const now = Math.floor(Date.now() / 1000);
  const header = b64url(JSON.stringify({ alg: "RS256", kid: KID, typ: "JWT" }));
  const payload = b64url(
    JSON.stringify({
      aud: [AUD],
      iss: `https://${TEAM}`,
      exp: now + 3600,
      iat: now,
      email: "engineer@example.org",
      ...claims,
    }),
  );
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${payload}`);
  const signature = signer.sign(createPrivateKey(readFileSync(KEY_FILE)), "base64url");
  return `${header}.${payload}.${signature}`;
}

/** A WebSocket upgrade of the hub's /ws with `headers`: the answer's status (101 = upgraded). */
function upgrade(headers: Record<string, string>): Promise<number> {
  return new Promise(async (resolve, reject) => {
    const url = new URL(BASE);
    const request = http.request({
      host: url.hostname,
      port: url.port,
      path: `/ws?token=${await token()}&proto=1`,
      headers: {
        Connection: "Upgrade",
        Upgrade: "websocket",
        "Sec-WebSocket-Version": "13",
        "Sec-WebSocket-Key": randomBytes(16).toString("base64"),
        ...headers,
      },
    });
    request.on("upgrade", (_response, socket) => {
      socket.destroy();
      resolve(101);
    });
    request.on("response", (response) => {
      response.resume();
      resolve(response.statusCode ?? 0);
    });
    request.on("error", reject);
    request.end();
  });
}

test.describe("The public name over HTTPS (the LAN path)", () => {
  test.use({ baseURL: HTTPS });

  test("the surface loads and connects over wss, as a PWA that keeps the screen on", async ({ page }) => {
    await page.addInitScript(stubWakeLock);
    const api = await (await page.request.get(`${BASE}/api/version`)).json();
    await openSurface(page);
    expect(page.url().startsWith(`${HTTPS}/`)).toBe(true);
    await expect(page.getByTestId("stage").getByTestId("version")).toHaveText(`v${api.version}`);
    const root = page.locator("html");
    await expect(root).toHaveAttribute("data-pwa", "on");
    await expect(root).toHaveAttribute("data-sw", "registered");
    const scope = await page.evaluate(async () => (await navigator.serviceWorker.getRegistration())?.scope);
    expect(scope).toBe(`${HTTPS}/`);
    // The wake lock: taken at load, taken again when the page comes back
    // after the system released it.
    await expect(root).toHaveAttribute("data-wake-lock", "held");
    expect(await wakeLockRequests(page)).toEqual(["screen"]);
    await releaseWakeLocks(page);
    await expect(root).toHaveAttribute("data-wake-lock", "released");
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await expect(root).toHaveAttribute("data-wake-lock", "held");
    expect(await wakeLockRequests(page)).toEqual(["screen", "screen"]);
  });

  test("a device that grants the wake lock only on a touch gets it at the end of the first touch", async ({
    page,
    hasTouch,
  }) => {
    // iPadOS refuses a wake lock asked for without user activation (the
    // Home Screen app reported "denied", #5 K4), and a finger's pointerdown
    // is no activation: the stub grants only as the device does.
    await page.addInitScript(stubWakeLock, { activation: true });
    await openSurface(page);
    const root = page.locator("html");
    // At load nobody has touched the page yet.
    await expect(root).toHaveAttribute("data-wake-lock", "denied");
    expect(await wakeLockRequests(page)).toEqual(["screen"]);
    // A touch on the version label (it does nothing): a finger on the
    // iPad, the mouse on the desktop.
    const at = await centre(page.getByTestId("stage").getByTestId("version"));
    const touch = () => (hasTouch ? page.touchscreen.tap(at.x, at.y) : page.mouse.click(at.x, at.y));
    await touch();
    await expect(root).toHaveAttribute("data-wake-lock", "held");
    expect(await wakeLockRequests(page)).toEqual(["screen", "screen"]);
    // While the lock is held a touch asks for nothing.
    await touch();
    await frames(page);
    expect(await wakeLockRequests(page)).toEqual(["screen", "screen"]);
    await expect(root).toHaveAttribute("data-wake-lock", "held");
  });

  test("a browser without the Wake Lock API is marked unsupported once, however often it is touched", async ({
    page,
    hasTouch,
  }) => {
    // Every touch asks again while no lock is held, and the app reports
    // each write of data-wake-lock to the hub: an unchanged state must not
    // be written again.
    await page.addInitScript(removeWakeLock);
    await page.addInitScript(recordWakeLockWrites);
    await openSurface(page);
    await expect(page.locator("html")).toHaveAttribute("data-wake-lock", "unsupported");
    const at = await centre(page.getByTestId("stage").getByTestId("version"));
    for (let i = 0; i < 2; i++) {
      if (hasTouch) await page.touchscreen.tap(at.x, at.y);
      else await page.mouse.click(at.x, at.y);
    }
    await frames(page);
    expect(await wakeLockWrites(page)).toEqual(["unsupported"]);
  });

  test("the manifest link asks for credentials (the Access cookie on the internet path); manifest and worker are served", async ({ page }) => {
    await page.goto("/");
    const manifest = page.locator('link[rel="manifest"]');
    await expect(manifest).toHaveAttribute("crossorigin", "use-credentials");
    const response = await page.request.get("/manifest.json");
    expect(response.status()).toBe(200);
    expect((await response.json()).start_url).toBe("/");
    const worker = await page.request.get("/sw.js");
    expect(worker.headers()["cache-control"]).toBe("no-cache, must-revalidate");
    expect(await worker.text()).toContain("event.respondWith(fetch(event.request))");
  });

  test("plain http on the public name is sent to https; by IP it stays plain", async ({ request }) => {
    const byName = await request.get(`http://${PUBLIC.hostname}:${new URL(BASE).port}/api/version?x=1`, {
      maxRedirects: 0,
    });
    expect(byName.status()).toBe(307);
    expect(byName.headers()["location"]).toBe(`${HTTPS}/api/version?x=1`);
    const byIp = await request.get(`${BASE}/api/version`, { maxRedirects: 0 });
    expect(byIp.status()).toBe(200);
  });
});

test.describe("The emergency plain-http path", () => {
  test("has neither the service worker nor the wake lock", async ({ page }) => {
    await page.addInitScript(stubWakeLock);
    await openSurface(page);
    const root = page.locator("html");
    await expect(root).toHaveAttribute("data-pwa", "off");
    await expect(root).not.toHaveAttribute("data-sw", /.*/);
    expect(await page.evaluate(async () => (await navigator.serviceWorker?.getRegistration()) ?? null)).toBeNull();
    expect(await wakeLockRequests(page)).toEqual([]);
  });
});

test.describe("Requests from the internet (through the tunnel)", () => {
  test("need a valid Access assertion; the LAN needs none", async ({ request }) => {
    const version = `${BASE}/api/version`;
    const status = async (headers: Record<string, string>) => (await request.get(version, { headers })).status();
    expect(await status({})).toBe(200);
    expect(await status(TUNNEL)).toBe(403);
    const refused = await request.get(version, { headers: TUNNEL });
    expect((await refused.json()).code).toBe("ACCESS_DENIED");
    expect(await status({ ...TUNNEL, "cf-access-jwt-assertion": accessJwt() })).toBe(200);
    expect(await status({ ...TUNNEL, cookie: `CF_Authorization=${accessJwt()}` })).toBe(200);
    const now = Math.floor(Date.now() / 1000);
    for (const bad of [
      accessJwt({ exp: now - 3600 }),
      accessJwt({ aud: ["another-app"] }),
      accessJwt({ iss: "https://evil.cloudflareaccess.com" }),
      accessJwt().slice(0, -4) + "AAAA",
    ]) {
      expect(await status({ ...TUNNEL, "cf-access-jwt-assertion": bad })).toBe(403);
    }
    // The PIN stays: an internet request with Access still needs the engineer's token.
    const statusApi = await request.get(`${BASE}/api/status`, {
      headers: { ...TUNNEL, "cf-access-jwt-assertion": accessJwt() },
    });
    expect(statusApi.status()).toBe(401);
  });

  test("the WebSocket upgrade is gated the same way, and a foreign page never gets it", async () => {
    const jwt = accessJwt();
    expect(await upgrade({})).toBe(101);
    expect(await upgrade(TUNNEL)).toBe(403);
    expect(await upgrade({ ...TUNNEL, "cf-access-jwt-assertion": jwt })).toBe(101);
    expect(await upgrade({ ...TUNNEL, Cookie: `CF_Authorization=${jwt}` })).toBe(101);
    // Cross-site WebSocket hijacking from a page the LAN browser visits.
    expect(await upgrade({ Origin: "https://evil.example" })).toBe(403);
    expect(await upgrade({ Origin: BASE })).toBe(101);
  });
});

test.describe("A browser on the internet path", () => {
  // The refused page load is reported as a failed resource load.
  test.use({ allowedConsole: [/^Failed to load resource: /] });

  test("is refused without Access and runs the mixer with the Access cookie", async ({ page, context }) => {
    await page.setExtraHTTPHeaders(TUNNEL);
    const refused = await page.goto("/");
    expect(refused?.status()).toBe(403);
    await context.addCookies([{ name: "CF_Authorization", value: accessJwt(), url: BASE }]);
    await openSurface(page);
  });
});
