//! The mixer surface (S4 design note §2, §5, §6): the layout's pages on one
//! scaled stage, the tab bars of the root pages and of the nested pagers,
//! the root overlay on every page, the connection badges and the version.
//! Only the pages on screen are mounted, so only their bindings are
//! subscribed; the selected page of every pager is remembered on the
//! device.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Frame, Layout, Orientation, Page, Tab, TabBar};
use leptos::html;
use leptos::prelude::*;

use crate::app::version_text;
use crate::binding::{choose, selected_path, visible_subs};
use crate::components::{ItemView, Settings};
use crate::dom;
use crate::stage::{self, TabLayout};
use crate::store::{Badge, LiveStore};

/// Where the selected pages are remembered (a JSON map: the id of the page
/// holding a pager, `""` for the root → the selected page's id).
const PAGES_KEY: &str = "fohmixer_pages";
/// The room kept at the right end of the root tab bar for the connection
/// badges and the version.
const STATUS_WIDTH: f64 = 420.0;

/// The page selection (a context of the surface).
#[derive(Clone, Copy)]
struct Nav {
    store: LiveStore,
    /// The index of the page shown on each level.
    path: RwSignal<Vec<usize>>,
    remembered: RwSignal<BTreeMap<String, String>>,
}

impl Nav {
    /// The page `index` of level `level` was tapped.
    fn select(self, level: usize, index: usize) {
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
    let nav = Nav {
        store,
        path: RwSignal::new(Vec::new()),
        remembered: RwSignal::new(remembered_pages()),
    };
    provide_context(nav);
    on_cleanup(move || store.stop());
    store.start();

    let viewport = RwSignal::new(dom::viewport());
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
    // The pages on screen are the ones subscribed.
    Effect::new(move |_| {
        let Some(layout) = store.layout.get() else {
            return;
        };
        let path = nav.path.get();
        store.set_wanted(visible_subs(&layout, &path));
    });

    let content = move || match store.layout.get() {
        Some(layout) => view! { <StageView layout=layout viewport=viewport /> }.into_any(),
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
    view! {
        <div
            class="surface"
            data-testid="surface"
            data-subs=move || store.subscribed.get().to_string()
            data-connected=move || store.connected.get().to_string()
            data-refreshes=move || store.refreshes.get().to_string()
        >
            {content}
        </div>
    }
}

/// The scaled stage of one layout.
#[component]
fn StageView(layout: Arc<Layout>, viewport: RwSignal<(f64, f64)>) -> impl IntoView {
    let canvas = layout.canvas;
    provide_context(Settings {
        shaping: layout.config.fader_shaping.unwrap_or(true),
        meter_source: layout.config.meter_source.unwrap_or_default(),
    });
    let fill = stage::stage_fill(layout.background.as_deref());
    let style = move || {
        let (w, h) = viewport.get();
        format!(
            "width:{}px;height:{}px;transform:{};{fill}",
            canvas.w,
            canvas.h,
            stage::transform(stage::fit(w, h, canvas))
        )
    };
    let area = Frame {
        x: 0.0,
        y: 0.0,
        w: canvas.w,
        h: canvas.h,
    };
    let bar_height = if layout.tabbar.bar_size > 0.0 {
        layout.tabbar.bar_size
    } else {
        48.0
    };
    let cluster = Frame {
        x: canvas.w - STATUS_WIDTH,
        y: 0.0,
        w: STATUS_WIDTH,
        h: bar_height,
    };
    let overlay = layout
        .overlay
        .iter()
        .cloned()
        .map(|item| view! { <ItemView item=item /> })
        .collect_view();
    view! {
        <div class="stage" data-testid="stage" style=style>
            <Pages
                pages={layout.pages.clone()}
                tabbar={layout.tabbar}
                area=area
                level=0
                reserve=STATUS_WIDTH
            />
            <div class="layer overlay">{overlay}</div>
            <StatusCluster frame=cluster />
        </div>
    }
}

/// One level of pages: its tab bar and the selected page.
#[component]
fn Pages(
    pages: Vec<Page>,
    tabbar: TabBar,
    area: Frame,
    level: usize,
    reserve: f64,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let selected = Memo::new(move |_| nav.path.with(|p| p.get(level).copied()));
    let tabs: Vec<(String, String, Tab)> = pages
        .iter()
        .map(|p| (p.id.clone(), p.title.clone(), p.tab.clone()))
        .collect();
    let orientation = tabbar.orientation;
    let bar = stage::tab_layout(area, &tabbar, pages.len(), reserve).map(|layout| {
        view! {
            <TabBarView layout=layout orientation=orientation tabs=tabs level=level selected=selected />
        }
    });
    let content = stage::content_frame(area, &tabbar);
    let page = move || {
        selected
            .get()
            .and_then(|i| pages.get(i).cloned())
            .map(|page| view! { <PageView page=page level=level content=content /> }.into_any())
    };
    view! {
        {bar}
        {page}
    }
}

/// A page: its fill over `content` (its pager's area or the canvas below the
/// tab bar), its items and its nested pager. The nested pager is a layer of
/// its own at its place among the page's items (#7): its fill, tab bar and
/// pages stack inside it.
#[component]
fn PageView(page: Page, level: usize, content: Frame) -> impl IntoView {
    let id = page.id.clone();
    let fill_page = page.id.clone();
    let fill = page.background.clone().map(|color| {
        view! {
            <div
                class="fill"
                data-testid="page-background"
                data-page=fill_page
                style={stage::fill_style(content, &color)}
            ></div>
        }
    });
    let conf = page
        .id
        .eq_ignore_ascii_case("conf")
        .then(|| view! { <ConfInfo /> });
    let items = page
        .items
        .into_iter()
        .map(|item| view! { <ItemView item=item /> })
        .collect_view();
    let deeper = level + 1;
    let pager = page.pager.map(|pager| {
        let layer = format!("z-index:{};", stage::pager_z(&pager));
        let pager_fill = pager.background.clone().map(|color| {
            view! {
                <div
                    class="fill"
                    data-testid="pager-background"
                    style={stage::fill_style(pager.frame, &color)}
                ></div>
            }
        });
        view! {
            <div class="layer pager" style=layer>
                {pager_fill}
                <Pages
                    pages={pager.pages}
                    tabbar={pager.tabbar}
                    area={pager.frame}
                    level=deeper
                    reserve=0.0
                />
            </div>
        }
        .into_any()
    });
    view! {
        <div class="layer page" data-testid="page" data-page=id>
            {fill}
            {items}
            {pager}
            {conf}
        </div>
    }
}

/// A tab bar: one tab per page, the selected one lit (its lit colour and
/// text size, #7). A vertical bar's titles run along it (#9). Every title is
/// fitted to its tab.
#[component]
fn TabBarView(
    layout: TabLayout,
    orientation: Orientation,
    tabs: Vec<(String, String, Tab)>,
    level: usize,
    selected: Memo<Option<usize>>,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let bar = layout.bar;
    let buttons = layout
        .tabs
        .into_iter()
        .zip(tabs)
        .enumerate()
        .map(|(index, (frame, (id, title, tab)))| {
            let css = stage::box_style(stage::relative(frame, bar));
            let lit = move || selected.get() == Some(index);
            let class = stage::tab_class(&tab);
            let node = NodeRef::<html::Div>::new();
            // The colour and the fitted size follow the selection; the
            // style attribute holds only the box, so nothing rewrites them.
            Effect::new(move |_| {
                let on = lit();
                if let Some(el) = node.get() {
                    dom::set_style(
                        &el,
                        "background",
                        stage::tab_background(&tab, on).unwrap_or(""),
                    );
                    dom::fit_text(&el, stage::tab_font(&tab, on));
                }
            });
            view! {
                <div
                    class=class
                    class:selected=lit
                    node_ref=node
                    data-testid="tab"
                    data-page=id
                    data-selected=move || lit().to_string()
                    style=css
                    on:pointerdown=move |_| nav.select(level, index)
                >
                    <span class="tab-title">{title}</span>
                </div>
            }
        })
        .collect_view();
    view! {
        <div
            class={stage::tabbar_class(orientation)}
            data-testid="tabbar"
            data-level={level.to_string()}
            style={stage::box_style(bar)}
        >
            {buttons}
        </div>
    }
}

/// The connection badges (per instance: online, busy, offline) and the
/// version, over the right end of the root tab bar.
#[component]
fn StatusCluster(frame: Frame) -> impl IntoView {
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
        <div class="status-cluster" style={stage::box_style(frame)}>
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
