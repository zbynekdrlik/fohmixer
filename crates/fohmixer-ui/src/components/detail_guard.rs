//! The channel detail's MUTE GUARD (#203; the owner, 2026-11-14: which
//! fader's mute is protected by a hold is set through its SEL): a calm
//! section of `detail-right` after MUTE ALARM, the alarm's look (its
//! `alarm-sec` and keys, `alarm.css`): its title, the state beside it and
//! two keys, OFF and HOLD. One tap (its `pointerdown`) asks the hub for
//! that state (`LiveStore::set_mute_guard`); the hub tells every page, so
//! the strip's mute follows on every tablet at once; a refusal flashes the
//! key (`data-refused`). While the layout guards the strip (`+MG`, the
//! frame's list) a dim note says the switch cannot lift that. Nothing here
//! writes a Live key.

use fohmixer_proto::layout::Strip;
use fohmixer_proto::sectors::StripRef;
use leptos::prelude::*;

use super::{fail_flash, owns_touches};
use crate::behave::mute_alarm::marked;
use crate::behave::mute_guard::{KEYS, guarded, layout_note, word};
use crate::binding::sectors::detail_ref;
use crate::store::{LiveStore, on_refusal};

/// Whether the hub marks a strip's mute as guarded, as last told (not
/// until then, and never for a strip with no identity).
pub fn strip_guard_mark(store: LiveStore, strip: &Strip) -> Signal<bool> {
    let id = StoredValue::new(detail_ref(strip));
    let memo = Memo::new(move |_| {
        store
            .mute_guards
            .try_with(|told| id.try_with_value(|id| marked(told.shown(), id.as_ref())))
            .flatten()
            .unwrap_or(false)
    });
    memo.into()
}

/// Whether a strip's mute is guarded: the layout's guard or the hub's mark
/// (`behave::mute_guard::guarded`); a memo, so a mark of another strip
/// changes nothing here.
pub fn strip_guard(store: LiveStore, strip: &Strip) -> Signal<bool> {
    let layout = strip.mute_guard;
    let mark = strip_guard_mark(store, strip);
    let memo = Memo::new(move |_| guarded(layout, mark.try_get().unwrap_or(false)));
    memo.into()
}

/// MUTE GUARD: the switch for the strip shown (its hub mark, `on`); the
/// note for a strip with no identity, which cannot keep one.
#[component]
pub fn DetailMuteGuard(strip: Strip, on: Signal<bool>) -> impl IntoView {
    let id = StoredValue::new(detail_ref(&strip));
    let named = id.try_with_value(Option::is_some).unwrap_or(false);
    let layout = strip.mute_guard;
    let now = move || word(guarded(layout, on.try_get().unwrap_or(false)));
    let note = layout_note(layout).map(|text| {
        view! {
            <div class="lay-note" data-testid="detail-guard-layout">
                {text}
            </div>
        }
    });
    let inner = if named {
        let keys = KEYS
            .into_iter()
            .map(|key| view! { <GuardKey pick=key strip_id=id on=on /> })
            .collect_view();
        view! { <div class="alarm-keys">{keys}</div> }.into_any()
    } else {
        view! {
            <div class="lay-note" data-testid="detail-guard-none">
                "This fader has no name yet, so it cannot keep a mute guard."
            </div>
        }
        .into_any()
    };
    view! {
        <div class="alarm-sec guard-sec" data-testid="detail-guard">
            <div class="col-head">
                <h2 class="detail-title">"MUTE GUARD"</h2>
                <span class="col-now" data-testid="guard-now">
                    {now}
                </span>
            </div>
            {inner}
            {note}
        </div>
    }
}

/// One key of the switch, pressed while it is the hub's mark. A tap asks
/// the hub for it; a refusal flashes it. It owns its touches and writes no
/// Live key.
#[component]
fn GuardKey(
    pick: bool,
    strip_id: StoredValue<Option<StripRef>>,
    on: Signal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(Some(strip)) = strip_id.try_get_value() else {
            return;
        };
        store.set_mute_guard(&strip, pick, Some(on_refusal(fail_flash(failed))));
    };
    let pressed = move || (on.try_get().unwrap_or(false) == pick).to_string();
    let name = word(pick);
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="col-key alarm-key guard-key"
            use:owns_touches=no_keys
            data-testid="guard-key"
            data-on=pick.to_string()
            aria-pressed=pressed
            data-refused=move || failed.try_get().unwrap_or(false).to_string()
            on:pointerdown=on_down
        >
            {name}
        </button>
    }
}
