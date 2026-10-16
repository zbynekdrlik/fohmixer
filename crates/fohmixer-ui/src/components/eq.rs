//! The Pro-Q 4 screen on the surface (#71 PR E, F28, D17; the approved
//! mockup `docs/mockups/channel-detail-v2.html`, its EQ card and EQ screen).
//!
//! - **The cards** ([`EqCards`], the channel detail's middle): the strip's
//!   Pro-Q 4 instances as the hub reads them (`eq_list`, asked when the
//!   detail opens, after every reconnect, whenever the strip's instance
//!   comes back online, and once the screen closed), one
//!   card each: its last picture (fetched with the token into a blob URL;
//!   a plain field before a first open), where it sits, and `OTVORIŤ EQ NA
//!   CELÚ OBRAZOVKU`, or `ZAMKNUTÉ` while another page holds an editor (one
//!   Pro-Q 4 on the PC's screen at a time): this one's holder since when,
//!   or that another engineer uses Pro-Q 4 since when.
//! - **The screen** ([`EqScreen`], over the whole surface, mounted while
//!   [`EqNav`] names an editor; outside the layout's shell, so a new layout
//!   keeps it): the bar (`← SPÄŤ NA KANÁL`, the chip, `Pro-Q 4 · <where>`,
//!   the lock mark, `?` with the touch legend) and the live picture: the
//!   binary frames drawn into a canvas through `createImageBitmap`, fitted
//!   (`object-fit: contain`), the newest frame winning while one decodes.
//!   One finger maps to `eq_input` in the picture's pixels through the
//!   page's view (`behave::eq::Finger`: the first finger waits 120 ms or
//!   6 px before its down, a lift meanwhile is a tap, its up 40 ms after its
//!   down on a frame (PR G); a move at most once per animation frame, the up
//!   and the cancel always, a lost capture as a cancel; a finger whose end
//!   never came ended by the next primary down of its type or a hide, #124).
//!   The open carries
//!   the picture area (PR G: the editor on the PC gets its aspect), sent
//!   once the area is in the page; a changed area (a phone turned, a
//!   window resized) goes as `eq_area` once it rested 300 ms while the
//!   editor is open (`behave::eq::AreaWatch`, on the frame loop). The
//!   picture's size is the frames'. Two fingers zoom and pan the picture on
//!   this page only (PR F, the approved mockup `docs/mockups/proq-zoom-v1.html`;
//!   `behave::eq::Viewer`, 1× to 4×, each open at 1×). Under 2× one finger
//!   never reaches the editor (the owner's ruling of 2026-10-11): a tap
//!   there zooms to 3× on its point (`Viewer::zoom_on`), a drag does
//!   nothing, and a hint over the picture says so. The canvas gets a
//!   CSS transform, and while zoomed the bar shows the overview (the latest
//!   frame small, in the picture's shape, a frame around the part in
//!   sight), the factor and `CELÝ EQ`. Leaving it sends `eq_close`; the hub
//!   closing it (refused, failed, its window gone, the socket lost) brings
//!   the detail back. The bar's ↶ VRÁTIŤ (#71 PR P2b, the approved mockup
//!   `docs/mockups/proq-undo-v1.html`) asks the hub to tap Pro-Q's own undo
//!   (`eq_undo`, numbered with the fingers' inputs); it takes no press
//!   while a finger keeps the PC's contact nor within 700 ms of the last
//!   one (`behave::eq::undo_ready`), and a note over the picture says the
//!   hub's answer (`behave::eq::undo_text`).
//!
//! The screen's open, close and lock refusal go to the flight recorder as
//! `detail` events (`eq_open`, `eq_close` with why, `eq_locked`) with the
//! strip's keys, and every finger on the picture as `eq_touch` (its down, up,
//! cancel or `lost`: the picture point and the zoom) and `eq_mv` (its moves
//! summarized; #71 PR I, `diag::trace::fingers`). Every tap target owns its
//! touches.

use std::rc::Rc;

use fohmixer_proto::eq::{Crop, EqItem, EqState, FrameHead, PRODUCT};
use fohmixer_proto::layout::Strip;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::buttons::colour_style;
use super::{down_of, owns_touches, trace_detail};
use crate::behave::eq::{
    AreaWatch, CardLock, Finger, ListLink, Look, Out, Point, STALE_TEXT, Spot, Viewer, can_open,
    card_lock, failure_text, hh_mm, in_use_text, list_text, lists_now, lock_refusal, locked,
    locked_text, look, on_picture, overview_size, place_text, point, spot, stale_landing,
    stale_note, takes_finger, undo_flash, undo_note_shown, undo_ready, undo_text, zoom_text,
};
use crate::behave::eq_view::{ViewWatch, lands_on, part_of, scaled_part};
use crate::binding::detail_keys;
use crate::diag::trace::eq_lat::EqLat;
use crate::diag::trace::fingers::{EqFingers, eq_touch, eq_view};
use crate::dom;
use crate::raf;
use crate::store::eq::{EqView, ended, screen_state, screen_view};
use crate::store::{FrameSink, LiveStore, Slot};

/// What the screen shows: an editor, its name as its card gives it
/// (`behave::eq::place_text`), and the strip's name, its chip's colours and
/// its keys (the flight recorder's).
#[derive(Debug, Clone, PartialEq)]
pub struct EqTarget {
    pub instance: String,
    pub path: String,
    pub title: String,
    pub label: String,
    pub chip: String,
    pub keys: Vec<String>,
}

/// The editor the screen shows (a context of the surface).
#[derive(Clone, Copy)]
pub struct EqNav {
    pub target: RwSignal<Option<EqTarget>>,
}

impl EqNav {
    /// A card's tap (pointer `pointer`) opens the screen on `target`.
    pub fn open(self, target: EqTarget, pointer: i32) {
        trace_detail("eq_open", None, &target.keys, Some(pointer));
        let _ = self.target.try_set(Some(target));
    }

    /// The screen closes (`why`: `exit`, `detail`, or the hub's reason).
    pub fn close(self, why: &str, pointer: Option<i32>) {
        let keys = self
            .target
            .try_with_untracked(|t| t.as_ref().map(|t| t.keys.clone()))
            .flatten();
        let Some(keys) = keys else {
            return;
        };
        trace_detail("eq_close", Some(why), &keys, pointer);
        let _ = self.target.try_set(None);
    }
}

