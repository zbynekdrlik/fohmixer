//! The mixer surface (the redesign, #21; spec §4.2; the control column,
//! #63): every page is its two sides and the control column between them.
//! The column holds the dropout counter (#43), the connection badges and
//! the version, the pages' tabs (the Stream Deck tab, #52, and AbleSet's,
//! #90, last, each only from a hub with one), the pager's tabs, the SOLO ✕
//! pill, the arrows of a line that does not fit, the page's rail and the
//! global controls at its foot; nothing sits above the faders. Where the
//! controls go is `arrange` (pure);
//! this file measures a side and the height and renders the arrangement
//! with keyed lists, so a sub-page switch or a shift remounts only the
//! controls that change. Only the controls on screen are mounted, and the
//! subscriptions are the selected path's with an open channel detail's
//! (`binding::wanted_subs`, #71); the selected page and sub-page are
//! remembered on the device, the extra tabs and the detail never are.
//! A Pro-Q 4 screen opened from the detail (#71 PR E) lies over everything,
//! outside the layout's shell: a new layout keeps it; the detail closing
//! closes it.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Binding, Control, Layout, Page, Strip};
use fohmixer_proto::sectors::SectorId;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use crate::app::version_text;
use crate::arrange::{
    Arrangement, Cell, GroupLabel, Item, LineKey, METRICS, PageModel, Shift, Side, Sides,
    arrange_in, items, narrow, pager_on_right, px_value, rooms, thin_line,
};
use crate::behave::ableset::tab_dot;
use crate::behave::arrange_drag::Arrange;
use crate::behave::solo::soloed;
use crate::binding::sectors::{PICKS_KEY, Picks, Sectored, parse_picks, sectors_on};
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
use crate::pages::ableset::{AbleSetView, take_return};
use crate::pages::deck::DeckView;
use crate::pages::sectors::SectorsView;
use crate::store::{Badge, LiveStore, Slot};

/// Where the selected pages are remembered (a JSON map: `""` for the pages,
/// a page's id for its pager → the selected page's id).
const PAGES_KEY: &str = "fohmixer_pages";
/// The page's padding, both sides (`.body` in the stylesheet).
const BODY_PAD: f64 = 12.0;
/// Between a side and the column (`.body`'s gap).
const BODY_GAP: f64 = 6.0;

/// A tab after the layout's pages (#52, #90), shown in the left half of
/// the page shown (#148: the left sectors, or the rows' left halves; the
/// right half and the column stay the page's; on a phone the whole screen
/// but the thin strip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Extra {
    /// None: the layout's page.
    #[default]
    None,
    /// The Stream Deck (#52).
    Deck,
    /// AbleSet's own page (#90).
    AbleSet,
}

impl Extra {
    /// The page's `data-extra`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Extra::None => "none",
            Extra::Deck => "deck",
            Extra::AbleSet => "ableset",
        }
    }

    /// It lies over the page's left half (#148 review): the controls that
    /// act only on the hidden left cells (the LU and LD layout tabs,
    /// ARRANGE, a left side's ◀ ▶) are inert meanwhile.
    pub(crate) fn covers_left(self) -> bool {
        self != Extra::None
    }

    /// The pager's tabs show (#148 review): with no extra tab, or with one
    /// while the pager's region lies in the live right half (`pager_right`,
    /// `arrange::pager_on_right`), whose sub-page they switch.
    pub(crate) fn pager_tabs(self, pager_right: bool) -> bool {
        !self.covers_left() || pager_right
    }
}

/// Whether the shown page's pager region lies in its right half (#148
/// review, `arrange::pager_on_right`): a context of `PageView`, read by the
/// column's head (none on the sectors page, which shows no pager tabs).
#[derive(Clone, Copy)]
pub(crate) struct PagerRight(pub(crate) Memo<bool>);

