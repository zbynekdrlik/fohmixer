//! The Pro-Q 4 screen's fingers and picture (#71 PR E, F28), pure: where the
//! editor's picture sits fitted into the screen ([`fit`], the stylesheet's
//! `object-fit: contain`), the page's own view of it (PR F: [`View`], a
//! zoom of 1 to [`MAX_ZOOM`] and a pan kept by [`placed`]; [`Viewer`], the
//! pinch that moves it; [`look`], how the screen draws it; [`overview_size`],
//! the overview's shape), a point of the
//! screen in the picture's pixels ([`to_picture`], [`point`]), the fingers
//! ([`Finger`]: a first finger waits [`HOLD_MS`] or [`SLOP`] before the
//! editor gets it, a lift meanwhile is a tap (PR G: its down at once, its
//! up [`TAP_MS`] later on a frame; no down goes before a tap's up went),
//! a second finger makes a pinch
//! that sends the editor nothing, a move goes at most once per animation
//! frame, the newest one, and the end always goes, a lost capture as a
//! cancel, a hidden page's finger lifted, and a finger whose end never came
//! ended by the next primary down of its type, #124, [`Finger::missed`];
//! the owner's ruling of 2026-10-11:
//! below [`EDIT_ZOOM_MIN`] the whole picture takes no edits, and a tap there
//! zooms to [`TAP_ZOOM`] on its point, [`Spot`], [`Viewer::zoom_on`]), when
//! the page's picture area
//! goes to the hub (PR G: [`AreaWatch`]), a card's lock ([`card_lock`]:
//! one editor on the PC's screen at a time, so another page's editor locks
//! every card) and its text, whether its open is offered ([`can_open`]),
//! when the cards are listed ([`lists_now`]), what names an editor
//! ([`place_text`]), a card's note and the cards' list note in Slovak
//! ([`failure_text`], [`list_text`]), which frames the screen shows
//! ([`shows_frame`]), a card's picture URL ([`picture_url`]) and the bar's
//! ↶ VRÁTIŤ (#71 PR P2b: when it takes a press, [`undo_ready`], its flash
//! and the note of its answer, [`undo_text`]).

use fohmixer_proto::eq::{EqLock, PRODUCT, Touch, UNDO_APART_MS, UNDO_DONE, UNDO_NOTHING, reason};

use super::Down;
use crate::diag::perf::Pointer;

/// Where a picture sits fitted into an area: its scale (CSS px a picture
/// pixel) and its top-left corner in the area (CSS px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub left: f64,
    pub top: f64,
}

/// A picture of `picture` pixels fitted whole into an area of `area` CSS
/// px, centred (`object-fit: contain`); none while either is empty.
pub fn fit(area: (f64, f64), picture: (u32, u32)) -> Option<Fit> {
    let (width, height) = (f64::from(picture.0), f64::from(picture.1));
    let empty = area.0.min(area.1).min(width).min(height) <= 0.0;
    if empty {
        return None;
    }
    let scale = (area.0 / width).min(area.1 / height);
    Some(Fit {
        scale,
        left: (area.0 - width * scale) / 2.0,
        top: (area.1 - height * scale) / 2.0,
    })
}

/// A point of the area (CSS px from its top-left) in the picture's pixels.
pub fn to_picture(fit: Fit, at: (f64, f64)) -> (f64, f64) {
    ((at.0 - fit.left) / fit.scale, (at.1 - fit.top) / fit.scale)
}

/// Whether a point (picture pixels) lies on a picture of `size`.
pub fn on_picture(at: (f64, f64), size: (u32, u32)) -> bool {
    let inside = |v: f64, n: u32| (0.0..f64::from(n)).contains(&v);
    inside(at.0, size.0) && inside(at.1, size.1)
}

/// The closest view of the picture (#71 PR F): 4 × the fitted picture.
pub const MAX_ZOOM: f64 = 4.0;

/// The zoom from which the screen counts as zoomed (its bar's zoom group
/// shows, the factor reads `1,1×` or more); a pinch that ends under it goes
/// back to the whole picture.
pub const ZOOMED_FROM: f64 = 1.05;

/// The least zoom at which one finger edits (#71, the owner's ruling of
/// 2026-10-11): a band point is about 2 mm under a 10 mm finger on the
/// whole picture, so below this zoom one finger never reaches the editor
/// (the overview), and a tap there zooms in ([`TAP_ZOOM`]).
pub const EDIT_ZOOM_MIN: f64 = 2.0;

/// The zoom a tap on the overview goes to, its point in the area's middle.
pub const TAP_ZOOM: f64 = 3.0;

/// How long after a tap zoomed in a first finger's down waits (ms; the PR I
/// review): the iPad's habit of a double tap to zoom would edit with its
/// second tap, at a point the finger aimed at on the whole picture. A
/// finger lifted within it reaches nothing; one still down at its end goes
/// then (a drag started right after the zoom, the PR I re-review).
pub const ZOOM_SETTLE_MS: f64 = 500.0;

/// Whether a finger down at `now` falls within the settle after a tap's
/// zoom taken at `zoomed` (page ms; none: no tap zoomed yet).
pub fn settling(zoomed: Option<f64>, now: f64) -> bool {
    zoomed.is_some_and(|at| now - at < ZOOM_SETTLE_MS)
}

/// Whether a settle ending at `until` is over at `now` (page ms).
fn settled(until: f64, now: f64) -> bool {
    now >= until
}

/// Whether one finger edits at `zoom` ([`EDIT_ZOOM_MIN`]).
pub fn edits(zoom: f64) -> bool {
    zoom >= EDIT_ZOOM_MIN
}

/// Where a first finger lands, as the view takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
    /// Off the picture: never the editor's (a second finger still pinches).
    Off,
    /// On the overview (the view under [`EDIT_ZOOM_MIN`]): no edit; a tap
    /// there zooms in on its point.
    Overview,
    /// On the zoomed picture: the editor's.
    Edit,
}

/// Whether a first finger landing on `spot` lands on an old picture (#71
/// PR J): on the zoomed picture (the editor's) while the picture shown does
/// not show every input the page sent (`caught_up`), or while the link
/// drops out (the down would wait in a stalled link, as a Stream Deck press
/// that cannot go). Such a finger reaches nothing; the screen says so.
pub fn stale_landing(spot: Spot, caught_up: bool, dropping_out: bool) -> bool {
    spot == Spot::Edit && (!caught_up || dropping_out)
}

/// Whether a first finger landing on the zoomed picture is taken now (#71
/// PR Q): the picture shows every input the page sent (`caught_up`), no
/// quick tap's up is still to go (`tap_pending`: the picture cannot show
/// it yet) and the link does not drop out (`dropping_out`). The screen's
/// pointer down decides with it (the part drawn last under the finger
/// checked as well) and its canvas says it as `data-takes-finger`, so a
/// test that waits for the attribute and the page never disagree.
pub fn takes_finger(caught_up: bool, tap_pending: bool, dropping_out: bool) -> bool {
    caught_up && !tap_pending && !dropping_out
}

/// How long the screen says a finger landed on an old picture (ms, PR J).
pub const STALE_NOTE_MS: f64 = 1500.0;

/// Whether the note of a finger refused at `at` still shows at `now` (page
/// ms; none: no refusal).
pub fn stale_note(at: Option<f64>, now: f64) -> bool {
    at.is_some_and(|at| now - at < STALE_NOTE_MS)
}

/// The note's words.
pub const STALE_TEXT: &str = "Obraz ešte dobieha, polož prst znova";

/// Where a first finger lands `on` the picture or off it, the view at
/// `zoom`.
pub fn spot(on: bool, zoom: f64) -> Spot {
    if !on {
        Spot::Off
    } else if edits(zoom) {
        Spot::Edit
    } else {
        Spot::Overview
    }
}

/// The page's own view of the picture (#71 PR F; the editor on the PC never
/// changes): the zoom over the fitted picture (1 to [`MAX_ZOOM`]) and where
/// the zoomed picture's top-left corner lies in the area (`pan`, CSS px).
/// [`placed`] keeps the corner so the picture covers the area where it is
/// larger than it and stays centred where it is smaller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub pan: (f64, f64),
}

impl View {
    /// The whole picture, fitted: each open starts here, and `CELÝ EQ`
    /// comes back to it.
    pub const WHOLE: Self = Self {
        zoom: 1.0,
        pan: (0.0, 0.0),
    };
}

/// The picture placed in the area by `view`: the fitted scale times the
/// zoom, the corner the view's pan kept within the area ([`place`]). None
/// while the area or the picture is empty.
pub fn placed(area: (f64, f64), picture: (u32, u32), view: View) -> Option<Fit> {
    let fitted = fit(area, picture)?;
    let scale = fitted.scale * view.zoom;
    Some(Fit {
        scale,
        left: place(view.pan.0, f64::from(picture.0) * scale, area.0),
        top: place(view.pan.1, f64::from(picture.1) * scale, area.1),
    })
}

/// One side of a placed picture: the corner from `pan` for a picture `size`
/// long in an area `room` long. A shorter picture is centred (both bounds
/// are the middle); a longer one lies within `room − size ..= 0`, so no gap
/// opens at either end.
fn place(pan: f64, size: f64, room: f64) -> f64 {
    let spare = room - size;
    let middle = spare / 2.0;
    pan.clamp(spare.min(middle), middle.max(0.0))
}

/// The fingers' midpoint.
fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// How far apart two points are.
fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Two fingers of a pinch: each one's pointer and its point on the area
/// (CSS px).
pub type Pair = [(i32, (f64, f64)); 2];

/// A pinch under way: its two pointers, the picture point under their
/// midpoint when it began, their distance then (at least 1 px) and the zoom
/// then.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pinch {
    pointers: (i32, i32),
    anchor: (f64, f64),
    distance: f64,
    zoom: f64,
}

/// The screen's view and the pinch that moves it (#71 PR F).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewer {
    view: View,
    pinch: Option<Pinch>,
}

impl Viewer {
    /// The whole picture and no pinch: a screen's start.
    pub const START: Self = Self {
        view: View::WHOLE,
        pinch: None,
    };

    /// The view shown.
    pub fn view(&self) -> View {
        self.view
    }

    /// The picture placed by the view ([`placed`]).
    pub fn placed(&self, area: (f64, f64), picture: (u32, u32)) -> Option<Fit> {
        placed(area, picture, self.view)
    }

    /// Follows the fingers, each frame. Two fingers new to it start a pinch
    /// from the view shown: the picture point under their midpoint, their
    /// distance and the zoom. The same two move it: the zoom is the start's
    /// times their distance over the start's (1 to [`MAX_ZOOM`]), and the
    /// start's picture point stays under their midpoint as far as the
    /// picture still covers the area ([`placed`]). None end it, and a pinch
    /// that ends under [`ZOOMED_FROM`] goes back to the whole picture.
    pub fn follow(&mut self, pair: Option<Pair>, area: (f64, f64), picture: (u32, u32)) {
        let Some([(first, a), (second, b)]) = pair else {
            if self.pinch.take().is_some() && !zoomed(self.view.zoom) {
                self.view = View::WHOLE;
            }
            return;
        };
        let (Some(fitted), Some(shown)) = (fit(area, picture), self.placed(area, picture)) else {
            return;
        };
        let middle = midpoint(a, b);
        match self.pinch {
            Some(pinch) if pinch.pointers == (first, second) => {
                let zoom = (pinch.zoom * distance(a, b) / pinch.distance).clamp(1.0, MAX_ZOOM);
                let scale = fitted.scale * zoom;
                let pan = (
                    middle.0 - pinch.anchor.0 * scale,
                    middle.1 - pinch.anchor.1 * scale,
                );
                if let Some(next) = placed(area, picture, View { zoom, pan }) {
                    self.view = View {
                        zoom,
                        pan: (next.left, next.top),
                    };
                }
            }
            _ => {
                self.pinch = Some(Pinch {
                    pointers: (first, second),
                    anchor: to_picture(shown, middle),
                    distance: distance(a, b).max(1.0),
                    zoom: self.view.zoom,
                });
            }
        }
    }

    /// Back to the whole picture (`CELÝ EQ`): fingers still pinching start
    /// a new pinch from there.
    pub fn whole(&mut self) {
        *self = Self::START;
    }

    /// A tap on the overview at picture point `at` (its pixels): [`TAP_ZOOM`]
    /// with that point in the area's middle, as far as the picture still
    /// covers the area ([`placed`], as a pinch keeps it); a pinch under way
    /// is forgotten. Nothing while the area or the picture is empty.
    pub fn zoom_on(&mut self, at: (f64, f64), area: (f64, f64), picture: (u32, u32)) {
        let Some(fitted) = fit(area, picture) else {
            return;
        };
        let scale = fitted.scale * TAP_ZOOM;
        let pan = (area.0 / 2.0 - at.0 * scale, area.1 / 2.0 - at.1 * scale);
        if let Some(next) = placed(
            area,
            picture,
            View {
                zoom: TAP_ZOOM,
                pan,
            },
        ) {
            *self = Self {
                view: View {
                    zoom: TAP_ZOOM,
                    pan: (next.left, next.top),
                },
                pinch: None,
            };
        }
    }
}

/// Whether the view counts as zoomed ([`ZOOMED_FROM`]).
pub fn zoomed(zoom: f64) -> bool {
    zoom >= ZOOMED_FROM
}

/// The zoom as the bar's factor reads it: one decimal, a decimal comma
/// (`2,4×`). Under [`EDIT_ZOOM_MIN`] it never reads `2,0×` (the PR I review:
/// a pinch to 1.97× read as editing while one finger did not edit).
pub fn zoom_text(zoom: f64) -> String {
    let shown = if edits(zoom) {
        zoom
    } else {
        zoom.min(EDIT_ZOOM_MIN - 0.1)
    };
    format!("{shown:.1}×").replace('.', ",")
}

/// How the screen draws a view (#71 PR F).
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The canvas's CSS transform. The canvas fills the area and fits the
    /// picture itself (`object-fit: contain`), so the view is its box
    /// scaled by the zoom from its corner (`transform-origin: 0 0`) and
    /// moved so the picture's corner lands where the view places it.
    pub transform: String,
    /// Whether the bar's zoom group shows ([`zoomed`]).
    pub zoomed: bool,
    /// Whether one finger edits ([`edits`]); else the overview's hint shows.
    pub edits: bool,
    /// The bar's factor ([`zoom_text`]).
    pub factor: String,
    /// The part of the picture in sight, as fractions of the picture (left,
    /// top, width, height): the overview's frame.
    pub seen: (f64, f64, f64, f64),
}

/// How the screen draws `view` of a picture in an area; none while either
/// is empty.
pub fn look(area: (f64, f64), picture: (u32, u32), view: View) -> Option<Look> {
    let fitted = fit(area, picture)?;
    let shown = placed(area, picture, view)?;
    let (width, height) = (f64::from(picture.0), f64::from(picture.1));
    let dx = shown.left - view.zoom * fitted.left;
    let dy = shown.top - view.zoom * fitted.top;
    Some(Look {
        transform: format!("translate({dx}px, {dy}px) scale({})", view.zoom),
        zoomed: zoomed(view.zoom),
        edits: edits(view.zoom),
        factor: zoom_text(view.zoom),
        seen: (
            (-shown.left / shown.scale).max(0.0) / width,
            (-shown.top / shown.scale).max(0.0) / height,
            (area.0 / shown.scale).min(width) / width,
            (area.1 / shown.scale).min(height) / height,
        ),
    })
}

/// The overview's drawing height (px of its canvas, PR F): its width
/// follows the picture's shape ([`overview_size`]).
pub const OVERVIEW_HEIGHT: u32 = 80;

