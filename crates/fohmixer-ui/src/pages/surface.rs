//! The mixer surface (the redesign, #21; spec §4.2; the control column,
//! #63): every page is its two sides and the control column between them.
//! The column holds the dropout counter (#43), the connection badges and
//! the version, the pages' tabs (the Stream Deck tab, #52, last, only from
//! a hub with one), the pager's tabs, the SOLO ✕ pill, the arrows of a line
//! that does not fit, the page's rail and the global controls at its foot;
//! nothing sits above the faders. Where the controls go is `arrange` (pure);
//! this file measures a side and the height and renders the arrangement
//! with keyed lists, so a sub-page switch or a shift remounts only the
//! controls that change. Only the controls on screen are mounted, and the
//! subscriptions are the selected path's with an open channel detail's
//! (`binding::wanted_subs`, #71); the selected page and sub-page are
//! remembered on the device, the Stream Deck tab and the detail never are.
//! A Pro-Q 4 screen opened from the detail (#71 PR E) lies over everything,
//! outside the layout's shell: a new layout keeps it; the detail closing
//! closes it.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Binding, Control, Layout, Page, Strip};
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use crate::app::version_text;
use crate::arrange::{Arrangement, Cell, Item, LineKey, METRICS, PageModel, Side, arrange, items};
use crate::behave::solo::soloed;
use crate::binding::{
    DetailChange, SubSpec, choose, detail_strip, detail_update, group_title, page_solos,
    selected_path, solo_sub, stored_pages, view_tap, wanted_subs,
};
use crate::components::detail::{DetailStrip, DetailView};
use crate::components::eq::{EqNav, EqScreen, EqTarget};
use crate::components::{ControlView, Settings, fail_flash, key_of, owns_surface, owns_touches};
use crate::dom;
use crate::flow::{is_column, shared_instance};
use crate::manual::manual_parts;
use crate::pages::deck::DeckView;
use crate::store::{Badge, LiveStore, Slot};

