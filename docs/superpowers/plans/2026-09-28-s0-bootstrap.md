# S0 Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Ticket:** #4 (S0), which also covers #1 (the CI pipeline).

**Goal:** Build an empty but complete fohmixer skeleton. It is a Rust workspace (proto, hub and UI crates) plus a Python Live-script/SimLive skeleton, with the iemmixer CI baseline green on a PR from `dev` to `master`. The hub serves the UI, and the UI shows the version read from the live DOM.

**Architecture:** This copies and adapts the proven iemmixer skeleton (local clone `~/devel/iemmixer` @ `22372bc`). No domain code is added; later sub-projects fill the crates.
- `fohmixer-proto`: WASM-safe build and version info.
- `fohmixer-hub`: axum, rust-embed, `/api/version`, `/api/client-error`, graceful stop.
- `fohmixer-ui`: Leptos 0.7 CSR built by Trunk, showing the version.
- Python: `live-script/FohMixer/` with a version module, and `sim/` with a minimal fake `Live` module, tested with unittest and linted with ruff.

**Tech Stack:**

| Tool | Version |
|---|---|
| Rust | 1.98.1 (edition 2024) |
| axum | 0.8 |
| tokio | 1 |
| rust-embed | 8 |
| Leptos | 0.7.8 (CSR) |
| Trunk | 0.21.14 |
| Playwright | Chromium + WebKit |
| Python | 3.11 (unittest, ruff) |

GitHub-hosted runners only.

**Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` (§5, §6 S0). It was approved on #3 on 2026-09-28.

## Global Constraints

**Local builds**
- **No local cargo compilation.** Owner policy, Tier 0; the `block-tier0-local-build.sh` hook enforces it.
- Allowed locally: `cargo fmt`, `cargo metadata`, `cargo update` / `generate-lockfile`, `cargo tree`, Python and ruff.
- Everything that compiles (clippy, test, build, trunk, Playwright) runs in CI only.
- Batch the work so each CI cycle carries the whole task set, and push once per cycle.

**Branches and versions**
- Branches are `master` (production) and `dev`. Work on `dev` and open a PR from `dev` to `master` with a merge commit. Never force-push or rewrite history.
- The first commit on `dev` sets the workspace version to `0.1.0-dev.1`.
- There is one version for the whole workspace (`[workspace.package].version`). It is shown in the UI as `v<version>` and served at `/api/version`.

**Repo hygiene** (the repo is public, spec §5.2)
- No host names, IPs, Windows user names, people's names, PINs or keys go into the repo.
- `.mcp.json` stays ignored.

**CI rules**
- Every `uses:` is SHA-pinned with a `# vX.Y.Z` comment.
- `permissions: contents: read`.
- Concurrency group `ci-${{ github.event_name }}-${{ github.ref }}`, with cancel-in-progress on push.
- No `continue-on-error`, no self-hosted runners, no `pull_request_target`.

**Tests and E2E**
- No `#[ignore]`, `.skip`, `.only` or `assume`.
- Every Playwright test fails on any console error or warning (the iemmixer console-guard fixture).

## Review Focus

- **The embedded UI is missing** (a native job built without `dist/`).
  - Build scripts and CI must create a placeholder `crates/fohmixer-ui/dist/index.html` before any native cargo job, as iemmixer does.
  - The hub must still start and serve that placeholder.
  - Test: the hub route test uses the placeholder.
- **Master has no `Cargo.toml` yet** on the first PR.
  - `check_version.py --base-ref origin/master` must treat a missing base version as `0.0.0` and pass when head > base.
  - Test: `scripts/test_check_version.py` covers the missing-base case.
- **Deep links and unknown paths.** `GET /some/client/route` returns `index.html`, and `GET /missing.js` returns 404 (not index.html). Tests: hub route tests for both.
- **WebKit has console output that Chromium does not.** The console guard runs in both projects, and the WebKit project is in the config from S0 on, so a WebKit-only warning fails CI immediately.
- **Version drift between the WASM build and the server.** The version test compares the DOM label with `/api/version`, as in iemmixer's `version.spec.ts`.

