//! The mixer surface (the redesign, #21; spec §4.2): the top bar (the pages'
//! tabs, the Stream Deck tab (#52, last, only from a hub with one), the page
//! pager's tabs, the SOLO ✕ pill, the dropout counter (#43), the connection
//! badges and the version), the page's rail with the global controls at its
//! foot, and the page's rows of sections (or the Stream Deck's keys,
//! `pages::deck`). Only the controls on screen are mounted, so only their
//! bindings are subscribed; the selected page and sub-page are remembered on
//! the device, the Stream Deck tab never is. The one number the stylesheet
//! cannot decide, the strip width all rows share, comes from `flow`.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Binding, Control, Group, Layout, Page, Section};
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use crate::app::version_text;
use crate::behave::solo::soloed;
use crate::binding::{SubSpec, choose, page_solos, selected_path, solo_sub, visible_subs};
use crate::components::{ControlView, Settings, fail_flash, key_of, owns_surface, owns_touches};
use crate::dom;
use crate::flow::{METRICS, Shape, overflows, pager_shape, row_shape, strip_width};
use crate::pages::deck::DeckView;
use crate::store::{Badge, LiveStore, Slot};

/// Where the selected pages are remembered (a JSON map: `""` for the pages,
/// a page's id for its pager → the selected page's id).
const PAGES_KEY: &str = "fohmixer_pages";
/// The rows' padding, both sides (`.rows` in the stylesheet).
const ROWS_PAD: f64 = 20.0;

/// The page selection (a context of the surface).
#[derive(Clone, Copy)]
struct Nav {
    store: LiveStore,
    /// The page shown, then its pager's sub-page.
    path: RwSignal<Vec<usize>>,
    remembered: RwSignal<BTreeMap<String, String>>,
    viewport: RwSignal<(f64, f64)>,
    /// The Stream Deck tab is shown (#52); never remembered: a reload opens
    /// the last mixer page. Written only on a change: a `try_set` notifies
    /// even with the same value.
    deck: RwSignal<bool>,
    /// `deck` as every reader reads it (one memo): a pager tap rebuilds no
    /// page, and a tap on the open deck tab does not mount its page again
    /// (which would lift the held keys).
    deck_shown: Memo<bool>,
}

impl Nav {
    /// The page `index` of level `level` was tapped.
    fn select(self, level: usize, index: usize) {
        // A layout tab leaves the deck; the page shown is selected as before.
        if self.deck.try_get_untracked() == Some(true) {
            let _ = self.deck.try_set(false);
        }
        let Some(Some(layout)) = self.store.layout.try_get_untracked() else {
            return;
        };
        let Some(path) = self.path.try_get_untracked() else {
            return;
        };
        let Some(remembered) = self.remembered.try_update(|r| {
            choose(&layout, r, &path, level, index);
            r.clone()
        }) else {
            return;
        };
        if let Ok(text) = serde_json::to_string(&remembered) {
            dom::storage_set(PAGES_KEY, &text);
        }
        let _ = self.path.try_set(selected_path(&layout, &remembered));
    }

    /// The Stream Deck tab was tapped (#52).
    fn show_deck(self) {
        if self.deck.try_get_untracked() == Some(false) {
            let _ = self.deck.try_set(true);
        }
    }
}

