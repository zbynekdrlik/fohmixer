//! The singers' panel (#140 PR 3c; the approved mockup
//! `docs/mockups/autotune-v2.html`, screen 2): a tap on the AUTOTUNE rail
//! button opens it over the strips, beside the column
//! (`components/rail.rs` `RuleButton` keeps it open or closed); a tap
//! outside it, or on AUTOTUNE again, closes it. Its head names the song and
//! the key, each row a singer (its stream and PA chips, its key, its manual
//! switch, and why it is on DRY when its key does not read back), its foot
//! the song map's counts, Live's bar and the hold's help. A row's switch
//! sends `set_hub` `autotune_singer` `{id, off}`; it acts only on the state
//! the hub told this connection, while connected, else it flashes refused.
//! It shows the hub's state as told to this connection only (it is not in
//! the page without one). Its natural height is measured once it is in the
//! page and again whenever its content changes size (a `ResizeObserver` on
//! `.tpanel-in`; PR #150 re-review m3: never the box its max height caps),
//! so it stays inside the screen and never over the button
//! (`behave::autotune::panel_place`). The decisions are
//! `behave::autotune`'s.

use fohmixer_proto::client::HUB_AUTOTUNE_SINGER;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{fail_flash, owns_touches, trace_rail};
use crate::behave::autotune::{
    Anchor, PanelRow, RowKey, natural_height, panel_foot, panel_head, panel_place, panel_rows,
    switch_acts, switch_press, switch_value, tap_opens_panel,
};
use crate::behave::hold::Lift;
use crate::behave::rail::AUT_BADGE;
use crate::dom;
use crate::store::LiveStore;

