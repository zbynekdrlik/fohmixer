//! The scene automation on the mixer (#207; the owner's approved mockup
//! `docs/mockups/scenes-v3.html`, frame 2): the column foot's calm line
//! `scene <name> · HH:MM` (the last scene the hub told and when it came, the
//! page's local time) and the mixer's edge flashing green once for a scene
//! that came while the page was open and moved a fader (`scene-flash`, a
//! layer over the surface that takes no touch; `scenes.css`). The texts and
//! the flash's decision are `behave::scenes`.

use leptos::prelude::*;

use crate::behave::scenes::foot_text;
use crate::store::LiveStore;

/// The page's local hours and minutes of a hub UTC ms.
fn local_hours_minutes(utc_ms: f64) -> (u32, u32) {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(utc_ms));
    (date.get_hours(), date.get_minutes())
}

/// The foot's line, once a scene came.
#[component]
pub fn SceneLine() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    move || {
        store
            .scene
            .try_with(|s| {
                s.as_ref().map(|news| {
                    let (h, m) = local_hours_minutes(news.at_ms);
                    foot_text(&news.name, h, m)
                })
            })
            .flatten()
            .map(|text| {
                view! {
                    <div class="scene-line" data-testid="scene-line">
                        {text}
                    </div>
                }
            })
    }
}

/// The edge's flash: a new layer for each scene that moved a fader, so its
/// one-time animation starts again.
#[component]
pub fn SceneFlash() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    move || {
        let n = store.scene_flashes.try_get().unwrap_or(0);
        let shown = n > 0;
        shown.then(|| {
            view! { <div class="scene-flash" data-testid="scene-flash" data-flash=n.to_string()></div> }
        })
    }
}