/// The remembered page selection (nothing when none is stored or readable).
fn remembered_pages() -> BTreeMap<String, String> {
    dom::storage_get(PAGES_KEY)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The surface of a logged-in device.
#[component]
pub fn Surface(token: String, session: RwSignal<Option<String>>) -> impl IntoView {
    let store = LiveStore::new(token, session);
    provide_context(store);
    let viewport = RwSignal::new(dom::viewport());
    let deck = RwSignal::new(false);
    let nav = Nav {
        store,
        path: RwSignal::new(Vec::new()),
        remembered: RwSignal::new(remembered_pages()),
        viewport,
        deck,
        deck_shown: Memo::new(move |_| deck.get()),
    };
    provide_context(nav);
    on_cleanup(move || store.stop());
    store.start();

    let resize = window_event_listener(leptos::ev::resize, move |_| {
        let _ = viewport.try_set(dom::viewport());
    });
    on_cleanup(move || resize.remove());

    // A new layout: the remembered pages on it (or its defaults).
    Effect::new(move |_| {
        if let Some(layout) = store.layout.get() {
            let remembered = nav.remembered.try_get_untracked().unwrap_or_default();
            let _ = nav.path.try_set(selected_path(&layout, &remembered));
        }
    });
    // The controls on screen are the ones subscribed.
    Effect::new(move |_| {
        let Some(layout) = store.layout.get() else {
            return;
        };
        let path = nav.path.get();
        store.set_wanted(visible_subs(&layout, &path));
    });

    let content = move || match store.layout.get() {
        Some(layout) => view! { <Shell layout=layout /> }.into_any(),
        None => {
            let note = move || {
                store
                    .layout_note
                    .get()
                    .unwrap_or_else(|| "Connecting to the hub…".to_string())
            };
            view! {
                <div class="surface-wait" data-testid="no-layout">
                    <p>{note}</p>
                    <span class="version" data-testid="version">{version_text()}</span>
                </div>
            }
            .into_any()
        }
    };
    // No context menu, selection or drag starts on the surface (#43 PR G).
    view! {
        <div
            class="surface"
            use:owns_surface
            data-testid="surface"
            data-subs=move || store.subscribed.get().to_string()
            data-connected=move || store.connected.get().to_string()
            data-unfolds=move || store.unfolds.get().to_string()
        >
            {content}
        </div>
    }
}

/// One layout: the top bar and the selected page.
#[component]
fn Shell(layout: Arc<Layout>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    provide_context(Settings {
        shaping: layout.config.fader_shaping.unwrap_or(true),
        meter_source: layout.config.meter_source.unwrap_or_default(),
    });
    let page = Memo::new(move |_| nav.path.with(|p| p.first().copied()));
    let sub = Memo::new(move |_| nav.path.with(|p| p.get(1).copied()));
    let tabs: Vec<(String, String)> = layout
        .pages
        .iter()
        .map(|p| (p.id.clone(), p.title.clone()))
        .collect();
    let for_pager = layout.clone();
    let pager_tabs = move || {
        let index = page.get()?;
        if nav.deck_shown.get() {
            return None;
        }
        let pager = for_pager.pages.get(index)?.pager()?;
        let tabs: Vec<(String, String)> = pager
            .pages
            .iter()
            .map(|s| (s.id.clone(), s.title.clone()))
            .collect();
        Some(view! { <TabBar tabs=tabs level=1 selected=sub /> })
    };
    let for_solo = layout.clone();
    let solo = move || {
        let solos = page
            .get()
            .and_then(|i| for_solo.pages.get(i))
            .map(page_solos)
            .unwrap_or_default();
        view! { <SoloClear bindings=solos /> }
    };
    let for_page = layout.clone();
    let viewport = nav.viewport;
    let body = move || {
        if nav.deck_shown.get() {
            let global = for_page.global.clone();
            return Some(view! { <DeckView global=global viewport=viewport /> }.into_any());
        }
        let index = page.get()?;
        let shown = for_page.pages.get(index)?.clone();
        let global = for_page.global.clone();
        Some(view! { <PageView page=shown global=global sub=sub /> }.into_any())
    };
    view! {
        <div class="mixer" data-testid="stage">
            <header class="topbar">
                <TabBar tabs=tabs level=0 selected=page />
                {pager_tabs}
                <div class="spacer"></div>
                {solo}
                <StatusCluster />
            </header>
            {body}
        </div>
    }
}

/// A tab bar (a segmented control): one tab per page, the selected one lit.
#[component]
fn TabBar(
    tabs: Vec<(String, String)>,
    level: usize,
    selected: Memo<Option<usize>>,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let buttons = tabs
        .into_iter()
        .enumerate()
        .map(|(index, (id, title))| {
            let lit =
                move || selected.get() == Some(index) && !(level == 0 && nav.deck_shown.get());
            // A tab owns its touches (#43 PR G); it writes no key.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="tab"
                    use:owns_touches=no_keys
                    class:selected=lit
                    data-testid="tab"
                    data-page=id
                    data-selected=move || lit().to_string()
                    on:pointerdown=move |_| nav.select(level, index)
                >
                    {title}
                </button>
            }
        })
        .collect_view();
    // The Stream Deck's tab ends the pages' bar, from a hub with one (#52).
    let deck_tab = (level == 0).then(|| {
        let has_deck = move || nav.store.deck.with(Option::is_some);
        view! { <Show when=has_deck><DeckTab /></Show> }
    });
    view! {
        <div class="seg" data-testid="tabbar" data-level={level.to_string()}>
            {buttons}
            {deck_tab}
        </div>
    }
}

