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
//! the page without one). Its height is measured once it is in the page,
//! so it stays inside the screen (`behave::autotune::panel_place`). The
//! decisions are `behave::autotune`'s.

use fohmixer_proto::client::HUB_AUTOTUNE_SINGER;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{fail_flash, owns_touches, trace_rail};
use crate::behave::autotune::{
    PanelRow, panel_foot, panel_head, panel_place, panel_rows, switch_value, tap_opens_panel,
};
use crate::behave::rail::AUT_BADGE;
use crate::store::LiveStore;

/// The panel: `anchor` the column's right edge and the button's top on the
/// screen at the tap that opened it. The button's page listener tells a tap
/// outside it by its class (`.tpanel`). Its root takes no touch of its own
/// (it holds the switches, which own theirs).
#[component]
pub fn AutotunePanel(anchor: (f64, f64)) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let panel = NodeRef::<html::Div>::new();
    // The panel's height once in the page, to keep it inside the screen.
    let height = RwSignal::new(0.0_f64);
    panel.on_load(move |el: web_sys::HtmlDivElement| {
        let _ = height.try_set(f64::from(el.offset_height()));
    });
    let place = move || {
        let view_h = web_sys::window()
            .and_then(|w| w.inner_height().ok())
            .and_then(|h| h.as_f64())
            .unwrap_or(0.0);
        let (left, top) = panel_place(anchor.0, anchor.1, view_h, height.try_get().unwrap_or(0.0));
        format!("--tp-left:{left}px;--tp-top:{top}px;")
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
    let lines = move || {
        rows.try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|row| view! { <SingerRow row=row /> })
            .collect_view()
    };
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
                {lines}
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

/// One singer's row, and its why line when its key does not read back.
#[component]
fn SingerRow(row: PanelRow) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let (id, off) = switch_value(&row);
    let id_attr = row.id.clone();
    // The switch is lit while the singer is on (its tap turns it off).
    let switch_on = !row.off;
    let switch = row.switch.map(|text| {
        let on_down = move |ev: web_sys::PointerEvent| {
            ev.prevent_default();
            let connected = store.connected.try_get_untracked().unwrap_or(false);
            let acts = store
                .autotune
                .try_with_untracked(|a| tap_opens_panel(a.fresh(), connected))
                .unwrap_or(false);
            if acts {
                trace_rail("singer", "autotune", &[], ev.pointer_id());
                store.set_hub(HUB_AUTOTUNE_SINGER, json!({"id": id, "off": off}));
            } else {
                trace_rail("refused", "autotune", &[], ev.pointer_id());
                fail_flash(failed)("refused".to_string());
            }
        };
        let none: Vec<String> = Vec::new();
        view! {
            <span
                class="tp-sw"
                class:on=switch_on
                class:failed=move || failed.try_get().unwrap_or(false)
                use:owns_touches=none
                data-testid="autotune-singer-switch"
                data-id=id_attr
                on:pointerdown=on_down
            >
                {text}
            </span>
        }
    });
    let why = row.why.clone().map(|why| {
        view! {
            <div class="tp-why" data-testid="autotune-singer-why">
                {why}
            </div>
        }
    });
    let key_class = if row.ok { "tp-key ok" } else { "tp-key bad" };
    let stream_class = if row.stream.on { "chip on" } else { "chip" };
    let pa_class = if row.pa.on { "chip on" } else { "chip" };
    let row_off = row.off;
    let row_id = row.id.clone();
    let off_attr = row.off.to_string();
    let ok_attr = row.ok.to_string();
    let label = row.label.clone();
    let key = row.key.clone();
    let (stream_text, pa_text) = (row.stream.text, row.pa.text);
    view! {
        <div
            class="tp-row"
            class:off=row_off
            data-testid="autotune-singer"
            data-id=row_id
            data-off=off_attr
            data-ok=ok_attr
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
