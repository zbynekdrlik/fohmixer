# Autopilot log

One line per ticket: issue, commits, RED -> GREEN tests, decisions, PR.

- #4 S0 bootstrap (PR #11, merge b58d8db, v0.1.0-dev.1; dev bumped to 0.1.0-dev.2 in 367281c): workspace + proto/hub/UI crates, FohMixer + SimLive skeleton, iemmixer CI baseline (copied @ 22372bc) with WebKit iPad E2E, dynamic mutation shard matrix (43 mutants, 4 shards, all caught), coverage 96.96 % (floor 95). Bug found in review: RUST_LOG could not raise the hub's level — RED 46cfed8 (graceful_stop.rs rust_log_raises_the_hub_s_own_level, a_bad_rust_log_exits_1_naming_it) -> GREEN c628541 (log_filter). Port tests serialized after two CI flakes (b3d26d6). Decisions: issue #4 comments 5860764863, 5860968436.
- #1 CI pipeline: delivered by #4's `.github/workflows/ci.yml` (GitHub-hosted ubuntu-24.04), closed by PR #11.
