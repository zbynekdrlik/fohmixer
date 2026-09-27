---
paths:
  - ".github/workflows/*.yml"
  - "rust-toolchain.toml"
  - ".cargo/mutants.toml"
  - ".config/nextest.toml"
  - "scripts/*mutants*.py"
  - "deny.toml"
  - "crates/**/*.rs"
---

# CI (the iemmixer baseline, copied @ 22372bc)

- **Tier 0:** nothing compiles locally (not even `cargo mutants --list`, which the tier-0 hook blocks); push `dev` and read CI. Locally: `cargo fmt`, `cargo metadata`/`tree`/`update`, `ruff check .`, `python3 -m unittest discover -s scripts -p 'test_*.py'`, `python3 -m unittest discover -s live-script/tests`, `python3 scripts/check_integrity.py`, `python3 scripts/check_version.py`, `actionlint`, and `npx playwright test --list` in `e2e/` (parses the specs, runs no browser).
- **Pinned toolchain** in `rust-toolchain.toml` (1.98.1 + rustfmt, clippy, llvm-tools-preview, wasm32). Every Rust job runs `rustup toolchain install` first. A new stable can add clippy lints: bump it in its own commit.
- **`--locked` everywhere;** tools come prebuilt from `taiki-e/install-action` at pinned versions (trunk 0.21.14, cargo-llvm-cov 0.9.1, cargo-deny 0.20.2, cargo-mutants 27.1.0, cargo-nextest 0.9.146). `Cargo.lock` was seeded from iemmixer's lock so shared crates (Leptos, wasm-bindgen whose CLI Trunk downloads, axum, tokio) run at versions iemmixer's CI already proves.
- **Every `uses:` is pinned to a full commit SHA** with a `# vX.Y.Z` comment (`scripts/check_integrity.py`); no `continue-on-error`, no self-hosted runners, no `pull_request_target`.
- **Placeholder dist:** `fohmixer-hub` embeds `crates/fohmixer-ui/dist/` (rust-embed fails to compile without the folder), so every native cargo job first writes `dist/index.html` = `placeholder`. The router tests serve that placeholder; only `e2e` builds the hub with the real Trunk bundle (downloaded from the `wasm` job).
- **Coverage floor:** `.github/coverage-floor`, line coverage of `fohmixer-proto` + `fohmixer-hub` (`cargo llvm-cov`, the hub's process tests included). Raise it to `floor(measured) - 2` when coverage grows; never lower it without a reason on the ticket.
- **Python:** the `python` job runs ruff 0.16.2 (explicit rule set in `pyproject.toml`) and the FohMixer tests on Python 3.11, Live 12's embedded Python.
- **Mutation gate (PR only):** `mutants-list` counts the PR's diff-scoped mutants (`cargo mutants --list --in-diff`, parsed only) and outputs one shard per 12 mutants as the `mutation` matrix, so no shard outgrows its 20-minute budget; `mutation-warmup` builds the `mutants` profile once and runs the unmutated suite under it. Shards: `--baseline=skip --timeout 60 --jobs 2 --copy-target=true`, then `scripts/mutants_recheck.py` re-tests any catch that was only a timeout (`mutants-recheck` nextest profile). A survivor is work not done: kill it with a test; exclude only browser-only code (web_sys, JS closures, Leptos component bodies) per function in `.cargo/mutants.toml` with a reason. `.config/nextest.toml` runs the bounded `serve_until_answers_then_stops_within_six_seconds` first (priority) and gives `fohmixer-hub` 30 s per test (its stop-drain test holds 5 s on purpose). The full-tree sweep is `mutation-full.yml`, on demand only.
- **build.rs (fohmixer-proto):** GIT_HASH from git when it answers, else CI's `GITHUB_SHA` prefix (cargo-mutants' scratch copy has no `.git`), else "unknown"; GIT_BRANCH from `GITHUB_REF_NAME`. The proto tests compare the branch with `GITHUB_REF_NAME` when it is set.
