//! Tauri's build step (the tray's context, `icons/icon.ico` as the exe's
//! icon), on Windows only: the tray's glue and Tauri itself are Windows-only
//! dependencies, and the library's decisions need no build step.
//!
//! `#[cfg(windows)]` here is the build HOST, while the
//! `[target.'cfg(windows)'.build-dependencies]` table follows the target:
//! both agree for the native builds CI makes (Linux on Linux, Windows on
//! Windows); a cross build from Linux to Windows is not supported.
//!
//! The Common-Controls v6 manifest tauri-build embeds reaches the binary
//! only (`rustc-link-arg-bins`), so the tray's tests run on Linux, where
//! Tauri is not linked at all.

fn main() {
    #[cfg(windows)]
    tauri_build::build();
}
