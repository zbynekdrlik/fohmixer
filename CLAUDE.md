# fohmixer — Project Instructions

## Overview

fohmixer is a custom touch FOH mixer for our two Ableton Live instances: a thin FohMixer remote script exposing Live's native LOM, a Rust hub, a Leptos PWA. Program spec: `docs/superpowers/specs/2026-09-28-fohmixer-program.md`; tickets #3 (program) and #4–#10 (S0–S6). **Public repository** — site data never enters it (spec §5.2).

## Branch policy

Two branches: `master` (production) and `dev`. Work on `dev`; open a PR `dev` → `master` and merge with a merge commit (no squash, no rebase, no force push).

## Playbook router

- Playwright E2E (console guard, Chromium + WebKit iPad projects, base URL) → `.claude/rules/e2e.md` (auto-loads on `e2e/**`)
- Leptos `view!` gotchas and disposal safety → `.claude/rules/leptos-view-macro.md` (auto-loads on `crates/fohmixer-ui/**`)
- Public-repo hygiene (what never enters this repo) → `.claude/rules/public-repo-hygiene.md` (auto-loads everywhere)
- CI, toolchain, placeholder dist, coverage floor, mutation gate → `.claude/rules/ci.md` (auto-loads on workflows, `.cargo/mutants.toml`, `.config/nextest.toml`, crates)

## Always-apply rules

**Local builds.** Tier 0: no local cargo compilation (no build/test/check/clippy/run, no `trunk build`) — push `dev` and verify in CI. Local `cargo fmt`, `cargo metadata`, `cargo tree`, `cargo generate-lockfile` / `cargo update`, `ruff` and the Python tests are fine.

**Versions.** One version for the workspace: `[workspace.package].version` in `Cargo.toml` (`0.1.0-dev.N` during S0–S6), mirrored in `live-script/FohMixer/version.py`. `scripts/check_version.py` enforces consistency (crates, `Cargo.lock`, `version.py`) and, on PRs to `master`, head > base. Bump it as the first commit after every merge to `master`.

**CI.** `.github/workflows/ci.yml` is the iemmixer baseline (copied from iemmixer @ `22372bc`): every `uses:` pinned to a full SHA with a `# vX.Y.Z` comment, no `continue-on-error`, GitHub-hosted runners only. `scripts/check_integrity.py` (the `integrity` job) enforces the pins, no skipped/focused tests and no force-kill verbs (spec I7).
