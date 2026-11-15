//! The sectors page's own parts of the column (#213, split from
//! `pages/sectors.rs`): the LAYOUTS block (a 2 × 2 grid of the sectors'
//! layout tabs; in ARRANGE it renames, adds and removes layouts,
//! `behave::arrange_edit`) and ARRANGE's `⇄` key beside the page tab
//! (#212), with the refused save's note. The page itself, its lines, the
//! drag and the sides' arrows stay in `pages/sectors.rs`.

use fohmixer_proto::sectors::{SectorId, SectorsDoc};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{Carrier, change_mode};
use crate::behave::arrange_drag::{Arrange, NoteTimer, Tap, tap_act};
use crate::behave::arrange_edit::{REMOVE_ASK, TabRow, TabTap, field_text, tab_rows};
use crate::behave::column::layout_pick_same;
use crate::binding::sectors::sector_name;
use crate::components::owns_touches;
use crate::diag;
use crate::diag::trace::{self, TabStep};
use crate::dom;
use crate::pages::surface::Nav;
use crate::store::ResultFn;

/// How long the refused save's note shows under ARRANGE (ms).
const NOTE_MS: u64 = 4000;

/// The refused save's note shows for `NOTE_MS`; its timer clears it only
/// while no newer note replaced it (`NoteTimer`).
fn flash_note(
    note: RwSignal<Option<&'static str>>,
    timer: StoredValue<NoteTimer>,
    text: &'static str,
) {
    let Some(mine) = timer.try_update_value(NoteTimer::show) else {
        return;
    };
    let _ = note.try_set(Some(text));
    set_timeout(
        move || {
            if timer.try_with_value(|t| t.expires(mine)).unwrap_or(false) {
                let _ = note.try_set(None);
            }
        },
        std::time::Duration::from_millis(NOTE_MS),
    );
}

/// A pointer listener of the page (capture phase, removed with the block).
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// Where a down keeps a pending REMOVE?: the asking tab and every ✕.
const ASK_SPOT: &str = ".tab.confirm, [data-testid=\"remove\"]";

/// Where a down keeps a rename going: its field.
const FIELD_SPOT: &str = ".tab-input";

/// The LAYOUTS block: the sectors' cells as the screen has them (`lu ru`
/// over `ld rd`), from `shown` (the hub's document, ARRANGE's edit while
/// the mode is on). In ARRANGE a down anywhere off the asking tab and the
/// ✕ keys drops a pending REMOVE?, and one off the field ends a rename
/// keeping its text (`Edit::outside_down`), before the down's own handler:
/// a capture-phase listener on the window.
#[component]
pub(super) fn LayoutsBlock(shown: Memo<Option<SectorsDoc>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
        let pending = mode
            .try_with_untracked(|m| {
                m.edit_now()
                    .is_some_and(|e| e.confirming.is_some() || e.renaming.is_some())
            })
            .unwrap_or(false);
        if !pending {
            return;
        }
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let within = |spot: &str| {
            target
                .as_ref()
                .and_then(|t| t.closest(spot).ok().flatten())
                .is_some()
        };
        let (on_ask, on_field) = (within(ASK_SPOT), within(FIELD_SPOT));
        let _ = change_mode(mode, |m| {
            if let Some(edit) = m.edit() {
                edit.outside_down(on_ask, on_field);
            }
        });
        if !on_field {
            blur_field();
        }
    }) as Box<dyn FnMut(web_sys::PointerEvent)>);
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback_and_bool(
            "pointerdown",
            listener.as_ref().unchecked_ref(),
            true,
        );
    }
    let any_down = StoredValue::new_local(Some::<DownListener>(listener));
    on_cleanup(move || {
        if let (Some(Some(listener)), Some(window)) =
            (any_down.try_update_value(Option::take), web_sys::window())
        {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    });
    let cells = [SectorId::Lu, SectorId::Ru, SectorId::Ld, SectorId::Rd]
        .into_iter()
        .map(|sector| view! { <LayoutCell sector=sector shown=shown /> })
        .collect_view();
    // No caption over it since #212 (the owner's approved
    // `docs/mockups/column-calm-v2.html`).
    view! {
        <div class="sectors" role="group" aria-label="sector layouts" data-testid="sector-layouts">
            {cells}
        </div>
    }
}

/// A rename field that has the focus lets go of it (its rename ended
/// elsewhere: the keyboard goes down).
fn blur_field() {
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element());
    if let Some(field) = active
        .filter(|el| el.matches(FIELD_SPOT).unwrap_or(false))
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = field.blur();
    }
}