/// The page selection (a context of the surface).
#[derive(Clone, Copy)]
pub(crate) struct Nav {
    pub(crate) store: LiveStore,
    /// The page shown, then its pager's sub-page.
    pub(crate) path: RwSignal<Vec<usize>>,
    remembered: RwSignal<BTreeMap<String, String>>,
    pub(crate) viewport: RwSignal<(f64, f64)>,
    /// The layout each fader sector shows on this device (#148), as
    /// stored under `PICKS_KEY`: a pick whose layout is gone stays until
    /// the user pages that sector (`binding::sectors::picked` falls back).
    pub(crate) picks: RwSignal<Picks>,
    /// ARRANGE's reorder mode (#148, `behave::arrange_drag::Arrange`):
    /// here, not on the sectors page, so a new layout's remount keeps an
    /// arrangement in progress; the page's first look at the hub's
    /// document ends one left from before.
    pub(crate) arrange: RwSignal<Arrange>,
    /// The extra tab shown (#52 the Stream Deck, #90 AbleSet); never
    /// remembered: a reload opens the last mixer page. Written only on a
    /// change: a `try_set` notifies even with the same value.
    extra: RwSignal<Extra>,
    /// `extra` as every reader reads it (one memo): a pager tap rebuilds no
    /// page, and a tap on the open deck or AbleSet tab does not mount its
    /// page again (which would lift the held keys, or reload AbleSet).
    shown: Memo<Extra>,
    /// The page shown before a view (#68): a second tap on the view's
    /// button goes back to it.
    before_view: RwSignal<Option<String>>,
    /// The tag manual's overlay is open (#68: the column's TAGS chip
    /// opens it). Here, not in a layout's shell: a Tuner edit in Live is a
    /// new layout, and it must not close the manual explaining it.
    manual: RwSignal<bool>,
    /// The channel detail's strip, as held (#71: a hold on a strip's SEL
    /// opens it). Here, not in a layout's shell: a new layout keeps it open
    /// while its strip is in it (`binding::detail_strip`).
    detail: RwSignal<Option<Strip>>,
    /// When a hold opened the detail (the page clock): its opening guard
    /// counts from it, so a new layout's remount does not guard again.
    detail_opened: StoredValue<f64>,
    /// The column is open over the faders (#63: on a phone, ☰ in the thin
    /// strip opens it; the tablet's column is always there and this does
    /// nothing to it). Written only on a change.
    pub(crate) column_open: RwSignal<bool>,
}

impl Nav {
    /// The phone's column opens over the faders, or closes (#63).
    pub(crate) fn open_column(self, open: bool) {
        if self.column_open.try_get_untracked() == Some(!open) {
            let _ = self.column_open.try_set(open);
        }
    }