/// `HH:MM` of the hub's UTC ms in the page's local time.
fn local_hh_mm(utc_ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(utc_ms));
    hh_mm(date.get_hours(), date.get_minutes())
}

/// A blob URL of a picture, or none.
fn blob_url(blob: &web_sys::Blob) -> Option<String> {
    web_sys::Url::create_object_url_with_blob(blob).ok()
}

fn revoke(url: &str) {
    let _ = web_sys::Url::revoke_object_url(url);
}

/// A card's last picture: fetched when the hub keeps one, its blob URL
/// into `shown` (the one before revoked; `kept` holds it for the cleanup).
fn load_picture(
    store: LiveStore,
    instance: String,
    path: String,
    shown: RwSignal<Option<String>>,
    kept: StoredValue<Option<String>, LocalStorage>,
) {
    leptos::task::spawn_local(async move {
        let Some(blob) = store.eq_picture(&instance, &path).await else {
            return;
        };
        let Some(url) = blob_url(&blob) else {
            return;
        };
        match kept.try_update_value(|k| k.replace(url.clone())) {
            Some(old) => {
                if let Some(old) = old {
                    revoke(&old);
                }
                let _ = shown.try_set(Some(url));
            }
            // The card went meanwhile.
            None => revoke(&url),
        }
    });
}

/// One Pro-Q 4 of the strip's track.
#[component]
fn EqCard(
    item: EqItem,
    instance: String,
    label: String,
    color: Option<RwSignal<Slot>>,
    keys: Vec<String>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let nav = use_context::<EqNav>();
    let shown = RwSignal::new(None::<String>);
    let kept = StoredValue::new_local(None::<String>);
    if item.picture {
        load_picture(store, instance.clone(), item.path.clone(), shown, kept);
    }
    on_cleanup(move || {
        if let Some(Some(url)) = kept.try_update_value(Option::take) {
            revoke(&url);
        }
    });
    let lock = {
        let (instance, path) = (instance.clone(), item.path.clone());
        Memo::new(move |_| {
            store
                .eq_locks
                .try_with(|locks| card_lock(locks, &instance, &path))
                .unwrap_or(CardLock::Free)
        })
    };
    let other = move || lock.try_get().is_some_and(locked);
    // Offered while free (or this page's) and the page is connected: an
    // open the socket cannot take is not offered (`can_open`).
    let disabled = move || {
        let lock = lock.try_get().unwrap_or(CardLock::Free);
        let connected = store.connected.try_get().unwrap_or(false);
        (!can_open(lock, connected)).to_string()
    };
    let since = move || match lock.try_get() {
        Some(CardLock::Other(since)) => Some(locked_text(&local_hh_mm(since))),
        Some(CardLock::InUse(since)) => Some(in_use_text(&local_hh_mm(since))),
        _ => None,
    };
    let note = {
        let (instance, path) = (instance.clone(), item.path.clone());
        move || {
            store
                .eq
                .try_with(|view| {
                    view.as_ref()
                        .filter(|v| v.is(&instance, &path) && v.state == EqState::Closed)
                        .and_then(|v| failure_text(v.reason.as_deref().unwrap_or("")))
                })
                .flatten()
        }
    };
    let title = place_text(&item.place, &item.name);
    let target = StoredValue::new(EqTarget {
        instance,
        path: item.path.clone(),
        title: title.clone(),
        label,
        chip: String::new(),
        keys,
    });
    let on_open = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let free = lock.try_get_untracked().unwrap_or(CardLock::Free);
        if !can_open(free, store.can_send()) {
            return;
        }
        let (Some(nav), Some(mut target)) = (nav, target.try_get_value()) else {
            return;
        };
        target.chip =
            colour_style(color.and_then(|c| c.try_with_untracked(Slot::number).flatten()));
        nav.open(target, ev.pointer_id());
    };
    let no_keys: Vec<String> = Vec::new();
    let path_attr = item.path.clone();
    let alt = format!("{PRODUCT}, posledný obraz");
    let picture = move || match shown.try_get().flatten() {
        Some(url) => {
            view! { <img class="eq-card-img" data-testid="eq-card-img" src=url alt=alt.clone() /> }
                .into_any()
        }
        None => view! { <div class="eq-card-none">{PRODUCT}</div> }.into_any(),
    };
    view! {
        <div
            class="eq-card"
            data-testid="eq-card"
            data-path=path_attr
            data-locked=move || other().to_string()
        >
            <div class="eq-card-pic">
                {picture}
                {move || since().map(|text| view! {
                    <div class="eq-card-lock" data-testid="eq-card-lock">
                        <span class="eq-card-lock-word">"ZAMKNUTÉ"</span>
                        <span>{text}</span>
                    </div>
                })}
            </div>
            <div class="eq-card-where" data-testid="eq-card-where">{title}</div>
            <button
                type="button"
                class="eq-card-open"
                data-testid="eq-open"
                use:owns_touches=no_keys
                aria-disabled=disabled
                on:pointerdown=on_open
            >
                {move || if other() { "ZAMKNUTÉ" } else { "OTVORIŤ EQ NA CELÚ OBRAZOVKU" }}
            </button>
            {move || note().map(|text| view! {
                <div class="eq-card-note" data-testid="eq-card-note">{text}</div>
            })}
        </div>
    }
}

