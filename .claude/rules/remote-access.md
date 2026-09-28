---
paths:
  - "crates/fohmixer-hub/src/access.rs"
  - "crates/fohmixer-hub/src/access/**"
  - "crates/fohmixer-hub/src/acme.rs"
  - "crates/fohmixer-hub/src/acme/**"
  - "crates/fohmixer-hub/src/cloudflare.rs"
  - "crates/fohmixer-hub/src/cf_token.rs"
  - "crates/fohmixer-hub/src/sealed.rs"
  - "crates/fohmixer-hub/src/dpapi.rs"
  - "crates/fohmixer-hub/src/tls.rs"
  - "crates/fohmixer-hub/src/https.rs"
  - "crates/fohmixer-hub/src/remote.rs"
  - "crates/fohmixer-hub/src/tunnel.rs"
  - "crates/fohmixer-hub/src/http_client.rs"
  - "crates/fohmixer-hub/src/test_keys.rs"
  - "crates/fohmixer-hub/tests/remote.rs"
  - "crates/fohmixer-ui/sw.js"
  - "crates/fohmixer-ui/index.html"
  - "scripts/cloudflare/**"
  - "scripts/fohmixer-pc/FohmixerRemote.ps1"
  - "e2e/tests/remote.spec.ts"
---

# Remote access (#17): one name for the LAN and the Cloudflare tunnel

Design and owner decision: #17 (the `Design-by: main` comment and the `ROZHODNUTÉ`); spec §2.4 "Remote access", D7, R5. Ported from restreamer (`access.rs`, `.claude/rules/access-control.md`), iemmixer (the rustls listener, `tunnel_watch.rs`), airuleset (`cli_webterm_access.py`, `cli_webterm_pwa.py`).

## Invariants — never regress

- **The plain-HTTP listener (8480) always runs** and is never redirected by IP: it is the emergency path (`http://<PC IP>:8480`, no DNS, no certificate) and the tunnel's origin. A taken HTTPS port never stops it: `lib.rs` `start_https` reports the bind error in `/api/status` (`remote.https.bind_error`) and retries the bind every `HTTPS_BIND_RETRY` (1 min). `routes::redirect_target` redirects only a request for the `[tls]` name with no proxy header (a tunnel request redirected would loop), and only while HTTPS serves (before the first certificate the bound socket accepts nothing). The redirect is 307 (browsers do not keep it).
- **LAN is never authenticated and the `Local` branch of `access::decide` does no network I/O**: with the internet, Cloudflare or the tunnel down the mixer still opens. Never move a key fetch above the classification.
- **A request is `Internet` when it carries any `PROXY_HEADERS` (cloudflared connects from 127.0.0.1: the header decides, not the address) or its peer is public.** RFC 1918, CGNAT and link-local are `Local`. A LAN client that sends a forwarded header only makes itself need Access (the safe direction).
- **`set_required_spec_claims(["exp", "aud", "iss"])` is load-bearing:** `set_audience` / `set_issuer` check a claim only when present.
- **Without `[access]` every internet request is refused** (default deny). The engineer PIN stays behind Access everywhere.
- **The Access and Origin middleware sit inside `router()` after the routes are merged**, so the SPA catch-all `/{*path}` is covered; `check_host` runs first (421), then the redirect (HTTP listener only), then Access (403). A WebSocket upgrade is a GET exempt from CORS: the Origin guard checks it explicitly.
- **Missing `ConnectInfo` counts as the internet (fail closed):** a listener that lost it refuses everything instead of letting a port-forwarded peer in. Both listeners serve `into_make_service_with_connect_info::<SocketAddr>()`; the router's `oneshot` tests add `MockConnectInfo` (`routes.rs` `lan()`); every plain-HTTP process test and `tests/remote.rs` `the_access_check_is_live_on_the_real_listeners` (a LAN request over TLS answers 200) break if a listener drops it.
- **Service worker and Wake Lock only on `location.protocol === 'https:'`** (index.html), not on every secure context: `http://127.0.0.1:8480` is one too, and the emergency path and the whole E2E suite use it. `sw.js` is network-only: never the Cache Storage (the `wasm` CI job greps for it).
- **Secrets:** the Cloudflare API token (`fohmixer-hub cloudflare set-token`, stdin) and the ACME account key are sealed with DPAPI for the hub's user (`sealed.rs`) — so the command runs **as the band user** on the PC, like `pin set-engineer`. The TLS key is a plain PEM in `<data>/tls/` under the data folder's protected DACL. The tunnel's connector token lives in `<ProgramData>\fohmixer-tunnel\tunnel-token` (SYSTEM + Administrators only, `--token-file`), never on a command line. None of them is ever logged or printed.