    /// The page `index` of level `level` was tapped: on a phone the column
    /// closes too (#63, the approved mockup).
    fn select(self, level: usize, index: usize) {
        self.open_column(false);
        // A page's tab leaves an extra tab; the page shown is selected as
        // before. A sub-page's tab keeps it (#148 review: it switches the
        // live right half beside it).
        if level == 0 && self.extra.try_get_untracked().unwrap_or_default() != Extra::None {
            let _ = self.extra.try_set(Extra::None);
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
        let extra = self.extra.try_get_untracked().unwrap_or_default() != Extra::None;
        let (target, remember) = view_tap(&layout, shown, extra, view, before.as_deref());
        let _ = self.before_view.try_set(remember);
        self.select(0, target);
    }

    /// A sector's layout tab was tapped (#148): that sector pages to it, on
    /// this device only, and the picks are stored (written only on a
    /// change).
    pub(crate) fn pick(self, sector: SectorId, layout: &str) {
        let same = self
            .picks
            .try_with_untracked(|picks| picks.get(&sector).map(String::as_str) == Some(layout))
            .unwrap_or(true);
        if same {
            return;
        }
        let Some(picks) = self.picks.try_update(|picks| {
            picks.insert(sector, layout.to_string());
            picks.clone()
        }) else {
            return;
        };
        if let Ok(text) = serde_json::to_string(&picks) {
            dom::storage_set(PICKS_KEY, &text);
        }
    }

    /// An extra tab lies over the left half (#148 review), read tracked
    /// (a control's look) or untracked (its handler's gate).
    pub(crate) fn left_covered(self) -> bool {
        self.shown.try_get().unwrap_or_default().covers_left()
    }

    /// `left_covered`, untracked: a handler's gate.
    pub(crate) fn left_covered_now(self) -> bool {
        self.shown
            .try_get_untracked()
            .unwrap_or_default()
            .covers_left()
    }

    /// The Stream Deck's tab (#52) or AbleSet's (#90) was tapped.
    fn show_extra(self, extra: Extra) {
        self.open_column(false);
        if self
            .extra
            .try_get_untracked()
            .is_some_and(|shown| shown != extra)
        {
            let _ = self.extra.try_set(extra);
        }
    }
}

/// The remembered page selection (nothing when none is stored or readable).
fn remembered_pages() -> BTreeMap<String, String> {
    dom::storage_get(PAGES_KEY)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The device's stored sector picks (#148; none when none is stored or
/// readable).
fn remembered_picks() -> Picks {
    dom::storage_get(PICKS_KEY)
        .map(|text| parse_picks(&text))
        .unwrap_or_default()
}

/// The surface of a logged-in device.
#[component]
pub fn Surface(token: String, session: RwSignal<Option<String>>) -> impl IntoView {
    let store = LiveStore::new(token, session);
    provide_context(store);
    let viewport = RwSignal::new(dom::viewport());
    let extra = RwSignal::new(Extra::None);
    let nav = Nav {
        store,
        path: RwSignal::new(Vec::new()),
        remembered: RwSignal::new(remembered_pages()),
        viewport,
        picks: RwSignal::new(remembered_picks()),
        arrange: RwSignal::new(Arrange::Off),
        extra,
        shown: Memo::new(move |_| extra.get()),
        before_view: RwSignal::new(None),
        manual: RwSignal::new(false),
        detail: RwSignal::new(None),
        detail_opened: StoredValue::new(f64::NEG_INFINITY),
        column_open: RwSignal::new(false),
    };
    provide_context(nav);
    // A strip's SEL opens the channel detail, its exit closes it (#71).
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
    // AbleSet's sign-in comes back to `#ableset` (#90 §5.4): the hash goes
    // at once, and the tab opens once the hub tells of AbleSet's page.
    let back_to_ableset = StoredValue::new(take_return());
    Effect::new(move |_| {
        if store.ableset.with(Option::is_some) && back_to_ableset.get_value() {
            back_to_ableset.set_value(false);
            nav.show_extra(Extra::AbleSet);
        }
    });

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
    // detail's strip and its pan (#71); on the sectors page (#148) the
    // picked layouts' controls instead of the rows.
    Effect::new(move |_| {
        let Some(layout) = store.layout.get() else {
            return;
        };
        let path = nav.path.get();
        let detail = nav.detail.try_get().flatten();
        let doc = store
            .sectors
            .try_with(|told| told.shown().cloned())
            .flatten();
        let picks = nav.picks.try_get().unwrap_or_default();
        let sectored = doc.as_ref().map(|doc| Sectored { doc, picks: &picks });
        store.set_wanted(wanted_subs(&layout, &path, detail.as_ref(), sectored));
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
    // `data-idle` (#51) is for the E2E and support only: idle changes
    // nothing on screen.
    view! {
        <div
            class="surface"
            use:owns_surface
            data-testid="surface"
            data-subs=move || store.subscribed.get().to_string()
            data-connected=move || store.connected.get().to_string()
            data-idle=move || store.idle.get().to_string()
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
pub(crate) struct Head {
    pub(crate) layout: Arc<Layout>,
    page: Memo<Option<usize>>,
    sub: Memo<Option<usize>>,
    /// The page shown is the sectors page (#148, `binding::sectors::
    /// sectors_on`): the column shows the LAYOUTS block, not the pager's
    /// tabs.
    pub(crate) sectored: Memo<bool>,
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
    let sectored = {
        let layout = layout.clone();
        let sectors = nav.store.sectors;
        Memo::new(move |_| {
            nav.path
                .try_with(|path| {
                    sectors
                        .try_with(|told| sectors_on(&layout, path, told.shown()))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
    };
    provide_context(Head {
        layout: layout.clone(),
        page,
        sub,
        sectored,
    });
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
    // The page shown; an extra tab (#148) lies over its left half
    // (`ExtraPane`, mounted once here beside the page, never inside it: a
    // switch between the sectors page and the rows, another tablet's
    // ARRANGE save or a hand edit, remounts the page and never the deck,
    // which would lift a held key, or AbleSet's frame, which would load
    // it again; #148 review), so the right half and the column stay the
    // page's and a tab's change mounts no page.
    let body = move || {
        let index = page.get()?;
        let shown = layout.pages.get(index)?.clone();
        let global = layout.global.clone();
        // The default page with sectors (#148): the four sectors in place
        // of its rows.
        if sectored.get() {
            return Some(view! { <SectorsView page=shown global=global /> }.into_any());
        }
        Some(view! { <PageView page=shown global=global sub=sub /> }.into_any())
    };
    view! {
        <div class="mixer" data-testid="stage">
            {body}
            <ExtraPane />
            {detail}
        </div>
    }
}

/// The head of a page's column: the pages' tabs first (each mixer page's
/// tab a row of its own, larger; STREAM DECK and ABLESET under them,
/// smaller: the owner's ruling of 29.10.2026 on #148), the pager's tabs
/// (none on an extra tab) and SOLO ✕. The status is the column's foot
/// ([`ColumnFoot`]).
#[component]
pub fn ColumnHead() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let Head {
        layout,
        page,
        sub,
        sectored,
    } = expect_context::<Head>();
    // A view (#68) is no page tab: the column's VIEWS shows it.
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
    // The sectors page (#148) has the LAYOUTS block in their place. Under
    // an extra tab they show only while the pager's region lies in the
    // live right half (#148 review, `Extra::pager_tabs`).
    let pager_right = use_context::<PagerRight>();
    let pager_tabs = move || {
        let index = page.try_get().flatten()?;
        let right = pager_right.is_some_and(|p| p.0.try_get().unwrap_or(false));
        if !nav.shown.try_get().unwrap_or_default().pager_tabs(right) {
            return None;
        }
        if sectored.try_get().unwrap_or(false) {
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
                        && nav.shown.try_get().unwrap_or_default() == Extra::None
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
            <div class="cap-title" data-testid="views-title">"Views"</div>
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
    // hold on a down there (a tab: outside its page before the column).
    view! {
        <div class="column-head">
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
                    && !(level == 0 && nav.shown.try_get().unwrap_or_default() != Extra::None)
            };
            // A tab owns its touches (#43 PR G); it writes no key. A mixer
            // page's tab (level 0) is the column's first, larger row (#148).
            let no_keys: Vec<String> = Vec::new();
            let page_tab = level == 0;
            view! {
                <button
                    type="button"
                    class="tab"
                    use:owns_touches=no_keys
                    class:page-tab=page_tab
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
    // The Stream Deck's tab (#52), then AbleSet's (#90), end the pages' bar,
    // each from a hub with one.
    let extra_tabs = (level == 0).then(|| {
        let has_deck = move || nav.store.deck.with(Option::is_some);
        let has_ableset = move || nav.store.ableset.with(Option::is_some);
        view! {
            <Show when=has_deck><DeckTab /></Show>
            <Show when=has_ableset><AbleSetTab /></Show>
        }
    });
    view! {
        <div class="seg" class:dim=dim data-testid="tabbar" data-level={level.to_string()}>
            {buttons}
            {extra_tabs}
        </div>
    }
}

/// An extra tab's content over the left half of the page shown (#148, the
/// owner's ruling of 29.10.2026: STREAM DECK and ABLESET open in the left
/// sectors; the column stays in the screen's middle and the right half
/// keeps working): the deck's keys or AbleSet's frame. Mounted once by
/// `Shell` beside the page (page.css places it over the left half), so a
/// new page (the sectors switched on or off) mounts it nothing again; one
/// `nav.shown` memo: a tap on the open tab or the pager mounts nothing
/// again either.
#[component]
pub(crate) fn ExtraPane() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let viewport = nav.viewport;
    move || match nav.shown.try_get().unwrap_or_default() {
        Extra::None => None,
        Extra::Deck => Some(view! { <DeckView viewport=viewport /> }.into_any()),
        Extra::AbleSet => Some(view! { <AbleSetView /> }.into_any()),
    }
}

/// The page's `data-extra` (#148): the extra tab over its left half.
pub(crate) fn extra_attr(nav: Nav) -> impl Fn() -> &'static str + Copy {
    move || nav.shown.try_get().unwrap_or_default().name()
}

/// The column's foot (#148, the owner's ruling of 29.10.2026): the status
/// (the dropout counter, the badges, the version, the tag manual's chip)
/// under the global controls, smaller and dimmer; a red state stays red.
/// The Stream Deck's capture listener lifts no hold on a down there, as in
/// the head (`.column-foot`).
#[component]
pub fn ColumnFoot() -> impl IntoView {
    view! {
        <div class="column-foot" data-testid="column-foot">
            <StatusCluster />
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
            "TAGS"
        </button>
    }
}

/// The tag manual over the screen (#68): `znacky.html`, fetched from the
/// hub (one source with the page the tray opens), its `<main>` with its
/// `<style>` (`manual::manual_parts`); "✕ Close" closes it.
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
        None => view! { <p class="manual-note">"Loading…"</p> }.into_any(),
        Some(Err(why)) => {
            view! { <p class="manual-note">{format!("The manual did not load: {why}")}</p> }
                .into_any()
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
                "✕ Close"
            </button>
            {content}
        </div>
    }
}

/// The Stream Deck's tab (#52): its title from the hub; a small red dot
/// while Companion (or the hub) is unreachable, no words. A row of the bar
/// of its own under the mixer pages' tabs, smaller (#148).
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
    let lit = move || nav.shown.get() == Extra::Deck;
    // A tab owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tab extra-tab deck-tab"
            use:owns_touches=no_keys
            class:selected=lit
            data-testid="deck-tab"
            data-selected=move || lit().to_string()
            data-offline=move || offline().to_string()
            on:pointerdown=move |_| nav.show_extra(Extra::Deck)
        >
            {title}
            <Show when=offline>
                <i class="deck-dot" data-testid="deck-dot"></i>
            </Show>
        </button>
    }
}

/// AbleSet's tab (#90, the owner's variant A: the last tab, right after the
/// Stream Deck's): its title from the hub; a small red dot while AbleSet
/// (or the hub) is unreachable, no words.
#[component]
fn AbleSetTab() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let title = move || {
        store
            .ableset
            .with(|a| a.as_ref().map(|a| a.title.clone()).unwrap_or_default())
    };
    let offline = move || {
        let online = store.ableset.with(|a| a.as_ref().and_then(|a| a.online));
        tab_dot(online, store.connected.get())
    };
    let lit = move || nav.shown.get() == Extra::AbleSet;
    // A tab owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tab extra-tab ableset-tab"
            use:owns_touches=no_keys
            class:selected=lit
            data-testid="ableset-tab"
            data-selected=move || lit().to_string()
            data-offline=move || offline().to_string()
            on:pointerdown=move |_| nav.show_extra(Extra::AbleSet)
        >
            {title}
            <Show when=offline>
                <i class="ableset-dot" data-testid="ableset-dot"></i>
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
    let thin_ref = NodeRef::<html::Div>::new();
    let measured = measure_page(nav.viewport, body_ref, column_ref, thin_ref);
    // Each line's window (a line that does not fit), from the column's
    // arrows, kept by the line's key (`PageModel::line_key`: its rows, and
    // the sub-page where the pager is): another sub-page, or a turned screen
    // that groups the rows into other lines, shows that line's window from
    // its start and coming back finds it where it was; a turned screen with
    // the same lines keeps each window (clamped by `shown`).
    let offsets = RwSignal::new(BTreeMap::<LineKey, usize>::new());
    let (arranged, line_rooms) = page_memos(
        model.clone(),
        measured.into(),
        sub.into(),
        offsets.into(),
        |model, sides, height| rooms(model, sides, height, &METRICS),
    );
    // Whether the pager's region lies in the right half, for the column's
    // head: its tabs under an extra tab (#148 review).
    {
        let model = model.clone();
        provide_context(PagerRight(Memo::new(move |_| {
            arranged
                .try_with(|a| a.as_ref().is_some_and(|a| pager_on_right(&model, a)))
                .unwrap_or(false)
        })));
    }
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
    // The narrow strips (#151, `arrange::narrow`): their marks' compact
    // forms and a taller SEL, by the same floored width as `--strip-w`.
    let narrow_flags = move || {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .map(|a| narrow((a.width * 100.0).floor() / 100.0, &METRICS))
            })
            .flatten()
    };
    let narrow_attr = move || narrow_flags().is_some_and(|n| n.normal).to_string();
    let narrow_wide_attr = move || narrow_flags().is_some_and(|n| n.wide).to_string();
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
        view! { <LineView arranged=arranged line_rooms=line_rooms model=model line=line /> }
    };
    let shift =
        move |line: usize| view! { <ShiftView arranged=arranged offsets=offsets line=line /> };
    // The phone's column over the faders (#63): its ✕ and the shade under
    // it close it; the faders under the shade take no touch meanwhile.
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        nav.open_column(false);
    };
    let close_keys: Vec<String> = Vec::new();
    let open = move || nav.column_open.try_get().unwrap_or(false);
    let shade = move || {
        open().then(|| {
            let shade_keys: Vec<String> = Vec::new();
            view! {
                <div
                    class="column-shade"
                    use:owns_touches=shade_keys
                    data-testid="column-shade"
                    on:pointerdown=close
                ></div>
            }
        })
    };
    view! {
        <div
            class="body"
            data-testid="page"
            data-page=page.id
            data-column-open=move || open().to_string()
            data-extra=extra_attr(nav)
            node_ref=body_ref
            style=body_style
            data-narrow=narrow_attr
            data-narrow-wide=narrow_wide_attr
        >
            <For each=move || 0..count.try_get().unwrap_or(0) key=|line| *line children=line_view />
            <ThinStrip arranged=arranged offsets=offsets node=thin_ref />
            {shade}
            <nav class="column" data-testid="column" node_ref=column_ref>
                <button
                    type="button"
                    class="column-close"
                    use:owns_touches=close_keys
                    data-testid="column-close"
                    on:pointerdown=close
                >
                    "✕ CLOSE"
                </button>
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
                <ColumnFoot />
            </nav>
        </div>
    }
}

/// A page's measures (#63; the sectors page's too, #148): a side's widths
/// and the lines' height, measured once laid out and on every resize.
pub(crate) fn measure_page(
    viewport: RwSignal<(f64, f64)>,
    body_ref: NodeRef<html::Div>,
    column_ref: NodeRef<html::Nav>,
    thin_ref: NodeRef<html::Div>,
) -> RwSignal<(Sides, f64)> {
    let measured = RwSignal::new((Sides::both(0.0), 0.0_f64));
    // A side's widths and the lines' height: measured once laid out and on
    // every resize. The middle is the column, or on a phone the thin strip
    // (#63, shown there only), the strips then counted as with the column
    // (`--count-col`, the phone's column width in `page.css`).
    Effect::new(move |_| {
        let _ = viewport.get();
        if let (Some(body), Some(column)) = (body_ref.get(), column_ref.get()) {
            let width = f64::from(body.client_width());
            let thin = thin_ref
                .get()
                .map(|thin| f64::from(thin.offset_width()))
                .filter(|thin| *thin > 0.0);
            let middle = thin.unwrap_or_else(|| f64::from(column.offset_width()));
            let counted = thin
                .and_then(|_| dom::computed(&body, "--count-col"))
                .and_then(|text| px_value(&text))
                .unwrap_or(middle);
            let side = |middle: f64| ((width - BODY_PAD - middle - 2.0 * BODY_GAP) / 2.0).max(0.0);
            let sides = Sides {
                count: side(counted),
                width: side(middle),
            };
            let height = f64::from(body.client_height()) - BODY_PAD;
            let _ = measured.try_set((sides, height.max(0.0)));
        }
    });
    measured
}

/// The page's arrangement and its lines' rooms (#127), each a memo of what
/// it depends on: the arrangement of the measured page (`measured`, none
/// before it is laid out), the shown sub-page and the lines' windows; the
/// rooms (`room_of`, `arrange::rooms` on the page; it arranges every
/// sub-page and window of each line) of the measured page alone, so
/// neither a sub-page switch nor an arrow recomputes them, only a new
/// measurement (each run of the page's measuring effect, even one that
/// measures the same; the #127 re-review's m3). Every read tolerates a disposed value (`try_*`): a
/// layout change disposes the page while its keyed lists' effects may
/// still run once.
fn page_memos(
    model: Arc<PageModel>,
    measured: Signal<(Sides, f64)>,
    sub: Signal<Option<usize>>,
    offsets: Signal<BTreeMap<LineKey, usize>>,
    room_of: impl Fn(&PageModel, Sides, f64) -> Vec<bool> + Send + Sync + 'static,
) -> (Memo<Option<Arrangement>>, Memo<Vec<bool>>) {
    let arranged = {
        let model = model.clone();
        Memo::new(move |_| {
            let (sides, height) = measured.try_get()?;
            let sub = sub.try_get()?;
            if !sides.measured() {
                return None;
            }
            offsets.try_with(|offsets| {
                let offset = |key: &LineKey| offsets.get(key).copied().unwrap_or(0);
                arrange_in(&model, sub, sides, height, offset, &METRICS)
            })
        })
    };
    let line_rooms = Memo::new(move |_| {
        measured
            .try_get()
            .filter(|(sides, _)| sides.measured())
            .map(|(sides, height)| room_of(&model, sides, height))
            .unwrap_or_default()
    });
    (arranged, line_rooms)
}

/// One line: its items in one keyed list (`arrange::items`), each on its
/// side of the column (`page.css` orders the left side's, `.split` taking
/// the rest of the left half and the column, the right side's), so a cell
/// that changes side or group run moves and is not rebuilt. Only a line
/// that shows a group's title over a run on any of its sub-pages and
/// windows keeps the title's room (`data-captions`, from `line_rooms`, the
/// page's `arrange::rooms`, #127): on any other its strips take it, and a
/// sub-page switch or an arrow never changes it.
#[component]
fn LineView(
    arranged: Memo<Option<Arrangement>>,
    line_rooms: Memo<Vec<bool>>,
    model: Arc<PageModel>,
    line: usize,
) -> impl IntoView {
    let list = {
        let model = model.clone();
        Memo::new(move |_| {
            arranged
                .try_with(|a| {
                    a.as_ref()
                        .and_then(|a| a.lines.get(line))
                        .map(|l| items(&model, l))
                        .unwrap_or_default()
                })
                .unwrap_or_default()
        })
    };
    let captioned = Memo::new(move |_| {
        line_rooms
            .try_with(|r| r.get(line).copied())
            .flatten()
            .unwrap_or(false)
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
        <div
            class="line"
            data-testid="line"
            data-line=line.to_string()
            data-captions=move || captioned.try_get().unwrap_or(false).to_string()
            style=place
        >
            <div class="split" aria-hidden="true"></div>
            <For each=move || list.try_get().unwrap_or_default() key=Item::key children=item />
        </div>
    }
}

/// One item of a line: a group's run anchor (`data-testid="group"`, which
/// the tests find a strip's group by) with, over a run that does not start
/// with a strip, the group's title when it has one to show (its marker, its
/// title and the instance its strips share, #63; `caption`), or a cell (on
/// the first strip of a run, the group's name on its side, #127). Its side, an anchor's run and title
/// and a cell's label are read from the line by its key (a keyed list
/// keeps the item it was made with; a shift or a sub-page switch can make
/// another strip a run's first).
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
            let shared = shared_instance(&group.controls);
            // The title over a run that does not start with a strip (#127).
            let caption = move || {
                matches!(
                    now.try_get().flatten(),
                    Some(Item::Title { caption: true, .. })
                )
            };
            let heading = move || {
                caption().then(|| {
                    let instance = shared.clone().map(|name| {
                        let instance_attr = name.clone();
                        view! {
                            <span class="group-instance" data-testid="group-instance" data-instance=instance_attr>
                                {name}
                            </span>
                        }
                    });
                    view! {
                        <h2 class="group-title">
                            <i class="group-mark"></i>
                            <span data-testid="group-title">{title.clone()}</span>
                            {instance}
                        </h2>
                    }
                })
            };
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
                    {heading}
                </div>
            }
            .into_any()
        }
        Item::Cell { cell, .. } => {
            let group = cell.group().and_then(|g| model.groups.get(g));
            let id = group.and_then(|g| g.id.clone());
            // The group's colour on a block's top edge (`group.css` `.block::before`;
            // a strip shows none since #151).
            let look = group
                .and_then(|g| g.color.as_ref())
                .map(|c| format!("--gc:{c};"))
                .unwrap_or_default();
            // The group's name on its side, on a run's first strip (#127).
            let label = Memo::new(move |_| match now.try_get().flatten() {
                Some(Item::Cell { label, .. }) => label,
                _ => None,
            });
            view! {
                <div class="slot" data-side=side data-group=id style=look>
                    <CellView model=model cell=cell label=label />
                </div>
            }
            .into_any()
        }
    }
}

