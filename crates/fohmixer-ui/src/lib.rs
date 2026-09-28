//! fohmixer UI: the engineer's touch surface (spec §2.5), a Leptos CSR app
//! the hub serves (Trunk builds it to WASM).
//!
//! - `behave`: the TouchOSC-parity behaviour as pure state machines;
//! - `binding`, `stage`, `net`: the pure rules of the subscriptions, the
//!   stage geometry and the hub connection;
//! - `store`: the hub connection and Live's state (`LiveStore`);
//! - `components`, `pages`: the view; `raf`: the shared animation loop;
//!   `dom`, `auth`: browser helpers.

pub mod app;
pub mod auth;
pub mod behave;
pub mod binding;
pub mod components;
pub mod dom;
pub mod flow;
pub mod lifecycle;
pub mod net;
pub mod pages;
pub mod raf;
pub mod stage;
pub mod store;

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
