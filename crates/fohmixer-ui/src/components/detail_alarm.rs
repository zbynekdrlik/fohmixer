//! The channel detail's MUTE ALARM (#188; the owner's ruling of 2026-11-13:
//! whether a fader belongs to the alarm is set through its SEL): a calm
//! section of `detail-right` before COLOR, its title, the state beside it
//! and two keys, OFF and ON, the state's key marked like a selected tab
//! (COLOR's keys, `colour.css`). One tap (its `pointerdown`) asks the hub
//! for that state (`LiveStore::set_mute_alarm`); the hub tells every page,
//! so the strip's alarm follows on every tablet at once; a refusal flashes
//! the key (`data-refused`). Nothing here writes a Live key.

use fohmixer_proto::layout::Strip;
use fohmixer_proto::sectors::StripRef;
use leptos::prelude::*;

use super::{fail_flash, owns_touches};
use crate::behave::mute_alarm::{KEYS, marked, word};
use crate::binding::sectors::detail_ref;
use crate::store::{LiveStore, on_refusal};

/// Whether a strip is marked for the mute alarm as the hub last told it
/// (not until then, and never for a strip with no identity): the strip's
/// `data-mute-alarm` and the detail's MUTE ALARM read it.
pub fn strip_alarm(store: LiveStore, strip: &Strip) -> Signal<bool> {
    let id = StoredValue::new(detail_ref(strip));
    let memo = Memo::new(move |_| {
        store
            .mute_alarms
            .try_with(|told| id.try_with_value(|id| marked(told.shown(), id.as_ref())))
            .flatten()
            .unwrap_or(false)
    });
    memo.into()
}

/// MUTE ALARM: the switch for the strip shown; the note for a strip with
/// no identity (a marker showing its placeholder), which cannot keep one.
#[component]
pub fn DetailMuteAlarm(strip: Strip, on: Signal<bool>) -> impl IntoView {
    let id = StoredValue::new(detail_ref(&strip));
    let named = id.try_with_value(Option::is_some).unwrap_or(false);
    let now = move || word(on.try_get().unwrap_or(false));
    let inner = if named {
        let keys = KEYS
            .into_iter()
            .map(|key| view! { <AlarmKey pick=key strip_id=id on=on /> })
            .collect_view();
        view! { <div class="alarm-keys">{keys}</div> }.into_any()
    } else {
        view! {
            <div class="lay-note" data-testid="detail-alarm-none">
                "This fader has no name yet, so it cannot keep a mute alarm."
            </div>
        }
        .into_any()
    };
    view! {
        <div class="alarm-sec" data-testid="detail-alarm">
            <div class="col-head">
                <h2 class="detail-title">"MUTE ALARM"</h2>
                <span class="col-now" data-testid="alarm-now">
                    {now}
                </span>
            </div>
            {inner}
        </div>
    }
}

/// One key of the switch, pressed while it is the strip's state. A tap
/// asks the hub for it; a refusal flashes it. It owns its touches and
/// writes no Live key.
#[component]
fn AlarmKey(
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
        store.set_mute_alarm(&strip, pick, Some(on_refusal(fail_flash(failed))));
    };
    let pressed = move || (on.try_get().unwrap_or(false) == pick).to_string();
    let name = word(pick);
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="col-key alarm-key"
            use:owns_touches=no_keys
            data-testid="alarm-key"
            data-on=pick.to_string()
            aria-pressed=pressed
            data-refused=move || failed.try_get().unwrap_or(false).to_string()
            on:pointerdown=on_down
        >
            {name}
        </button>
    }
}
