//! The channel detail's channel selector (#205, the owner's approved mockup
//! `docs/mockups/channel-select-v1.html`; it replaced the bar's ◀ ▶): the
//! bar's name chip, while the hub's document places faders, is a button
//! (`chan-open`, a `▾` after the name) that opens a sheet
//! (`components::sheet`) of every fader of the pool, grouped by Live set,
//! a fader no layout holds marked NOT PLACED and the one shown outlined.
//! One tap (at its lift) opens that fader's detail (`DetailStrip::pick`,
//! recorded as an open with `why: "pick"`), where LAYOUTS' `+ ADD TO
//! LAYOUT` (#208) puts it back. The list is `binding::channels`; nothing
//! here writes a Live key.

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

use super::detail::DetailStrip;
use super::owns_touches;
use super::sheet::{LiftKey, Sheet, after_lift};
use crate::binding::channels::{Channel, ChannelGroup, channel_groups, picked};
use crate::store::LiveStore;

/// The bar's name chip (`detail-name`, the strip's colour as `style`) and,
/// while the selector is live, the button around it and its sheet.
#[component]
pub fn ChannelSelect(strip: Strip, label: String, chip: Signal<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let strip = StoredValue::new(strip);
    let groups = Memo::new(move |_| {
        let Some(layout) = store.layout.try_get().flatten() else {
            return Vec::new();
        };
        store
            .sectors
            .try_with(|told| strip.try_with_value(|s| channel_groups(told.shown(), &layout, s)))
            .flatten()
            .unwrap_or_default()
    });
    let live = Memo::new(move |_| groups.try_with(|g| !g.is_empty()).unwrap_or(false));
    let open = RwSignal::new(false);
    let label = StoredValue::new(label);
    let name = move || {
        view! {
            <span class="detail-chip" data-testid="detail-name" style=move || chip.try_get().unwrap_or_default()>
                {label.try_get_value().unwrap_or_default()}
            </span>
        }
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = open.try_set(true);
    };
    let sheet = move || {
        open.try_get().unwrap_or(false).then(|| {
            // Drawn anew when the list changes (another page's ARRANGE, a
            // new layout): rare, and no state lives in a pick.
            let list = move || {
                groups
                    .try_get()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|group| view! { <GroupView group=group open=open /> })
                    .collect_view()
            };
            view! {
                <Sheet title="CHANNEL" testid="chan-sheet" open=open wide=true>
                    {list}
                </Sheet>
            }
        })
    };
    move || {
        if live.try_get().unwrap_or(false) {
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="chan-open"
                    use:owns_touches=no_keys
                    data-testid="chan-open"
                    aria-label="Pick another channel"
                    on:pointerdown=on_down
                >
                    {name()}
                </button>
                {sheet}
            }
            .into_any()
        } else {
            name().into_any()
        }
    }
}

/// One Live set's name and its faders.
#[component]
fn GroupView(group: ChannelGroup, open: RwSignal<bool>) -> impl IntoView {
    let picks = group
        .channels
        .into_iter()
        .map(|channel| view! { <ChannelPick channel=channel open=open /> })
        .collect_view();
    view! {
        <div class="sheet-group" data-testid="chan-group" data-instance=group.instance>
            {group.name}
        </div>
        <div class="sheet-grid">{picks}</div>
    }
}

/// One fader of the sheet: its name, `NOT PLACED` when no layout holds it,
/// outlined while it is the one shown. Its tap (at its lift) opens its
/// detail a task later (`after_lift`: the new detail replaces this sheet).
#[component]
fn ChannelPick(channel: Channel, open: RwSignal<bool>) -> impl IntoView {
    let detail = use_context::<DetailStrip>();
    let key = LiftKey::default();
    let target = StoredValue::new(channel.strip.clone());
    let current = channel.current;
    let on_up = move |ev: web_sys::PointerEvent| {
        if !key.up(&ev) {
            return;
        }
        let pointer = ev.pointer_id();
        after_lift(move || {
            if current {
                let _ = open.try_set(false);
            } else if let (Some(detail), Some(strip)) = (detail, target.try_get_value()) {
                detail.pick(picked(&strip), pointer);
            }
        });
    };
    let mark = (!channel.placed).then(|| view! { <small class="chan-np">"NOT PLACED"</small> });
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="sheet-pick chan-pick"
            class:current=current
            use:owns_touches=no_keys
            data-testid="chan-pick"
            data-instance=channel.id.instance
            data-label=channel.id.label
            data-placed=channel.placed.to_string()
            data-current=current.to_string()
            on:pointerdown=move |ev| key.down(&ev)
            on:pointermove=move |ev| key.moved(&ev)
            on:pointerup=on_up
            on:pointercancel=move |ev| key.cancel(&ev)
            on:lostpointercapture=move |ev| key.cancel(&ev)
        >
            <span class="sheet-pick-name">{channel.label}</span>
            {mark}
        </button>
    }
}
