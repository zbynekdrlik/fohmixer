//! The fohmixer tray's entry point (Windows; the library's decisions build
//! and are tested everywhere).

// A release build has no console window (the tray starts at the band user's
// logon); a debug build keeps one for its errors.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    fohmixer_tray::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("fohmixer-tray runs on Windows only (the Ableton PC)");
    std::process::exit(2);
}