/// The overview's shape (width over height) stays within this (#74 review):
/// Pro-Q's own and an upright phone's picture keep theirs; one past it is
/// bent no further into the bar.
pub const OVERVIEW_ASPECT: (f64, f64) = (0.4, 2.5);

/// The overview canvas's size for a picture of `picture` px (#74 review:
/// the picture's shape, so neither it nor the frame around the part in
/// sight is bent): [`OVERVIEW_HEIGHT`] high, the picture's aspect wide
/// (within [`OVERVIEW_ASPECT`]); a picture with no height counts as 1 px
/// high.
pub fn overview_size(picture: (u32, u32)) -> (u32, u32) {
    let aspect = f64::from(picture.0) / f64::from(picture.1.max(1));
    let (least, most) = OVERVIEW_ASPECT;
    let width = f64::from(OVERVIEW_HEIGHT) * aspect.clamp(least, most);
    (width.round() as u32, OVERVIEW_HEIGHT)
}

/// How long a first finger waits before its down reaches the editor (ms):
/// a second finger meanwhile makes a pinch, never a click in the editor.
pub const HOLD_MS: f64 = 120.0;

/// How far a waiting finger may move (CSS px) before its down goes at once.
pub const SLOP: f64 = 6.0;

/// The shortest contact the PC gets (ms, PR G): a quick tap's up goes on the
/// first frame this long after its down, never with it (the editor never
/// gets a contact of no time).
pub const TAP_MS: f64 = 40.0;

/// Whether a tap whose down went at `since` may lift at `now` (page ms).
fn tapped(since: f64, now: f64) -> bool {
    now - since >= TAP_MS
}

/// Whether a finger down since `since` waited its [`HOLD_MS`] at `now`
/// (page ms).
fn waited(since: f64, now: f64) -> bool {
    now - since >= HOLD_MS
}

/// Whether a finger moved from `first` to `at` (area px) further than
/// [`SLOP`].
fn slid(first: (f64, f64), at: (f64, f64)) -> bool {
    distance(first, at) > SLOP
}

/// What a finger sends: its phase and its point (picture pixels).
pub type Out = (Touch, f64, f64);

/// A finger's point: on the area (CSS px from its top-left: the slop and
/// the pinch) and on the picture (its pixels: what the editor gets).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub area: (f64, f64),
    pub picture: (f64, f64),
}

/// A point of the area and its picture pixel through `fit` (the view's).
pub fn point(fit: Fit, area: (f64, f64)) -> Point {
    Point {
        area,
        picture: to_picture(fit, area),
    }
}

/// `touch` at a point's picture pixel.
fn sent(touch: Touch, at: Point) -> Out {
    (touch, at.picture.0, at.picture.1)
}

/// Where the fingers are in their gesture.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Stage {
    /// No finger down.
    #[default]
    Idle,
    /// The first finger waits ([`HOLD_MS`], [`SLOP`]): its first point,
    /// whether that lies on the picture, and when it went down (page ms).
    Waiting { first: Point, on: bool, since: f64 },
    /// The first finger landed on the zoomed picture within the settle
    /// after a tap's zoom ([`ZOOM_SETTLE_MS`]): its first point and the
    /// settle's end (page ms). Lifted before it, nothing; still down then,
    /// its down at its first point and its move.
    Settling { first: Point, until: f64 },
    /// The first finger is on the editor (`moved`: a move not sent yet).
    Touching { moved: bool },
    /// The first finger on the overview (the view under [`EDIT_ZOOM_MIN`]):
    /// nothing reaches the editor; a lift within [`SLOP`] of where it landed
    /// zooms in there ([`Finger::take_zoom`]).
    Overview { first: Point },
    /// The first finger stays off the editor: it went down off the picture,
    /// or slid on the overview.
    Ignored,
    /// Two fingers pinch (one may have lifted since): nothing reaches the
    /// editor until every finger lifted.
    Pinching,
}

/// A finger down: its pointer, its type (#124: a primary down of that type
/// ends it, [`Finger::missed`]) and its last point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    pointer: i32,
    kind: Option<Pointer>,
    at: Point,
}

/// The fingers a primary down ended (#124, [`Finger::missed`]): each one's
/// pointer (the flight recorder's `lost`), and what goes to the editor for
/// them (a cancel at its last point for a first finger on it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Missed {
    pub pointers: Vec<i32>,
    pub out: Vec<Out>,
}

/// The fingers on the picture (#71 PR E, PR F): the first one reaches the
/// editor once it waited or slid, two of them pinch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Finger {
    stage: Stage,
    /// The fingers down, at most two: each one's pointer, its type and its
    /// last point.
    fingers: [Option<Held>; 2],
    /// A quick tap's up still to go (PR G): its point, and when its down
    /// went (page ms).
    tap: Option<(Point, f64)>,
    /// A quick tap that lifted while another tap's up was still to go: its
    /// down's point and its up's. Its down goes once that up went (#74
    /// review: never a second down before the first tap's up).
    next: Option<(Point, Point)>,
    /// The picture point an overview tap zooms in on, until the frame loop
    /// takes it.
    zoom_on: Option<(f64, f64)>,
    /// When the frame loop took the last tap's zoom (page ms): a first
    /// finger reaches nothing for [`ZOOM_SETTLE_MS`] after it.
    zoomed: Option<f64>,
}

impl Finger {
    /// Where `pointer` is among the fingers.
    fn slot(&self, pointer: i32) -> Option<usize> {
        self.fingers
            .iter()
            .position(|f| f.is_some_and(|held| held.pointer == pointer))
    }

    /// Whether `pointer` is one of the fingers (the glue captures it).
    pub fn has(&self, pointer: i32) -> bool {
        self.slot(pointer).is_some()
    }

    /// The fingers whose end the browser never delivered (#124: iOS took a
    /// finger for a system gesture and sent no `pointerup`, `pointercancel`
    /// or `lostpointercapture`), ended by `down` before its own
    /// [`Finger::down`]: a primary pointer of their type ([`Down::ends`]:
    /// no other pointer of that type is down) ends every finger of that
    /// type, each one as its cancel. A first finger on the editor gets its
    /// cancel at its last point (the PC's contact ends, no band stays
    /// grabbed); one waiting, settling, on the overview or off the picture
    /// never reached it, and a pinch's contact was cancelled when the pinch
    /// began: nothing goes for them. The new finger then starts a gesture
    /// of its own, never a pinch with a finger that left. A second finger,
    /// or a pointer of another type, ends nothing (#124 M1).
    pub fn missed(&mut self, down: Down) -> Missed {
        let mut missed = Missed::default();
        let fingers = self.fingers;
        for held in fingers.into_iter().flatten() {
            if down.ends(held.kind) {
                missed.out.extend(self.cancel(held.pointer));
                missed.pointers.push(held.pointer);
            }
        }
        missed
    }

    /// A pointer's down (`down`: its id and type) at `at` where it lands
    /// (`spot`: off the picture, on the overview, on the zoomed picture;
    /// `now`: page ms). A first finger on the zoomed picture waits: nothing
    /// goes yet, until the settle's end after a tap's zoom ([`settling`]);
    /// one on the overview never reaches the editor. A second one makes a
    /// pinch: a first finger already on the editor gets its cancel at its
    /// last point. A third one, or a pointer already down, is nothing.
    pub fn down(&mut self, down: Down, at: Point, spot: Spot, now: f64) -> Vec<Out> {
        if self.has(down.id) {
            return Vec::new();
        }
        let held = Held {
            pointer: down.id,
            kind: down.kind,
            at,
        };
        match self.stage {
            Stage::Idle => {
                let stage = match (spot, settling(self.zoomed, now)) {
                    (Spot::Overview, _) => Stage::Overview { first: at },
                    (Spot::Edit, true) => Stage::Settling {
                        first: at,
                        until: self.zoomed.map_or(now, |at| at + ZOOM_SETTLE_MS),
                    },
                    (Spot::Edit | Spot::Off, _) => Stage::Waiting {
                        first: at,
                        on: spot == Spot::Edit,
                        since: now,
                    },
                };
                *self = Self {
                    stage,
                    fingers: [Some(held), None],
                    tap: self.tap,
                    next: self.next,
                    zoom_on: self.zoom_on,
                    zoomed: self.zoomed,
                };
                Vec::new()
            }
            Stage::Touching { .. } => {
                let ended = self.last(Touch::Cancel);
                self.join(held);
                ended
            }
            Stage::Waiting { .. }
            | Stage::Settling { .. }
            | Stage::Overview { .. }
            | Stage::Ignored
            | Stage::Pinching => {
                self.join(held);
                Vec::new()
            }
        }
    }

    /// A finger joins the pinch in a free place (a third one finds none).
    fn join(&mut self, held: Held) {
        if let Some(free) = self.fingers.iter_mut().find(|f| f.is_none()) {
            *free = Some(held);
            self.stage = Stage::Pinching;
        }
    }

    /// The first finger's last point as `touch`.
    fn last(&self, touch: Touch) -> Vec<Out> {
        self.fingers[0]
            .map(|held| vec![sent(touch, held.at)])
            .unwrap_or_default()
    }

    /// A finger leaves: its place is free, and the gesture ends with the
    /// last one.
    fn lift(&mut self, slot: usize) {
        self.fingers[slot] = None;
        if !self.held() {
            self.stage = Stage::Idle;
        }
    }

    /// A pointer moved to `at` (a pointer that is no finger is nothing):
    /// the first finger's move goes at the next frame, the newest one only;
    /// a pinch's fingers move the view ([`Finger::pair`]).
    pub fn moved(&mut self, pointer: i32, at: Point) {
        let Some(slot) = self.slot(pointer) else {
            return;
        };
        if let Some(held) = self.fingers[slot].as_mut() {
            held.at = at;
        }
        if let Stage::Touching { moved } = &mut self.stage {
            *moved = true;
        }
        // A finger that slides on the overview is no tap: nothing.
        if let Stage::Overview { first } = self.stage
            && slid(first.area, at.area)
        {
            self.stage = Stage::Ignored;
        }
    }

    /// The picture point an overview tap asked to zoom in on, once (the
    /// frame loop's [`Viewer::zoom_on`], at `now`: page ms); the settle
    /// counts from here.
    pub fn take_zoom(&mut self, now: f64) -> Option<(f64, f64)> {
        let at = self.zoom_on.take();
        if at.is_some() {
            self.zoomed = Some(now);
        }
        at
    }

    /// A quick tap's up still to go, now (PR G: when the fingers leave).
    fn tap_up(&mut self) -> Option<Out> {
        self.tap.take().map(|(at, _)| sent(Touch::Up, at))
    }

    /// A quick tap's up once [`TAP_MS`] passed since its down at `now`, and
    /// then the down of a tap waiting for it (its own up [`TAP_MS`] later):
    /// the PC never gets a contact shorter than that, nor a down before the
    /// last tap's up.
    fn tap_due(&mut self, now: f64) -> Vec<Out> {
        if !self.tap.is_some_and(|(_, since)| tapped(since, now)) {
            return Vec::new();
        }
        let mut out: Vec<Out> = self.tap_up().into_iter().collect();
        if let Some((down, up)) = self.next.take() {
            out.push(sent(Touch::Down, down));
            self.tap = Some((up, now));
        }
        out
    }

    /// The frame (`now`: page ms). A quick tap's up goes once [`TAP_MS`]
    /// passed since its down, and a tap waiting for it goes down then
    /// ([`Finger::tap_due`]). A waiting finger that waited [`HOLD_MS`] or
    /// slid past [`SLOP`] reaches the editor once no tap's up is still to go
    /// (#74 review): its down at its first point, then its move to where it
    /// is now (nothing when it went down off the picture); so does a
    /// settling finger at the settle's end. A finger on the editor sends the
    /// move not sent yet, the newest.
    pub fn frame(&mut self, now: f64) -> Vec<Out> {
        let mut out = self.tap_due(now);
        let free = self.tap.is_none();
        match (self.stage, self.fingers[0].map(|held| held.at)) {
            (Stage::Waiting { first, on, since }, Some(at))
                if free && (waited(since, now) || slid(first.area, at.area)) =>
            {
                if !on {
                    self.stage = Stage::Ignored;
                    return out;
                }
                self.stage = Stage::Touching { moved: false };
                out.push(sent(Touch::Down, first));
                if at != first {
                    out.push(sent(Touch::Move, at));
                }
            }
            (Stage::Settling { first, until }, Some(at)) if free && settled(until, now) => {
                self.stage = Stage::Touching { moved: false };
                out.push(sent(Touch::Down, first));
                if at != first {
                    out.push(sent(Touch::Move, at));
                }
            }
            (Stage::Touching { moved: true }, Some(at)) => {
                self.stage = Stage::Touching { moved: false };
                out.push(sent(Touch::Move, at));
            }
            _ => {}
        }
        out
    }