/// The Stream Deck's tab (#52): its title from the hub; a small red dot
/// while Companion (or the hub) is unreachable, no words.
#[component]
fn DeckTab() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let title = move || {
        store
            .deck
            .with(|d| d.as_ref().map(|d| d.title.clone()).unwrap_or_default())
    };
    let offline = move || store.deck.with(|d| d.as_ref().is_some_and(|d| !d.online));
    let lit = move || nav.deck_shown.get();
    // A tab owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tab deck-tab"
            use:owns_touches=no_keys
            class:selected=lit
            data-testid="deck-tab"
            data-selected=move || lit().to_string()
            data-offline=move || offline().to_string()
            on:pointerdown=move |_| nav.show_deck()
        >
            {title}
            <Show when=offline>
                <i class="deck-dot" data-testid="deck-dot"></i>
            </Show>
        </button>
    }
}

/// A page: its rail (the global controls at its foot) and its rows.
#[component]
fn PageView(page: Page, global: Vec<Control>, sub: Memo<Option<usize>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let rows_ref = NodeRef::<html::Div>::new();
    let avail = RwSignal::new(0.0_f64);
    // The rows' room: measured once they are laid out and on every resize.
    Effect::new(move |_| {
        let _ = nav.viewport.get();
        if let Some(el) = rows_ref.get() {
            let _ = avail.try_set(f64::from(el.client_width()) - ROWS_PAD);
        }
    });
    let shapes: Vec<Shape> = page.rows.iter().map(|r| row_shape(r, &METRICS)).collect();
    let width_shapes = shapes.clone();
    let width = Memo::new(move |_| strip_width(&width_shapes, avail.get(), &METRICS));
    let rail = page
        .rail
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let global = global
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let rows = page
        .rows
        .into_iter()
        .zip(shapes)
        .map(|(row, shape)| {
            let scrolls = move || overflows(shape, width.get(), avail.get());
            let flex = format!("flex:{} 1 0;", row.weight);
            let sections = row
                .sections
                .into_iter()
                .map(|section| match section {
                    Section::Group(group) => view! { <GroupView group=group /> }.into_any(),
                    Section::Pager(pager) => {
                        // As wide as its widest sub-page, whichever it shows.
                        let shape = pager_shape(&pager, &METRICS);
                        let size = move || format!("width:{:.1}px;", shape.width(width.get()));
                        let shown = move || {
                            let page = sub.get().and_then(|i| pager.pages.get(i).cloned())?;
                            let groups = page
                                .sections
                                .iter()
                                .flat_map(Section::groups)
                                .cloned()
                                .map(|group| view! { <GroupView group=group /> })
                                .collect_view();
                            Some(view! {
                                <div class="pager-page" data-testid="pager" data-page=page.id>
                                    {groups}
                                </div>
                            })
                        };
                        view! { <div class="pager" style=size>{shown}</div> }.into_any()
                    }
                })
                .collect_view();
            view! {
                <div class="row" class:scrolls=scrolls data-testid="row" style=flex>
                    {sections}
                </div>
            }
        })
        .collect_view();
    let conf = page
        .id
        .eq_ignore_ascii_case("conf")
        .then(|| view! { <ConfInfo /> });
    let strip_width_style = move || format!("--strip-w:{:.1}px;", width.get());
    view! {
        <div class="body" data-testid="page" data-page=page.id>
            <nav class="rail" data-testid="rail">
                <div class="rail-main">{rail}</div>
                <div class="rail-foot">{global}</div>
            </nav>
            <div class="rows" node_ref=rows_ref style=strip_width_style>
                {rows}
                {conf}
            </div>
        </div>
    }
}

