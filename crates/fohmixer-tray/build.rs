//! Tauri's build step (the tray's context, `icons/icon.ico` as the exe's
//! icon), on Windows only: the tray's glue and Tauri itself are Windows-only
//! dependencies, and the library's decisions need no build step.

fn main() {
    #[cfg(windows)]
    tauri_build::build();
}