    /// A pointer lifts at `at` (`now`: page ms): a waiting finger's tap
    /// (PR G: its down at its first point at once, its up here on the first
    /// frame [`TAP_MS`] later; a tap's up still to go first: one due goes
    /// now, one not due yet keeps this tap waiting for its frame, #74
    /// review, and a third lift meanwhile is dropped), the first finger's up
    /// here; a pinch's fingers send nothing.
    pub fn up(&mut self, pointer: i32, at: Point, now: f64) -> Vec<Out> {
        let Some(slot) = self.slot(pointer) else {
            return Vec::new();
        };
        let stage = self.stage;
        let out = match stage {
            Stage::Waiting {
                first, on: true, ..
            } => {
                let mut out = self.tap_due(now);
                if self.tap.is_none() {
                    out.push(sent(Touch::Down, first));
                    self.tap = Some((at, now));
                } else if self.next.is_none() {
                    self.next = Some((first, at));
                }
                out
            }
            Stage::Touching { .. } => vec![sent(Touch::Up, at)],
            // A tap on the overview zooms in where it landed; the editor
            // gets nothing.
            Stage::Overview { first } => {
                if !slid(first.area, at.area) {
                    self.zoom_on = Some(first.picture);
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        self.lift(slot);
        out
    }

    /// A pointer's touch is cancelled (or its capture lost): the first
    /// finger on the editor gets a cancel where it was; a waiting one never
    /// reached it, so nothing goes.
    pub fn cancel(&mut self, pointer: i32) -> Vec<Out> {
        let Some(slot) = self.slot(pointer) else {
            return Vec::new();
        };
        let out = if matches!(self.stage, Stage::Touching { .. }) {
            self.last(Touch::Cancel)
        } else {
            Vec::new()
        };
        self.lift(slot);
        out
    }

    /// The screen goes away under the fingers: a quick tap's up still to
    /// go goes now (a tap waiting for it never went: nothing of it goes), a
    /// cancel for a finger on the editor; every finger is forgotten.
    pub fn leave(&mut self) -> Vec<Out> {
        let mut out: Vec<Out> = self.tap_up().into_iter().collect();
        if matches!(self.stage, Stage::Touching { .. }) {
            out.extend(self.last(Touch::Cancel));
        }
        *self = Self::default();
        out
    }

    /// Whether a quick tap's up is still to go (PR J re-review: a finger
    /// landing then lands on a picture that cannot show that up yet).
    pub fn tap_pending(&self) -> bool {
        self.tap.is_some()
    }

    /// The pointer of a first finger on the editor or on its way there
    /// ([`Finger::on_editor`]).
    pub fn editor_pointer(&self) -> Option<i32> {
        self.fingers[0]
            .map(|held| held.pointer)
            .filter(|_| self.on_editor())
    }

    /// Whether a first finger is on the editor or on its way there
    /// (touching, waiting on the picture, settling): what [`Finger::off_editor`]
    /// would end.
    pub fn on_editor(&self) -> bool {
        matches!(
            self.stage,
            Stage::Touching { .. } | Stage::Waiting { on: true, .. } | Stage::Settling { .. }
        )
    }

    /// The fingers lose the editor under them: the view goes back to the
    /// whole picture (`CELÝ EQ`, the PR I review: its moves would map
    /// through another view), or the link drops out mid-drag (PR J review:
    /// the moves queued in a stalled socket would come in a burst). A finger
    /// on the editor ends with its cancel where it was, and one still
    /// waiting or settling never reaches it; a pinch's fingers go on
    /// pinching, and a quick tap's up still goes.
    pub fn off_editor(&mut self) -> Vec<Out> {
        match self.stage {
            Stage::Touching { .. } => {
                let out = self.last(Touch::Cancel);
                self.stage = Stage::Ignored;
                out
            }
            Stage::Waiting { .. } | Stage::Settling { .. } => {
                self.stage = Stage::Ignored;
                Vec::new()
            }
            Stage::Idle | Stage::Overview { .. } | Stage::Ignored | Stage::Pinching => Vec::new(),
        }
    }

    /// The page went hidden (`hidden`) or was shown again: a hidden page
    /// lifts its fingers (a cancel for one on the editor), as the Stream
    /// Deck's keys go up. A hidden page still pings, so the hub's 2 s
    /// silence would never end the PC's contact.
    pub fn visibility(&mut self, hidden: bool) -> Vec<Out> {
        if hidden { self.leave() } else { Vec::new() }
    }

    /// Whether a finger is down.
    pub fn held(&self) -> bool {
        self.fingers.iter().any(Option::is_some)
    }

    /// Whether a finger is on the picture or a quick tap's up is still to
    /// go (the PC's contact is not over yet): the bar's undo takes no press
    /// then (#71 PR P2b, [`undo_ready`]).
    pub fn busy(&self) -> bool {
        self.held() || self.tap_pending()
    }

    /// The two fingers of a pinch while both are down (in their places'
    /// order), on the area: what moves the view ([`Viewer::follow`]).
    pub fn pair(&self) -> Option<Pair> {
        match self.fingers {
            [Some(a), Some(b)] => Some([(a.pointer, a.at.area), (b.pointer, b.at.area)]),
            _ => None,
        }
    }
}

/// How far a picture area's side must change (CSS px, PR G) for the hub to
/// hear it: a sub-pixel layout wobble is no new shape.
pub const AREA_STEP: f64 = 1.0;

/// How long a changed picture area rests before the hub hears it (ms, PR
/// G): a phone turning or a window being resized changes it many times,
/// and the editor is resized once, for the last.
pub const AREA_SETTLE_MS: f64 = 300.0;

/// Whether two areas differ by [`AREA_STEP`] or more on a side.
fn reshaped(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() >= AREA_STEP || (a.1 - b.1).abs() >= AREA_STEP
}

/// Whether an area changed at `since` rested its [`AREA_SETTLE_MS`] at
/// `now` (page ms).
fn rested(since: f64, now: f64) -> bool {
    now - since >= AREA_SETTLE_MS
}

/// The page's picture area as the hub knows it (PR G): the one it last
/// sent (`eq_open`'s, then `eq_area`'s) and a changed one resting, looked
/// at each frame (no timer).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AreaWatch {
    sent: Option<(f64, f64)>,
    /// The newest area and when it changed (page ms).
    resting: Option<((f64, f64), f64)>,
}

impl AreaWatch {
    /// The area an open carried: the hub has it.
    pub fn opened(area: (f64, f64)) -> Self {
        Self {
            sent: Some(area),
            resting: None,
        }
    }

    /// The frame's look at the area (`area`: CSS px; `open`: the page's
    /// editor is open; `now`: page ms): an area that changed since the one
    /// sent, by [`AREA_STEP`] or more, goes once it rested
    /// [`AREA_SETTLE_MS`] while the editor is open (each change starts the
    /// rest again; one back where it was sends nothing).
    pub fn frame(&mut self, area: (f64, f64), open: bool, now: f64) -> Option<(f64, f64)> {
        let newest = self.resting.map(|(at, _)| at).or(self.sent);
        if newest.is_none_or(|at| reshaped(at, area)) {
            self.resting = Some((area, now));
        }
        let (at, since) = self.resting?;
        if !(open && rested(since, now)) {
            return None;
        }
        self.resting = None;
        let news = self.sent.is_none_or(|sent| reshaped(sent, at));
        news.then(|| {
            self.sent = Some(at);
            at
        })
    }
}

/// A card's editor as the locks show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CardLock {
    Free,
    /// This page holds it.
    Mine,
    /// Another page holds it, since the hub's UTC ms.
    Other(f64),
    /// Another page holds another editor, since the hub's UTC ms: one
    /// Pro-Q 4 on the PC's screen at a time.
    InUse(f64),
}

/// The lock of the editor at `path` on `instance`: its own lock, else
/// another page's editor anywhere (every card of every instance is locked
/// then; this page's own editor elsewhere leaves it free: its open
/// switches).
pub fn card_lock(locks: &[EqLock], instance: &str, path: &str) -> CardLock {
    let own = locks
        .iter()
        .find(|lock| lock.instance == instance && lock.path == path);
    match own {
        Some(lock) if lock.mine => CardLock::Mine,
        Some(lock) => CardLock::Other(lock.since),
        None => locks
            .iter()
            .find(|lock| !lock.mine)
            .map_or(CardLock::Free, |lock| CardLock::InUse(lock.since)),
    }
}

/// Whether another page's editor locks the card (ZAMKNUTÉ): its own, or
/// any other.
pub fn locked(lock: CardLock) -> bool {
    matches!(lock, CardLock::Other(_) | CardLock::InUse(_))
}

/// Whether a card's down opens its editor (`can_send`: the page's socket
/// takes a message now): not while another page holds an editor
/// (ZAMKNUTÉ), and only while the socket can take the open, as a Stream
/// Deck press (`behave::deck::can_press`): an open the socket drops would
/// leave the screen waiting on "Otváram Pro-Q 4…" for nothing.
pub fn can_open(lock: CardLock, can_send: bool) -> bool {
    can_send && !locked(lock)
}

/// What the cards' list waits on: the page's socket past its hello, and the
/// strip's instance online.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListLink {
    pub connected: bool,
    pub online: bool,
}

/// Whether the cards ask the hub for the strip's list now (`before`: the
/// link the last time they looked; none when the detail opens): at the open
/// while connected (an offline instance's list then says so), and each time
/// the page is connected with the instance online after either was not: a
/// reconnect (once the hub's instance states follow its hello) or Live back
/// online, so a list that failed while Live was away is read again.
pub fn lists_now(before: Option<ListLink>, now: ListLink) -> bool {
    let up = |link: ListLink| link.connected && link.online;
    match before {
        None => now.connected,
        Some(before) => up(now) && !up(before),
    }
}

/// What names an editor on its card and on its screen: the product, where
/// it sits (`na tracku`, or its racks and chains), and the device's name
/// when it was renamed (`Pro-Q 4 · Vocal FX › Main · De-ess`).
pub fn place_text(place: &str, name: &str) -> String {
    if name == PRODUCT {
        format!("{PRODUCT} · {place}")
    } else {
        format!("{PRODUCT} · {place} · {name}")
    }
}

/// `HH:MM` of a clock's hours and minutes.
pub fn hh_mm(hours: u32, minutes: u32) -> String {
    format!("{hours:02}:{minutes:02}")
}

/// A locked card's line: who holds it and since when (`HH:MM`).
pub fn locked_text(since: &str) -> String {
    format!("Upravuje ho iný zvukár (od {since})")
}

/// The line of a card locked by another page's editor elsewhere, since
/// when (`HH:MM`).
pub fn in_use_text(since: &str) -> String {
    format!("Pro-Q 4 práve používa iný zvukár (od {since})")
}

/// Whether the hub refused an open for a lock (`locked`, `in use`): the
/// screen's `eq_locked` step.
pub fn lock_refusal(why: &str) -> bool {
    why == reason::LOCKED || why == reason::IN_USE
}

/// Whether the screen showing the session `open` (none: not open yet)
/// draws a frame of `session`.
pub fn shows_frame(session: u32, open: Option<u32>) -> bool {
    open == Some(session)
}

/// A URL query's component: every byte but the unreserved ones (letters,
/// digits, `-`, `.`, `_`, `~`) percent-encoded, as `encodeURIComponent`.
pub fn encode_component(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The URL of an editor's last picture (`GET /api/eq/picture`).
pub fn picture_url(instance: &str, path: &str) -> String {
    format!(
        "/api/eq/picture?instance={}&path={}",
        encode_component(instance),
        encode_component(path)
    )
}

/// What the screen's place in the detail says when the PC's hub has it off
/// (`[eq] backend = "off"`, the default on Windows until the PC's check
/// passed): a plain note, never an error.
pub const OFF_TEXT: &str = "EQ je na PC vypnuté.";

/// The cards' note when the hub gave no list (`error`, its reason): the
/// screen off, or why the track's devices could not be read, in Slovak
/// through the same table as a card's note ([`failure_text`]); only an
/// unexpected reason (the script's own words) shows the hub's text.
pub fn list_text(error: &str) -> String {
    if error == reason::OFF {
        return OFF_TEXT.to_string();
    }
    format!(
        "Pro-Q 4 sa nedá prečítať: {}",
        reason_text(error).unwrap_or(error)
    )
}

/// A reason the protocol names (`fohmixer_proto::eq::reason`) as its
/// Slovak sentence; none for any other text.
fn reason_text(why: &str) -> Option<&'static str> {
    let text = match why {
        reason::OFF => OFF_TEXT,
        reason::CLOSING => "ešte sa zatvára, skús znova",
        reason::UNKNOWN => "zoznam je starý, otvor kanál znova",
        reason::MOVED => "Pro-Q 4 sa presunul, otvor kanál znova",
        reason::OPEN_ON_PC => "Pro-Q 4 je otvorený priamo na PC, zatvor ho tam",
        reason::UNREAD => "Pro-Q 4 sa nedá prečítať, skús znova",
        reason::NO_WINDOW => "okno Pro-Q 4 sa neotvorilo",
        reason::GONE => "okno Pro-Q 4 sa zavrelo",
        reason::OFFLINE => "Live je nedostupný, skús znova",
        reason::NO_ANSWER => "Live neodpovedá, skús znova",
        reason::UNKNOWN_INSTANCE => "neznámy Live",
        reason::SEVERAL => "otvorilo sa viac okien, skús znova",
        reason::NO_PICTURE => "okno nie je Pro-Q 4",
        reason::NOT_ON_TOP => "okno Pro-Q 4 sa nedostalo navrch, skús znova",
        reason::STOPPED => "hub sa zastavuje",
        reason::LEFT_OPEN => "Pro-Q 4 sa nepodarilo bezpečne zavrieť, ostáva otvorený na PC",
        reason::TOO_DEEP => "zariadenia sú vnorené príliš hlboko",
        reason::HOLD_ON => "na PC sa nedá vypnúť podržanie prsta, Pro-Q 4 sa preto neotvorí",
        // An undo's refusals (#71 PR P2a); the bar's note says the last
        // three its own way (`undo_text`).
        reason::NO_EDITOR => "Pro-Q 4 už nie je otvorený",
        reason::UNDO_BUSY => "Pro-Q 4 je práve zaneprázdnený, skús znova",
        reason::UNDO_SOON => "príliš rýchlo po poslednom vrátení, skús znova",
        reason::UNDO_MISSING => "šípka späť sa v Pro-Q 4 nenašla",
        _ => return None,
    };
    Some(text)
}

/// The line under a card after its editor did not open or closed (`why`,
/// the hub's reason), in Slovak: none for a close the page or the hub made
/// on purpose (`exit`, `switch`, `detach`, the page's own `socket`) and for
/// a lock, `locked` or `in use` (the card already reads ZAMKNUTÉ with its
/// line); each reason the protocol
/// names (`fohmixer_proto::eq::reason`) has its sentence ([`reason_text`]);
/// only an unexpected failure shows the hub's own words.
pub fn failure_text(why: &str) -> Option<String> {
    match why {
        reason::EXIT
        | reason::SWITCH
        | reason::DETACH
        | "socket"
        | reason::LOCKED
        | reason::IN_USE => None,
        other => Some(
            reason_text(other).map_or_else(|| format!("Neotvoril sa: {other}"), str::to_string),
        ),
    }
}

/// How long the bar's ↶ VRÁTIŤ flashes when a press goes (ms, #71 PR P2b,
/// the approved mockup `docs/mockups/proq-undo-v1.html`).
pub const UNDO_FLASH_MS: f64 = 180.0;

/// How long the note of an undo's answer shows over the picture (ms).
pub const UNDO_NOTE_MS: f64 = 1600.0;

/// Whether the bar's ↶ VRÁTIŤ takes a press at `now` (page ms; #71 PR P2b):
/// not while the fingers keep a contact on the PC (`busy`, [`Finger::busy`]:
/// the hub would refuse it, `UNDO_BUSY`), nor within
/// [`UNDO_APART_MS`] of the last press it took (`pressed`; none yet): two
/// taps on Pro-Q's own ↶ never make a double click. The button reads
/// `aria-disabled` whenever this says no.
pub fn undo_ready(busy: bool, pressed: Option<f64>, now: f64) -> bool {
    !busy && pressed.is_none_or(|at| now - at >= UNDO_APART_MS)
}

/// Whether the ↶ VRÁTIŤ pressed at `pressed` (page ms) still flashes at
/// `now`.
pub fn undo_flash(pressed: Option<f64>, now: f64) -> bool {
    pressed.is_some_and(|at| now - at < UNDO_FLASH_MS)
}

/// Whether the note of an undo answered at `at` (page ms; none: no answer
/// yet) still shows at `now`.
pub fn undo_note_shown(at: Option<f64>, now: f64) -> bool {
    at.is_some_and(|at| now - at < UNDO_NOTE_MS)
}