/// The panel content's size watcher: a `ResizeObserver` and its callback,
/// kept until the panel goes.
type PanelObserver = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// The panel: `anchor` where it opens from (the column's right edge, the
/// button's top and bottom on the screen; the button places it again at a
/// resize). The button's page listener tells a tap outside it by its class
/// (`.tpanel`). Its root takes no touch of its own (it holds the
/// switches, which own theirs).
#[component]
pub fn AutotunePanel(anchor: Anchor) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let panel = NodeRef::<html::Div>::new();
    // The panel's natural height, to keep it inside the screen and off the
    // button: measured once in the page and again at every size change of
    // its content (a why line, a singer added), which its max height never
    // caps. The observer goes with the panel.
    let height = RwSignal::new(0.0_f64);
    let watcher = StoredValue::new_local(None::<PanelObserver>);
    panel.on_load(move |el: web_sys::HtmlDivElement| {
        let measure = {
            let el = el.clone();
            move || {
                natural_height(
                    f64::from(el.scroll_height()),
                    f64::from(el.offset_height()),
                    f64::from(el.client_height()),
                )
            }
        };
        let _ = height.try_set(measure());
        let Some(inner) = el.first_element_child() else {
            return;
        };
        let callback = Closure::wrap(Box::new(move || {
            let _ = height.try_set(measure());
        }) as Box<dyn FnMut()>);
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&inner);
            let _ = watcher.try_set_value(Some((observer, callback)));
        }
    });
    on_cleanup(move || {
        if let Some(Some((observer, _callback))) = watcher.try_update_value(Option::take) {
            observer.disconnect();
        }
    });
    let place = move || {
        let size =
            |get: fn(&web_sys::Window) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
                web_sys::window()
                    .and_then(|w| get(&w).ok())
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
            };
        let place = panel_place(
            anchor,
            size(web_sys::Window::inner_width),
            size(web_sys::Window::inner_height),
            height.try_get().unwrap_or(0.0),
        );
        format!(
            "--tp-left:{}px;--tp-top:{}px;--tp-max:{}px;",
            place.left, place.top, place.max_h
        )
    };
    let head = Memo::new(move |_| {
        store
            .autotune
            .try_with(|a| a.fresh().map(panel_head))
            .flatten()
            .unwrap_or_default()
    });
    let foot = Memo::new(move |_| {
        store
            .autotune
            .try_with(|a| a.fresh().map(panel_foot))
            .flatten()
            .unwrap_or_default()
    });
    let rows = Memo::new(move |_| {
        store
            .autotune
            .try_with(|a| a.fresh().map(panel_rows))
            .flatten()
            .unwrap_or_default()
    });
    let aut_on = move || {
        store
            .autotune
            .try_with(|a| a.fresh().is_some_and(|s| s.aut))
            .unwrap_or(false)
    };
    let badge = move || {
        aut_on().then(|| {
            view! {
                <span class="aut-badge" data-testid="autotune-panel-aut">
                    {AUT_BADGE}
                </span>
            }
        })
    };
    // Keyed by singer (PR #150 review N2): a state change keeps each row,
    // its refused flash included; a row reads itself from the list.
    let keys = move || {
        rows.try_with(|r| r.iter().map(PanelRow::key).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let row = move |k: RowKey| view! { <SingerRow at=k rows=rows /> };
    let map_line = move || foot.try_with(|f| f[0].clone()).unwrap_or_default();
    let live_line = move || foot.try_with(|f| f[1].clone()).unwrap_or_default();
    let song = move || head.try_get().unwrap_or_default();
    view! {
        <div
            class="tpanel"
            node_ref=panel
            style=place
            data-testid="autotune-panel"
        >
            <div class="tpanel-in">
                <div class="tp-head">
                    <span class="tp-title">"AUTOTUNE"</span>
                    {badge}
                    <span class="tp-song" data-testid="autotune-panel-song">
                        {song}
                    </span>
                </div>
                <For each=keys key=|k: &RowKey| k.clone() children=row />
                <div class="tp-foot">
                    <span data-testid="autotune-panel-map">{map_line}</span>
                    <span class="small" data-testid="autotune-panel-live">
                        {live_line}
                    </span>
                </div>
            </div>
        </div>
    }
}

/// One singer's row (`at` its key in `rows`), and its why line when it is
/// not right. Its switch acts at a tap's lift (PR #150 review M1: a slide
/// past 10 px, a cancel or a lost capture ends it with nothing done).
#[component]
fn SingerRow(at: RowKey, rows: Memo<Vec<PanelRow>>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let has_id = !at.0.is_empty();
    let id_attr = at.0.clone();
    let row_id = at.0.clone();
    let label = at.1.clone();
    let me = Memo::new(move |_| {
        rows.try_with(|r| r.iter().find(|x| x.id == at.0 && x.label == at.1).cloned())
            .flatten()
    });
    let read = move |f: fn(&PanelRow) -> String| {
        me.try_with(|r| r.as_ref().map(f))
            .flatten()
            .unwrap_or_default()
    };
    let flag =
        move |f: fn(&PanelRow) -> bool| me.try_with(|r| r.as_ref().is_some_and(f)).unwrap_or(false);
    let failed = RwSignal::new(false);
    let press = StoredValue::new(switch_press());
    // The lift of a tap: the singer off by hand or back on, as the row
    // shows it now, on this connection's state while connected.
    let act = move |pointer: i32| {
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        let acts = store
            .autotune
            .try_with_untracked(|a| tap_opens_panel(a.fresh(), connected))
            .unwrap_or(false);
        let value = me.try_get_untracked().flatten().map(|r| switch_value(&r));
        match value.filter(|_| acts) {
            Some((id, off)) => {
                trace_rail("singer", "autotune", &[], pointer);
                store.set_hub(HUB_AUTOTUNE_SINGER, json!({"id": id, "off": off}));
            }
            None => {
                trace_rail("refused", "autotune", &[], pointer);
                fail_flash(failed)("refused".to_string());
            }
        }
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let id = ev.pointer_id();
        let pressed = press
            .try_update_value(|h| h.down(id, ev.is_primary(), dom::now()))
            .unwrap_or(false);
        if pressed && let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(id);
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let _ = press.try_update_value(|h| h.moved(ev.pointer_id(), x, y));
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let lift = press
            .try_update_value(|h| h.up(id, dom::now()))
            .unwrap_or(Lift::Nothing);
        if switch_acts(lift) {
            act(id);
        }
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let _ = press.try_update_value(|h| h.cancel(ev.pointer_id()));
    };
    let switch = has_id.then(|| {
        let none: Vec<String> = Vec::new();
        let text = move || {
            me.try_with(|r| r.as_ref().and_then(|r| r.switch))
                .flatten()
                .unwrap_or("")
        };
        view! {
            <span
                class="tp-sw"
                class:on=move || !flag(|r| r.off)
                class:failed=move || failed.try_get().unwrap_or(false)
                use:owns_touches=none
                data-testid="autotune-singer-switch"
                data-id=id_attr
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_cancel
                on:lostpointercapture=on_cancel
            >
                {text}
            </span>
        }
    });
    let why = move || {
        me.try_with(|r| r.as_ref().and_then(|r| r.why.clone()))
            .flatten()
            .map(|why| {
                view! {
                    <div class="tp-why" data-testid="autotune-singer-why">
                        {why}
                    </div>
                }
            })
    };
    let key_class = move || {
        if flag(|r| r.ok) {
            "tp-key ok"
        } else {
            "tp-key bad"
        }
    };
    let chip_class = |on: bool| if on { "chip on" } else { "chip" };
    let stream_class = move || chip_class(flag(|r| r.stream.on));
    let pa_class = move || chip_class(flag(|r| r.pa.on));
    let stream_text = move || read(|r| r.stream.text.to_string());
    let pa_text = move || read(|r| r.pa.text.to_string());
    let key = move || read(|r| r.key.clone());
    view! {
        <div
            class="tp-row"
            class:off=move || flag(|r| r.off)
            data-testid="autotune-singer"
            data-id=row_id
            data-off=move || flag(|r| r.off).to_string()
            data-ok=move || flag(|r| r.ok).to_string()
        >
            <span class="tp-name">{label}</span>
            <span class="tp-chips">
                <span class=stream_class data-testid="autotune-chip-stream">
                    {stream_text}
                </span>
                <span class=pa_class data-testid="autotune-chip-pa">
                    {pa_text}
                </span>
            </span>
            <span class=key_class data-testid="autotune-singer-key">
                {key}
            </span>
            {switch}
        </div>
        {why}
    }
}
