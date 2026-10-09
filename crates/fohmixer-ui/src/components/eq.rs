//! The Pro-Q 4 screen on the surface (#71 PR E, F28, D17; the approved
//! mockup `docs/mockups/channel-detail-v2.html`, its EQ card and EQ screen).
//!
//! - **The cards** ([`EqCards`], the channel detail's middle): the strip's
//!   Pro-Q 4 instances as the hub reads them (`eq_list`, asked when the
//!   detail opens, after every reconnect, whenever the strip's instance
//!   comes back online, and once the screen closed), one
//!   card each: its last picture (fetched with the token into a blob URL;
//!   a plain field before a first open), where it sits, and `OTVORIŤ EQ NA
//!   CELÚ OBRAZOVKU`, or `ZAMKNUTÉ` with who holds it since when.
//! - **The screen** ([`EqScreen`], over the whole surface, mounted while
//!   [`EqNav`] names an editor; outside the layout's shell, so a new layout
//!   keeps it): the bar (`← SPÄŤ NA KANÁL`, the chip, `Pro-Q 4 · <where>`,
//!   the lock mark, `?` with the touch legend) and the live picture: the
//!   binary frames drawn into a canvas through `createImageBitmap`, fitted
//!   (`object-fit: contain`), the newest frame winning while one decodes.
//!   One finger maps to `eq_input` in the picture's pixels
//!   (`behave::eq::Finger`: a move at most once per animation frame, the up
//!   and the cancel always, a lost capture as a cancel, a second finger
//!   ignored). Leaving it sends `eq_close`; the hub closing it (refused,
//!   failed, its window gone, the socket lost) brings the detail back.
//!
//! The screen's open, close and lock refusal go to the flight recorder as
//! `detail` events (`eq_open`, `eq_close` with why, `eq_locked`) with the
//! strip's keys. Every tap target owns its touches.

use std::rc::Rc;

use fohmixer_proto::eq::{EqItem, EqState, PRODUCT};
use fohmixer_proto::layout::Strip;
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::buttons::colour_style;
use super::{owns_touches, trace_detail};
use crate::behave::eq::{
    CardLock, Finger, Fit, ListLink, can_open, card_lock, failure_text, fit, hh_mm, list_text,
    lists_now, locked_text, on_picture, place_text, to_picture,
};
use crate::binding::detail_keys;
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
    let other = move || matches!(lock.try_get(), Some(CardLock::Other(_)));
    // Offered while free (or this page's) and the page is connected: an
    // open the socket cannot take is not offered (`can_open`).
    let disabled = move || {
        let lock = lock.try_get().unwrap_or(CardLock::Free);
        let connected = store.connected.try_get().unwrap_or(false);
        (!can_open(lock, connected)).to_string()
    };
    let since = move || match lock.try_get() {
        Some(CardLock::Other(since)) => Some(locked_text(&local_hh_mm(since))),
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

/// The picture's painter: the canvas, whether a frame decodes, the newest
/// frame waiting meanwhile, the frames drawn and the picture's size.
#[derive(Default)]
struct Painter {
    canvas: Option<web_sys::HtmlCanvasElement>,
    busy: bool,
    next: Option<web_sys::Blob>,
    frames: u32,
    size: Option<(u32, u32)>,
}

/// The painter, kept with the screen (its handle is `Copy` and `Send`, as
/// the view's closures must be; the painter itself stays on the page's
/// thread).
type PainterBox = StoredValue<Painter, LocalStorage>;

/// Draws one frame: decoded off the page's thread (`createImageBitmap`),
/// then onto the canvas at its own size.
async fn draw(painter: PainterBox, blob: &web_sys::Blob) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(promise) = window.create_image_bitmap_with_blob(blob) else {
        return;
    };
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
    if let Some(canvas) = canvas {
        let (width, height) = (bitmap.width(), bitmap.height());
        if (canvas.width(), canvas.height()) != (width, height) {
            canvas.set_width(width);
            canvas.set_height(height);
            dom::set_attr(&canvas, "data-width", &width.to_string());
            dom::set_attr(&canvas, "data-height", &height.to_string());
        }
        let context = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok());
        if let Some(context) = context {
            let _ = context.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0);
            let frames = painter.try_update_value(|p| {
                p.frames += 1;
                p.size = Some((width, height));
                p.frames
            });
            if let Some(frames) = frames {
                dom::set_attr(&canvas, "data-frames", &frames.to_string());
            }
        }
    }
    bitmap.close();
}

/// A frame for the painter: drawn now, or kept as the newest while one
/// decodes (an older waiting one is dropped).
fn paint(painter: PainterBox, blob: web_sys::Blob) {
    let waiting = painter.try_update_value(|p| {
        if p.busy {
            p.next = Some(blob);
            None
        } else {
            p.busy = true;
            Some(blob)
        }
    });
    let Some(Some(blob)) = waiting else {
        return;
    };
    leptos::task::spawn_local(async move {
        let mut blob = blob;
        loop {
            draw(painter, &blob).await;
            match painter.try_update_value(|p| p.next.take()).flatten() {
                Some(newer) => blob = newer,
                None => break,
            }
        }
        let _ = painter.try_update_value(|p| p.busy = false);
    });
}

