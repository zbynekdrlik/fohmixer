//! The hub's integration tests, one test binary (`it`, #137): every mutant
//! of the PR mutation gate rebuilds and links one test crate against the
//! mutated hub instead of one per file. A new test file goes in this folder
//! with a `mod` line here; nextest filters name its tests as
//! `binary_id(fohmixer-hub::it) & test(/^<mod>::/)` (`.config/nextest.toml`).
//! The fixtures stay in `tests/fixtures/` (read by path at run time).

mod support;

mod auth;
mod autotune;
mod autotune_keys;
mod autotune_safe;
mod autotune_switch;
mod colors;
mod companion_client;
mod config_cli;
mod deck;
mod eq;
mod eq_probe_cli;
mod fader_link;
mod fanout;
mod graceful_stop;
mod iem;
mod layout;
mod live_client;
mod lufs;
mod lufs_listener;
mod markers;
mod names;
mod pin_cli;
mod ping;
mod remote;
mod reverb;
mod sectors;
mod serve_until;
mod setter;
mod songmap;
mod stage_aut;
mod token_cli;
mod unfold;
mod ws_log;