---

### Task 1: Workspace, version and hygiene

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.github/coverage-floor`, `deny.toml`, `.gitleaks.toml`, `.cargo/mutants.toml`, `.config/nextest.toml`.
- Modify: `.gitignore` (append `target/`, `crates/fohmixer-ui/dist/`, `e2e/node_modules/`, `e2e/playwright-report/`, `e2e/test-results/`).
- Modify: `CLAUDE.md` (Overview, Branch policy `master`/`dev`, Playbook router).

**Interfaces:**
- Produces: workspace members `crates/fohmixer-proto`, `crates/fohmixer-hub`, `crates/fohmixer-ui`, with `version = "0.1.0-dev.1"`, `edition = "2024"` and `license = "MIT OR Apache-2.0"`.

- [ ] **Step 1: Root manifests.** Copy iemmixer `Cargo.toml` and change:
  - `members` to the three crates above;
  - `version` to `0.1.0-dev.1`;
  - `authors` to `["fohmixer contributors"]`;
  - `repository` to `https://github.com/zbynekdrlik/fohmixer`.

  Keep the release, dev and mutants profiles. Copy `rust-toolchain.toml` verbatim.
- [ ] **Step 2: Config files.**
  - Copy verbatim: `deny.toml`, `.gitleaks.toml`, `.config/nextest.toml`.
  - Copy `.cargo/mutants.toml` and drop every exclude that names an iemmixer path.
  - Write `.github/coverage-floor` containing `50`. Task 6 raises it to the measured value minus 2.
- [ ] **Step 3: CLAUDE.md.**
  - Overview: "fohmixer is a custom touch FOH mixer for our two Ableton Live instances: a thin FohMixer remote script exposing Live's native LOM, a Rust hub, a Leptos PWA. Program spec: `docs/superpowers/specs/2026-09-28-fohmixer-program.md`; tickets #3 (program) and #4–#10 (S0–S6)."
  - Branch policy: `master` + `dev`.
  - Playbook router, one line each: `.claude/rules/e2e.md`, `.claude/rules/leptos-view-macro.md`, `.claude/rules/public-repo-hygiene.md` (Task 6 creates these).
  - A "Local builds" line: Tier 0, CI only.
- [ ] **Step 4: Lockfile.** Run `cargo generate-lockfile` (non-compiling; allowed). This must run after Tasks 2–4 have created the crate manifests, so do this step last in the batch.
- [ ] **Step 5: Commit** — `git commit -m "chore: bump to 0.1.0-dev.1 and add workspace skeleton (#4)"`.

### Task 2: `fohmixer-proto` (build and version info)

**Files:**
- Create: `crates/fohmixer-proto/Cargo.toml`, `crates/fohmixer-proto/build.rs`, `crates/fohmixer-proto/src/lib.rs`.

**Interfaces:**
- Produces:
  - `pub const VERSION: &str`
  - `pub fn version_label() -> String`, which returns `"v" + VERSION`
  - `pub fn full_version() -> String`
  - `pub struct VersionInfo { version, git_hash, branch, build_time }`, with serde `Serialize` and `Deserialize`
  - `pub fn version_info() -> VersionInfo`

- [ ] **Step 1: Write the tests first** in `src/lib.rs` `#[cfg(test)]`:
  - `version_label()` starts with `v` and equals `format!("v{}", env!("CARGO_PKG_VERSION"))`;
  - `version_info().version == VERSION`;
  - `VersionInfo` round-trips through `serde_json`.
- [ ] **Step 2: Implement** by copying `iem-core/build.rs` verbatim (it emits `GIT_HASH`, `GIT_BRANCH` from `GITHUB_REF_NAME`, and `BUILD_TIME`) and `iem-core/src/lib.rs:35-100`. Keep only the version items and rename `iem` to `fohmixer`.
  - Dependencies: `serde` with `derive`, and `serde_json` as a dev-dependency.
  - No `config`/toml feature. The crate must compile for `wasm32-unknown-unknown`.
