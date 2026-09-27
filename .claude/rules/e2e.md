---
paths:
  - "e2e/**"
---

# E2E (Playwright)

- **Two projects, every spec in both:** `chromium` (Desktop Chrome) and `ipad` (`devices["iPad Pro 11 landscape"]`, WebKit, touch). The iPad is the engineer's real device, so a WebKit-only console warning or layout problem fails CI. The `e2e` job installs `chromium webkit` with `--with-deps`.
- **Console guard:** specs import `test`/`expect` from `./support/fixtures` (copied from iemmixer): an auto fixture fails any test whose browser console shows an error, a warning or a page error it did not declare in `allowedConsole`. A test that deliberately provokes a failed request declares exactly that message with `test.use({ allowedConsole: [/…/] })` in its own `describe`, with a comment saying why. Never a global allowance; where the message is an app bug, fix the app. Two or more patterns need the tuple form `test.use({ allowedConsole: [[/a/, /b/], { scope: "test" }] })` (Playwright reads a bare `[/a/, /b/]` as `[value, options]`). Browsers word the same failure differently (Chromium `net::ERR_CONNECTION_FAILED`, WebKit its own text): declare the common prefix, not one browser's wording.
- **No retries, no skips:** `retries: 0`, `forbidOnly` in CI; `scripts/check_integrity.py` fails on `.skip`/`.only`/`.fixme`/`test.fail`.
- **Base URL:** `E2E_BASE_URL` (CI: `http://127.0.0.1:8480`, the hub started from the release build with the real Trunk bundle embedded); the config's fallback is the hub's default port 8480.
- **Version test (`version.spec.ts`, verbatim from iemmixer):** the DOM label `data-testid="version"` must read `v` + `/api/version`'s `version`, so a WASM bundle and a hub from different builds fail.
- **App download failure (`app-load.spec.ts`):** the loader in `crates/fohmixer-ui/Trunk.toml` (`pattern_script`) catches a failed bindings/WASM download and the shell shows `load-error` with Try Again. The spec aborts `fohmixer-ui-<hash>_bg.wasm` (the hash is an unpadded hex u64, 1–16 digits) and declares the browser's `Failed to load resource: ` line. There is no service worker (spec R5), so no route interference from one.
- Failure evidence (`test-results/`, the HTML report, `hub.log`) is uploaded as the `e2e-failure` artifact.