/// Where the finger's area was when it went down: the picture's fit and
/// the area's corner on the page (the moves map by it, no layout read).
#[derive(Debug, Clone, Copy)]
struct Grip {
    fit: Fit,
    left: f64,
    top: f64,
}

/// One Pro-Q 4 editor over the whole surface.
#[component]
pub fn EqScreen(target: EqTarget) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let nav = expect_context::<EqNav>();
    let painter: PainterBox = StoredValue::new_local(Painter::default());
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let area_ref = NodeRef::<html::Div>::new();
    let finger = StoredValue::new(Finger::default());
    let grip = StoredValue::new(None::<Grip>);
    let legend = RwSignal::new(false);
    canvas_ref.on_load(move |canvas| {
        let _ = painter.try_update_value(|p| p.canvas = Some(canvas));
    });
    let sink: FrameSink = Rc::new(move |blob: web_sys::Blob| paint(painter, blob));
    store.eq_frames(Some(sink));
    store.eq_open(&target.instance, &target.path);
    // A hidden page lifts its finger (`Finger::visibility`), as the Stream
    // Deck lifts its keys: `visibilitychange` bubbles to the window.
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if let Some(Some(out)) = finger.try_update_value(|f| f.visibility(dom::hidden())) {
            store.eq_input(out);
        }
    });
    // Leaving the screen: a finger still down ends, the hub closes the
    // editor, the frames stop. (The component's own signals stay untouched.)
    on_cleanup(move || {
        hidden.remove();
        if let Some(Some(out)) = finger.try_update_value(Finger::leave) {
            store.eq_input(out);
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
                if why == fohmixer_proto::eq::reason::LOCKED {
                    trace_detail("eq_locked", None, &keys, None);
                }
                nav.close(&why, None);
            }
        });
    }
    // The editor this screen shows: the page's editor counts only when it
    // is this one (a late close of the one before is not this screen's).
    let editor = StoredValue::new((target.instance.clone(), target.path.clone()));
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
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let (Some(area), Some(canvas)) = (area_ref.get_untracked(), canvas_ref.get_untracked())
        else {
            return;
        };
        let rect = canvas.get_bounding_client_rect();
        let Some(picture) = size() else {
            return;
        };
        let Some(fit) = fit((rect.width(), rect.height()), picture) else {
            return;
        };
        let id = ev.pointer_id();
        let at = to_picture(
            fit,
            (
                f64::from(ev.client_x()) - rect.left(),
                f64::from(ev.client_y()) - rect.top(),
            ),
        );
        let down = finger
            .try_update_value(|f| f.down(id, at, on_picture(at, picture)))
            .flatten();
        if let Some(out) = down {
            let _ = area.set_pointer_capture(id);
            let _ = grip.try_set_value(Some(Grip {
                fit,
                left: rect.left(),
                top: rect.top(),
            }));
            store.eq_input(out);
        }
    };
    let at_of = move |ev: &web_sys::PointerEvent| {
        grip.try_get_value().flatten().map(|g| {
            to_picture(
                g.fit,
                (
                    f64::from(ev.client_x()) - g.left,
                    f64::from(ev.client_y()) - g.top,
                ),
            )
        })
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        if let Some(at) = at_of(&ev) {
            let _ = finger.try_update_value(|f| f.moved(ev.pointer_id(), at));
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let Some(at) = at_of(&ev) else {
            return;
        };
        if let Some(Some(out)) = finger.try_update_value(|f| f.up(ev.pointer_id(), at)) {
            store.eq_input(out);
        }
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        if let Some(Some(out)) = finger.try_update_value(|f| f.cancel(ev.pointer_id())) {
            store.eq_input(out);
        }
    };
    // A move goes at most once per frame, the newest.
    raf::animate(area_ref, move |_el| {
        Box::new(move |_now: f64, _step: f64| {
            if let Some(Some(out)) = finger.try_update_value(Finger::frame) {
                store.eq_input(out);
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
    let exit_keys: Vec<String> = Vec::new();
    let help_keys: Vec<String> = Vec::new();
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
            <div class="eq-bar">
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
                <canvas class="eq-canvas" data-testid="eq-canvas" data-frames="0" node_ref=canvas_ref></canvas>
                {move || opening().then(|| view! { <div class="eq-wait">"Otváram Pro-Q 4…"</div> })}
            </div>
            {move || legend.try_get().unwrap_or(false).then(|| view! { <EqLegend /> })}
        </div>
    }
}

/// What a finger does on the Pro-Q (`?`): real touches reach the PC.
#[component]
fn EqLegend() -> impl IntoView {
    view! {
        <div class="eq-legend" data-testid="eq-legend">
            <h3>"DOTYK V PRO-Q"</h3>
            <table>
                <tr><td>"1 prst ťuk"</td><td>"klik (tlačidlá, výber pásma)"</td></tr>
                <tr><td>"1 prst ťah"</td><td>"ťahanie (pásmo, gombík); kurzor na PC počas ťahu skočí"</td></tr>
                <tr><td>"dvojťuk"</td><td>"dvojklik (nové pásmo v krivke; na pásme jeho frekvencia)"</td></tr>
                <tr><td>"podržať"</td><td>"pravé tlačidlo (menu pásma) "<span class="eq-tbd">"· ešte overiť na PC"</span></td></tr>
                <tr><td>"2 prsty"</td><td>"druhý prst sa nepoužije"</td></tr>
            </table>
        </div>
    }
}