/// A sector's cell of the block: a tab per layout of `shown`, the shown one
/// marked; a tap pages the sector to it (on `pointerdown`). In ARRANGE
/// each tab carries ✎ and ✕ (none on the sector's last layout) and the
/// cell a `+` while it holds fewer than four layouts (`TabRow`,
/// `Edit::can_add`); a new layout is shown at once on this device.
#[component]
fn LayoutCell(sector: SectorId, shown: Memo<Option<SectorsDoc>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let rows = Memo::new(move |_| {
        shown
            .try_with(|doc| {
                let doc = doc.as_ref()?;
                nav.picks
                    .try_with(|picks| mode.try_with(|m| tab_rows(doc, picks, sector, m.edit_now())))
                    .flatten()
            })
            .flatten()
            .unwrap_or_default()
    });
    let ids = Memo::new(move |_| {
        rows.try_with(|rows| rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    let can_add = Memo::new(move |_| {
        mode.try_with(|m| m.edit_now().is_some_and(|e| e.can_add(sector)))
            .unwrap_or(false)
    });
    // A left sector's cell under an extra tab (#148 review): inert and
    // dimmed, its layouts hidden under the pane.
    let covered = move || sector.left() && nav.left_covered();
    let on_add = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if sector.left() && nav.left_covered_now() {
            return;
        }
        let added = change_mode(mode, |m| m.edit().and_then(|e| e.add(sector))).flatten();
        if let Some(id) = added {
            nav.pick(sector, &id);
        }
    };
    let add = move || {
        can_add.try_get().unwrap_or(false).then(|| {
            // It owns its touches (#43 PR G); it writes no key.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="tab-add"
                    use:owns_touches=no_keys
                    data-testid="add-layout"
                    aria-label="add a layout"
                    on:pointerdown=on_add
                >
                    "+"
                </button>
            }
        })
    };
    let tab = move |id: String| view! { <LayoutTab sector=sector layout=id rows=rows /> };
    view! {
        <div
            class="sec-cell"
            data-sector=sector.as_str()
            title=sector_name(sector)
            aria-disabled=move || covered().to_string()
        >
            <For each=move || ids.try_get().unwrap_or_default() key=|id: &String| id.clone() children=tab />
            {add}
        </div>
    }
}

/// One layout's tab (keyed by its id, so a rename's field keeps its focus
/// while its row changes): the tab (`REMOVE?` while its ✕ asks), its
/// rename field (a real `<input>`: the iPad's keyboard; hidden until ✎,
/// which fills it, focuses it and selects its text in the tap itself) and
/// its ✎ ✕ keys (`sectors.css` shows them only in ARRANGE).
#[component]
fn LayoutTab(sector: SectorId, layout: String, rows: Memo<Vec<TabRow>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let id_attr = layout.clone();
    let tab_attr = layout.clone();
    let id = StoredValue::new(layout);
    let row = Memo::new(move |_| {
        id.try_with_value(|id| {
            rows.try_with(|rows| rows.iter().find(|r| r.id == *id).cloned())
                .flatten()
        })
        .flatten()
    });
    let field = NodeRef::<html::Input>::new();
    let flag = move |pick: fn(&TabRow) -> bool| {
        row.try_with(|r| r.as_ref().is_some_and(pick))
            .unwrap_or(false)
    };
    let asks = move || flag(|r| r.asks);
    let lit = move || flag(|r| r.selected && !r.asks);
    let renaming = move || flag(|r| r.renaming);
    let removable = move || flag(|r| r.removable);
    let text = move || {
        row.try_with(|r| {
            r.as_ref().map(|r| {
                if r.asks {
                    REMOVE_ASK.to_string()
                } else {
                    r.title.clone()
                }
            })
        })
        .flatten()
        .unwrap_or_default()
    };
    // A left sector's layouts under an extra tab take no tap (#148 review).
    let inert = move || sector.left() && nav.left_covered_now();
    let on_tab = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let Some(id) = id.try_get_value() else {
            return;
        };
        let arranging = mode.try_with_untracked(Arrange::is_on).unwrap_or(false);
        // The layout its sector shows before the tap (`layout_pick_same`).
        let shown = rows
            .try_with_untracked(|rows| rows.iter().find(|r| r.selected).map(|r| r.id.clone()))
            .flatten();
        let tapped = change_mode(mode, |m| m.edit().map(|e| e.tab_tapped(&id))).flatten();
        if tapped != Some(TabTap::Removed) {
            // The pick goes to the recorder (#212: the column's use counted
            // from the event log); a REMOVE? confirm is no pick.
            let title = row
                .try_with_untracked(|r| r.as_ref().map(|r| r.title.clone()))
                .flatten()
                .unwrap_or_default();
            diag::record(&trace::tab(
                dom::epoch_now(),
                TabStep::Layout {
                    sector: sector.as_str(),
                    id: &id,
                    title: &title,
                    arranging,
                    same: layout_pick_same(shown.as_deref(), &id),
                },
            ));
            nav.pick(sector, &id);
        }
    };
    let on_pen = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let Some(id) = id.try_get_value() else {
            return;
        };
        let title = change_mode(mode, |m| m.edit().and_then(|e| e.rename_start(&id))).flatten();
        if let (Some(title), Some(input)) = (title, field.try_get_untracked().flatten()) {
            input.set_value(&title);
            let _ = input.focus();
            input.select();
        }
    };
    let on_x = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let _ = id.try_with_value(|id| {
            change_mode(mode, |m| {
                if let Some(edit) = m.edit() {
                    edit.ask_remove(id);
                }
            })
        });
    };
    let on_input = move || {
        let Some(input) = field.try_get_untracked().flatten() else {
            return;
        };
        let value = input.value();
        let shown = change_mode(mode, |m| m.edit().map(|e| e.typed(&value)))
            .flatten()
            .unwrap_or_else(|| field_text(&value));
        if shown != value {
            input.set_value(&shown);
        }
    };
    let finish = move |keep: bool| {
        let _ = id.try_with_value(|id| {
            change_mode(mode, |m| {
                if let Some(edit) = m.edit() {
                    edit.rename_end_of(id, keep);
                }
            })
        });
    };
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let keep = match ev.key().as_str() {
            "Enter" => true,
            "Escape" => false,
            _ => return,
        };
        ev.prevent_default();
        finish(keep);
        if let Some(input) = field.try_get_untracked().flatten() {
            let _ = input.blur();
        }
    };
    let cross = move || {
        removable().then(|| {
            // It owns its touches (#43 PR G); it writes no key.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="tool x"
                    use:owns_touches=no_keys
                    data-testid="remove"
                    aria-label="remove the layout"
                    on:pointerdown=on_x
                >
                    "✕"
                </button>
            }
        })
    };
    // The tab and ✎ own their touches (#43 PR G); they write no key.
    let tab_keys: Vec<String> = Vec::new();
    let pen_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="lay-ed"
            data-layout=id_attr
            data-renaming=move || renaming().to_string()
        >
            <button
                type="button"
                class="tab"
                use:owns_touches=tab_keys
                class:selected=lit
                class:confirm=asks
                data-testid="sector-tab"
                data-layout=tab_attr
                aria-pressed=move || lit().to_string()
                on:pointerdown=on_tab
            >
                {text}
            </button>
            <input
                class="tab-input"
                type="text"
                node_ref=field
                maxlength="8"
                autocapitalize="characters"
                autocomplete="off"
                spellcheck="false"
                enterkeyhint="done"
                aria-label="layout name"
                data-testid="rename-input"
                on:input=move |_| on_input()
                on:keydown=on_key
                on:blur=move |_| finish(true)
            />
            <div class="lay-tools">
                <button
                    type="button"
                    class="tool pen"
                    use:owns_touches=pen_keys
                    data-testid="rename"
                    aria-label="rename the layout"
                    on:pointerdown=on_pen
                >
                    "✎"
                </button>
                {cross}
            </div>
        </div>
    }
}