/// Where the selected pages are remembered (a JSON map: `""` for the pages,
/// a page's id for its pager → the selected page's id).
const PAGES_KEY: &str = "fohmixer_pages";
/// The page's padding, both sides (`.body` in the stylesheet).
const BODY_PAD: f64 = 12.0;
/// Between a side and the column (`.body`'s gap).
const BODY_GAP: f64 = 6.0;

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
    /// The page shown before a view (#68): a second tap on the view's
    /// button goes back to it.
    before_view: RwSignal<Option<String>>,
    /// The tag manual's overlay is open (#68: the column's ZNAČKY chip
    /// opens it). Here, not in a layout's shell: a Tuner edit in Live is a
    /// new layout, and it must not close the manual explaining it.
    manual: RwSignal<bool>,
    /// The channel detail's strip, as held (#71: a hold on a strip's ☰
    /// opens it). Here, not in a layout's shell: a new layout keeps it open
    /// while its strip is in it (`binding::detail_strip`).
    detail: RwSignal<Option<Strip>>,
    /// When a hold opened the detail (the page clock): its opening guard
    /// counts from it, so a new layout's remount does not guard again.
    detail_opened: StoredValue<f64>,
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
        // A view shown is never stored (#68): a reload opens the page
        // before it, with its tabs.
        let before = self.before_view.try_get_untracked().flatten();
        let stored = stored_pages(&layout, &remembered, before.as_deref());
        if let Ok(text) = serde_json::to_string(&stored) {
            dom::storage_set(PAGES_KEY, &text);
        }
        let _ = self.path.try_set(selected_path(&layout, &remembered));
    }

    /// A view's button was tapped (#68): the view, or back to the page shown
    /// before it (`binding::view_tap`).
    fn tap_view(self, view: usize) {
        let Some(Some(layout)) = self.store.layout.try_get_untracked() else {
            return;
        };
        let shown = self
            .path
            .try_with_untracked(|p| p.first().copied())
            .flatten();
        let before = self.before_view.try_get_untracked().flatten();
        let deck = self.deck.try_get_untracked().unwrap_or(false);
        let (target, remember) = view_tap(&layout, shown, deck, view, before.as_deref());
        let _ = self.before_view.try_set(remember);
        self.select(0, target);
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
        before_view: RwSignal::new(None),
        manual: RwSignal::new(false),
        detail: RwSignal::new(None),
        detail_opened: StoredValue::new(f64::NEG_INFINITY),
    };
    provide_context(nav);
    // A strip's ☰ opens the channel detail, its exit closes it (#71).
    provide_context(DetailStrip {
        strip: nav.detail,
        opened_at: nav.detail_opened,
    });
    // A Pro-Q 4 screen (#71 PR E): opened from the detail's cards; closed
    // with the detail.
    let eq = EqNav {
        target: RwSignal::new(None::<EqTarget>),
    };
    provide_context(eq);
    Effect::new(move |_| {
        if nav.detail.try_with(Option::is_none).unwrap_or(false) {
            eq.close("detail", None);
        }
    });
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
    // The controls on screen are the ones subscribed, with an open
    // detail's strip and its pan (#71).
    Effect::new(move |_| {
        let Some(layout) = store.layout.get() else {
            return;
        };
        let path = nav.path.get();
        let detail = nav.detail.try_get().flatten();
        store.set_wanted(wanted_subs(&layout, &path, detail.as_ref()));
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
    // The tag manual (#68) stays open through a new layout.
    let manual_view = move || nav.manual.get().then(|| view! { <ManualView /> });
    // The Pro-Q 4 screen too (#71 PR E).
    let eq_view = move || {
        eq.target
            .try_get()
            .flatten()
            .map(|target| view! { <EqScreen target=target /> })
    };
    // No context menu, selection or drag starts on the surface (#43 PR G).
    view! {
        <div
            class="surface"
            use:owns_surface
            data-testid="surface"
            data-subs=move || store.subscribed.get().to_string()
            data-connected=move || store.connected.get().to_string()
        >
            {content}
            {manual_view}
            {eq_view}
        </div>
    }
}

/// What the column's head shows (a context of the layout's shell, read by
/// every page's column).
#[derive(Clone)]
struct Head {
    layout: Arc<Layout>,
    page: Memo<Option<usize>>,
    sub: Memo<Option<usize>>,
}

/// One layout: the selected page or the Stream Deck tab, and an open
/// channel detail over it (#71).
#[component]
fn Shell(layout: Arc<Layout>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    provide_context(Settings {
        shaping: layout.config.fader_shaping.unwrap_or(true),
        meter_source: layout.config.meter_source.unwrap_or_default(),
        law: layout.config.fader_law.unwrap_or_default().into(),
    });
    let page = Memo::new(move |_| nav.path.with(|p| p.first().copied()));
    let sub = Memo::new(move |_| nav.path.with(|p| p.get(1).copied()));
    provide_context(Head {
        layout: layout.clone(),
        page,
        sub,
    });
    let viewport = nav.viewport;
    // The channel detail (#71): the layout's strip that is the held one
    // (`binding::detail_strip`: a marker strip follows its marker to any
    // index; its binding, label, guard and mark this layout's), over the
    // page. A new layout mounts it again with the shell, as it does the
    // page. The held strip follows what the layout makes of it
    // (`binding::detail_update`), written back so the wanted set subscribes
    // the keys the detail reads: at once for the detail carried in from the
    // previous layout (a placeholder-labelled marker closes: it has no
    // identity to follow), then on every change; a layout without that
    // strip, or with it only in conflict, closes it, and the flight
    // recorder hears why.
    let held = nav.detail;
    let detail_nav = DetailStrip {
        strip: held,
        opened_at: nav.detail_opened,
    };
    let apply = move |change: Option<DetailChange>| match change {
        Some(DetailChange::Follow(strip)) => {
            let _ = held.try_set(Some(strip));
        }
        Some(DetailChange::Close(why)) => detail_nav.close(why, None),
        None => {}
    };
    let carried = held.try_get_untracked().flatten();
    apply(detail_update(&layout, carried.as_ref(), true));
    let current = {
        let layout = layout.clone();
        Memo::new(move |_| {
            held.try_get()
                .flatten()
                .and_then(|strip| detail_strip(&layout, &strip))
        })
    };
    {
        let layout = layout.clone();
        Effect::new(move |_| {
            let change = held
                .try_with(|strip| detail_update(&layout, strip.as_ref(), false))
                .flatten();
            apply(change);
        });
    }
    let titles = layout.clone();
    let detail = move || {
        current.try_get().flatten().map(|strip| {
            let group = group_title(&titles, &strip.binding);
            view! { <DetailView strip=strip group=group /> }
        })
    };
    let body = move || {
        if nav.deck_shown.get() {
            let global = layout.global.clone();
            return Some(view! { <DeckView global=global viewport=viewport /> }.into_any());
        }
        let index = page.get()?;
        let shown = layout.pages.get(index)?.clone();
        let global = layout.global.clone();
        Some(view! { <PageView page=shown global=global sub=sub /> }.into_any())
    };
    view! {
        <div class="mixer" data-testid="stage">
            {body}
            {detail}
        </div>
    }
}

/// The head of a page's column: the dropout counter, the badges and the
/// version, the pages' tabs, the pager's tabs (none on the Stream Deck
/// tab) and SOLO ✕.
#[component]
pub fn ColumnHead() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let Head { layout, page, sub } = expect_context::<Head>();
    // A view (#68) is no page tab: the column's POHĽADY shows it.
    let tabs: Vec<(usize, String, String)> = layout
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.view)
        .map(|(i, p)| (i, p.id.clone(), p.title.clone()))
        .collect();
    let for_pager = layout.clone();
    // Every read tolerates a disposed value: a layout change disposes the
    // selection while the old column's effects may still run once.
    // While a view is shown (#68), the pager's tabs of the page before it
    // stay, dimmed.
    let pager_tabs = move || {
        let index = page.try_get().flatten()?;
        if nav.deck_shown.try_get().unwrap_or(false) {
            return None;
        }
        let shown = for_pager.pages.get(index)?;
        let (holder, dim) = if shown.view {
            let before = nav.before_view.try_get().flatten()?;
            (for_pager.pages.iter().find(|p| p.id == before)?, true)
        } else {
            (shown, false)
        };
        let pager = holder.pager()?;
        let tabs: Vec<(usize, String, String)> = pager
            .pages
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.id.clone(), s.title.clone()))
            .collect();
        Some(view! { <TabBar tabs=tabs level=1 selected=sub dim=dim /> })
    };
    // The views (#68): a button for each tag group the frame does not show.
    let views: Vec<(usize, String, String)> = layout
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.view)
        .map(|(i, p)| (i, p.id.clone(), p.title.clone()))
        .collect();
    let views_bar = (!views.is_empty()).then(|| {
        let buttons = views
            .into_iter()
            .map(|(index, id, title)| {
                let lit = move || {
                    page.try_get().flatten() == Some(index)
                        && !nav.deck_shown.try_get().unwrap_or(false)
                };
                // A view's button owns its touches (#43 PR G); it writes no key.
                let no_keys: Vec<String> = Vec::new();
                view! {
                    <button
                        type="button"
                        class="tab view"
                        use:owns_touches=no_keys
                        class:selected=lit
                        data-testid="view"
                        data-page=id
                        data-selected=move || lit().to_string()
                        on:pointerdown=move |_| nav.tap_view(index)
                    >
                        {title}
                    </button>
                }
            })
            .collect_view();
        view! {
            <div class="cap-title" data-testid="views-title">"Pohľady"</div>
            <div class="seg views" data-testid="views">
                {buttons}
            </div>
        }
    });
    let solo = move || {
        let solos = page
            .try_get()
            .flatten()
            .and_then(|i| layout.pages.get(i))
            .map(page_solos)
            .unwrap_or_default();
        view! { <SoloClear bindings=solos /> }
    };
    // `.column-head` draws no box of its own (`display: contents`): it marks
    // the head for the Stream Deck page, whose capture listener lifts no
    // hold on a down there (a tab or the counter: outside its page before
    // the column).
    view! {
        <div class="column-head">
            <StatusCluster />
            <TabBar tabs=tabs level=0 selected=page />
            {pager_tabs}
            {views_bar}
            {solo}
        </div>
    }
}