- [ ] **Step 3: Commit** — `feat(proto): version and build info (#4)`.

### Task 3: `fohmixer-hub` (server skeleton)

**Files:**
- Create: `crates/fohmixer-hub/Cargo.toml`, `src/lib.rs`, `src/routes.rs`, `src/bin/fohmixer-hub.rs`, `tests/graceful_stop.rs`.

**Interfaces:**
- Consumes: `fohmixer_proto::{version_info, VERSION}`.
- Produces:
  - `pub fn app_router() -> axum::Router`
  - `pub async fn serve_until(addr: SocketAddr, ready: oneshot::Sender<SocketAddr>, stop: impl Future<Output=()>) -> anyhow::Result<()>`
  - The binary reads `PORT` (default `8480`) and binds `0.0.0.0`.

- [ ] **Step 1: Write the tests first.** `src/routes.rs` `#[cfg(test)]` uses `tower::ServiceExt::oneshot` on `app_router()`:
  - `GET /api/version` → 200, JSON `version == VERSION`;
  - `POST /api/client-error` with `{"panic_message":"x"}` → 204, and with a body over 10,240 bytes → 413;
  - `GET /` → 200 `text/html`;
  - `GET /deep/route` → 200, the same body as `/`;
  - `GET /missing.js` → 404;
  - the security headers `X-Frame-Options: DENY` and `X-Content-Type-Options: nosniff` are present.

  `tests/graceful_stop.rs` is copied from iemmixer and adapted to `serve_until`: start, hit `/api/version`, signal stop, and assert the task ends within 6 s.
- [ ] **Step 2: Implement** by copying and trimming from iemmixer:
  - `lib.rs:399-435` (rust-embed `Assets` on `../fohmixer-ui/dist/`, the router with security headers and CSP, no CORS);
  - `routes.rs:30-39, 51, 82-86, 210-252, 688-770` (version, client-error, static SPA with content-hash caching);
  - `bin/server.rs:145-227` (tracing env filter `fohmixer_hub=info`, own runtime, `ctrl_c` and `ctrl_break` shutdown installed before bind);
  - graceful stop with a 5 s drain.

  Drop TLS, auth, WebSocket and tunnel code; they come in S3. Dependencies: axum 0.8 (`macros`, `json`), tokio (`rt-multi-thread`, `macros`, `signal`, `net`), tower-http 0.6 (`set-header`), rust-embed 8, serde, serde_json, tracing, tracing-subscriber (`env-filter`), anyhow. Dev-dependencies: tower (`util`), http-body-util, reqwest (`json`, `rustls-tls`, no default features).
- [ ] **Step 3: Commit** — `feat(hub): axum skeleton serving the UI and /api/version (#4)`.

### Task 4: `fohmixer-ui` (Leptos app showing the version)

**Files:**
- Create: `crates/fohmixer-ui/Cargo.toml`, `Trunk.toml`, `index.html`, `manifest.json`, `icons/` (placeholder icons: a plain dark square with "FOH"; generate the PNGs with Python PIL in CI-free form: commit the generated files), `src/lib.rs`, `src/app.rs`.

**Interfaces:**
- Consumes: `fohmixer_proto::version_label`.
- Produces: a root component `App` that renders `<main data-testid="app">` with a header containing `<span data-testid="version">{version_label()}</span>`, plus a status line "fohmixer — čaká na pripojenie k Abletonu" (the hub connection comes in S3).

- [ ] **Step 1: Write the test first.** `src/app.rs` has a native `#[cfg(test)]` unit test: `version_text()` (a pure helper used by the view) equals `fohmixer_proto::version_label()`.
- [ ] **Step 2: Implement.**
  - Copy iemmixer `crates/iem-ui/Trunk.toml`, including the `pattern_script` loader with the "Try Again" failure path.
  - Copy iemmixer `index.html`: meta tags, inline app shell and the `data-trunk rel="rust" data-wasm-opt="0" data-integrity="none"` line. **Remove the service-worker registration** (spec R5: v1 has no SW).
  - `manifest.json`: `display: standalone`, `orientation: landscape`, name `fohmixer`, `theme_color #000000`.
  - Dependencies: leptos 0.7.8 (`csr`), wasm-bindgen 0.2, console_error_panic_hook, and `fohmixer-proto` with `default-features = false`.
  - Panic hook: copy `lifecycle.rs` `post_panic_report` to POST to `/api/client-error` (about 60 lines).