/// The channel detail's middle: the strip's Pro-Q 4 instances.
#[component]
pub fn EqCards(strip: Strip, label: String, color: Option<RwSignal<Slot>>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let nav = use_context::<EqNav>();
    let binding = StoredValue::new(strip.binding.clone());
    let keys = detail_keys(&strip);
    // The strip's instance online, as the hub last said (only its own
    // flag: another instance's state or a busy flag lists nothing).
    let online = {
        let instance = strip.binding.instance.clone();
        Memo::new(move |_| {
            store
                .instances
                .try_with(|all| all.get(&instance).is_some_and(|v| v.online))
                .unwrap_or(false)
        })
    };
    // Asked when the detail opens, after every reconnect and whenever Live
    // comes back online (`lists_now`).
    Effect::new(move |before: Option<ListLink>| {
        let now = ListLink {
            connected: store.connected.try_get().unwrap_or(false),
            online: online.try_get().unwrap_or(false),
        };
        if lists_now(before, now) {
            let _ = binding.try_with_value(|b| store.list_eq(b));
        }
        now
    });
    // Asked again once the screen closed: a new last picture.
    Effect::new(move |was_open: Option<bool>| {
        let open = nav
            .and_then(|n| n.target.try_with(Option::is_some))
            .unwrap_or(false);
        if was_open == Some(true) && !open {
            let _ = binding.try_with_value(|b| store.list_eq(b));
        }
        open
    });
    let list = Memo::new(move |_| {
        let wanted = binding.try_get_value();
        store
            .eq_list
            .try_get()
            .flatten()
            .filter(|l| Some(&l.binding) == wanted.as_ref())
    });
    let instance = strip.binding.instance.clone();
    let body = move || {
        let Some(list) = list.try_get().flatten() else {
            return view! { <p class="eq-note" data-testid="eq-note">"Hľadám Pro-Q 4…"</p> }
                .into_any();
        };
        if let Some(error) = list.error {
            let off = (error == fohmixer_proto::eq::reason::OFF).to_string();
            let text = list_text(&error);
            return view! { <p class="eq-note" data-testid="eq-note" data-off=off>{text}</p> }
                .into_any();
        }
        if list.items.is_empty() {
            return view! {
                <p class="eq-note" data-testid="eq-note">"Na tomto tracku nie je Pro-Q 4."</p>
            }
            .into_any();
        }
        list.items
            .into_iter()
            .map(|item| {
                view! {
                    <EqCard
                        item=item
                        instance=instance.clone()
                        label=label.clone()
                        color=color
                        keys=keys.clone()
                    />
                }
            })
            .collect_view()
            .into_any()
    };
    view! {
        <h2 class="detail-title">"EQ"</h2>
        <div class="eq-cards" data-testid="eq-cards">{body}</div>
    }
}

/// The picture's painter: the canvas, the overview's (PR F: the frame
/// drawn small), whether a frame decodes, the newest frame waiting
/// meanwhile, the frames drawn and the picture's size.
#[derive(Default)]
struct Painter {
    canvas: Option<web_sys::HtmlCanvasElement>,
    small: Option<web_sys::HtmlCanvasElement>,
    busy: bool,
    next: Option<(FrameHead, web_sys::Blob)>,
    frames: u32,
    size: Option<(u32, u32)>,
    /// The part of the picture the last frame drawn held (PR L).
    part: Option<Crop>,
}

/// Where a drawn frame is told (#71 PR J): the store, which knows the input
/// the picture is the first to show, and the screen's latency window.
#[derive(Clone, Copy)]
struct Drawn {
    store: LiveStore,
    lat: StoredValue<EqLat>,
}

/// The painter, kept with the screen (its handle is `Copy` and `Send`, as
/// the view's closures must be; the painter itself stays on the page's
/// thread).
type PainterBox = StoredValue<Painter, LocalStorage>;

/// A canvas's 2D context.
fn context_2d(canvas: &web_sys::HtmlCanvasElement) -> Option<web_sys::CanvasRenderingContext2d> {
    canvas
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
}

/// The overview's canvas and box in the shape of a picture of `picture`
/// px (#74 review, `behave::eq::overview_size`): its drawing size, and the
/// box's `aspect-ratio` (the bar's height holds it; its width follows).
fn shape_overview(small: &web_sys::HtmlCanvasElement, picture: (u32, u32)) {
    let (width, height) = overview_size(picture);
    if (small.width(), small.height()) == (width, height) {
        return;
    }
    small.set_width(width);
    small.set_height(height);
    let parent = small
        .parent_element()
        .and_then(|p| p.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(parent) = parent {
        dom::set_style(&parent, "aspect-ratio", &format!("{width} / {height}"));
    }
}

/// Draws one frame: decoded off the page's thread (`createImageBitmap`),
/// then onto the canvas at its own size, and small onto the overview's,
/// whose box takes the picture's shape (PR F; #74 review). PR J: the draw
/// is told (the latency of the input the picture is the first to show, the
/// decode's time), and each second of them goes to the flight recorder.
async fn draw(painter: PainterBox, drawn: Drawn, head: FrameHead, blob: &web_sys::Blob) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(promise) = window.create_image_bitmap_with_blob(blob) else {
        return;
    };
    let started = dom::now();
    let bitmap = match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(value) => value.dyn_into::<web_sys::ImageBitmap>().ok(),
        Err(e) => {
            dom::log(&format!("a Pro-Q frame did not decode: {e:?}"));
            None
        }
    };
    let Some(bitmap) = bitmap else {
        return;
    };
    let canvas = painter.try_with_value(|p| p.canvas.clone()).flatten();
    // The whole picture's size (PR L: a frame may hold a part of it, at its
    // corner in the header; the canvas keeps the rest it drew, out of
    // sight at that zoom).
    let (width, height) = (head.width, head.height);
    let corner = (head.x, head.y);
    if let Some(canvas) = canvas {
        if (canvas.width(), canvas.height()) != (width, height) {
            canvas.set_width(width);
            canvas.set_height(height);
            dom::set_attr(&canvas, "data-width", &width.to_string());
            dom::set_attr(&canvas, "data-height", &height.to_string());
        }
        if let Some(context) = context_2d(&canvas) {
            let _ = context.draw_image_with_image_bitmap(
                &bitmap,
                f64::from(corner.0),
                f64::from(corner.1),
            );
            let part = Crop {
                x: corner.0,
                y: corner.1,
                w: bitmap.width(),
                h: bitmap.height(),
            };
            let frames = painter.try_update_value(|p| {
                p.frames += 1;
                p.size = Some((width, height));
                p.part = Some(part);
                p.frames
            });
            if let Some(frames) = frames {
                dom::set_attr(&canvas, "data-frames", &frames.to_string());
            }
            dom::set_attr(
                &canvas,
                "data-part",
                &format!("{},{},{},{}", part.x, part.y, part.w, part.h),
            );
            let now = dom::epoch_now();
            let measured = drawn.store.eq_drawn(head, now);
            let decode_ms = dom::now() - started;
            let second = drawn
                .lat
                .try_update_value(|lat| lat.drawn(now, decode_ms, &measured.samples, measured.lost))
                .flatten();
            if let Some(event) = second {
                crate::diag::record(&event);
            }
        }
    }
    let small = painter.try_with_value(|p| p.small.clone()).flatten();
    if let Some(small) = &small {
        shape_overview(small, (width, height));
    }
    if let Some(small) = small
        && let Some(context) = context_2d(&small)
    {
        let (left, top, w, h) = scaled_part(
            corner,
            (bitmap.width(), bitmap.height()),
            (width, height),
            (small.width(), small.height()),
        );
        let _ = context.draw_image_with_image_bitmap_and_dw_and_dh(&bitmap, left, top, w, h);
    }
    bitmap.close();
}

