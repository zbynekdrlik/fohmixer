//! Loudness on the PA mics and the PA output (#151, spec
//! `docs/superpowers/specs/2026-10-26-lufs-design.md`).
//!
//! - [`dsp`] is the reference of what the `FohMixer LUFS` Max for Live
//!   device measures (§3.3): BS.1770-4 K-weighting, 100 ms sub-blocks, the
//!   short-term reading `S` and the gated `V`. The device's `gen~` code
//!   (`live-device/FohMixer-LUFS/fohmixer_lufs.genexpr`) does the same
//!   steps on the same constants; `scripts/check_lufs_device.py` keeps the
//!   two tables equal.

pub mod dsp;