- [ ] **Step 3: Commit** — `feat(ui): Leptos shell showing the version (#4)`.

### Task 5: Python skeleton: FohMixer script and SimLive

**Files:**
- Create:
  - `live-script/FohMixer/__init__.py`: `create_instance(c_instance)` returns `FohMixer(c_instance)`; the class body arrives in S2. In S0 it is a minimal `ControlSurface` subclass that only logs start at WARNING level.
  - `live-script/FohMixer/version.py`: `VERSION` read from the workspace version by `scripts/check_version.py` consistency.
  - `sim/Live/__init__.py`, `sim/_Framework/ControlSurface.py`: the minimal fakes `create_instance` needs.
  - `live-script/tests/test_skeleton.py`.
  - `pyproject.toml`: ruff config only (`target-version = "py311"`, `line-length = 100`).

**Interfaces:**
- Produces:
  - `live-script/FohMixer/version.py: VERSION = "0.1.0-dev.1"`.
  - SimLive import path: tests add `sim/` and `live-script/` to `sys.path`.

- [ ] **Step 1: Write the failing test.** `test_skeleton.py`:
  - importing `FohMixer` with SimLive on the path and calling `create_instance(FakeCInstance())` returns an object whose `disconnect()` runs without error;
  - `FohMixer.version.VERSION` equals the `[workspace.package].version` parsed from `Cargo.toml` with `tomllib`.
- [ ] **Step 2: Run it** — `python3 -m unittest discover -s live-script/tests -v`. Expected: FAIL (no module).
- [ ] **Step 3: Implement** the files above, minimal.
- [ ] **Step 4: Run** — the same command passes. `ruff check live-script sim scripts` is clean.
- [ ] **Step 5: Commit** — `feat(live-script): FohMixer and SimLive skeleton (#4)`.

### Task 6: CI, scripts, E2E and rules

**Files:**
- Create:
  - `.github/workflows/ci.yml`
  - `scripts/check_integrity.py`, `scripts/check_version.py`, `scripts/test_check_version.py`
  - `scripts/check_disposal_safety.py`, `scripts/test_check_disposal_safety.py`
  - `e2e/package.json`, `e2e/package-lock.json`, `e2e/playwright.config.ts`
  - `e2e/tests/support/fixtures.ts`, `e2e/tests/version.spec.ts`, `e2e/tests/app-load.spec.ts`
  - `.claude/rules/e2e.md`, `.claude/rules/leptos-view-macro.md`, `.claude/rules/public-repo-hygiene.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the CI job names used as branch-protection checks. Jobs, in `ci.yml`:

| Job | What it runs |
|---|---|
| `integrity` | `check_integrity.py`, the script self-tests, `check_disposal_safety.py`, `check_version.py` |
| `python` | ruff 0.x pinned + `python3 -m unittest discover -s live-script/tests -v` |
| `lint` | placeholder dist; `cargo fmt --check`; clippy native `--workspace --all-targets --all-features -D warnings`; clippy `-p fohmixer-ui -p fohmixer-proto --target wasm32-unknown-unknown` |
| `test` | placeholder dist; `cargo test -p fohmixer-ui --lib`; `cargo llvm-cov` over `fohmixer-proto` and `fohmixer-hub` against `.github/coverage-floor` |
| `wasm` | pinned Trunk; `trunk build --release --locked` in `crates/fohmixer-ui`; verify the loader in `dist/index.html`; upload artifact `wasm-dist` |
| `e2e` | needs `wasm`; download `wasm-dist`; `cargo build --locked --release -p fohmixer-hub`; start the hub on `PORT=8480`; poll `/api/version`; `npx playwright install --with-deps chromium webkit`; `npx playwright test` with `E2E_BASE_URL=http://127.0.0.1:8480`; upload the report on failure |
| `supply-chain` | `cargo deny check advisories bans licenses sources` |
| `secrets` | gitleaks 8.30.1, checksum-verified, full history |
| `version` | PR to `master` only; `check_version.py --base-ref origin/master` |
| `mutants-list` + `mutation` | PR only; diff-scoped cargo-mutants 27.1.0 with nextest, copied from iemmixer with the shard count computed from the in-diff mutant count (≤ 12 per shard), each shard ≤ 20 min |