/// A frame for the painter: drawn now, or kept as the newest while one
/// decodes (an older waiting one is dropped).
fn paint(painter: PainterBox, drawn: Drawn, head: FrameHead, blob: web_sys::Blob) {
    let waiting = painter.try_update_value(|p| {
        if p.busy {
            p.next = Some((head, blob));
            None
        } else {
            p.busy = true;
            Some((head, blob))
        }
    });
    let Some(Some(frame)) = waiting else {
        return;
    };
    leptos::task::spawn_local(async move {
        let mut frame = frame;
        loop {
            draw(painter, drawn, frame.0, &frame.1).await;
            match painter.try_update_value(|p| p.next.take()).flatten() {
                Some(newer) => frame = newer,
                None => break,
            }
        }
        let _ = painter.try_update_value(|p| p.busy = false);
    });
}

/// Where the fingers' area lies on the page and its size (CSS px), read at
/// each down, on a window resize and whenever the area's size changes
/// ([`watch_area`]): a move and a frame read no layout.
#[derive(Debug, Clone, Copy)]
struct Grip {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

impl Grip {
    /// The area's box now.
    fn of(area: &web_sys::HtmlDivElement) -> Self {
        let rect = area.get_bounding_client_rect();
        Self {
            left: rect.left(),
            top: rect.top(),
            width: rect.width(),
            height: rect.height(),
        }
    }

    /// A pointer's point on the area.
    fn at(self, ev: &web_sys::PointerEvent) -> (f64, f64) {
        (
            f64::from(ev.client_x()) - self.left,
            f64::from(ev.client_y()) - self.top,
        )
    }
}

/// The area's size watcher: a `ResizeObserver` and its callback, kept
/// until the screen goes.
type AreaObserver = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// Watches `area`'s size (#74 review): each change reads its box into
/// `grip`, so the frame loop's area (`AreaWatch`, the view) never reads a
/// stale one after a layout change with no window `resize` (the bar's
/// height, a safe area, iOS laying out after its `resize`). None when the
/// browser has no `ResizeObserver`.
fn watch_area(
    area: &web_sys::HtmlDivElement,
    grip: StoredValue<Option<Grip>>,
) -> Option<AreaObserver> {
    let observed = area.clone();
    let callback = Closure::wrap(Box::new(move || {
        let _ = grip.try_set_value(Some(Grip::of(&observed)));
    }) as Box<dyn FnMut()>);
    let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()).ok()?;
    observer.observe(area);
    Some((observer, callback))
}

/// Draws a view's look (PR F, `behave::eq::look`), only when it changed:
/// the canvas's transform, the overview's frame (percentages of the
/// picture), the bar's zoom group (shown, its factor) and whether one
/// finger edits (the overview's hint), each signal written only when it
/// differs.
fn show_look(
    look: &Look,
    canvas: NodeRef<html::Canvas>,
    frame: NodeRef<html::Div>,
    zoomed: RwSignal<bool>,
    factor: RwSignal<String>,
    editing: RwSignal<bool>,
) {
    if let Some(canvas) = canvas.get_untracked() {
        dom::set_style(&canvas, "transform", &look.transform);
    }
    if let Some(frame) = frame.get_untracked() {
        let (left, top, width, height) = look.seen;
        for (name, part) in [
            ("left", left),
            ("top", top),
            ("width", width),
            ("height", height),
        ] {
            dom::set_style(&frame, name, &format!("{}%", part * 100.0));
        }
    }
    if zoomed.try_get_untracked() != Some(look.zoomed) {
        let _ = zoomed.try_set(look.zoomed);
    }
    if editing.try_get_untracked() != Some(look.edits) {
        let _ = editing.try_set(look.edits);
    }
    if factor
        .try_with_untracked(|f| *f != look.factor)
        .unwrap_or(false)
    {
        let _ = factor.try_set(look.factor.clone());
    }
}

