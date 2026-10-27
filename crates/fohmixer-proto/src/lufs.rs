//! Loudness on the PA mics and the PA output (#151, spec
//! `docs/superpowers/specs/2026-10-26-lufs-design.md`).
//!
//! The `FohMixer LUFS` Max for Live device measures (§3.3) and sends its
//! readings to the hub over localhost (§4.4). Its numbers are checked on the
//! PC against Sonible trueLevel's short-term on the same signal (§9, the
//! owner's ruling of 27.10), not against a reference of our own.