## ACME (`acme.rs`)

- `keep` loop: a stored certificate for the name with ≥ 30 days is kept (checked every 12 h); otherwise one attempt; a failure is in `/api/status` (`remote.https.acme_error`, `acme_failures`) and retried after `next_attempt` (1 min doubling to 6 h; without a Cloudflare token every 5 min, warned of once: `warns`) while the old certificate keeps being served.
- DNS-01: zone found by the longest parent the token sees (`cloudflare::zone_candidates`); leftover `_acme-challenge` TXT records of this hub (comment `TXT_COMMENT`; another client's records are never touched) are deleted first; the record is removed after the check **also on failure**; `propagation_s` (default 20) before answering the challenge. A valid authorization the CA still holds for the account (Let's Encrypt: 30 days) needs no record: `AuthorizationStatus::Valid` is skipped (the double reuses one).
- The account is per directory: switching `[acme] directory` (staging ↔ production) makes a new account; a stored file that does not parse is logged and replaced (`stored_account`).
- A plain-`http://` directory is only allowed on loopback (`config::url_allowed`) and is what the tests use (`acme/double.rs`, an RFC 8555 double that checks the JWS signatures and the TXT record against the key authorization; `cloudflare::double`).

## Tests

- RSA keys for Access JWTs come from `openssl genrsa -traditional` (`test_keys.rs`): the `rsa` crate carries an unfixed advisory (cargo-deny), ring cannot make RSA keys. Token-shaped test values are low-entropy (`"t".repeat(24)`): gitleaks reads a realistic one as a leak.
- `tests/remote.rs` talks TLS with tokio-rustls and SNI (no DNS needed); the CA and leaf come from rcgen. Unit tests use `tls::test_certs::handshake` (trusts one CA, returns the served certificate's names and end): a certificate swap or "the old certificate still serves" is proven over TLS, never by `serving()` alone.
- E2E (`remote.spec.ts`): the `e2e` job makes a per-run CA + leaf for `foh.e2e.test`, trusts the CA in the system store (WebKit/GnuTLS) and the NSS db `~/.pki/nssdb` (Chromium), maps the name in `/etc/hosts` (Chromium's `--host-resolver-rules` would not reach WebKit), and an RSA key whose public half the harness serves as the team's key set (`--access-key`). A request "from the internet" is one with `cf-connecting-ip`. The Wake Lock is stubbed (`navigator.wakeLock`: headless browsers hold none); what is tested is the page's acquire/re-acquire logic.

## On the PC and at Cloudflare

- Installer (`FohmixerRemote.ps1`, only with `-PublicName`; `Install-Fohmixer.ps1` calls `Resolve-FohRemote` and hands its plan to `Invoke-FohInstall -Remote`): `[tls]`/`[acme]`/`[access]`/`[tunnel]` in the toml, the hosts block (`# BEGIN fohmixer … # END fohmixer` → 127.0.0.1, in the file's own encoding), the band user's desktop shortcut `fohmixer.url`, firewall `fohmixer-hub-https` (`FohmixerFirewall.ps1`), and with `-SetTunnelToken` (token piped on stdin) the service `fohmixer-tunnel` (`--protocol http2`, metrics 127.0.0.1:20241). Refused before any change: a toml value the hub would refuse (ACME directory not https, a quote/space in the e-mail or an AUD), `-HttpsPort` taken by another program (`Test-FohPortFree`), a cloudflared without `tunnel run --token-file` (`Test-FohCloudflared`). An install without `-PublicName` keeps the installed remote tables (`Get-FohInstalledRemoteToml`) and touches nothing else of it. An older `Cloudflared` service on the PC is never touched.
- Cloudflare side: `scripts/cloudflare/fohmixer_cloudflare.py` (dry-run by default, `--apply`): tunnel, Access app + policy (before the name is published), ingress → `http://127.0.0.1:8480`, proxied CNAME; prints the hub's `[access]` table; `--token-out` writes the connector token owner-only. A healthy public name answers **302** to the Access login, not 200.
- Router: a static DNS record of the name → the PC's LAN IP (MikroTik, its own project's workflow).