/// One cell: a control (in a pager's slot a wide strip is one strip wide;
/// a strip shows `label` in its scale column, #127), a group of buttons as
/// one block, or an empty slot.
#[component]
fn CellView(model: Arc<PageModel>, cell: Cell, label: Memo<Option<GroupLabel>>) -> impl IntoView {
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
                view! { <ControlView control=control shared=shared group_name=label /> }.into_any()
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
            let views = controls
                .into_iter()
                .map(|control| view! { <ControlView control=control /> })
                .collect_view();
            let width = format!("--units:{units};");
            view! { <div class="block" style=width>{views}</div> }.into_any()
        }
        Cell::Blank { .. } => {
            view! { <div class="strip blank" data-testid="blank"></div> }.into_any()
        }
    }
}

/// A window moved a step (`forward` or back), kept by its line's key.
fn step_window(
    offsets: RwSignal<BTreeMap<LineKey, usize>>,
    shift: Option<(LineKey, Shift)>,
    forward: bool,
) {
    let Some((key, now)) = shift else {
        return;
    };
    let next = now.moved(forward);
    let _ = offsets.try_update(|all| {
        all.insert(key, next);
    });
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
            .try_with(|a| a.as_ref().and_then(|a| a.shift_of(line)))
            .flatten()
    });
    let step = move |forward: bool| {
        step_window(offsets, shift.try_get_untracked().flatten(), forward);
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

/// The phone's thin strip in the column's place (#63, the approved mockup
/// phone-burger-v1; `page.css` shows it on a phone only): the link's dot,
/// ◀ and ▶ for the first line with a window (`arrange::thin_line`; hidden
/// while no line has one) and ☰, which opens the column over the faders.
#[component]
pub(crate) fn ThinStrip(
    arranged: Memo<Option<Arrangement>>,
    offsets: RwSignal<BTreeMap<LineKey, usize>>,
    node: NodeRef<html::Div>,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let shift = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                let a = a.as_ref()?;
                a.shift_of(thin_line(a)?)
            })
            .flatten()
    });
    let step = move |forward: bool| {
        step_window(offsets, shift.try_get_untracked().flatten(), forward);
    };
    let on_back = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(false);
    };
    let on_next = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(true);
    };
    let on_burger = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        nav.open_column(true);
    };
    let now = move || shift.try_get().flatten().map(|(_, s)| s);
    let arrows = move || if now().is_some() { "visible" } else { "hidden" };
    let at_start = move || now().is_none_or(|s| s.offset == 0);
    let at_end = move || now().is_none_or(|s| s.offset >= s.last);
    let connected = move || store.connected.try_get().unwrap_or(false).to_string();
    // Each button owns its touches (#43 PR G); none writes a key.
    let back_keys: Vec<String> = Vec::new();
    let burger_keys: Vec<String> = Vec::new();
    let next_keys: Vec<String> = Vec::new();
    view! {
        <div class="thin" data-testid="thin" node_ref=node>
            <span class="thin-live" data-testid="thin-live" data-connected=connected></span>
            <button
                type="button"
                class="btn thin-btn"
                use:owns_touches=back_keys
                class:end=at_start
                style:visibility=arrows
                data-testid="thin-back"
                on:pointerdown=on_back
            >
                "◀"
            </button>
            <button
                type="button"
                class="btn thin-btn thin-burger"
                use:owns_touches=burger_keys
                data-testid="thin-burger"
                on:pointerdown=on_burger
            >
                "☰"
            </button>
            <button
                type="button"
                class="btn thin-btn"
                use:owns_touches=next_keys
                class:end=at_end
                style:visibility=arrows
                data-testid="thin-next"
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
            // The badges in a box of their own (no box on a tablet:
            // `display: contents`), so a low phone's foot wraps a third
            // instance's badge under the others (#148 review).
            <span class="badges">{badges}</span>
            <span class="version" data-testid="version">{version_text()}</span>
            <ManualChip />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn an_extra_tab_covers_the_left_half_and_keeps_the_pager_tabs_of_a_right_region() {
        // #148 review: the left half's own controls are inert under an
        // extra tab; the pager's tabs stay while its region is right.
        assert!(!Extra::None.covers_left());
        assert!(Extra::Deck.covers_left());
        assert!(Extra::AbleSet.covers_left());
        assert!(Extra::None.pager_tabs(false));
        assert!(Extra::None.pager_tabs(true));
        assert!(!Extra::Deck.pager_tabs(false));
        assert!(Extra::Deck.pager_tabs(true));
        assert!(!Extra::AbleSet.pager_tabs(false));
        assert!(Extra::AbleSet.pager_tabs(true));
    }

    #[test]
    fn the_pages_data_extra_names_the_extra_tab() {
        // #148: page.css hides the left half under any extra tab but "none".
        assert_eq!(Extra::None.name(), "none");
        assert_eq!(Extra::Deck.name(), "deck");
        assert_eq!(Extra::AbleSet.name(), "ableset");
        assert_eq!(Extra::default().name(), "none");
    }

    fn strip(name: &str) -> serde_json::Value {
        json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
               "strip_kind": "standard"})
    }

    fn toggle(label: &str) -> serde_json::Value {
        json!({"kind": "param_toggle", "label": label, "press": "toggle",
               "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "T"}},
                            "prop": "mute", "on": false, "off": true}]})
    }

    #[test]
    fn a_sub_page_switch_or_an_arrow_never_recomputes_the_lines_rooms() {
        // #127 re-review m3: a pager whose second sub-page holds a titled
        // block, beside five strips; on 135 px sides the line shows a
        // window. The rooms are counted each time they are computed.
        let page: Page = serde_json::from_value(json!({"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "pager", "id": "pg", "default_page": "a", "pages": [
                {"id": "a", "title": "A", "sections": [
                    {"kind": "group", "id": "a", "title": "A", "controls": [strip("A0"), strip("A1")]}]},
                {"id": "b", "title": "B", "sections": [
                    {"kind": "group", "id": "b", "title": "CUE", "controls": [toggle("T0"), toggle("T1")]}]}]},
            {"kind": "group", "id": "s", "title": "S",
             "controls": [strip("S0"), strip("S1"), strip("S2"), strip("S3"), strip("S4")]}]}]}))
        .expect("a page");
        let owner = Owner::new();
        owner.set();
        let model = Arc::new(PageModel::of(&page));
        let measured = RwSignal::new((Sides::both(0.0), 822.0));
        let sub = RwSignal::new(Some(0));
        let offsets = RwSignal::new(BTreeMap::<LineKey, usize>::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let (arranged, line_rooms) = page_memos(
            model.clone(),
            measured.into(),
            sub.into(),
            offsets.into(),
            move |model, sides, height| {
                counted.fetch_add(1, Ordering::SeqCst);
                rooms(model, sides, height, &METRICS)
            },
        );
        let ran = || runs.load(Ordering::SeqCst);
        // Not laid out yet: no arrangement, no rooms, nothing computed.
        assert_eq!(arranged.get_untracked(), None);
        assert_eq!(line_rooms.get_untracked(), Vec::<bool>::new());
        assert_eq!(ran(), 0);
        // Laid out: the rooms once.
        let _ = measured.try_set((Sides::both(135.0), 822.0));
        let on_a = arranged.get_untracked().expect("arranged");
        assert!(on_a.lines[0].shift.is_some(), "the premise: a window");
        assert_eq!(line_rooms.get_untracked(), vec![true]);
        assert_eq!(ran(), 1);
        // A sub-page switch: a new arrangement, the rooms not computed again.
        let _ = sub.try_set(Some(1));
        let on_b = arranged.get_untracked().expect("arranged");
        assert_ne!(on_a, on_b, "the premise: another arrangement");
        assert_eq!(line_rooms.get_untracked(), vec![true]);
        assert_eq!(ran(), 1, "a sub-page switch");
        // An arrow: the window moves, the rooms not computed again.
        let key = model.line_key(&[0], Some(1));
        let _ = offsets.try_set(BTreeMap::from([(key, 1)]));
        let moved = arranged.get_untracked().expect("arranged");
        assert_eq!(moved.lines[0].shift.map(|s| s.offset), Some(1));
        assert_eq!(line_rooms.get_untracked(), vec![true]);
        assert_eq!(ran(), 1, "an arrow");
        // A new measurement: the rooms again, once.
        let _ = measured.try_set((Sides::both(519.0), 822.0));
        assert!(arranged.get_untracked().is_some());
        assert_eq!(line_rooms.get_untracked(), vec![true]);
        assert_eq!(ran(), 2, "a new measurement");
        // Laid out at no width again: no arrangement, no rooms.
        let _ = measured.try_set((Sides::both(0.0), 822.0));
        assert_eq!(arranged.get_untracked(), None);
        assert_eq!(line_rooms.get_untracked(), Vec::<bool>::new());
        assert_eq!(ran(), 2);
    }
}