- [ ] **Step 1: Scripts, test first.**
  - `test_check_version.py` cases:
    - workspace version equals every crate (inheritance) and `live-script/FohMixer/version.py`;
    - `--base-ref` with a base that has no `Cargo.toml` passes when head is `0.1.0-dev.1`;
    - head ≤ base fails.
  - Then adapt iemmixer `check_version.py`:
    - drop `tauri.conf.json`;
    - add `version.py`;
    - map a missing base `Cargo.toml` to `0.0.0`.
  - Copy `check_integrity.py` and drop the iemmixer-only checks: parity manifest, engine deps.
  - Copy `check_disposal_safety.py` and its test, and point them at `crates/fohmixer-ui/src`.
- [ ] **Step 2: E2E.**
  - Copy `playwright.config.ts` with two projects:
    - `chromium` (`Desktop Chrome`);
    - `ipad`: `devices["iPad Pro 11"]`, browser WebKit, `hasTouch`, landscape.
  - Copy `support/fixtures.ts` (the console guard) verbatim.
  - `version.spec.ts` is iemmixer's test verbatim.
  - `app-load.spec.ts`:
    - `/` renders `data-testid="app"`;
    - a deep link `/foo/bar` renders the same app;
    - the page title is `fohmixer`.
- [ ] **Step 3: Rules.**
  - `.claude/rules/e2e.md` (paths `e2e/**`): console guard, both projects, no retries, and the base URL environment variable.
  - `.claude/rules/leptos-view-macro.md` (paths `crates/fohmixer-ui/**`): copied from iemmixer; `try_set`/`try_update` in async and JS callbacks, no bare `<`/`>` in `view!` attributes.
  - `.claude/rules/public-repo-hygiene.md` (paths `**`): the §5.2 list of what never enters the repo.
- [ ] **Step 4: Local checks** (allowed): `cargo fmt --all -- --check`, `ruff check .`, `python3 -m unittest discover -s scripts -p 'test_*.py' -v`, `python3 scripts/check_integrity.py`, `python3 scripts/check_version.py`.
- [ ] **Step 5: Commit** — `ci: iemmixer CI baseline with WebKit E2E and the Python job (#4, #1)`.

### Task 7: Push, CI green, PR, merge

- [ ] **Step 1: Push.** Run `git fetch origin && git merge origin/master` (no-op), then `git push origin dev`. Monitor the run in ONE foreground bounded poll until every job reaches a terminal state.
- [ ] **Step 2: If any job fails,** read `gh run view <id> --log-failed`, fix every failure in ONE commit, and push once. Never rerun blindly.
- [ ] **Step 3: Coverage floor.** Once `test` is green, read the measured line coverage from the job log, set `.github/coverage-floor` to `floor(measured) - 2`, and commit that with any other fix.
- [ ] **Step 4: Open the PR.** `gh pr create -B master -H dev -t "S0: bootstrap — workspace, CI baseline, version display (#4)"`. The body carries "Closes #4" and "Closes #1", and the attribution line.
- [ ] **Step 5: Merge.** Wait for every PR check including `version` and `mutation`. Confirm `mergeable: true` and `mergeStateStatus: CLEAN`, then `gh pr merge --merge`. Verify that `master` contains the merge commit.
- [ ] **Step 6: Bump dev.** Set the `dev` version to `0.1.0-dev.2` in `Cargo.toml`, the lockfile and `version.py`, so the next work starts above `master`. Commit and push.