/// A group of controls: its marker and title, then its controls side by
/// side (strips) or in a grid (buttons).
#[component]
fn GroupView(group: Group) -> impl IntoView {
    // Strips side by side; buttons in a grid; texts (the Conf page) one per
    // line.
    let columns = group.controls.iter().any(crate::flow::is_column);
    let texts = !columns
        && group
            .controls
            .iter()
            .all(|c| matches!(c, Control::Text { .. }));
    let buttons = !columns && !texts;
    let look = group
        .color
        .as_ref()
        .map(|c| format!("--gc:{c};"))
        .unwrap_or_default();
    let id = group.id.clone().unwrap_or_default();
    let title = group.title.clone().unwrap_or_default();
    let controls = group
        .controls
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    view! {
        <section
            class="group"
            class:buttons=buttons
            class:texts=texts
            data-testid="group"
            data-group=id
            style=look
        >
            <h2 class="group-title">
                <i class="group-mark"></i>
                <span data-testid="group-title">{title}</span>
            </h2>
            <div class="group-body">{controls}</div>
        </section>
    }
}

/// SOLO ✕ (#21): shown while any solo of the page is on; a tap turns them
/// off (owning instance only, X4: each solo's own binding).
#[component]
fn SoloClear(bindings: Vec<Binding>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let solos: Vec<(SubSpec, RwSignal<Slot>)> = bindings
        .iter()
        .filter_map(solo_sub)
        .map(|spec| {
            let slot = store.slot(&spec);
            (spec, slot)
        })
        .collect();
    let slots: Vec<RwSignal<Slot>> = solos.iter().map(|(_, s)| *s).collect();
    // The pill owns its touches (#43 PR G) and names the solos it clears.
    let touch_keys: Vec<String> = solos.iter().map(|(spec, _)| key_of(spec)).collect();
    let any = Memo::new(move |_| {
        let states: Vec<Option<bool>> = slots.iter().map(|s| s.with(Slot::flag)).collect();
        !soloed(&states).is_empty()
    });
    let solos = StoredValue::new(solos);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = solos.try_with_value(|all| {
            let states: Vec<Option<bool>> = all
                .iter()
                .map(|(_, s)| s.try_with_untracked(Slot::flag).flatten())
                .collect();
            for i in soloed(&states) {
                let (spec, _) = &all[i];
                // A tap: a final `set` per solo (#43).
                store.set(
                    &spec.instance,
                    &spec.target,
                    &spec.prop,
                    json!(false),
                    true,
                    Some(fail_flash(failed)),
                );
            }
        });
    };
    view! {
        <button
            type="button"
            class="solo-clear"
            use:owns_touches=touch_keys
            class:on=move || any.get()
            class:failed=move || failed.get()
            data-testid="solo-clear"
            data-on=move || any.get().to_string()
            on:pointerdown=on_down
        >
            "SOLO ✕"
        </button>
    }
}

/// The dropout counter (#43, §4.4; the owner's ruling of 2026-10-03): a
/// small number, the link's dropouts since the last tap, red while one
/// lasts; a tap resets it to 0. No words, no sound, no blinking: space on
/// the tablet is scarce and live mixing must not be disturbed.
#[component]
fn DropoutCounter() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let counter = store.dropouts;
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        store.reset_dropouts();
    };
    let active = move || counter.get().active;
    // The counter is a tap target: it owns its touches (#43 PR G).
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="dropouts"
            use:owns_touches=no_keys
            class:active=active
            data-testid="dropouts"
            data-active=move || active().to_string()
            on:pointerdown=on_down
        >
            {move || counter.get().count.to_string()}
        </button>
    }
}

/// The dropout counter, the connection badges (per instance: online, busy,
/// offline) and the version, at the right end of the top bar.
#[component]
fn StatusCluster() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let badges = move || {
        store
            .instances
            .get()
            .into_iter()
            .map(|(name, instance)| {
                let state = Badge::of(&instance).name();
                let instance_name = name.clone();
                view! {
                    <span
                        class={format!("badge {state}")}
                        data-testid="badge"
                        data-instance=instance_name
                        data-state=state
                    >
                        <i class="badge-dot"></i>
                        {name}
                    </span>
                }
            })
            .collect_view()
    };
    view! {
        <div class="status-cluster">
            <DropoutCounter />
            <span class="hub-dot" class:online=move || store.connected.get()></span>
            {badges}
            <span class="version" data-testid="version">{version_text()}</span>
        </div>
    }
}

/// The build on the Conf page (S4 design note §6).
#[component]
fn ConfInfo() -> impl IntoView {
    view! {
        <div class="conf-info" data-testid="conf-version">
            {format!("fohmixer {}", fohmixer_proto::full_version())}
        </div>
    }
}