/// A tab bar (a segmented control): one tab per page, the selected one lit.
#[component]
fn TabBar(
    tabs: Vec<(usize, String, String)>,
    level: usize,
    selected: Memo<Option<usize>>,
    /// Dimmed and inert (#68: the pager's tabs while a view is shown).
    #[prop(optional)]
    dim: bool,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let buttons = tabs
        .into_iter()
        .map(|(index, id, title)| {
            let lit = move || {
                // A disposed selection (a layout change) lights nothing.
                selected.try_get().flatten() == Some(index)
                    && !(level == 0 && nav.deck_shown.try_get().unwrap_or(false))
            };
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
        <div class="seg" class:dim=dim data-testid="tabbar" data-level={level.to_string()}>
            {buttons}
            {deck_tab}
        </div>
    }
}

/// The tag manual's chip on the column's status line (#68): opens the
/// manual's overlay.
#[component]
fn ManualChip() -> impl IntoView {
    let open = expect_context::<Nav>().manual;
    // The chip owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="manual-chip"
            use:owns_touches=no_keys
            data-testid="manual-open"
            on:pointerdown=move |_| {
                let _ = open.try_set(true);
            }
        >
            "ZNAČKY"
        </button>
    }
}

/// The tag manual over the screen (#68): `znacky.html`, fetched from the
/// hub (one source with the page the tray opens), its `<main>` with its
/// `<style>` (`manual::manual_parts`); "✕ Zavrieť" closes it.
#[component]
fn ManualView() -> impl IntoView {
    let open = expect_context::<Nav>().manual;
    let parts: RwSignal<Option<Result<(String, String), String>>> = RwSignal::new(None);
    leptos::task::spawn_local(async move {
        let answer = match crate::net::fetch_text("GET", "/znacky.html", None, None).await {
            Ok((200, text)) => {
                manual_parts(&text).ok_or_else(|| "no manual in the page".to_string())
            }
            Ok((status, _)) => Err(format!("HTTP {status}")),
            Err(e) => Err(e),
        };
        if let Err(why) = &answer {
            dom::log(&format!("the tag manual did not load: {why}"));
        }
        let _ = parts.try_set(Some(answer));
    });
    let no_keys: Vec<String> = Vec::new();
    let content = move || match parts.try_get().flatten() {
        None => view! { <p class="manual-note">"Načítavam…"</p> }.into_any(),
        Some(Err(why)) => {
            view! { <p class="manual-note">{format!("Návod sa nenačítal: {why}")}</p> }.into_any()
        }
        Some(Ok((style, main))) => view! {
            <style>{style}</style>
            <div class="manual-body" inner_html=main></div>
        }
        .into_any(),
    };
    view! {
        <div class="manual-overlay" data-testid="manual">
            <button
                type="button"
                class="manual-close"
                use:owns_touches=no_keys
                data-testid="manual-close"
                on:pointerdown=move |_| {
                    let _ = open.try_set(false);
                }
            >
                "✕ Zavrieť"
            </button>
            {content}
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

/// A page: its lines, each cut in two by the column (the column: its head,
/// the arrows of the lines that do not fit, the rail and the global controls
/// at its foot).
#[component]
fn PageView(page: Page, global: Vec<Control>, sub: Memo<Option<usize>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let model = Arc::new(PageModel::of(&page));
    let weights: Vec<f64> = page.rows.iter().map(|r| r.weight).collect();
    let body_ref = NodeRef::<html::Div>::new();
    let column_ref = NodeRef::<html::Nav>::new();
    // A side's width and the lines' height: measured once laid out and on
    // every resize.
    let room = RwSignal::new((0.0_f64, 0.0_f64));
    Effect::new(move |_| {
        let _ = nav.viewport.get();
        if let (Some(body), Some(column)) = (body_ref.get(), column_ref.get()) {
            let width = f64::from(body.client_width());
            let column = f64::from(column.offset_width());
            let side = (width - BODY_PAD - column - 2.0 * BODY_GAP) / 2.0;
            let height = f64::from(body.client_height()) - BODY_PAD;
            let _ = room.try_set((side.max(0.0), height.max(0.0)));
        }
    });
    // Each line's window (a line that does not fit), from the column's
    // arrows, kept by the line's key (`PageModel::line_key`: its rows, and
    // the sub-page where the pager is): another sub-page, or a turned screen
    // that groups the rows into other lines, shows that line's window from
    // its start and coming back finds it where it was; a turned screen with
    // the same lines keeps each window (clamped by `shown`).
    let offsets = RwSignal::new(BTreeMap::<LineKey, usize>::new());
    // Every read below tolerates a disposed value (`try_*`): a layout change
    // disposes the page while its keyed lists' effects may still run once.
    let arranged = {
        let model = model.clone();
        Memo::new(move |_| {
            let (side, height) = room.try_get()?;
            let sub = sub.try_get()?;
            if side <= 0.0 {
                return None;
            }
            offsets.try_with(|offsets| {
                let offset = |key: &LineKey| offsets.get(key).copied().unwrap_or(0);
                arrange(&model, sub, side, height, offset, &METRICS)
            })
        })
    };
    let count = Memo::new(move |_| {
        arranged
            .try_with(|a| a.as_ref().map_or(0, |a| a.lines.len()))
            .unwrap_or(0)
    });
    let shifted = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .map(|a| {
                        a.lines
                            .iter()
                            .enumerate()
                            .filter(|(_, line)| line.shift.is_some())
                            .map(|(index, _)| index)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });
    // The lines' heights by their rows' weights (a line of several rows: 1),
    // and the strip width every line shares (floored: a side never overflows
    // by rounding).
    let body_style = move || {
        arranged
            .try_with(|a| {
                let Some(a) = a else {
                    return String::new();
                };
                // Each line's share as `arrange::lines` counts it: its
                // weight over the lines' (fractions summing to the lines'
                // count, so the lines fill the height).
                let line_weights: Vec<f64> = a
                    .lines
                    .iter()
                    .map(|line| match line.rows.as_slice() {
                        [row] => weights.get(*row).copied().unwrap_or(1.0),
                        _ => 1.0,
                    })
                    .collect();
                let total: f64 = line_weights.iter().sum();
                let count = line_weights.len() as f64;
                let rows: Vec<String> = line_weights
                    .iter()
                    .map(|w| format!("minmax(0,{}fr)", w / total * count))
                    .collect();
                let width = (a.width * 100.0).floor() / 100.0;
                format!(
                    "grid-template-rows:{};--strip-w:{width:.2}px;",
                    rows.join(" ")
                )
            })
            .unwrap_or_default()
    };
    let rail = page
        .rail
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let global = global
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let line_view = move |line: usize| {
        let model = model.clone();
        view! { <LineView arranged=arranged model=model line=line /> }
    };
    let shift =
        move |line: usize| view! { <ShiftView arranged=arranged offsets=offsets line=line /> };
    view! {
        <div class="body" data-testid="page" data-page=page.id node_ref=body_ref style=body_style>
            <For each=move || 0..count.try_get().unwrap_or(0) key=|line| *line children=line_view />
            <nav class="column" data-testid="column" node_ref=column_ref>
                <ColumnHead />
                <div class="shifts">
                    <For
                        each=move || shifted.try_get().unwrap_or_default()
                        key=|line| *line
                        children=shift
                    />
                </div>
                <div class="rail" data-testid="rail">
                    <div class="rail-main">{rail}</div>
                    <div class="rail-foot">{global}</div>
                </div>
            </nav>
        </div>
    }
}

/// One line: its items in one keyed list (`arrange::items`), each on its
/// side of the column (`page.css` orders the left side's, `.split` taking
/// the rest of the left half and the column, the right side's), so a cell
/// that changes side or group run moves and is not rebuilt.
#[component]
fn LineView(
    arranged: Memo<Option<Arrangement>>,
    model: Arc<PageModel>,
    line: usize,
) -> impl IntoView {
    let list = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .and_then(|a| a.lines.get(line))
                    .map(items)
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });
    // The line's row, and its left side's span (`--l-units` strips and
    // `--l-cells` cells): `.split` takes the rest of the left side and the
    // column, so the right side starts right after the column.
    let place = move || {
        let (units, cells) = arranged
            .try_with(|a| {
                a.as_ref()
                    .and_then(|a| a.lines.get(line))
                    .map(|l| (l.left.iter().map(Cell::units).sum::<f64>(), l.left.len()))
            })
            .flatten()
            .unwrap_or((0.0, 0));
        format!("grid-row:{};--l-units:{units};--l-cells:{cells};", line + 1)
    };
    let item = move |item: Item| {
        let model = model.clone();
        view! { <ItemView list=list model=model item=item /> }
    };
    view! {
        <div class="line" data-testid="line" data-line=line.to_string() style=place>
            <div class="split" aria-hidden="true"></div>
            <For each=move || list.try_get().unwrap_or_default() key=Item::key children=item />
        </div>
    }
}