/// One Pro-Q 4 editor over the whole surface.
#[component]
pub fn EqScreen(target: EqTarget) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let nav = expect_context::<EqNav>();
    let painter: PainterBox = StoredValue::new_local(Painter::default());
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let small_ref = NodeRef::<html::Canvas>::new();
    let frame_ref = NodeRef::<html::Div>::new();
    let area_ref = NodeRef::<html::Div>::new();
    let finger = StoredValue::new(Finger::default());
    // The fingers as the flight recorder keeps them (PR I).
    let fingers = StoredValue::new(EqFingers::default());
    // The page's own view of the picture (PR F): each open starts at 1×.
    let viewer = StoredValue::new(Viewer::START);
    let grip = StoredValue::new(None::<Grip>);
    // The area's size watcher (#74 review), from its load to the cleanup.
    let observer = StoredValue::new_local(None::<AreaObserver>);
    // The picture area the hub knows (PR G): the open's, then each change.
    let watch = StoredValue::new(AreaWatch::default());
    // The part of the picture the hub sends (PR L): the whole one at first.
    let views = StoredValue::new(ViewWatch::default());
    // The look last drawn (none: the stylesheet's, the whole picture).
    let drawn = StoredValue::new(None::<Look>);
    let zoomed = RwSignal::new(false);
    let factor = RwSignal::new(zoom_text(1.0));
    // Whether one finger edits (2× or more); else the overview's hint.
    let editing = RwSignal::new(false);
    let legend = RwSignal::new(false);
    canvas_ref.on_load(move |canvas| {
        let _ = painter.try_update_value(|p| p.canvas = Some(canvas));
    });
    small_ref.on_load(move |small| {
        let _ = painter.try_update_value(|p| p.small = Some(small));
    });
    let send = move |outs: Vec<Out>| {
        for out in outs {
            store.eq_input(out);
        }
    };
    // The view's zoom now, and the fingers' events into the flight recorder
    // (PR I).
    let zoom_now = move || viewer.try_with_value(|v| v.view().zoom).unwrap_or(1.0);
    let record = move |events: Vec<serde_json::Value>| {
        for event in &events {
            crate::diag::record(event);
        }
    };
    // Every finger goes: each one's moves and its `what` where it was.
    let fingers_leave = move |what: &str| {
        record(
            fingers
                .try_update_value(|f| f.leave(dom::epoch_now(), what, zoom_now()))
                .unwrap_or_default(),
        );
    };
    // The latency of each second of frames drawn (PR J), and when a finger
    // last landed on an old picture (its note shows for a while).
    let lat = StoredValue::new(EqLat::default());
    let stale_at = StoredValue::new(None::<f64>);
    let stale_shown = RwSignal::new(false);
    // Whether the picture shows every input sent, and whether a quick tap's
    // up is still to go: with the store's dropout counter, what a first
    // finger meets (`behave::eq::takes_finger`). The canvas says both
    // (`data-caught-up`, `data-takes-finger`): an E2E waits for them before
    // a new first finger.
    let caught_up = RwSignal::new(true);
    let tap_pending = RwSignal::new(false);
    // The bar's ↶ VRÁTIŤ (PR P2b): the last press it took (page ms),
    // whether it takes none now (`aria-disabled`), its flash, and the note
    // of the hub's answer (its text, when it came, whether it shows).
    let undo_at = StoredValue::new(None::<f64>);
    let undo_off = RwSignal::new(false);
    let undo_flashing = RwSignal::new(false);
    let undo_note = RwSignal::new(String::new());
    let undo_note_at = StoredValue::new(None::<f64>);
    let undo_note_on = RwSignal::new(false);
    // They follow the store and the finger right after every send (#71
    // PR Q: written by the frame loop alone, the canvas still read caught
    // up a frame after a drag's up went, and a tap then was refused) and
    // on every frame (a frame drawn catches the picture up; the undo's
    // 700 ms run out). Never from the cleanup: the screen's own signals
    // stay untouched there.
    let note_takes = move || {
        let caught = store.eq_caught_up();
        if caught_up.try_get_untracked() != Some(caught) {
            let _ = caught_up.try_set(caught);
        }
        let pending = finger.try_with_value(Finger::tap_pending).unwrap_or(false);
        if tap_pending.try_get_untracked() != Some(pending) {
            let _ = tap_pending.try_set(pending);
        }
        let now = dom::now();
        let busy = finger.try_with_value(Finger::busy).unwrap_or(false);
        let pressed = undo_at.try_get_value().flatten();
        let off = !undo_ready(busy, pressed, now);
        if undo_off.try_get_untracked() != Some(off) {
            let _ = undo_off.try_set(off);
        }
        let flash = undo_flash(pressed, now);
        if undo_flashing.try_get_untracked() != Some(flash) {
            let _ = undo_flashing.try_set(flash);
        }
    };
    let told = Drawn { store, lat };
    let sink: FrameSink = Rc::new(move |head: FrameHead, blob: web_sys::Blob| {
        paint(painter, told, head, blob);
    });
    store.eq_frames(Some(sink));
    // Opening from now on; the open itself goes with the area, at its load.
    store.eq_opening(&target.instance, &target.path);
    // A hidden page lifts its finger (`Finger::visibility`), as the Stream
    // Deck lifts its keys: `visibilitychange` bubbles to the window. A
    // hidden Home Screen app keeps no touch going, so the recorder has each
    // finger `lost` (#124).
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        let hidden = dom::hidden();
        send(
            finger
                .try_update_value(|f| f.visibility(hidden))
                .unwrap_or_default(),
        );
        note_takes();
        if hidden {
            fingers_leave("lost");
        }
    });
    // A turned or resized screen: the area's box again (the frame loop
    // draws the view for it).
    let resized = window_event_listener_untyped("resize", move |_| {
        if let Some(area) = area_ref.get_untracked() {
            let _ = grip.try_set_value(Some(Grip::of(&area)));
        }
    });
    // Leaving the screen: a finger still down ends, the hub closes the
    // editor, the frames stop. (The component's own signals stay untouched.)
    on_cleanup(move || {
        hidden.remove();
        resized.remove();
        if let Some(Some((watcher, _callback))) = observer.try_update_value(Option::take) {
            watcher.disconnect();
        }
        send(finger.try_update_value(Finger::leave).unwrap_or_default());
        fingers_leave("cancel");
        if let Some(Some(event)) = lat.try_update_value(|l| l.leave(dom::epoch_now())) {
            crate::diag::record(&event);
        }
        store.eq_close();
        store.eq_frames(None);
    });
    // The hub closed it (refused, failed, its window gone, the socket):
    // back to the detail.
    {
        let (instance, path, keys) = (
            target.instance.clone(),
            target.path.clone(),
            target.keys.clone(),
        );
        Effect::new(move |_| {
            let why = store
                .eq
                .try_with(|view| ended(view.as_ref(), &instance, &path))
                .flatten();
            if let Some(why) = why {
                if lock_refusal(&why) {
                    trace_detail("eq_locked", None, &keys, None);
                }
                nav.close(&why, None);
            }
        });
    }
    // The hub's answer to an undo of this screen's editor (PR P2b): its
    // note over the picture (the store keeps only the open editor's, and
    // none from before this screen opened).
    Effect::new(move |_| {
        if let Some((_, result)) = store.eq_undone.try_get().flatten() {
            let _ = undo_note.try_set(undo_text(&result));
            let _ = undo_note_at.try_set_value(Some(dom::now()));
        }
    });
    // The editor this screen shows: the page's editor counts only when it
    // is this one (a late close of the one before is not this screen's).
    let editor = StoredValue::new((target.instance.clone(), target.path.clone()));
    // Whether it is open now (the frame loop's look, untracked).
    let editor_open = move || {
        editor
            .try_with_value(|(instance, path)| {
                store
                    .eq
                    .try_with_untracked(|v| screen_state(v.as_ref(), instance, path) == "open")
            })
            .flatten()
            .unwrap_or(false)
    };
    // The picture's size: the last frame's, else the hub's word at the open.
    let size = move || {
        painter.try_with_value(|p| p.size).flatten().or_else(|| {
            editor
                .try_with_value(|(instance, path)| {
                    store
                        .eq
                        .try_with_untracked(|v| {
                            screen_view(v.as_ref(), instance, path).and_then(|v| v.size)
                        })
                        .flatten()
                })
                .flatten()
        })
    };
    // A pointer's point now: on the area, and on the picture through the
    // view (it changes only while two fingers pinch, when the editor gets
    // nothing), with the picture's size.
    let point_of = move |ev: &web_sys::PointerEvent| -> Option<(Point, (u32, u32))> {
        let g = grip.try_get_value().flatten()?;
        let picture = size()?;
        let shown = viewer
            .try_with_value(|v| v.placed((g.width, g.height), picture))
            .flatten()?;
        Some((point(shown, g.at(ev)), picture))
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(area) = area_ref.get_untracked() else {
            return;
        };
        let _ = grip.try_set_value(Some(Grip::of(&area)));
        let Some((at, picture)) = point_of(&ev) else {
            return;
        };
        let by = down_of(&ev);
        let id = by.id;
        let now = dom::now();
        // The finger's own time (the PR I review: the handler's time shifts
        // with a long frame).
        let t = dom::event_epoch(&ev);
        // A finger whose end the browser never delivered (#124: iOS took it
        // for a system gesture) ends first, at a primary down of its type:
        // its contact cancelled at the PC, recorded `lost` before this
        // finger's down. This finger then starts a gesture of its own, a
        // first finger under the picture's rule below, never a pinch.
        let missed = finger
            .try_update_value(|f| f.missed(by))
            .unwrap_or_default();
        for pointer in missed.pointers {
            record(
                fingers
                    .try_update_value(|f| f.end(t, "lost", pointer, None, zoom_now()))
                    .unwrap_or_default(),
            );
        }
        send(missed.out);
        let down = fingers.try_update_value(|f| f.down(t, id, at.picture, zoom_now()));
        record(down.into_iter().collect());
        let lands = spot(on_picture(at.picture, picture), zoom_now());
        // A first finger on an old picture, or in a stalled link, reaches
        // nothing (PR J): the screen says so, the recorder too; one that may
        // edit marks the picture it landed on (its down's `seen`).
        let first = !finger.try_with_value(Finger::held).unwrap_or(false);
        // A quick tap's up still to go: the picture cannot show it yet.
        let pending = finger.try_with_value(Finger::tap_pending).unwrap_or(false);
        // It lands on the part drawn last (the PR L re-review: after a pan
        // past the margin, the strip that came into sight is old until the
        // new part is drawn; a finger elsewhere is on current pixels).
        let on_part = painter
            .try_with_value(|p| p.part)
            .flatten()
            .is_none_or(|part| lands_on(part, picture, at.picture));
        // The page's one rule for a first finger (PR Q), which the canvas
        // says as `data-takes-finger`; the dropout is in it, so none is
        // passed again.
        let takes = takes_finger(store.eq_caught_up(), pending, store.dropping_out());
        let stale = first && stale_landing(lands, takes && on_part, false);
        if stale {
            let _ = stale_at.try_set_value(Some(now));
            // The recorder tells a finger on the old strip from one the
            // rule refused (a picture behind the page's inputs, a quick
            // tap's up to go, a dropout).
            let what = if takes { "uncovered" } else { "stale" };
            record(vec![eq_touch(t, what, id, at.picture, zoom_now())]);
        } else if first && lands == Spot::Edit {
            store.eq_land();
        }
        let lands = if stale { Spot::Off } else { lands };
        let out = finger
            .try_update_value(|f| f.down(by, at, lands, now))
            .unwrap_or_default();
        if finger.try_with_value(|f| f.has(id)).unwrap_or(false) {
            let _ = area.set_pointer_capture(id);
        }
        send(out);
        note_takes();
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        if let Some((at, _)) = point_of(&ev) {
            let id = ev.pointer_id();
            let _ = finger.try_update_value(|f| f.moved(id, at));
            let t = dom::event_epoch(&ev);
            let summary = fingers
                .try_update_value(|f| f.moved(t, id, at.picture))
                .flatten();
            record(summary.into_iter().collect());
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let Some((at, _)) = point_of(&ev) else {
            return;
        };
        let now = dom::now();
        let id = ev.pointer_id();
        let t = dom::event_epoch(&ev);
        record(
            fingers
                .try_update_value(|f| f.end(t, "up", id, Some(at.picture), zoom_now()))
                .unwrap_or_default(),
        );
        send(
            finger
                .try_update_value(|f| f.up(id, at, now))
                .unwrap_or_default(),
        );
        note_takes();
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let t = dom::event_epoch(&ev);
        record(
            fingers
                .try_update_value(|f| f.end(t, "cancel", id, None, zoom_now()))
                .unwrap_or_default(),
        );
        send(
            finger
                .try_update_value(|f| f.cancel(id))
                .unwrap_or_default(),
        );
        note_takes();
    };
    // Once the area is in the page: its box, its size watched, and the open
    // with it (PR G: the editor gets its aspect). Each frame: a waiting
    // finger's down once it waited or slid, a quick tap's up once 40 ms
    // passed, a move at most once (the newest); a changed area once it
    // rested while the editor is open; the view follows a pinch, and a
    // changed look is drawn (only then: nothing is restyled while the view
    // rests).
    let (instance, path) = (target.instance.clone(), target.path.clone());
    raf::animate(area_ref, move |el| {
        let opened = Grip::of(&el);
        let area = (opened.width, opened.height);
        let _ = grip.try_set_value(Some(opened));
        let _ = observer.try_set_value(watch_area(&el, grip));
        let _ = watch.try_set_value(AreaWatch::opened(area));
        store.eq_open(&instance, &path, area);
        Box::new(move |_now: f64, _step: f64| {
            let now = dom::now();
            // The link drops out under a finger on the editor (PR J
            // review): its contact ends with its cancel, as for an old
            // picture, so no move queued in a stalled socket comes later;
            // before the frame's own sends (the re-review: a waiting finger
            // would first send its down into the stalled link). The recorder
            // gets the finger's cancel.
            let stalled = finger.try_with_value(Finger::editor_pointer).flatten();
            if let Some(pointer) = stalled.filter(|_| store.dropping_out()) {
                send(
                    finger
                        .try_update_value(Finger::off_editor)
                        .unwrap_or_default(),
                );
                let _ = stale_at.try_set_value(Some(now));
                record(
                    fingers
                        .try_update_value(|f| {
                            f.end(dom::epoch_now(), "cancel", pointer, None, zoom_now())
                        })
                        .unwrap_or_default(),
                );
            }
            send(
                finger
                    .try_update_value(|f| f.frame(now))
                    .unwrap_or_default(),
            );
            note_takes();
            let noted = stale_note(stale_at.try_get_value().flatten(), now);
            if stale_shown.try_get_untracked() != Some(noted) {
                let _ = stale_shown.try_set(noted);
            }
            let noted = undo_note_shown(undo_note_at.try_get_value().flatten(), now);
            if undo_note_on.try_get_untracked() != Some(noted) {
                let _ = undo_note_on.try_set(noted);
            }
            let Some(g) = grip.try_get_value().flatten() else {
                return;
            };
            let area = (g.width, g.height);
            let open = editor_open();
            if let Some(changed) = watch
                .try_update_value(|w| w.frame(area, open, now))
                .flatten()
            {
                store.eq_area(changed);
            }
            let Some(picture) = size() else {
                return;
            };
            let pair = finger.try_with_value(Finger::pair).flatten();
            // An overview tap's zoom on its point (taken once the picture
            // and the area are known).
            let tapped = finger.try_update_value(|f| f.take_zoom(now)).flatten();
            let wanted = viewer
                .try_update_value(|v| {
                    if let Some(at) = tapped {
                        v.zoom_on(at, area, picture);
                    }
                    v.follow(pair, area, picture);
                    look(area, picture, v.view())
                })
                .flatten();
            let Some(wanted) = wanted else {
                return;
            };
            // At 2× and more, only the part in sight goes (PR L), once the
            // view rested; a part names the picture's size it is of.
            let changed = views
                .try_update_value(|w| w.frame(part_of(picture, &wanted), open, now))
                .flatten();
            if let Some(part) = changed {
                store.eq_view(part);
                record(vec![eq_view(dom::epoch_now(), part)]);
            }
            if drawn
                .try_with_value(|d| d.as_ref() != Some(&wanted))
                .unwrap_or(false)
            {
                show_look(&wanted, canvas_ref, frame_ref, zoomed, factor, editing);
                let _ = drawn.try_set_value(Some(wanted));
            }
        })
    });
    let exit = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        nav.close("exit", Some(ev.pointer_id()));
    };
    let help = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = legend.try_update(|on| *on = !*on);
    };
    // `CELÝ EQ`: the whole picture again (the frame loop draws it). A finger
    // on the editor ends first, with its cancel where it was (the PR I
    // review: a drag went on at 1×, its points remapped); a pinch goes on.
    let whole = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        send(
            finger
                .try_update_value(Finger::off_editor)
                .unwrap_or_default(),
        );
        note_takes();
        let _ = viewer.try_update_value(Viewer::whole);
    };
    // ↶ VRÁTIŤ (PR P2b): Pro-Q's own undo, when the bar's rule takes the
    // press (`undo_ready`); it goes numbered as the editor's next input,
    // then the control rests 700 ms and flashes, and a first finger waits
    // until a picture shows the undo (`note_takes`, as after a finger's
    // send).
    let undo = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let now = dom::now();
        let busy = finger.try_with_value(Finger::busy).unwrap_or(false);
        if !undo_ready(busy, undo_at.try_get_value().flatten(), now) {
            return;
        }
        if store.eq_undo(dom::event_epoch(&ev)).is_none() {
            return;
        }
        let _ = undo_at.try_set_value(Some(now));
        note_takes();
    };
    let undo_disabled = move || undo_off.try_get().unwrap_or(false).to_string();
    let undo_flash_text = move || undo_flashing.try_get().unwrap_or(false).to_string();
    let undo_note_text = move || undo_note.try_get().unwrap_or_default();
    let undo_note_shown_text = move || undo_note_on.try_get().unwrap_or(false).to_string();
    let shown_zoom = move || zoomed.try_get().unwrap_or(false).to_string();
    let factor_text = move || factor.try_get().unwrap_or_default();
    let state = move || {
        editor
            .try_with_value(|(instance, path)| {
                store
                    .eq
                    .try_with(|v| screen_state(v.as_ref(), instance, path))
            })
            .flatten()
            .unwrap_or("opening")
    };
    let session = move || {
        editor
            .try_with_value(|(instance, path)| {
                store
                    .eq
                    .try_with(|v| {
                        screen_view(v.as_ref(), instance, path).and_then(EqView::open_session)
                    })
                    .flatten()
            })
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let opening = move || state() == "opening";
    let open = move || state() == "open";
    let hint_shown = move || (open() && !editing.try_get().unwrap_or(false)).to_string();
    let stale_text_shown = move || stale_shown.try_get().unwrap_or(false).to_string();
    let caught_up_text = move || caught_up.try_get().unwrap_or(true).to_string();
    // Whether a first finger is taken now (PR Q): the down's own rule over
    // the signals above and the dropout counter's (`LiveStore::dropping_out`
    // reads the same watch the counter's signal shows).
    let takes_text = move || {
        takes_finger(
            caught_up.try_get().unwrap_or(true),
            tap_pending.try_get().unwrap_or(false),
            store.dropouts.try_with(|c| c.active).unwrap_or(false),
        )
        .to_string()
    };
    let exit_keys: Vec<String> = Vec::new();
    let help_keys: Vec<String> = Vec::new();
    let whole_keys: Vec<String> = Vec::new();
    let undo_keys: Vec<String> = Vec::new();
    let area_keys: Vec<String> = Vec::new();
    let where_text = target.title.clone();
    let chip = target.chip.clone();
    view! {
        <div
            class="eq-screen"
            data-testid="eq-screen"
            data-state=state
            data-session=session
            data-path=target.path.clone()
        >
            <div class="eq-bar" data-testid="eq-bar">
                <button
                    type="button"
                    class="detail-exit eq-exit"
                    data-testid="eq-exit"
                    use:owns_touches=exit_keys
                    on:pointerdown=exit
                >
                    <span class="eq-exit-long">"← SPÄŤ NA KANÁL"</span>
                    <span class="eq-exit-short">"← KANÁL"</span>
                </button>
                <div class="eq-name">
                    <span class="detail-chip" style=chip>{target.label.clone()}</span>
                    <span class="eq-where" data-testid="eq-where">{where_text}</span>
                </div>
                <div class="eq-zoom" data-testid="eq-zoom" data-shown=shown_zoom>
                    <div class="eq-overview" data-testid="eq-overview">
                        <canvas class="eq-overview-pic" width="134" height="80" node_ref=small_ref></canvas>
                        <div class="eq-overview-view" data-testid="eq-overview-view" node_ref=frame_ref></div>
                    </div>
                    <span class="eq-factor" data-testid="eq-factor">{factor_text}</span>
                    <button
                        type="button"
                        class="eq-whole"
                        data-testid="eq-whole"
                        use:owns_touches=whole_keys
                        on:pointerdown=whole
                    >
                        "CELÝ EQ"
                    </button>
                </div>
                <button
                    type="button"
                    class="eq-undo"
                    data-testid="eq-undo"
                    use:owns_touches=undo_keys
                    aria-disabled=undo_disabled
                    data-flash=undo_flash_text
                    on:pointerdown=undo
                >
                    <span class="eq-undo-arrow" data-testid="eq-undo-arrow">"↶"</span>
                    <span class="eq-undo-word" data-testid="eq-undo-word">"VRÁTIŤ"</span>
                </button>
                <span class="eq-mine" data-testid="eq-mine" data-shown=move || open().to_string()>
                    "zamknuté pre teba"
                </span>
                <button
                    type="button"
                    class="eq-help"
                    data-testid="eq-help"
                    use:owns_touches=help_keys
                    on:pointerdown=help
                >
                    "?"
                </button>
            </div>
            <div
                class="eq-area"
                data-testid="eq-area"
                node_ref=area_ref
                use:owns_touches=area_keys
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_cancel
                on:lostpointercapture=on_cancel
            >
                <canvas
                    class="eq-canvas"
                    data-testid="eq-canvas"
                    data-frames="0"
                    data-caught-up=caught_up_text
                    data-takes-finger=takes_text
                    node_ref=canvas_ref
                ></canvas>
                <div class="eq-hint" data-testid="eq-hint" data-shown=hint_shown>
                    "Ťukni na miesto, ktoré chceš upraviť"
                </div>
                <div class="eq-hint eq-stale" data-testid="eq-stale" data-shown=stale_text_shown>
                    {STALE_TEXT}
                </div>
                <div class="eq-undo-note" data-testid="eq-undo-note" data-shown=undo_note_shown_text>
                    {undo_note_text}
                </div>
                {move || opening().then(|| view! { <div class="eq-wait">"Otváram Pro-Q 4…"</div> })}
            </div>
            {move || legend.try_get().unwrap_or(false).then(|| view! { <EqLegend /> })}
        </div>
    }
}

