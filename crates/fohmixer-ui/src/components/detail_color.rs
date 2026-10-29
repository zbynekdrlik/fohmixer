//! The channel detail's COLOR (#164; the owner's approved mockup
//! `docs/mockups/strip-color-v1.html`): a section of `detail-right` under
//! LAYOUTS, a 7 × 2 grid of NONE (two cells wide) and the twelve calm
//! colours (`behave::colour::palette_keys`), the chosen key marked like a
//! selected tab and its name beside the title. One tap (its `pointerdown`)
//! asks the hub to set the strip's colour (`LiveStore::set_color`); the hub
//! tells every page, so the strip's 3 px edge and the detail's chip follow
//! on every tablet at once; a refusal flashes the key (`data-refused`).
//! Nothing here writes a Live key.

use fohmixer_proto::colors::{ColorKey, shown_name};
use fohmixer_proto::layout::Strip;
use fohmixer_proto::sectors::StripRef;
use leptos::prelude::*;

use super::{fail_flash, owns_touches};
use crate::behave::colour::{colour_of, key_id, palette_keys, swatch_style};
use crate::binding::sectors::detail_ref;
use crate::store::{LiveStore, on_refusal};

/// A strip's colour as the hub last told it (NONE until then, and for a
/// strip with no identity): the strip's edge, the detail's chip and COLOR
/// read it.
pub fn strip_colour(store: LiveStore, strip: &Strip) -> Signal<Option<ColorKey>> {
    let id = StoredValue::new(detail_ref(strip));
    let memo = Memo::new(move |_| {
        store
            .colors
            .try_with(|told| id.try_with_value(|id| colour_of(told.shown(), id.as_ref())))
            .flatten()
            .flatten()
    });
    memo.into()
}

/// COLOR: the palette for the strip shown; the note for a strip with no
/// identity (a marker showing its placeholder), which cannot keep one.
#[component]
pub fn DetailColor(strip: Strip, colour: Signal<Option<ColorKey>>) -> impl IntoView {
    let id = StoredValue::new(detail_ref(&strip));
    let named = id.try_with_value(Option::is_some).unwrap_or(false);
    let now = move || shown_name(colour.try_get().flatten());
    let inner = if named {
        let keys = palette_keys()
            .into_iter()
            .map(|key| view! { <PaletteKey pick=key strip_id=id colour=colour /> })
            .collect_view();
        view! { <div class="col-grid">{keys}</div> }.into_any()
    } else {
        view! {
            <div class="lay-note" data-testid="detail-colour-none">
                "This fader has no name yet, so it cannot keep a colour."
            </div>
        }
        .into_any()
    };
    view! {
        <div class="col-sec" data-testid="detail-colour">
            <div class="col-head">
                <h2 class="detail-title">"COLOR"</h2>
                <span class="col-now" data-testid="colour-now">
                    {now}
                </span>
            </div>
            {inner}
        </div>
    }
}

/// One key of the palette: its swatch (NONE with its word), pressed while
/// it is the strip's colour. A tap asks the hub for it; a refusal flashes
/// it. It owns its touches and writes no Live key.
#[component]
fn PaletteKey(
    pick: Option<ColorKey>,
    strip_id: StoredValue<Option<StripRef>>,
    colour: Signal<Option<ColorKey>>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let key = pick;
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(Some(strip)) = strip_id.try_get_value() else {
            return;
        };
        store.set_color(&strip, key, Some(on_refusal(fail_flash(failed))));
    };
    let pressed = move || (colour.try_get().flatten() == key).to_string();
    let none = key.is_none();
    let word = none.then(|| view! { <span>"NONE"</span> });
    let name = shown_name(key);
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="col-key"
            class:none=none
            use:owns_touches=no_keys
            data-testid="colour-key"
            data-colour=key_id(key)
            aria-label=name
            title=name
            aria-pressed=pressed
            data-refused=move || failed.try_get().unwrap_or(false).to_string()
            style=swatch_style(key)
            on:pointerdown=on_down
        >
            <span class="sw"></span>
            {word}
        </button>
    }
}