/// One item of a line: a group's title (its marker, its title and the
/// instance its strips share, #63) over its run of cells, or a cell. Its
/// side and a title's run are read from the line by its key (a keyed list
/// keeps the item it was made with).
#[component]
fn ItemView(list: Memo<Vec<Item>>, model: Arc<PageModel>, item: Item) -> impl IntoView {
    let id = item.key();
    let now = Memo::new(move |_| {
        list.try_with(|all| all.iter().find(|i| i.key() == id).cloned())
            .flatten()
    });
    let side = move || match now.try_get().flatten().map_or(Side::Left, |i| i.side()) {
        Side::Left => "left",
        Side::Right => "right",
    };
    match item {
        Item::Title { group, .. } => {
            let Some(group) = model.groups.get(group) else {
                return ().into_any();
            };
            let id = group.id.clone();
            let title = group.title.clone().unwrap_or_default();
            let color = group.color.clone();
            let instance = shared_instance(&group.controls).map(|name| {
                let instance_attr = name.clone();
                view! {
                    <span class="group-instance" data-testid="group-instance" data-instance=instance_attr>
                        {name}
                    </span>
                }
            });
            // The title spans its run: `--n` cells, `--units` strips wide.
            let look = move || {
                let (cells, units) = match now.try_get().flatten() {
                    Some(Item::Title { cells, units, .. }) => (cells, units),
                    _ => (1, 1.0),
                };
                let mark = color
                    .as_ref()
                    .map(|c| format!("--gc:{c};"))
                    .unwrap_or_default();
                format!("{mark}--n:{cells};--units:{units};")
            };
            view! {
                <div class="run-title" data-testid="group" data-group=id data-side=side style=look>
                    <h2 class="group-title">
                        <i class="group-mark"></i>
                        <span data-testid="group-title">{title}</span>
                        {instance}
                    </h2>
                </div>
            }
            .into_any()
        }
        Item::Cell { cell, .. } => {
            let group = cell.group().and_then(|g| model.groups.get(g));
            let id = group.and_then(|g| g.id.clone());
            // The group's colour on the strip's top edge (`.strip::before`).
            let look = group
                .and_then(|g| g.color.as_ref())
                .map(|c| format!("--gc:{c};"))
                .unwrap_or_default();
            view! {
                <div class="slot" data-side=side data-group=id style=look>
                    <CellView model=model cell=cell />
                </div>
            }
            .into_any()
        }
    }
}

