//! The root overlay's items (spec F6, F16): the TechAlert overlay blinking
//! while its track is unmuted, and REFRESH ALL.

use fohmixer_proto::layout::{Binding, Frame, Style};
use leptos::html;
use leptos::prelude::*;

use crate::behave::timing::{Debounce, REFRESH_FLASH_MS, blink_on};
use crate::binding::{SubSpec, target_of};
use crate::dom;
use crate::raf;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// The TechAlert overlay (spec F16): shows every other `period_ms` while
/// the TechAlert track is unmuted; never takes a touch.
#[component]
pub fn AlertView(
    frame: Frame,
    z: i64,
    style: Style,
    binding: Binding,
    period_ms: u32,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = target_of(&binding, "").map_or_else(
        || RwSignal::new(Slot::Pending),
        |t| store.slot(&SubSpec::new(&binding.instance, t, "mute", false)),
    );
    let root = NodeRef::<html::Div>::new();
    let frame_id = StoredValue::new(None::<usize>);
    let period = f64::from(period_ms);
    root.on_load(move |el: web_sys::HtmlDivElement| {
        let mut shown: Option<bool> = None;
        let id = raf::register(Box::new(move |now: f64, _step: f64| {
            let active = slot.try_with_untracked(Slot::flag).flatten() == Some(false);
            let on = blink_on(active, now, period);
            if shown != Some(on) {
                dom::set_style(&el, "visibility", if on { "visible" } else { "hidden" });
                dom::set_attr(&el, "data-visible", if on { "true" } else { "false" });
                dom::set_attr(&el, "data-active", if active { "true" } else { "false" });
                shown = Some(on);
            }
        }));
        frame_id.set_value(Some(id));
    });
    on_cleanup(move || {
        if let Some(Some(id)) = frame_id.try_get_value() {
            raf::unregister(id);
        }
    });
    view! {
        <div
            class="item alert"
            data-testid="alert"
            data-visible="false"
            data-active="false"
            style={format!("{}visibility:hidden;", stage::item_style(frame, z, &style))}
            node_ref=root
        ></div>
    }
}

/// REFRESH ALL (spec F6): at most every 0.5 s, a 300 ms yellow flash;
/// unsubscribes everything, unfolds the configured groups and subscribes
/// again.
#[component]
pub fn RefreshView(frame: Frame, z: i64, style: Style, label: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let debounce = StoredValue::new(Debounce::default());
    let flash = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let fired = debounce
            .try_update_value(|d| d.fire(dom::now()))
            .unwrap_or(false);
        if !fired {
            return;
        }
        let _ = flash.try_set(true);
        set_timeout(
            move || {
                let _ = flash.try_set(false);
            },
            std::time::Duration::from_millis(REFRESH_FLASH_MS as u64),
        );
        store.refresh();
    };
    let text = label.unwrap_or_else(|| "REFRESH ALL".to_string());
    view! {
        <div
            class="item button refresh"
            class:flash=move || flash.get()
            data-testid="refresh"
            data-flash=move || flash.get().to_string()
            style={stage::item_style(frame, z, &style)}
            on:pointerdown=on_down
        >
            {text}
        </div>
    }
}
