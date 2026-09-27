//! fohmixer UI: the Leptos CSR app the hub serves (Trunk builds it to WASM).
//!
//! S0 shows the version and a status line; the touch surfaces arrive in S4
//! (spec §2.5).

pub mod app;
pub mod lifecycle;

use wasm_bindgen::prelude::*;

/// WASM entry point.
#[wasm_bindgen(start)]
pub fn main() {
    // Panic hook with the reload overlay and the server report, before the
    // mount so a panic during mount is captured.
    lifecycle::install_panic_hook();

    leptos::mount::mount_to_body(app::App);

    // Remove the pre-WASM loading shell now that Leptos has mounted.
    if let Some(shell) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("app-shell"))
    {
        shell.remove();
    }
}
