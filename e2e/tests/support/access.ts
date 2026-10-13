import { createPrivateKey, createSign } from "node:crypto";
import { readFileSync } from "node:fs";

// Cloudflare Access in the E2E (#17; shared since #90 PR C): the job gives the
// hub an [access] section whose key set the harness serves from a per-run RSA
// key (E2E_ACCESS_KEY), which signs the assertions here the way Cloudflare
// Access signs them. A request "from the internet" is one with the tunnel's
// forwarded header (`cf-connecting-ip`): cloudflared connects from the PC
// itself, so the header, not the address, is what makes it an internet request.

const KEY_FILE = process.env.E2E_ACCESS_KEY || "";
const TEAM = process.env.E2E_ACCESS_TEAM || "fohmixer-e2e.cloudflareaccess.com";
/** fohmixer's own Access application. */
export const AUD = process.env.E2E_ACCESS_AUD || "fohmixer-e2e-aud";
/** AbleSet's Access application (#90: the harness's `--ableset-aud`). */
export const ABLESET_AUD = "fohmixer-e2e-ableset-aud";
const KID = "e2e-kid";
/** The tunnel's forwarded header: the request comes from the internet. */
export const TUNNEL = { "cf-connecting-ip": "203.0.113.7" };

function b64url(data: string | Buffer): string {
  return Buffer.from(data).toString("base64url");
}

/** An Access assertion for fohmixer's application, `claims` overriding (`aud` for AbleSet's). */
export function accessJwt(claims: Record<string, unknown> = {}): string {
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