/// One cell: a control (in a pager's slot a wide strip is one strip wide),
/// a group of buttons or texts as one block, or an empty slot.
#[component]
fn CellView(model: Arc<PageModel>, cell: Cell) -> impl IntoView {
    match cell {
        Cell::Control { at, units, .. } => {
            let Some(group) = model.groups.get(at.group) else {
                return ().into_any();
            };
            let Some(control) = group.controls.get(at.control).cloned() else {
                return ().into_any();
            };
            let shared = shared_instance(&group.controls);
            let control = match control {
                Control::Strip(mut strip) if strip.wide && units < METRICS.wide => {
                    strip.wide = false;
                    Control::Strip(strip)
                }
                other => other,
            };
            if is_column(&control) {
                view! { <ControlView control=control shared=shared /> }.into_any()
            } else {
                view! {
                    <div class="cell">
                        <ControlView control=control shared=shared />
                    </div>
                }
                .into_any()
            }
        }
        Cell::Block { group, units } => {
            let controls = model
                .groups
                .get(group)
                .map(|g| g.controls.clone())
                .unwrap_or_default();
            let texts = controls.iter().all(|c| matches!(c, Control::Text { .. }));
            let views = controls
                .into_iter()
                .map(|control| view! { <ControlView control=control /> })
                .collect_view();
            let width = format!("--units:{units};");
            view! { <div class="block" class:texts=texts style=width>{views}</div> }.into_any()
        }
        Cell::Blank { .. } => {
            view! { <div class="strip blank" data-testid="blank"></div> }.into_any()
        }
    }
}