/// ARRANGE (#148): a tap starts the edit mode; the next one sends every
/// sector's layouts as one `SectorEdit` (a rename still in its field kept)
/// and leaves once the hub's new document arrives (a refusal: at once, the
/// hub's document shows again with a short note), or just leaves with
/// nothing changed (`Arrange::tap`). Lit while on; it stays live, and so
/// does the LAYOUTS block, while the rest of the column is inert. Since
/// #212 a small `⇄` key at the right of the page tab (`ColumnHead`'s
/// `beside`), named ARRANGE for assistive tech; each press goes to the
/// recorder (`tab` `arrange` with `tap_act`).
#[component]
pub(super) fn ArrangeButton(
    carrier: Carrier,
    refused: RwSignal<Option<&'static str>>,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let timer = StoredValue::new(NoteTimer::default());
    let store = nav.store;
    let mode = nav.arrange;
    let on = Memo::new(move |_| mode.try_with(Arrange::is_on).unwrap_or(false));
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        // Under an extra tab the left sectors it would arrange are hidden
        // (#148 review): inert and dimmed.
        if nav.left_covered_now() {
            return;
        }
        let Some(doc) = store
            .sectors
            .try_with_untracked(|told| told.shown().cloned())
            .flatten()
        else {
            return;
        };
        let _ = refused.try_set(None);
        // A disposed mode (`change_mode`'s outer none) is no press to
        // record; a save on its way is `busy`.
        let tapped = change_mode(mode, |m| m.tap(&doc));
        if let Some(tap) = tapped.as_ref() {
            diag::record(&trace::tab(
                dom::epoch_now(),
                TabStep::Arrange {
                    act: tap_act(tap.as_ref()),
                },
            ));
        }
        let tap = tapped.flatten();
        carrier.leave();
        if let Some(Tap::Send(sectors, rev)) = tap {
            let done: ResultFn = Box::new(move |outcome| {
                let note = change_mode(mode, |m| m.heard(&outcome)).flatten();
                if let Some(note) = note {
                    flash_note(refused, timer, note);
                }
            });
            store.sector_edit(sectors, rev, Some(done));
        }
    };
    let lit = move || on.try_get().unwrap_or(false);
    // It owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tab arrange-key"
            use:owns_touches=no_keys
            class:selected=lit
            data-testid="arrange"
            aria-label="ARRANGE"
            title="ARRANGE"
            aria-pressed=move || lit().to_string()
            aria-disabled=move || nav.left_covered().to_string()
            on:pointerdown=on_down
        >
            "⇄"
        </button>
    }
}