/// The note over the picture for an undo's answer (`result`, the hub's
/// `eq_undo`): done, nothing to undo, and the three refusals the owner can
/// act on, each its own sentence; any other reason after "Vrátiť sa
/// nepodarilo:" in Slovak through [`reason_text`], the hub's own words only
/// for an unexpected one (a grab's or a tap's error).
pub fn undo_text(result: &str) -> String {
    match result {
        UNDO_DONE => "Vrátené: posledná zmena v Pro-Q".to_string(),
        UNDO_NOTHING => "Nie je čo vrátiť".to_string(),
        reason::UNDO_MISSING => "Späť sa v Pro-Q nenašlo — nič sa nestlačilo".to_string(),
        reason::UNDO_BUSY => "Prst je na obraze — vráť po zdvihnutí prsta".to_string(),
        reason::UNDO_SOON => "Príliš rýchlo za sebou — stlač znova".to_string(),
        other => format!(
            "Vrátiť sa nepodarilo: {}",
            reason_text(other).unwrap_or(other)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::perf::Pointer;

    #[test]
    fn a_picture_fits_whole_and_centred() {
        // Wider than the area: the width decides, bands above and below.
        assert_eq!(
            fit((1000.0, 1000.0), (2000, 1000)),
            Some(Fit {
                scale: 0.5,
                left: 0.0,
                top: 250.0
            })
        );
        // Taller: the height decides, bands left and right.
        assert_eq!(
            fit((1000.0, 500.0), (500, 500)),
            Some(Fit {
                scale: 1.0,
                left: 250.0,
                top: 0.0
            })
        );
        assert_eq!(fit((0.0, 10.0), (10, 10)), None);
        assert_eq!(fit((10.0, 0.0), (10, 10)), None);
        assert_eq!(fit((10.0, 10.0), (0, 10)), None);
        assert_eq!(fit((10.0, 10.0), (10, 0)), None);
        assert!(fit((1.0, 1.0), (1, 1)).is_some());
    }

    #[test]
    fn a_point_of_the_screen_is_a_point_of_the_picture() {
        let fit = fit((1000.0, 1000.0), (2000, 1000)).unwrap();
        assert_eq!(to_picture(fit, (0.0, 250.0)), (0.0, 0.0));
        assert_eq!(to_picture(fit, (500.0, 500.0)), (1000.0, 500.0));
        assert_eq!(to_picture(fit, (10.0, 260.5)), (20.0, 21.0));
        // Taller: bands left and right, the left one counts.
        let tall = super::fit((1000.0, 1000.0), (1000, 2000)).unwrap();
        assert_eq!(to_picture(tall, (250.0, 0.0)), (0.0, 0.0));
        assert_eq!(to_picture(tall, (260.0, 5.0)), (20.0, 10.0));
        assert!(on_picture((0.0, 0.0), (2000, 1000)));
        assert!(on_picture((1999.9, 999.9), (2000, 1000)));
        assert!(!on_picture((2000.0, 10.0), (2000, 1000)));
        assert!(!on_picture((10.0, 1000.0), (2000, 1000)));
        assert!(!on_picture((-0.1, 10.0), (2000, 1000)));
        assert!(!on_picture((10.0, -0.1), (2000, 1000)));
    }

    /// A point whose picture pixel is twice its area point: a test sees
    /// which one a finger uses.
    fn p(x: f64, y: f64) -> Point {
        Point {
            area: (x, y),
            picture: (x * 2.0, y * 2.0),
        }
    }

    /// A first finger's down: a touch, primary (no other finger is down).
    fn first(id: i32) -> Down {
        Down::touch(id, true)
    }

    /// A second finger's down: a touch, not primary (another one is down).
    fn second(id: i32) -> Down {
        Down::touch(id, false)
    }

    /// A finger on the editor: down at `at` at 0 ms, the down sent at the
    /// hold.
    fn touching(finger: &mut Finger, pointer: i32, at: Point) {
        assert_eq!(finger.down(first(pointer), at, Spot::Edit, 0.0), vec![]);
        assert_eq!(
            finger.frame(HOLD_MS),
            vec![(Touch::Down, at.picture.0, at.picture.1)]
        );
    }

    #[test]
    fn one_finger_moves_once_a_frame_and_always_ends() {
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(5.0, 6.0));
        assert!(finger.held());
        assert!(finger.has(1));
        assert!(!finger.has(2));
        assert_eq!(finger.frame(200.0), vec![], "nothing moved");
        finger.moved(2, p(9.0, 9.0));
        assert_eq!(finger.frame(201.0), vec![], "another pointer's move");
        finger.moved(1, p(7.0, 6.0));
        finger.moved(1, p(8.0, 6.0));
        assert_eq!(
            finger.frame(202.0),
            vec![(Touch::Move, 16.0, 12.0)],
            "the newest"
        );
        assert_eq!(finger.frame(203.0), vec![], "sent once");
        assert_eq!(
            finger.up(2, p(1.0, 1.0), 0.0),
            vec![],
            "another pointer's up"
        );
        finger.moved(1, p(9.0, 6.0));
        assert_eq!(
            finger.up(1, p(10.0, 6.0), 0.0),
            vec![(Touch::Up, 20.0, 12.0)]
        );
        assert!(!finger.held());
        assert_eq!(
            finger.frame(204.0),
            vec![],
            "a move before the up is not sent after it"
        );
        assert_eq!(finger.cancel(1), vec![], "its lost capture after the up");
        assert_eq!(finger.leave(), vec![]);
        // A cancel ends it where it was.
        touching(&mut finger, 3, p(1.0, 2.0));
        finger.moved(3, p(4.0, 5.0));
        assert_eq!(finger.cancel(4), vec![]);
        assert_eq!(finger.cancel(3), vec![(Touch::Cancel, 8.0, 10.0)]);
        assert!(!finger.held());
        assert_eq!(finger.frame(300.0), vec![], "its move is not sent after");
        // Leaving the screen with a finger on the editor cancels it.
        touching(&mut finger, 5, p(1.0, 1.0));
        assert_eq!(finger.leave(), vec![(Touch::Cancel, 2.0, 2.0)]);
        assert!(!finger.held());
        assert_eq!(finger.leave(), vec![]);
        touching(&mut finger, 6, p(2.0, 2.0));
        // A down of a pointer already down is nothing.
        assert_eq!(
            finger.down(second(6), p(3.0, 3.0), Spot::Edit, 400.0),
            vec![]
        );
        assert_eq!(finger.pair(), None);
        assert_eq!(finger.frame(401.0), vec![]);
    }

    #[test]
    fn a_first_finger_waits_then_goes_down_where_it_landed() {
        let mut finger = Finger::default();
        assert_eq!(
            finger.down(first(1), p(10.0, 20.0), Spot::Edit, 1000.0),
            vec![]
        );
        assert!(finger.held());
        assert!(finger.has(1));
        assert_eq!(finger.frame(1119.0), vec![], "it waits");
        // A move within the slop (4 px on the area, 8 on the picture: the
        // slop is the area's).
        finger.moved(1, p(14.0, 20.0));
        assert_eq!(finger.frame(1120.0_f64.next_down()), vec![], "a hair short");
        assert_eq!(
            finger.frame(1120.0),
            vec![(Touch::Down, 20.0, 40.0), (Touch::Move, 28.0, 40.0)],
            "the hold: its down where it landed, then its move"
        );
        assert_eq!(finger.frame(1121.0), vec![], "sent once");
        finger.moved(1, p(15.0, 20.0));
        assert_eq!(finger.frame(1122.0), vec![(Touch::Move, 30.0, 40.0)]);
        assert_eq!(
            finger.up(1, p(16.0, 21.0), 0.0),
            vec![(Touch::Up, 32.0, 42.0)]
        );
        // A finger that rests sends only its down.
        assert_eq!(
            finger.down(first(2), p(10.0, 20.0), Spot::Edit, 2000.0),
            vec![]
        );
        assert_eq!(finger.frame(2120.0), vec![(Touch::Down, 20.0, 40.0)]);
        assert_eq!(finger.frame(2500.0), vec![]);
        assert_eq!(
            finger.up(2, p(10.0, 20.0), 0.0),
            vec![(Touch::Up, 20.0, 40.0)]
        );
    }

    #[test]
    fn a_finger_that_slides_past_the_slop_goes_down_at_once() {
        let mut finger = Finger::default();
        assert_eq!(
            finger.down(first(1), p(10.0, 20.0), Spot::Edit, 0.0),
            vec![]
        );
        finger.moved(1, p(16.0, 20.0));
        assert_eq!(finger.frame(1.0), vec![], "6 px: still waiting");
        let past = 16.0_f64.next_up();
        finger.moved(1, p(past, 20.0));
        assert_eq!(
            finger.frame(2.0),
            vec![(Touch::Down, 20.0, 40.0), (Touch::Move, past * 2.0, 40.0)],
            "past 6 px: its down where it landed, then its move"
        );
        assert_eq!(finger.frame(3.0), vec![]);
        // Upwards and to the left counts the same.
        let mut finger = Finger::default();
        finger.down(first(2), p(10.0, 20.0), Spot::Edit, 0.0);
        finger.moved(2, p(10.0, 13.0));
        assert_eq!(
            finger.frame(1.0),
            vec![(Touch::Down, 20.0, 40.0), (Touch::Move, 20.0, 26.0)]
        );
        assert!(slid((10.0, 20.0), (3.0, 20.0)));
        assert!(!slid((10.0, 20.0), (4.0, 20.0)));
        assert!(!slid((10.0, 20.0), (10.0, 26.0)));
        assert!(slid((10.0, 20.0), (10.0, 26.0_f64.next_up())));
    }

    #[test]
    fn a_hold_counts_from_the_fingers_down() {
        assert!(waited(1000.0, 1120.0));
        assert!(!waited(1000.0, 1120.0_f64.next_down()));
        assert!(waited(1000.0, 5000.0));
        assert!(!waited(1000.0, 1000.0));
        assert!(!waited(1000.0, 999.0));
        assert_eq!(HOLD_MS, 120.0);
        assert_eq!(SLOP, 6.0);
    }

    #[test]
    fn a_quick_lift_is_a_tap_its_down_at_once_its_up_40_ms_later() {
        assert_eq!(TAP_MS, 40.0);
        assert!(tapped(1000.0, 1040.0));
        assert!(!tapped(1000.0, 1040.0_f64.next_down()));
        assert!(tapped(1000.0, 5000.0));
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Edit, 0.0);
        finger.moved(1, p(12.0, 21.0));
        assert_eq!(
            finger.up(1, p(12.0, 21.0), 50.0),
            vec![(Touch::Down, 20.0, 40.0)],
            "its down at once, where it landed"
        );
        assert!(!finger.held());
        assert_eq!(finger.frame(89.0), vec![], "under 40 ms: the PC holds it");
        assert_eq!(
            finger.frame(90.0),
            vec![(Touch::Up, 24.0, 42.0)],
            "its up where it lifted, 40 ms after its down"
        );
        assert_eq!(finger.frame(500.0), vec![], "nothing after");
        // A finger reaching the editor waits for a tap's up still to go:
        // that up first, 40 ms after its down, then the finger's down.
        finger.down(first(2), p(10.0, 20.0), Spot::Edit, 600.0);
        assert_eq!(
            finger.up(2, p(10.0, 20.0), 610.0),
            vec![(Touch::Down, 20.0, 40.0)]
        );
        finger.down(first(3), p(30.0, 20.0), Spot::Edit, 615.0);
        finger.moved(3, p(40.0, 20.0));
        assert_eq!(finger.frame(620.0), vec![], "slid, the tap's up not due");
        assert_eq!(finger.frame(650.0_f64.next_down()), vec![]);
        assert_eq!(
            finger.frame(650.0),
            vec![
                (Touch::Up, 20.0, 40.0),
                (Touch::Down, 60.0, 40.0),
                (Touch::Move, 80.0, 40.0)
            ]
        );
        assert_eq!(finger.frame(700.0), vec![], "the tap's up went once");
        assert_eq!(
            finger.up(3, p(40.0, 20.0), 701.0),
            vec![(Touch::Up, 80.0, 40.0)]
        );
        // A second tap while the first one's up waits: its down waits for
        // that up (the first contact 40 ms long, the second down after it),
        // and its own up 40 ms after its down.
        finger.down(first(4), p(1.0, 1.0), Spot::Edit, 800.0);
        assert_eq!(
            finger.up(4, p(1.0, 1.0), 805.0),
            vec![(Touch::Down, 2.0, 2.0)]
        );
        finger.down(first(5), p(3.0, 3.0), Spot::Edit, 810.0);
        assert_eq!(finger.up(5, p(3.0, 3.0), 815.0), vec![], "its down waits");
        assert!(!finger.held());
        assert_eq!(finger.frame(844.0), vec![]);
        assert_eq!(
            finger.frame(845.0),
            vec![(Touch::Up, 2.0, 2.0), (Touch::Down, 6.0, 6.0)]
        );
        assert_eq!(finger.frame(884.0), vec![]);
        assert_eq!(finger.frame(885.0), vec![(Touch::Up, 6.0, 6.0)]);
        assert_eq!(finger.frame(900.0), vec![], "each went once");
        // A third lift while one tap waits (three within 40 ms, no human
        // hand's) is dropped: the waiting one goes.
        finger.down(first(11), p(1.0, 1.0), Spot::Edit, 920.0);
        finger.up(11, p(1.0, 1.0), 921.0);
        finger.down(first(12), p(3.0, 3.0), Spot::Edit, 925.0);
        assert_eq!(finger.up(12, p(3.0, 3.0), 930.0), vec![]);
        finger.down(first(13), p(5.0, 5.0), Spot::Edit, 935.0);
        assert_eq!(finger.up(13, p(5.0, 5.0), 940.0), vec![]);
        assert_eq!(
            finger.frame(961.0),
            vec![(Touch::Up, 2.0, 2.0), (Touch::Down, 6.0, 6.0)]
        );
        assert_eq!(finger.frame(1001.0), vec![(Touch::Up, 6.0, 6.0)]);
        assert_eq!(finger.frame(1100.0), vec![]);
        // A lift after the up is due, before a frame sent it: that up,
        // then the new down, at once.
        finger.down(first(14), p(1.0, 1.0), Spot::Edit, 1200.0);
        finger.up(14, p(1.0, 1.0), 1201.0);
        finger.down(first(15), p(3.0, 3.0), Spot::Edit, 1230.0);
        assert_eq!(
            finger.up(15, p(3.0, 3.0), 1241.0),
            vec![(Touch::Up, 2.0, 2.0), (Touch::Down, 6.0, 6.0)]
        );
        assert_eq!(finger.frame(1280.0), vec![]);
        assert_eq!(finger.frame(1281.0), vec![(Touch::Up, 6.0, 6.0)]);
        // Leaving the screen, or a hidden page, sends a tap's up at once;
        // a tap still waiting for it never went, so nothing of it goes.
        finger.down(first(6), p(1.0, 1.0), Spot::Edit, 1400.0);
        finger.up(6, p(1.0, 1.0), 1401.0);
        finger.down(first(16), p(3.0, 3.0), Spot::Edit, 1405.0);
        assert_eq!(finger.up(16, p(3.0, 3.0), 1410.0), vec![]);
        assert_eq!(finger.leave(), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.frame(1500.0), vec![]);
        finger.down(first(7), p(1.0, 1.0), Spot::Edit, 1600.0);
        finger.up(7, p(1.0, 1.0), 1601.0);
        finger.down(first(17), p(3.0, 3.0), Spot::Edit, 1605.0);
        assert_eq!(finger.up(17, p(3.0, 3.0), 1610.0), vec![]);
        assert_eq!(finger.visibility(true), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.frame(1700.0), vec![]);
        // A pinch that starts meanwhile: the tap's up goes on its time.
        finger.down(first(8), p(1.0, 1.0), Spot::Edit, 1800.0);
        finger.up(8, p(1.0, 1.0), 1801.0);
        assert_eq!(
            finger.down(first(9), p(5.0, 5.0), Spot::Edit, 1805.0),
            vec![]
        );
        assert_eq!(
            finger.down(second(10), p(9.0, 9.0), Spot::Edit, 1806.0),
            vec![]
        );
        assert_eq!(finger.frame(1840.0), vec![]);
        assert_eq!(finger.frame(1841.0), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.up(9, p(5.0, 5.0), 1850.0), vec![]);
        assert_eq!(finger.up(10, p(9.0, 9.0), 1851.0), vec![]);
        assert!(!finger.held());
        // A waiting finger cancelled (or its capture lost) never reached
        // the editor: nothing goes.
        finger.down(first(2), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(finger.cancel(3), vec![], "another pointer");
        assert!(finger.held());
        assert_eq!(finger.cancel(2), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.frame(500.0), vec![]);
        assert_eq!(finger.up(2, p(10.0, 20.0), 500.0), vec![]);
        // Nor when the screen goes or the page hides.
        finger.down(first(4), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(finger.leave(), vec![]);
        assert!(!finger.held());
        finger.down(first(5), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(finger.visibility(true), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.frame(500.0), vec![]);
    }

    #[test]
    fn a_finger_off_the_picture_never_reaches_the_editor_but_pinches() {
        let mut finger = Finger::default();
        finger.down(first(1), p(1.0, 1.0), Spot::Off, 0.0);
        assert!(finger.held());
        assert_eq!(finger.frame(HOLD_MS), vec![], "held: nothing");
        finger.moved(1, p(50.0, 1.0));
        assert_eq!(finger.frame(200.0), vec![]);
        assert_eq!(finger.cancel(1), vec![]);
        assert!(!finger.held());
        // Slid off at once: nothing either.
        finger.down(first(2), p(1.0, 1.0), Spot::Off, 0.0);
        finger.moved(2, p(50.0, 1.0));
        assert_eq!(finger.frame(1.0), vec![]);
        assert_eq!(finger.up(2, p(50.0, 1.0), 0.0), vec![]);
        // A tap off the picture: nothing.
        finger.down(first(3), p(1.0, 1.0), Spot::Off, 0.0);
        assert_eq!(finger.up(3, p(1.0, 1.0), 0.0), vec![]);
        assert!(!finger.held());
        // Off the picture, then a second finger: a pinch.
        finger.down(first(4), p(1.0, 1.0), Spot::Off, 0.0);
        assert_eq!(finger.frame(HOLD_MS), vec![]);
        assert_eq!(
            finger.down(second(5), p(9.0, 9.0), Spot::Edit, 130.0),
            vec![]
        );
        assert_eq!(finger.pair(), Some([(4, (1.0, 1.0)), (5, (9.0, 9.0))]));
    }

    #[test]
    fn a_second_finger_while_the_first_waits_pinches_and_sends_nothing() {
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(finger.pair(), None, "one finger");
        assert_eq!(
            finger.down(second(2), p(30.0, 20.0), Spot::Edit, 50.0),
            vec![]
        );
        assert!(finger.has(2));
        assert_eq!(finger.pair(), Some([(1, (10.0, 20.0)), (2, (30.0, 20.0))]));
        assert_eq!(finger.frame(500.0), vec![], "no down after the hold");
        finger.moved(1, p(0.0, 20.0));
        finger.moved(2, p(40.0, 21.0));
        assert_eq!(finger.frame(510.0), vec![], "no move");
        assert_eq!(finger.pair(), Some([(1, (0.0, 20.0)), (2, (40.0, 21.0))]));
        // A third finger is nothing.
        assert_eq!(
            finger.down(second(3), p(5.0, 5.0), Spot::Edit, 520.0),
            vec![]
        );
        assert!(!finger.has(3));
        finger.moved(3, p(6.0, 6.0));
        assert_eq!(finger.pair(), Some([(1, (0.0, 20.0)), (2, (40.0, 21.0))]));
        // One lifts: no pair, nothing sent; the other one stays a pinch's.
        assert_eq!(finger.up(1, p(0.0, 20.0), 0.0), vec![]);
        assert_eq!(finger.pair(), None);
        assert!(finger.held());
        finger.moved(2, p(60.0, 20.0));
        assert_eq!(finger.frame(600.0), vec![], "the finger left sends nothing");
        assert_eq!(finger.frame(900.0), vec![]);
        // A finger again: a pair again, in the free place.
        assert_eq!(
            finger.down(second(4), p(7.0, 7.0), Spot::Edit, 610.0),
            vec![]
        );
        assert_eq!(finger.pair(), Some([(4, (7.0, 7.0)), (2, (60.0, 20.0))]));
        assert_eq!(finger.cancel(2), vec![]);
        assert_eq!(finger.up(4, p(7.0, 7.0), 0.0), vec![]);
        assert!(!finger.held());
        // The pinch is over: the next finger is a first finger again.
        assert_eq!(
            finger.down(first(5), p(10.0, 20.0), Spot::Edit, 700.0),
            vec![]
        );
        assert_eq!(finger.frame(820.0), vec![(Touch::Down, 20.0, 40.0)]);
    }

    #[test]
    fn a_second_finger_on_a_touch_cancels_it_where_it_was_then_pinches() {
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(10.0, 20.0));
        finger.moved(1, p(11.0, 22.0));
        assert_eq!(
            finger.down(second(2), p(40.0, 20.0), Spot::Edit, 130.0),
            vec![(Touch::Cancel, 22.0, 44.0)],
            "the contact's cancel at its last point"
        );
        assert_eq!(finger.frame(140.0), vec![], "its move never follows");
        assert_eq!(finger.pair(), Some([(1, (11.0, 22.0)), (2, (40.0, 20.0))]));
        assert_eq!(
            finger.up(1, p(11.0, 22.0), 0.0),
            vec![],
            "no up after the cancel"
        );
        assert_eq!(finger.leave(), vec![], "a pinch's finger left: no cancel");
        assert!(!finger.held());
        // While pinching a cancel, a hidden page and an up send nothing.
        touching(&mut finger, 3, p(10.0, 20.0));
        finger.down(second(4), p(40.0, 20.0), Spot::Edit, 130.0);
        assert_eq!(finger.cancel(3), vec![]);
        assert_eq!(finger.visibility(true), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.up(4, p(40.0, 20.0), 0.0), vec![]);
    }

    #[test]
    fn a_primary_down_ends_a_finger_whose_end_never_came_and_starts_on_its_own() {
        // #124: pointer 1 rests on the editor and its end never comes (iOS
        // took it for a system gesture). The next first finger (primary:
        // no other finger is down) ends it with its cancel where it was, so
        // the PC's contact ends, and goes on alone: no pinch.
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(10.0, 20.0));
        finger.moved(1, p(11.0, 22.0));
        assert_eq!(
            finger.missed(first(2)),
            Missed {
                pointers: vec![1],
                out: vec![(Touch::Cancel, 22.0, 44.0)],
            },
            "the lost contact's cancel at its last point"
        );
        assert!(!finger.held(), "the lost finger left");
        assert!(!finger.has(1));
        assert_eq!(
            finger.down(first(2), p(40.0, 20.0), Spot::Edit, 200.0),
            vec![]
        );
        assert_eq!(finger.pair(), None, "no pinch");
        assert_eq!(finger.frame(319.0), vec![], "it waits its own hold");
        assert_eq!(
            finger.frame(320.0),
            vec![(Touch::Down, 80.0, 40.0)],
            "a single finger's down where it landed"
        );
        finger.moved(2, p(50.0, 20.0));
        assert_eq!(finger.frame(330.0), vec![(Touch::Move, 100.0, 40.0)]);
        assert_eq!(
            finger.up(2, p(50.0, 20.0), 340.0),
            vec![(Touch::Up, 100.0, 40.0)]
        );
        // The lost finger's own end, should it ever come, is nothing.
        assert_eq!(finger.up(1, p(11.0, 22.0), 350.0), vec![]);
        assert_eq!(finger.cancel(1), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.missed(first(3)), Missed::default(), "none held");
    }

    #[test]
    fn a_lost_finger_off_the_editor_or_in_a_pinch_ends_with_nothing_sent() {
        // #124: a pinch's contact was cancelled when the pinch began; a
        // waiting finger, one on the overview or off the picture never had
        // one. Two lost pinch fingers end both, and the next finger edits
        // alone.
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(10.0, 20.0));
        finger.down(second(2), p(40.0, 20.0), Spot::Edit, 130.0);
        assert_eq!(
            finger.missed(first(3)),
            Missed {
                pointers: vec![1, 2],
                out: vec![],
            }
        );
        assert!(!finger.held());
        finger.down(first(3), p(5.0, 5.0), Spot::Edit, 200.0);
        assert_eq!(finger.pair(), None);
        assert_eq!(finger.frame(320.0), vec![(Touch::Down, 10.0, 10.0)]);
        // A pinch's other finger lifted, this one lost: the pinch ends with
        // it.
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Edit, 0.0);
        finger.down(second(2), p(40.0, 20.0), Spot::Edit, 10.0);
        assert_eq!(finger.up(2, p(40.0, 20.0), 20.0), vec![]);
        assert_eq!(
            finger.missed(first(3)),
            Missed {
                pointers: vec![1],
                out: vec![],
            }
        );
        finger.down(first(3), p(5.0, 5.0), Spot::Edit, 30.0);
        assert_eq!(finger.pair(), None);
        assert_eq!(finger.frame(150.0), vec![(Touch::Down, 10.0, 10.0)]);
        // A waiting finger: nothing goes, and the new one waits its own
        // hold, not what is left of the lost one's.
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(
            finger.missed(first(2)),
            Missed {
                pointers: vec![1],
                out: vec![],
            }
        );
        finger.down(first(2), p(30.0, 30.0), Spot::Edit, 50.0);
        assert_eq!(finger.frame(170.0_f64.next_down()), vec![]);
        assert_eq!(finger.frame(170.0), vec![(Touch::Down, 60.0, 60.0)]);
        // One on the overview: no zoom ever comes of it.
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0);
        assert_eq!(
            finger.missed(first(2)),
            Missed {
                pointers: vec![1],
                out: vec![],
            }
        );
        assert_eq!(finger.take_zoom(10.0), None);
        assert!(!finger.held());
        // One off the picture: the same.
        finger.down(first(3), p(1.0, 1.0), Spot::Off, 20.0);
        assert_eq!(
            finger.missed(first(4)),
            Missed {
                pointers: vec![3],
                out: vec![],
            }
        );
        assert!(!finger.held());
        // A down with the lost finger's own pointer id: the old touch ends,
        // and a new one starts.
        let mut finger = Finger::default();
        touching(&mut finger, 7, p(10.0, 20.0));
        assert_eq!(
            finger.missed(first(7)),
            Missed {
                pointers: vec![7],
                out: vec![(Touch::Cancel, 20.0, 40.0)],
            }
        );
        assert_eq!(
            finger.down(first(7), p(30.0, 30.0), Spot::Edit, 200.0),
            vec![]
        );
        assert_eq!(finger.frame(320.0), vec![(Touch::Down, 60.0, 60.0)]);
    }

    #[test]
    fn a_second_finger_or_another_pointer_type_ends_no_finger() {
        // #124 M1: a finger down while a second one comes is not lost (the
        // second one is not primary), and a mouse or a pen is primary only
        // among its own kind.
        let mouse = |id| Down {
            id,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        let pen = Down {
            id: 4,
            kind: Some(Pointer::Pen),
            primary: true,
        };
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(10.0, 20.0));
        assert_eq!(finger.missed(second(2)), Missed::default());
        assert_eq!(finger.missed(mouse(3)), Missed::default());
        assert_eq!(finger.missed(pen), Missed::default());
        assert!(finger.has(1));
        // The second finger pinches as ever: the contact's cancel first.
        assert_eq!(
            finger.down(second(2), p(40.0, 20.0), Spot::Edit, 130.0),
            vec![(Touch::Cancel, 20.0, 40.0)]
        );
        assert_eq!(finger.pair(), Some([(1, (10.0, 20.0)), (2, (40.0, 20.0))]));
        // A mouse on the editor: a finger's primary down leaves it, its own
        // kind's ends it.
        let mut finger = Finger::default();
        finger.down(mouse(1), p(10.0, 20.0), Spot::Edit, 0.0);
        assert_eq!(finger.frame(HOLD_MS), vec![(Touch::Down, 20.0, 40.0)]);
        assert_eq!(finger.missed(first(2)), Missed::default());
        assert!(finger.has(1));
        assert_eq!(
            finger.missed(mouse(1)),
            Missed {
                pointers: vec![1],
                out: vec![(Touch::Cancel, 20.0, 40.0)],
            }
        );
        // A mouse and a finger pinch, the finger lost: only the finger
        // ends; the new finger pinches with the mouse still down.
        let mut finger = Finger::default();
        finger.down(mouse(1), p(10.0, 20.0), Spot::Edit, 0.0);
        finger.down(first(2), p(40.0, 20.0), Spot::Edit, 10.0);
        assert_eq!(
            finger.missed(first(3)),
            Missed {
                pointers: vec![2],
                out: vec![],
            }
        );
        assert!(finger.has(1));
        assert_eq!(
            finger.down(first(3), p(30.0, 30.0), Spot::Edit, 20.0),
            vec![]
        );
        assert_eq!(finger.pair(), Some([(1, (10.0, 20.0)), (3, (30.0, 30.0))]));
    }

    #[test]
    fn a_cards_lock_and_its_text() {
        let lock = |path: &str, mine: bool| EqLock {
            instance: "band".into(),
            path: path.into(),
            mine,
            since: 1_790_000_000_000.0,
        };
        // This page's switch: the one it leaves and the one it waits for.
        let mine = [lock("a", true), lock("c", true)];
        assert_eq!(card_lock(&mine, "band", "a"), CardLock::Mine);
        assert_eq!(card_lock(&mine, "band", "b"), CardLock::Free);
        assert_eq!(card_lock(&mine, "master", "a"), CardLock::Free);
        assert_eq!(card_lock(&[], "band", "a"), CardLock::Free);
        // Another page holds b: b is its, every other card is in use.
        let other = [lock("b", false)];
        assert_eq!(
            card_lock(&other, "band", "b"),
            CardLock::Other(1_790_000_000_000.0)
        );
        assert_eq!(
            card_lock(&other, "band", "c"),
            CardLock::InUse(1_790_000_000_000.0)
        );
        assert_eq!(
            card_lock(&other, "master", "b"),
            CardLock::InUse(1_790_000_000_000.0),
            "another instance's card too"
        );
        assert!(locked(CardLock::Other(1.0)) && locked(CardLock::InUse(1.0)));
        assert!(!locked(CardLock::Free));
        assert!(!locked(CardLock::Mine));
        assert_eq!(hh_mm(9, 5), "09:05");
        assert_eq!(hh_mm(12, 41), "12:41");
        assert_eq!(locked_text("12:41"), "Upravuje ho iný zvukár (od 12:41)");
        assert_eq!(
            in_use_text("12:41"),
            "Pro-Q 4 práve používa iný zvukár (od 12:41)"
        );
    }

    #[test]
    fn only_the_open_sessions_frames_are_drawn() {
        assert!(shows_frame(3, Some(3)));
        assert!(!shows_frame(2, Some(3)));
        assert!(!shows_frame(3, None));
    }

    #[test]
    fn a_pictures_url_encodes_its_query() {
        assert_eq!(encode_component("AZaz09-._~"), "AZaz09-._~");
        assert_eq!(encode_component("a b/c›"), "a%20b%2Fc%E2%80%BA");
        assert_eq!(
            encode_component("[name=x]&?=#"),
            "%5Bname%3Dx%5D%26%3F%3D%23"
        );
        assert_eq!(
            picture_url("band", "live_set tracks 1 devices 0"),
            "/api/eq/picture?instance=band&path=live_set%20tracks%201%20devices%200"
        );
    }

    #[test]
    fn only_a_failed_open_says_why_under_its_card() {
        // A close the page or the hub made on purpose, and a lock the card
        // already shows (ZAMKNUTÉ), say nothing.
        for quiet in ["exit", "switch", "detach", "socket", "locked", "in use"] {
            assert_eq!(failure_text(quiet), None, "{quiet}");
        }
        assert_eq!(failure_text(reason::IN_USE), None);
        // A lock refusal is the screen's `eq_locked` step.
        assert!(lock_refusal("locked") && lock_refusal("in use"));
        assert!(lock_refusal(reason::LOCKED) && lock_refusal(reason::IN_USE));
        assert!(!lock_refusal("exit"));
        assert!(!lock_refusal("moved"));
        assert_eq!(
            failure_text("off"),
            Some("EQ je na PC vypnuté.".to_string())
        );
    }

    #[test]
    fn the_hubs_known_reasons_read_in_slovak_under_the_card() {
        let said = |why: &str| failure_text(why).unwrap_or_default();
        assert_eq!(said("closing"), "ešte sa zatvára, skús znova");
        assert_eq!(said("unknown"), "zoznam je starý, otvor kanál znova");
        assert_eq!(said("moved"), "Pro-Q 4 sa presunul, otvor kanál znova");
        assert_eq!(
            said("open on the PC"),
            "Pro-Q 4 je otvorený priamo na PC, zatvor ho tam"
        );
        assert_eq!(said("no window"), "okno Pro-Q 4 sa neotvorilo");
        assert_eq!(said("window closed"), "okno Pro-Q 4 sa zavrelo");
        assert_eq!(said("unread"), "Pro-Q 4 sa nedá prečítať, skús znova");
        // The protocol's names for them.
        assert_eq!(said(reason::CLOSING), said("closing"));
        assert_eq!(said(reason::UNKNOWN), said("unknown"));
        assert_eq!(said(reason::MOVED), said("moved"));
        assert_eq!(said(reason::OPEN_ON_PC), said("open on the PC"));
        assert_eq!(said(reason::GONE), said("window closed"));
        assert_eq!(said(reason::UNREAD), said("unread"));
        assert_eq!(said(reason::NO_WINDOW), said("no window"));
        // Anything else keeps the hub's own words.
        assert_eq!(
            failure_text("EnumWindows: access denied"),
            Some("Neotvoril sa: EnumWindows: access denied".to_string())
        );
        assert_eq!(
            failure_text("the guard's tap failed"),
            Some("Neotvoril sa: the guard's tap failed".to_string())
        );
    }

    #[test]
    fn every_reason_the_hub_sends_a_page_reads_in_slovak() {
        let said = |why: &str| failure_text(why).unwrap_or_default();
        assert_eq!(said("instance offline"), "Live je nedostupný, skús znova");
        assert_eq!(said("Live did not answer"), "Live neodpovedá, skús znova");
        assert_eq!(said("unknown instance"), "neznámy Live");
        assert_eq!(
            said("several windows"),
            "otvorilo sa viac okien, skús znova"
        );
        assert_eq!(
            said("the window has no Pro-Q picture (FF_UIWindow)"),
            "okno nie je Pro-Q 4"
        );
        assert_eq!(said("the window worker stopped"), "hub sa zastavuje");
        assert_eq!(
            said("the window did not come on top"),
            "okno Pro-Q 4 sa nedostalo navrch, skús znova"
        );
        assert_eq!(
            said("left open in Live"),
            "Pro-Q 4 sa nepodarilo bezpečne zavrieť, ostáva otvorený na PC"
        );
        assert_eq!(
            said("press-and-hold could not be turned off"),
            "na PC sa nedá vypnúť podržanie prsta, Pro-Q 4 sa preto neotvorí"
        );
        // An undo's refusals (#71 PR P2a).
        assert_eq!(said("no such editor"), "Pro-Q 4 už nie je otvorený");
        assert_eq!(
            said("the editor is busy"),
            "Pro-Q 4 je práve zaneprázdnený, skús znova"
        );
        assert_eq!(
            said("too soon after the last undo"),
            "príliš rýchlo po poslednom vrátení, skús znova"
        );
        assert_eq!(
            said("Pro-Q's undo is not where it was measured"),
            "šípka späť sa v Pro-Q 4 nenašla"
        );
        // The protocol's names for them.
        for (named, word) in [
            (reason::OFFLINE, "instance offline"),
            (reason::NO_ANSWER, "Live did not answer"),
            (reason::UNKNOWN_INSTANCE, "unknown instance"),
            (reason::SEVERAL, "several windows"),
            (
                reason::NO_PICTURE,
                "the window has no Pro-Q picture (FF_UIWindow)",
            ),
            (reason::STOPPED, "the window worker stopped"),
            (reason::NOT_ON_TOP, "the window did not come on top"),
            (reason::LEFT_OPEN, "left open in Live"),
            (reason::HOLD_ON, "press-and-hold could not be turned off"),
            (reason::NO_EDITOR, "no such editor"),
            (reason::UNDO_BUSY, "the editor is busy"),
            (reason::UNDO_SOON, "too soon after the last undo"),
            (
                reason::UNDO_MISSING,
                "Pro-Q's undo is not where it was measured",
            ),
        ] {
            assert_eq!(said(named), said(word), "{word}");
            assert!(!said(named).starts_with("Neotvoril sa"), "{word}");
        }
    }

    #[test]
    fn the_undo_takes_no_press_while_a_finger_keeps_a_contact_nor_within_700_ms() {
        // #71 PR P2b: the approved mockup's rule.
        assert_eq!(UNDO_APART_MS, 700.0);
        assert!(undo_ready(false, None, 0.0), "no finger, never pressed");
        // A finger on the picture, or a quick tap's up still to go.
        assert!(!undo_ready(true, None, 0.0));
        assert!(!undo_ready(true, Some(1_000.0), 5_000.0));
        // From the last press it took: 699 refused, 700 taken.
        assert!(!undo_ready(false, Some(1_000.0), 1_000.0));
        assert!(!undo_ready(false, Some(1_000.0), 1_699.0));
        assert!(!undo_ready(false, Some(1_000.0), 1_700.0_f64.next_down()));
        assert!(undo_ready(false, Some(1_000.0), 1_700.0));
        assert!(undo_ready(false, Some(1_000.0), 5_000.0));
    }

    #[test]
    fn the_fingers_keep_the_pcs_contact_while_one_is_down_or_a_taps_up_is_to_go() {
        let mut finger = Finger::default();
        assert!(!finger.busy(), "no finger");
        finger.down(first(1), p(10.0, 10.0), Spot::Edit, 0.0);
        assert!(finger.busy(), "a finger waiting on the picture");
        // A quick lift: its down at once, its up 40 ms later.
        assert_eq!(
            finger.up(1, p(10.0, 10.0), 10.0),
            vec![(Touch::Down, 20.0, 20.0)]
        );
        assert!(!finger.held());
        assert!(finger.busy(), "its up still to go");
        assert_eq!(finger.frame(50.0), vec![(Touch::Up, 20.0, 20.0)]);
        assert!(!finger.busy());
        // A finger off the picture counts as well.
        finger.down(first(2), p(10.0, 10.0), Spot::Off, 100.0);
        assert!(finger.busy());
        assert_eq!(finger.up(2, p(10.0, 10.0), 110.0), vec![]);
        assert!(!finger.busy());
    }

    #[test]
    fn the_undo_flashes_180_ms_and_its_note_shows_1600_ms() {
        assert_eq!(UNDO_FLASH_MS, 180.0);
        assert!(!undo_flash(None, 0.0), "never pressed");
        assert!(undo_flash(Some(1_000.0), 1_000.0));
        assert!(undo_flash(Some(1_000.0), 1_180.0_f64.next_down()));
        assert!(!undo_flash(Some(1_000.0), 1_180.0));
        assert!(!undo_flash(Some(1_000.0), 5_000.0));
        assert_eq!(UNDO_NOTE_MS, 1600.0);
        assert!(!undo_note_shown(None, 0.0), "no answer yet");
        assert!(undo_note_shown(Some(1_000.0), 1_000.0));
        assert!(undo_note_shown(Some(1_000.0), 2_600.0_f64.next_down()));
        assert!(!undo_note_shown(Some(1_000.0), 2_600.0));
        assert!(!undo_note_shown(Some(1_000.0), 9_000.0));
    }

    #[test]
    fn each_answer_to_an_undo_has_its_note() {
        // The approved mockup's notes, and the refusals the owner can act on.
        assert_eq!(undo_text("done"), "Vrátené: posledná zmena v Pro-Q");
        assert_eq!(undo_text("nothing"), "Nie je čo vrátiť");
        assert_eq!(
            undo_text("Pro-Q's undo is not where it was measured"),
            "Späť sa v Pro-Q nenašlo — nič sa nestlačilo"
        );
        assert_eq!(
            undo_text("the editor is busy"),
            "Prst je na obraze — vráť po zdvihnutí prsta"
        );
        assert_eq!(
            undo_text("too soon after the last undo"),
            "Príliš rýchlo za sebou — stlač znova"
        );
        // The protocol's names for them.
        for (named, word) in [
            (UNDO_DONE, "done"),
            (UNDO_NOTHING, "nothing"),
            (
                reason::UNDO_MISSING,
                "Pro-Q's undo is not where it was measured",
            ),
            (reason::UNDO_BUSY, "the editor is busy"),
            (reason::UNDO_SOON, "too soon after the last undo"),
        ] {
            assert_eq!(undo_text(named), undo_text(word), "{word}");
        }
        // Any other reason the protocol names, in Slovak.
        assert_eq!(
            undo_text(reason::NO_EDITOR),
            "Vrátiť sa nepodarilo: Pro-Q 4 už nie je otvorený"
        );
        assert_eq!(
            undo_text(reason::OFF),
            "Vrátiť sa nepodarilo: EQ je na PC vypnuté."
        );
        // An unexpected one (a grab's or a tap's error): the hub's words.
        assert_eq!(
            undo_text("BitBlt failed"),
            "Vrátiť sa nepodarilo: BitBlt failed"
        );
    }

    #[test]
    fn a_hidden_page_lifts_its_finger() {
        let mut finger = Finger::default();
        assert_eq!(finger.visibility(true), vec![], "no finger down");
        touching(&mut finger, 4, p(10.0, 20.0));
        finger.moved(4, p(11.0, 21.0));
        assert_eq!(finger.visibility(false), vec![], "shown: it stays down");
        assert!(finger.held());
        assert_eq!(
            finger.visibility(true),
            vec![(Touch::Cancel, 22.0, 42.0)],
            "hidden: a cancel where it was"
        );
        assert!(!finger.held());
        assert_eq!(
            finger.frame(300.0),
            vec![],
            "its last move is not sent after"
        );
        assert_eq!(
            finger.up(4, p(12.0, 22.0), 0.0),
            vec![],
            "its lift sends nothing"
        );
        assert_eq!(finger.visibility(true), vec![]);
    }

    #[test]
    fn one_finger_edits_from_twice_the_whole_picture_on() {
        assert_eq!(EDIT_ZOOM_MIN, 2.0);
        assert_eq!(TAP_ZOOM, 3.0);
        assert_eq!(ZOOM_SETTLE_MS, 500.0);
        assert!(!settling(None, 0.0));
        assert!(settling(Some(1000.0), 1000.0));
        assert!(settling(Some(1000.0), 1500.0_f64.next_down()));
        assert!(!settling(Some(1000.0), 1500.0));
        assert!(edits(2.0));
        assert!(!edits(2.0_f64.next_down()));
        assert!(!edits(1.99));
        assert!(!edits(1.0));
        assert!(edits(4.0));
        assert_eq!(spot(true, 1.0), Spot::Overview);
        assert_eq!(spot(true, 1.99), Spot::Overview);
        assert_eq!(spot(true, 2.0), Spot::Edit);
        assert_eq!(spot(false, 1.0), Spot::Off);
        assert_eq!(spot(false, 3.0), Spot::Off);
    }

    #[test]
    fn a_finger_on_an_old_picture_or_a_stalled_link_is_refused_and_the_screen_says_so() {
        // PR J: only a finger that would edit.
        assert!(!stale_landing(Spot::Edit, true, false));
        assert!(stale_landing(Spot::Edit, false, false), "an old picture");
        assert!(stale_landing(Spot::Edit, true, true), "a stalled link");
        for spot in [Spot::Off, Spot::Overview] {
            assert!(!stale_landing(spot, false, true), "{spot:?}");
        }
        assert_eq!(STALE_NOTE_MS, 1500.0);
        assert!(!stale_note(None, 0.0));
        assert!(stale_note(Some(100.0), 100.0));
        assert!(stale_note(Some(100.0), 1600.0_f64.next_down()));
        assert!(!stale_note(Some(100.0), 1600.0));
        assert_eq!(STALE_TEXT, "Obraz ešte dobieha, polož prst znova");
    }

    #[test]
    fn a_first_finger_is_taken_only_caught_up_with_no_taps_up_to_go_and_no_dropout() {
        // PR Q: every combination; only the one with all three holding.
        assert!(
            takes_finger(true, false, false),
            "caught up, nothing to go, the link up"
        );
        assert!(!takes_finger(false, false, false), "an old picture");
        assert!(!takes_finger(true, true, false), "a quick tap's up to go");
        assert!(!takes_finger(true, false, true), "a dropout");
        assert!(
            !takes_finger(false, true, false),
            "an old picture, a tap's up to go"
        );
        assert!(
            !takes_finger(false, false, true),
            "an old picture, a dropout"
        );
        assert!(
            !takes_finger(true, true, true),
            "a tap's up to go, a dropout"
        );
        assert!(!takes_finger(false, true, true), "none holds");
    }

    #[test]
    fn a_tap_on_the_overview_zooms_in_and_sends_the_editor_nothing() {
        let mut finger = Finger::default();
        assert_eq!(finger.take_zoom(0.0), None);
        assert_eq!(
            finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0),
            vec![]
        );
        // Its hold runs out and it moves within the slop: nothing goes.
        assert_eq!(finger.frame(200.0), vec![]);
        finger.moved(1, p(16.0, 20.0));
        assert_eq!(finger.frame(220.0), vec![]);
        // Lifted within the slop (a long tap too): a zoom on where it
        // landed, its picture point; once.
        assert_eq!(finger.up(1, p(16.0, 20.0), 900.0), vec![]);
        assert_eq!(finger.frame(950.0), vec![], "no tap's up to send");
        assert_eq!(finger.take_zoom(0.0), Some((20.0, 40.0)));
        assert_eq!(finger.take_zoom(0.0), None);
        assert!(!finger.held());
        // A quick one lifted at once: the same.
        finger.down(first(2), p(30.0, 30.0), Spot::Overview, 1000.0);
        assert_eq!(finger.up(2, p(30.0, 30.0), 1010.0), vec![]);
        assert_eq!(finger.frame(1060.0), vec![]);
        assert_eq!(finger.take_zoom(0.0), Some((60.0, 60.0)));
    }

    #[test]
    fn a_first_finger_within_the_settle_after_a_tap_zoomed_in_reaches_nothing() {
        // The PR I review: the iPad's double tap to zoom. Its first tap
        // zooms in; its second, about 250 ms later, edited at a point it
        // aimed at on the whole picture.
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0);
        assert_eq!(finger.up(1, p(10.0, 20.0), 30.0), vec![]);
        assert_eq!(finger.take_zoom(40.0), Some((20.0, 40.0)));
        // The second tap lands on the zoomed picture 250 ms later: nothing
        // goes, neither at the hold nor at its lift.
        assert_eq!(
            finger.down(first(2), p(30.0, 30.0), Spot::Edit, 290.0),
            vec![]
        );
        assert_eq!(finger.frame(420.0), vec![]);
        assert_eq!(finger.up(2, p(30.0, 30.0), 430.0), vec![]);
        assert_eq!(finger.frame(480.0), vec![]);
        // A quick tap within the settle: nothing either.
        assert_eq!(
            finger.down(first(3), p(30.0, 30.0), Spot::Edit, 539.0),
            vec![]
        );
        assert_eq!(finger.frame(539.0), vec![]);
        assert_eq!(finger.up(3, p(30.0, 30.0), 539.5), vec![]);
        assert_eq!(finger.frame(600.0), vec![]);
        // From the settle's end a finger edits again.
        assert_eq!(
            finger.down(first(4), p(5.0, 5.0), Spot::Edit, 540.0),
            vec![]
        );
        assert_eq!(finger.frame(660.0), vec![(Touch::Down, 10.0, 10.0)]);
        assert_eq!(
            finger.up(4, p(5.0, 5.0), 670.0),
            vec![(Touch::Up, 10.0, 10.0)]
        );
        // A drag on the overview zooms nothing, and starts no settle.
        finger.down(first(5), p(10.0, 20.0), Spot::Overview, 700.0);
        finger.moved(5, p(30.0, 20.0));
        finger.up(5, p(30.0, 20.0), 720.0);
        assert_eq!(finger.take_zoom(730.0), None);
        finger.down(first(6), p(5.0, 5.0), Spot::Edit, 740.0);
        assert_eq!(finger.frame(860.0), vec![(Touch::Down, 10.0, 10.0)]);
    }

    #[test]
    fn a_finger_held_through_the_settle_reaches_the_editor_at_its_end() {
        // The PR I re-review: a drag started right after a tap's zoom was
        // swallowed whole; it goes once the settle is over.
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0);
        finger.up(1, p(10.0, 20.0), 30.0);
        assert_eq!(finger.take_zoom(40.0), Some((20.0, 40.0)));
        // It lands 450 ms after the zoom and drags: nothing until the
        // settle's end (540 ms).
        assert_eq!(
            finger.down(first(2), p(30.0, 30.0), Spot::Edit, 490.0),
            vec![]
        );
        finger.moved(2, p(40.0, 30.0));
        assert_eq!(finger.frame(539.0), vec![]);
        // At its end: its down where it landed, then its move.
        assert_eq!(
            finger.frame(540.0),
            vec![(Touch::Down, 60.0, 60.0), (Touch::Move, 80.0, 60.0)]
        );
        finger.moved(2, p(45.0, 30.0));
        assert_eq!(finger.frame(556.0), vec![(Touch::Move, 90.0, 60.0)]);
        assert_eq!(
            finger.up(2, p(45.0, 30.0), 570.0),
            vec![(Touch::Up, 90.0, 60.0)]
        );
        // One still where it landed at the settle's end: its down only.
        finger.down(first(3), p(1.0, 1.0), Spot::Overview, 600.0);
        finger.up(3, p(1.0, 1.0), 610.0);
        finger.take_zoom(620.0);
        finger.down(first(4), p(5.0, 5.0), Spot::Edit, 700.0);
        assert_eq!(finger.frame(1120.0), vec![(Touch::Down, 10.0, 10.0)]);
        finger.up(4, p(5.0, 5.0), 1130.0);
        // A second finger within the settle makes a pinch: nothing goes.
        finger.down(first(5), p(1.0, 1.0), Spot::Overview, 1200.0);
        finger.up(5, p(1.0, 1.0), 1210.0);
        finger.take_zoom(1220.0);
        finger.down(first(6), p(10.0, 10.0), Spot::Edit, 1300.0);
        assert_eq!(
            finger.down(second(7), p(50.0, 10.0), Spot::Edit, 1310.0),
            vec![]
        );
        assert!(finger.pair().is_some());
        assert_eq!(finger.frame(1800.0), vec![]);
    }

    #[test]
    fn the_whole_picture_ends_a_finger_on_the_editor_and_keeps_a_pinch() {
        // The PR I review: CELÝ EQ during a drag went on with the drag at
        // 1×, its points remapped.
        let mut finger = Finger::default();
        assert_eq!(finger.off_editor(), vec![], "no finger");
        assert!(!finger.on_editor());
        assert_eq!(finger.editor_pointer(), None);
        finger.down(first(1), p(10.0, 10.0), Spot::Edit, 0.0);
        assert!(finger.on_editor(), "waiting on the picture");
        assert_eq!(finger.editor_pointer(), Some(1));
        assert_eq!(finger.frame(120.0), vec![(Touch::Down, 20.0, 20.0)]);
        assert!(finger.on_editor(), "touching");
        finger.moved(1, p(12.0, 10.0));
        // Its cancel where it was; the move not sent yet never goes.
        assert_eq!(finger.off_editor(), vec![(Touch::Cancel, 24.0, 20.0)]);
        assert!(!finger.on_editor(), "off it now");
        assert_eq!(finger.editor_pointer(), None);
        finger.moved(1, p(20.0, 10.0));
        assert_eq!(finger.frame(140.0), vec![]);
        assert_eq!(finger.up(1, p(20.0, 10.0), 150.0), vec![]);
        assert!(!finger.held());
        // A waiting finger never reaches it.
        finger.down(first(2), p(10.0, 10.0), Spot::Edit, 200.0);
        assert_eq!(finger.off_editor(), vec![]);
        assert_eq!(finger.frame(400.0), vec![]);
        assert_eq!(finger.up(2, p(10.0, 10.0), 410.0), vec![]);
        // A finger off the picture is not on the editor.
        finger.down(first(8), p(10.0, 10.0), Spot::Off, 450.0);
        assert!(!finger.on_editor());
        finger.up(8, p(10.0, 10.0), 460.0);
        // A pinch goes on pinching.
        finger.down(first(3), p(10.0, 10.0), Spot::Edit, 500.0);
        finger.down(second(4), p(50.0, 10.0), Spot::Edit, 510.0);
        assert!(!finger.on_editor(), "a pinch");
        assert_eq!(finger.off_editor(), vec![]);
        assert_eq!(finger.pair(), Some([(3, (10.0, 10.0)), (4, (50.0, 10.0))]));
        finger.up(3, p(10.0, 10.0), 600.0);
        finger.up(4, p(50.0, 10.0), 600.0);
        // A quick tap's up still goes.
        assert!(!finger.tap_pending());
        finger.down(first(5), p(10.0, 10.0), Spot::Edit, 700.0);
        assert_eq!(
            finger.up(5, p(10.0, 10.0), 710.0),
            vec![(Touch::Down, 20.0, 20.0)]
        );
        assert!(finger.tap_pending(), "its up still to go");
        assert_eq!(finger.off_editor(), vec![]);
        assert_eq!(finger.frame(750.0), vec![(Touch::Up, 20.0, 20.0)]);
        assert!(!finger.tap_pending());
    }

    #[test]
    fn a_drag_on_the_overview_does_nothing() {
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0);
        finger.moved(1, p(16.1, 20.0));
        assert_eq!(finger.frame(10.0), vec![]);
        finger.moved(1, p(10.0, 20.0));
        assert_eq!(finger.frame(300.0), vec![], "back where it landed");
        assert_eq!(finger.up(1, p(10.0, 20.0), 400.0), vec![]);
        assert_eq!(finger.take_zoom(0.0), None, "it slid: no tap");
        // A lift past the slop with no move seen: no tap either.
        finger.down(first(2), p(10.0, 20.0), Spot::Overview, 500.0);
        assert_eq!(finger.up(2, p(10.0, 26.5), 520.0), vec![]);
        assert_eq!(finger.take_zoom(0.0), None);
        // A cancel: nothing.
        finger.down(first(3), p(10.0, 20.0), Spot::Overview, 600.0);
        assert_eq!(finger.cancel(3), vec![]);
        assert_eq!(finger.take_zoom(0.0), None);
        assert!(!finger.held());
    }

    #[test]
    fn two_fingers_on_the_overview_pinch_and_zoom_nowhere() {
        let mut finger = Finger::default();
        finger.down(first(1), p(10.0, 20.0), Spot::Overview, 0.0);
        assert_eq!(
            finger.down(second(2), p(40.0, 20.0), Spot::Overview, 30.0),
            vec![]
        );
        assert_eq!(finger.pair(), Some([(1, (10.0, 20.0)), (2, (40.0, 20.0))]));
        assert_eq!(finger.up(1, p(10.0, 20.0), 300.0), vec![]);
        assert_eq!(finger.up(2, p(40.0, 20.0), 310.0), vec![]);
        assert_eq!(finger.take_zoom(0.0), None);
        // Once zoomed, one finger edits again: its down after the hold.
        finger.down(first(3), p(5.0, 5.0), Spot::Edit, 400.0);
        assert_eq!(finger.frame(520.0), vec![(Touch::Down, 10.0, 10.0)]);
    }

    #[test]
    fn a_tap_zooms_to_three_times_with_its_point_in_the_middle_as_far_as_the_picture_covers_the_area()
     {
        // The picture's middle (1000, 500): 3× (scale 1.5), its corner at
        // (500 − 1500, 300 − 750).
        let mut viewer = Viewer::START;
        viewer.zoom_on((1000.0, 500.0), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -1000.0, -450.0));
        let shown = viewer.placed(AREA, PICTURE).unwrap();
        assert_eq!(to_picture(shown, (500.0, 300.0)), (1000.0, 500.0));
        // Near a corner: the picture still covers the area.
        viewer.zoom_on((100.0, 50.0), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, 0.0, 0.0));
        viewer.zoom_on((2000.0, 1000.0), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -2000.0, -900.0));
        // From a zoom already: 3× again, about the new point.
        viewer.zoom_on((800.0, 400.0), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -700.0, -300.0));
        // Nothing while the area is empty.
        viewer.zoom_on((1000.0, 500.0), (0.0, 600.0), PICTURE);
        assert_eq!(viewer.view(), view(3.0, -700.0, -300.0));
    }

    #[test]
    fn a_tap_zoom_forgets_a_pinch_under_way() {
        let mut viewer = Viewer::START;
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        viewer.zoom_on((1000.0, 500.0), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -1000.0, -450.0));
        // The same two fingers start a new pinch from what is shown: nothing
        // moves (the old pinch would have put back its own 1×).
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -1000.0, -450.0));
    }

    /// The area of the view tests: 1000 × 600, a 2000 × 1000 picture fitted
    /// at 0.5 (1000 × 500, 50 px above and below).
    const AREA: (f64, f64) = (1000.0, 600.0);
    const PICTURE: (u32, u32) = (2000, 1000);

    fn view(zoom: f64, x: f64, y: f64) -> View {
        View { zoom, pan: (x, y) }
    }

    #[test]
    fn a_view_places_the_picture_covering_the_area_or_centred() {
        let fit = |zoom: f64, x: f64, y: f64| placed(AREA, PICTURE, view(zoom, x, y));
        let at = |scale: f64, left: f64, top: f64| Some(Fit { scale, left, top });
        // The whole picture is the fitted one, wherever the pan says.
        assert_eq!(fit(1.0, 0.0, 0.0), fit_whole());
        assert_eq!(fit(1.0, 0.0, 0.0), at(0.5, 0.0, 50.0));
        assert_eq!(fit(1.0, 37.0, 300.0), at(0.5, 0.0, 50.0));
        assert_eq!(fit(1.0, -37.0, -300.0), at(0.5, 0.0, 50.0));
        // 2×: 2000 × 1000 covers the area; the corner stays within it.
        assert_eq!(fit(2.0, -300.0, -100.0), at(1.0, -300.0, -100.0));
        assert_eq!(fit(2.0, 100.0, 50.0), at(1.0, 0.0, 0.0));
        assert_eq!(fit(2.0, -5000.0, -5000.0), at(1.0, -1000.0, -400.0));
        // 1.125×: 1125 × 562.5, wider than the area but not as tall: it
        // covers the width and is centred in the height.
        assert_eq!(fit(1.125, -112.5, 120.0), at(0.5625, -112.5, 18.75));
        assert_eq!(fit(1.125, -30.0, -80.0), at(0.5625, -30.0, 18.75));
        assert_eq!(fit(1.125, 30.0, 0.0), at(0.5625, 0.0, 18.75));
        assert_eq!(fit(1.125, -500.0, 0.0), at(0.5625, -125.0, 18.75));
        assert_eq!(placed((0.0, 600.0), PICTURE, View::WHOLE), None);
        assert_eq!(placed(AREA, (0, 1000), View::WHOLE), None);
        // One side alone.
        assert_eq!(place(0.0, 800.0, 1000.0), 100.0, "centred");
        assert_eq!(place(120.0, 800.0, 1000.0), 100.0);
        assert_eq!(place(-120.0, 800.0, 1000.0), 100.0);
        assert_eq!(place(-500.0, 2000.0, 1000.0), -500.0);
        assert_eq!(place(10.0, 2000.0, 1000.0), 0.0);
        assert_eq!(place(-1500.0, 2000.0, 1000.0), -1000.0);
        assert_eq!(place(-3.0, 1000.0, 1000.0), 0.0, "just fits");
    }

    fn fit_whole() -> Option<Fit> {
        fit(AREA, PICTURE)
    }

    #[test]
    fn two_points_have_a_midpoint_and_a_distance() {
        assert_eq!(midpoint((400.0, 300.0), (600.0, 320.0)), (500.0, 310.0));
        assert_eq!(midpoint((-10.0, 4.0), (30.0, 8.0)), (10.0, 6.0));
        assert_eq!(distance((1.0, 2.0), (4.0, 6.0)), 5.0);
        assert_eq!(distance((4.0, 6.0), (1.0, 2.0)), 5.0);
        assert_eq!(distance((7.0, 7.0), (7.0, 7.0)), 0.0);
    }

    /// Two fingers 1 and 2 at `a` and `b`.
    fn pair(a: (f64, f64), b: (f64, f64)) -> Option<Pair> {
        Some([(1, a), (2, b)])
    }

    #[test]
    fn a_pinch_zooms_about_the_fingers_midpoint() {
        let mut viewer = Viewer::START;
        assert_eq!(viewer.view(), View::WHOLE);
        // The start: the picture point under the midpoint (500, 300) is
        // (1000, 500); nothing moves yet.
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), View::WHOLE);
        // Twice as far apart: 2×, (1000, 500) still under the midpoint.
        viewer.follow(pair((300.0, 300.0), (700.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(2.0, -500.0, -200.0));
        // Moved together: the picture follows.
        viewer.follow(pair((350.0, 350.0), (750.0, 350.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(2.0, -450.0, -150.0));
        // Three times as far: 3×, the anchor under the new midpoint.
        viewer.follow(pair((250.0, 320.0), (850.0, 320.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(3.0, -950.0, -430.0));
        let shown = viewer.placed(AREA, PICTURE).unwrap();
        assert_eq!(to_picture(shown, (550.0, 320.0)), (1000.0, 500.0));
        // Five times as far: at most 4×.
        viewer.follow(pair((0.0, 320.0), (1000.0, 320.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(4.0, -1500.0, -680.0));
        // Half as far: at least 1×, the whole picture centred.
        viewer.follow(pair((450.0, 320.0), (550.0, 320.0)), AREA, PICTURE);
        assert_eq!(viewer.view().zoom, 1.0);
        assert_eq!(viewer.placed(AREA, PICTURE), fit_whole());
        // The fingers lift at 1×: the whole picture.
        viewer.follow(None, AREA, PICTURE);
        assert_eq!(viewer.view(), View::WHOLE);
        assert_eq!(MAX_ZOOM, 4.0);
    }

    #[test]
    fn a_pinch_keeps_the_picture_covering_the_area() {
        let mut viewer = Viewer::START;
        // The midpoint (100, 50) is the picture's (200, 0): at 2× its top
        // would leave 50 px empty above, so the picture's top stays at the
        // area's.
        viewer.follow(pair((0.0, 50.0), (200.0, 50.0)), AREA, PICTURE);
        viewer.follow(pair((0.0, 50.0), (400.0, 50.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(2.0, 0.0, 0.0));
        // Two fingers on one point start at 1 px apart.
        let mut viewer = Viewer::START;
        viewer.follow(pair((500.0, 300.0), (500.0, 300.0)), AREA, PICTURE);
        viewer.follow(pair((499.0, 300.0), (501.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(2.0, -500.0, -200.0));
        // No picture yet: no pinch starts.
        let mut viewer = Viewer::START;
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, (0, 0));
        viewer.follow(pair((300.0, 300.0), (700.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), View::WHOLE, "the pinch starts here");
        viewer.follow(pair((200.0, 300.0), (800.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view().zoom, 1.5);
    }

    #[test]
    fn new_fingers_start_a_new_pinch_from_what_is_shown() {
        let mut viewer = Viewer::START;
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        viewer.follow(pair((300.0, 300.0), (700.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view(), view(2.0, -500.0, -200.0));
        // Another second finger: a new pinch, nothing jumps; the midpoint
        // (400, 300) is the picture's (900, 500).
        viewer.follow(
            Some([(1, (300.0, 300.0)), (3, (500.0, 300.0))]),
            AREA,
            PICTURE,
        );
        assert_eq!(viewer.view(), view(2.0, -500.0, -200.0));
        viewer.follow(
            Some([(1, (200.0, 300.0)), (3, (600.0, 300.0))]),
            AREA,
            PICTURE,
        );
        assert_eq!(viewer.view(), view(4.0, -1400.0, -700.0));
        // The fingers lift zoomed: the view stays.
        viewer.follow(None, AREA, PICTURE);
        assert_eq!(viewer.view(), view(4.0, -1400.0, -700.0));
        viewer.follow(None, AREA, PICTURE);
        assert_eq!(viewer.view(), view(4.0, -1400.0, -700.0));
        // The same pointers again are a new pinch too.
        viewer.follow(
            Some([(1, (100.0, 100.0)), (3, (300.0, 100.0))]),
            AREA,
            PICTURE,
        );
        assert_eq!(viewer.view(), view(4.0, -1400.0, -700.0));
        // CELÝ EQ: the whole picture, and fingers still there start anew.
        viewer.whole();
        assert_eq!(viewer, Viewer::START);
        viewer.follow(
            Some([(1, (100.0, 100.0)), (3, (300.0, 100.0))]),
            AREA,
            PICTURE,
        );
        assert_eq!(viewer.view(), View::WHOLE);
    }

    #[test]
    fn a_pinch_that_ends_barely_zoomed_shows_the_whole_picture() {
        let mut viewer = Viewer::START;
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        viewer.follow(pair((396.0, 300.0), (604.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view().zoom, 1.04);
        viewer.follow(None, AREA, PICTURE);
        assert_eq!(viewer.view(), View::WHOLE);
        // 1.05× stays.
        viewer.follow(pair((400.0, 300.0), (600.0, 300.0)), AREA, PICTURE);
        viewer.follow(pair((395.0, 300.0), (605.0, 300.0)), AREA, PICTURE);
        assert_eq!(viewer.view().zoom, 1.05);
        viewer.follow(None, AREA, PICTURE);
        assert_eq!(viewer.view().zoom, 1.05);
        assert!(zoomed(1.05));
        assert!(!zoomed(1.05_f64.next_down()));
        assert!(zoomed(4.0));
        assert!(!zoomed(1.0));
        assert_eq!(ZOOMED_FROM, 1.05);
    }

    #[test]
    fn the_overview_takes_the_pictures_shape_within_bounds() {
        assert_eq!((OVERVIEW_HEIGHT, OVERVIEW_ASPECT), (80, (0.4, 2.5)));
        // Pro-Q's own (1.67), an upright phone's (0.49), a tablet's (1.53).
        assert_eq!(overview_size((1349, 809)), (133, 80));
        assert_eq!(overview_size((667, 1361)), (39, 80));
        assert_eq!(overview_size((1293, 844)), (123, 80));
        // At the bounds and past them: no further.
        assert_eq!(overview_size((5, 2)), (200, 80));
        assert_eq!(overview_size((2, 5)), (32, 80));
        assert_eq!(overview_size((2544, 400)), (200, 80));
        assert_eq!(overview_size((100, 3000)), (32, 80));
        // A picture with no size.
        assert_eq!(overview_size((0, 0)), (32, 80));
        assert_eq!(overview_size((40, 0)), (200, 80));
        assert_eq!(overview_size((2, 0)), (160, 80));
    }

    #[test]
    fn the_factor_reads_with_a_decimal_comma() {
        assert_eq!(zoom_text(1.0), "1,0×");
        assert_eq!(zoom_text(2.5), "2,5×");
        assert_eq!(zoom_text(2.44), "2,4×");
        assert_eq!(zoom_text(3.96), "4,0×");
        assert_eq!(zoom_text(4.0), "4,0×");
        // Under 2× (one finger does not edit) it never reads 2,0× (the PR I
        // review).
        assert_eq!(zoom_text(1.97), "1,9×");
        assert_eq!(zoom_text(2.0_f64.next_down()), "1,9×");
        assert_eq!(zoom_text(1.86), "1,9×");
        assert_eq!(zoom_text(1.84), "1,8×");
        assert_eq!(zoom_text(2.0), "2,0×");
        assert_eq!(zoom_text(2.04), "2,0×");
    }

    #[test]
    fn the_screen_draws_a_view_as_the_canvas_transform_and_the_bars_group() {
        // The whole picture: no transform, no group, all of it in sight.
        assert_eq!(
            look(AREA, PICTURE, View::WHOLE),
            Some(Look {
                transform: "translate(0px, 0px) scale(1)".into(),
                zoomed: false,
                edits: false,
                factor: "1,0×".into(),
                seen: (0.0, 0.0, 1.0, 1.0),
            })
        );
        // 2×, its corner at (−500, −200): the canvas's box (the area, the
        // picture fitted 50 px down) scaled 2× and moved so the picture's
        // corner lands there; a quarter in from the left, a fifth down.
        assert_eq!(
            look(AREA, PICTURE, view(2.0, -500.0, -200.0)),
            Some(Look {
                transform: "translate(-500px, -300px) scale(2)".into(),
                zoomed: true,
                edits: true,
                factor: "2,0×".into(),
                seen: (0.25, 0.2, 0.5, 0.6),
            })
        );
        // 1.125×: wider than the area, centred in the height.
        assert_eq!(
            look(AREA, PICTURE, view(1.125, -112.5, 0.0)),
            Some(Look {
                transform: "translate(-112.5px, -37.5px) scale(1.125)".into(),
                zoomed: true,
                edits: false,
                factor: "1,1×".into(),
                seen: (0.1, 0.0, 1000.0 / 0.5625 / 2000.0, 1.0),
            })
        );
        // A low area: the picture fitted at 0.25 (500 × 250, 250 px left
        // and right). 2×: 1000 × 500 just as wide as the area, its corner
        // 100 px up.
        let low = (1000.0, 250.0);
        assert_eq!(
            look(low, PICTURE, View::WHOLE).map(|l| l.transform),
            Some("translate(0px, 0px) scale(1)".to_string())
        );
        assert_eq!(
            look(low, PICTURE, view(2.0, 0.0, -100.0)),
            Some(Look {
                transform: "translate(-500px, -100px) scale(2)".into(),
                zoomed: true,
                edits: true,
                factor: "2,0×".into(),
                seen: (0.0, 0.2, 1.0, 0.5),
            })
        );
        assert_eq!(look((1000.0, 0.0), PICTURE, View::WHOLE), None);
    }

    #[test]
    fn a_point_of_the_area_is_mapped_through_the_view() {
        let shown = placed(AREA, PICTURE, view(2.0, -500.0, -200.0)).unwrap();
        assert_eq!(
            point(shown, (300.0, 100.0)),
            Point {
                area: (300.0, 100.0),
                picture: (800.0, 300.0)
            }
        );
        let whole = fit_whole().unwrap();
        assert_eq!(point(whole, (500.0, 300.0)).picture, (1000.0, 500.0));
    }

    #[test]
    fn a_card_opens_only_while_free_or_mine_and_the_socket_takes_it() {
        assert!(can_open(CardLock::Free, true));
        assert!(can_open(CardLock::Mine, true));
        assert!(
            !can_open(CardLock::Other(1.0), true),
            "another page holds it"
        );
        assert!(
            !can_open(CardLock::InUse(1.0), true),
            "another page holds another one"
        );
        assert!(!can_open(CardLock::Free, false), "the socket is down");
        assert!(!can_open(CardLock::Mine, false));
        assert!(!can_open(CardLock::Other(1.0), false));
    }

    #[test]
    fn the_cards_are_listed_at_the_open_and_whenever_the_link_comes_back() {
        let link = |connected: bool, online: bool| ListLink { connected, online };
        let up = link(true, true);
        // The detail opens: listed while connected (an offline instance's
        // list then says so); not while the socket is down.
        assert!(lists_now(None, up));
        assert!(lists_now(None, link(true, false)));
        assert!(!lists_now(None, link(false, true)));
        assert!(!lists_now(None, link(false, false)));
        // Nothing changed: no list (another instance's busy flag, say).
        assert!(!lists_now(Some(up), up));
        // Live goes away and comes back (the socket stays): listed again
        // once it is online.
        assert!(!lists_now(Some(up), link(true, false)));
        assert!(lists_now(Some(link(true, false)), up));
        // The page's socket reconnects: the hello alone (every instance
        // offline until the hub's states) lists nothing; the instance's
        // state does.
        assert!(!lists_now(Some(up), link(false, false)));
        assert!(!lists_now(Some(link(false, false)), link(true, false)));
        assert!(lists_now(Some(link(true, false)), up));
        assert!(lists_now(Some(link(false, true)), up));
        assert!(lists_now(Some(link(false, false)), up));
        // Offline or disconnected stays quiet.
        assert!(!lists_now(Some(link(true, false)), link(true, false)));
        assert!(!lists_now(Some(link(false, false)), link(false, true)));
    }

    #[test]
    fn a_card_and_its_screen_name_the_editor_alike() {
        assert_eq!(place_text("na tracku", "Pro-Q 4"), "Pro-Q 4 · na tracku");
        assert_eq!(
            place_text("Vocal FX › Main", "Pro-Q 4"),
            "Pro-Q 4 · Vocal FX › Main"
        );
        // A renamed device's name after where it sits.
        assert_eq!(
            place_text("Vocal FX › Main", "De-ess"),
            "Pro-Q 4 · Vocal FX › Main · De-ess"
        );
        assert_eq!(
            place_text("na tracku", "Air EQ"),
            "Pro-Q 4 · na tracku · Air EQ"
        );
    }

    #[test]
    fn a_list_the_hub_did_not_give_says_why_and_the_screen_off_plainly() {
        assert_eq!(list_text("off"), "EQ je na PC vypnuté.");
        assert_eq!(
            list_text("unknown instance"),
            "Pro-Q 4 sa nedá prečítať: neznámy Live"
        );
        assert_eq!(list_text(""), "Pro-Q 4 sa nedá prečítať: ");
    }

    #[test]
    fn a_lists_reason_reads_in_slovak_as_a_cards_note_does() {
        assert_eq!(
            list_text("instance offline"),
            "Pro-Q 4 sa nedá prečítať: Live je nedostupný, skús znova"
        );
        assert_eq!(
            list_text("Live did not answer"),
            "Pro-Q 4 sa nedá prečítať: Live neodpovedá, skús znova"
        );
        assert_eq!(
            list_text("the devices nest too deep"),
            "Pro-Q 4 sa nedá prečítať: zariadenia sú vnorené príliš hlboko"
        );
        // The protocol's names, through the same table as a card's note.
        for why in [
            reason::OFFLINE,
            reason::NO_ANSWER,
            reason::UNKNOWN_INSTANCE,
            reason::MOVED,
        ] {
            let note = failure_text(why).unwrap_or_default();
            assert_eq!(list_text(why), format!("Pro-Q 4 sa nedá prečítať: {note}"));
        }
        // Only an unexpected reason (the script's own words) stays raw.
        assert_eq!(
            list_text("not found: tracks[name=Nobody #]"),
            "Pro-Q 4 sa nedá prečítať: not found: tracks[name=Nobody #]"
        );
    }

    #[test]
    fn a_changed_area_goes_once_it_rested_300_ms_while_the_editor_is_open() {
        assert_eq!((AREA_STEP, AREA_SETTLE_MS), (1.0, 300.0));
        let (wide, tall) = ((1194.0, 790.0), (390.0, 796.0));
        let mut watch = AreaWatch::opened(wide);
        assert_eq!(watch.frame(wide, true, 0.0), None, "the open carried it");
        assert_eq!(
            watch.frame(wide, true, 999.0),
            None,
            "rested, and still never sent: the hub has it"
        );
        // A phone turned: once it rested.
        assert_eq!(watch.frame(tall, true, 1000.0), None);
        assert_eq!(watch.frame(tall, true, 1300.0_f64.next_down()), None);
        assert_eq!(watch.frame(tall, true, 1300.0), Some(tall));
        assert_eq!(watch.frame(tall, true, 2000.0), None, "sent once");
        // Each change starts the rest again; the last one goes.
        assert_eq!(watch.frame((500.0, 700.0), true, 3000.0), None);
        assert_eq!(watch.frame(wide, true, 3200.0), None);
        assert_eq!(watch.frame(wide, true, 3499.0), None);
        assert_eq!(watch.frame(wide, true, 3500.0), Some(wide));
        // Back where it was before it rested: nothing.
        assert_eq!(watch.frame(tall, true, 4000.0), None);
        assert_eq!(watch.frame(wide, true, 4100.0), None);
        assert_eq!(watch.frame(wide, true, 4400.0), None);
        assert_eq!(watch.frame(wide, true, 9000.0), None);
    }

    #[test]
    fn an_area_waits_for_the_open_editor_and_a_sub_pixel_wobble_is_no_change() {
        let mut watch = AreaWatch::opened((1194.0, 790.0));
        // Opening: kept, sent once the editor is open.
        assert_eq!(watch.frame((390.0, 796.0), false, 0.0), None);
        assert_eq!(watch.frame((390.0, 796.0), false, 500.0), None);
        assert_eq!(
            watch.frame((390.0, 796.0), true, 501.0),
            Some((390.0, 796.0))
        );
        // Under a pixel on each side: no change; a pixel on either: one.
        // (A side 0.9 px off: `390.0 + 1.0_f64.next_down()` rounds to
        // 391.0 itself, a whole pixel.)
        let wobble = (390.9, 795.1);
        assert_eq!(watch.frame(wobble, true, 600.0), None);
        assert_eq!(watch.frame(wobble, true, 1000.0), None);
        assert_eq!(watch.frame((391.0, 796.0), true, 1100.0), None);
        assert_eq!(
            watch.frame((391.0, 796.0), true, 1400.0),
            Some((391.0, 796.0))
        );
        assert_eq!(watch.frame((391.0, 795.0), true, 1500.0), None);
        assert_eq!(
            watch.frame((391.0, 795.0), true, 1800.0),
            Some((391.0, 795.0))
        );
        assert!(reshaped((10.0, 10.0), (9.0, 10.0)));
        assert!(reshaped((10.0, 10.0), (10.0, 11.0)));
        assert!(!reshaped((10.0, 10.0), (10.5, 9.5)));
        assert!(!reshaped((0.0, 0.0), (1.0_f64.next_down(), 0.0)));
        assert!(!reshaped((0.0, 0.0), (0.0, -1.0_f64.next_down())));
        assert!(rested(0.0, 300.0));
        assert!(!rested(0.0, 300.0_f64.next_down()));
        // Nothing sent yet (no open): the first area waits its rest too.
        let mut fresh = AreaWatch::default();
        assert_eq!(fresh.frame((800.0, 600.0), true, 0.0), None);
        assert_eq!(
            fresh.frame((800.0, 600.0), true, 300.0),
            Some((800.0, 600.0))
        );
    }
}