/// A line's arrows in the column (a line that does not fit): ◀ and ▶ move
/// its window a step, and between them where it stands.
#[component]
fn ShiftView(
    arranged: Memo<Option<Arrangement>>,
    offsets: RwSignal<BTreeMap<LineKey, usize>>,
    line: usize,
) -> impl IntoView {
    let shift = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .and_then(|a| a.lines.get(line))
                    .and_then(|l| l.shift.map(|s| (l.key.clone(), s)))
            })
            .flatten()
    });
    let step = move |forward: bool| {
        let Some(Some((key, now))) = shift.try_get_untracked() else {
            return;
        };
        let next = now.moved(forward);
        let _ = offsets.try_update(|all| {
            all.insert(key, next);
        });
    };
    let on_back = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(false);
    };
    let on_next = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(true);
    };
    let now = move || shift.try_get().flatten().map(|(_, s)| s);
    let at_start = move || now().is_none_or(|s| s.offset == 0);
    let at_end = move || now().is_none_or(|s| s.offset >= s.last);
    let label = move || now().map(|s| s.label()).unwrap_or_default();
    // The arrows own their touches (#43 PR G); they write no key.
    let back_keys: Vec<String> = Vec::new();
    let next_keys: Vec<String> = Vec::new();
    view! {
        <div class="shift" data-testid="shift" data-line=line.to_string()>
            <button
                type="button"
                class="btn shift-btn"
                use:owns_touches=back_keys
                class:end=at_start
                data-testid="shift-back"
                on:pointerdown=on_back
            >
                "◀"
            </button>
            <span class="shift-where" data-testid="shift-where">{label}</span>
            <button
                type="button"
                class="btn shift-btn"
                use:owns_touches=next_keys
                class:end=at_end
                data-testid="shift-next"
                on:pointerdown=on_next
            >
                "▶"
            </button>
        </div>
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
        let states: Vec<Option<bool>> = slots
            .iter()
            .map(|s| s.try_with(Slot::flag).flatten())
            .collect();
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
            class:on=move || any.try_get().unwrap_or(false)
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="solo-clear"
            data-on=move || any.try_get().unwrap_or(false).to_string()
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
/// offline; every one offline while the page has no hub connection, so no
/// dot of its own says that, #63) and the version, at the top of the column.
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
            {badges}
            <span class="version" data-testid="version">{version_text()}</span>
            <ManualChip />
        </div>
    }
}