/// What a finger does on the Pro-Q (`?`): on the whole picture a tap zooms
/// in (the owner's ruling of 2026-10-11); zoomed 2× or more real touches
/// reach the PC; two fingers zoom this page's picture only (PR F, the
/// mockup's lines); a hold opens nothing (PR O proved it on the PC); ↶
/// VRÁTIŤ takes back Pro-Q's last change (PR P2b).
#[component]
fn EqLegend() -> impl IntoView {
    view! {
        <div class="eq-legend" data-testid="eq-legend">
            <h3>"DOTYK V PRO-Q"</h3>
            <table>
                <tr class="eq-legend-zoom" data-testid="eq-legend-overview">
                    <td>"ťuk na celý EQ"</td>
                    <td>"Ťukni na miesto, ktoré chceš upraviť: priblíži sa 3×. Upravuje sa až pri priblížení 2× a viac."</td>
                </tr>
                <tr><td>"1 prst ťuk"</td><td>"klik (tlačidlá, výber pásma)"</td></tr>
                <tr><td>"1 prst ťah"</td><td>"ťahanie (pásmo, gombík); kurzor na PC počas ťahu skočí"</td></tr>
                <tr><td>"dvojťuk"</td><td>"dvojklik (nové pásmo v krivke; na pásme jeho frekvencia)"</td></tr>
                <tr data-testid="eq-legend-hold"><td>"podržať"</td><td>"nič: ponuka pásma sa neotvorí"</td></tr>
                <tr class="eq-legend-zoom"><td>"2 prsty od seba / k sebe"</td><td>"priblížiť / oddialiť (1× až 4×)"</td></tr>
                <tr class="eq-legend-zoom"><td>"2 prsty spolu"</td><td>"posúvať priblížený obraz"</td></tr>
                <tr class="eq-legend-zoom"><td>"CELÝ EQ"</td><td>"späť na celý obraz"</td></tr>
                <tr class="eq-legend-undo" data-testid="eq-legend-undo">
                    <td>"↶ VRÁTIŤ"</td>
                    <td>"vráti poslednú zmenu v Pro-Q"</td>
                </tr>
            </table>
            <p class="eq-legend-note">
                "Pri priblížení obraz na PC zostáva rovnaký: mení sa len pohľad na tomto zariadení."
            </p>
        </div>
    }
}
