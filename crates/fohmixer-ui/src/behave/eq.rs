//! The Pro-Q 4 screen's fingers and picture (#71 PR E, F28), pure: where the
//! editor's picture sits fitted into the screen ([`fit`], the stylesheet's
//! `object-fit: contain`), the page's own view of it (PR F: [`View`], a
//! zoom of 1 to [`MAX_ZOOM`] and a pan kept by [`placed`]; [`Viewer`], the
//! pinch that moves it; [`look`], how the screen draws it), a point of the
//! screen in the picture's pixels ([`to_picture`], [`point`]), the fingers
//! ([`Finger`]: a first finger waits [`HOLD_MS`] or [`SLOP`] before the
//! editor gets it, a lift meanwhile is a tap (PR G: its down at once, its
//! up [`TAP_MS`] later on a frame), a second finger makes a pinch
//! that sends the editor nothing, a move goes at most once per animation
//! frame, the newest one, and the end always goes, a lost capture as a
//! cancel, a hidden page's finger lifted), when the page's picture area
//! goes to the hub (PR G: [`AreaWatch`]), a card's lock ([`card_lock`]:
//! one editor on the PC's screen at a time, so another page's editor locks
//! every card) and its text, whether its open is offered ([`can_open`]),
//! when the cards are listed ([`lists_now`]), what names an editor
//! ([`place_text`]), a card's note and the cards' list note in Slovak
//! ([`failure_text`], [`list_text`]), which frames the screen shows
//! ([`shows_frame`]) and a card's picture URL ([`picture_url`]).

use fohmixer_proto::eq::{EqLock, PRODUCT, Touch, reason};

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
}

/// Whether the view counts as zoomed ([`ZOOMED_FROM`]).
pub fn zoomed(zoom: f64) -> bool {
    zoom >= ZOOMED_FROM
}

/// The zoom as the bar's factor reads it: one decimal, a decimal comma
/// (`2,4×`).
pub fn zoom_text(zoom: f64) -> String {
    format!("{zoom:.1}×").replace('.', ",")
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
        factor: zoom_text(view.zoom),
        seen: (
            (-shown.left / shown.scale).max(0.0) / width,
            (-shown.top / shown.scale).max(0.0) / height,
            (area.0 / shown.scale).min(width) / width,
            (area.1 / shown.scale).min(height) / height,
        ),
    })
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
    /// The first finger is on the editor (`moved`: a move not sent yet).
    Touching { moved: bool },
    /// The first finger stays off the editor: it went down off the picture.
    Ignored,
    /// Two fingers pinch (one may have lifted since): nothing reaches the
    /// editor until every finger lifted.
    Pinching,
}

/// The fingers on the picture (#71 PR E, PR F): the first one reaches the
/// editor once it waited or slid, two of them pinch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Finger {
    stage: Stage,
    /// The fingers down, at most two: each one's pointer and its last point.
    fingers: [Option<(i32, Point)>; 2],
    /// A quick tap's up still to go (PR G): its point, and when its down
    /// went (page ms).
    tap: Option<(Point, f64)>,
}

impl Finger {
    /// Where `pointer` is among the fingers.
    fn slot(&self, pointer: i32) -> Option<usize> {
        self.fingers
            .iter()
            .position(|f| matches!(f, Some((id, _)) if *id == pointer))
    }

    /// Whether `pointer` is one of the fingers (the glue captures it).
    pub fn has(&self, pointer: i32) -> bool {
        self.slot(pointer).is_some()
    }

    /// A pointer's down at `at` (`on`: on the picture; `now`: page ms). A
    /// first finger waits: nothing goes yet. A second one makes a pinch: a
    /// first finger already on the editor gets its cancel at its last
    /// point. A third one, or a pointer already down, is nothing.
    pub fn down(&mut self, pointer: i32, at: Point, on: bool, now: f64) -> Vec<Out> {
        if self.has(pointer) {
            return Vec::new();
        }
        match self.stage {
            Stage::Idle => {
                *self = Self {
                    stage: Stage::Waiting {
                        first: at,
                        on,
                        since: now,
                    },
                    fingers: [Some((pointer, at)), None],
                    tap: self.tap,
                };
                Vec::new()
            }
            Stage::Touching { .. } => {
                let ended = self.last(Touch::Cancel);
                self.join(pointer, at);
                ended
            }
            Stage::Waiting { .. } | Stage::Ignored | Stage::Pinching => {
                self.join(pointer, at);
                Vec::new()
            }
        }
    }

    /// A finger joins the pinch in a free place (a third one finds none).
    fn join(&mut self, pointer: i32, at: Point) {
        if let Some(free) = self.fingers.iter_mut().find(|f| f.is_none()) {
            *free = Some((pointer, at));
            self.stage = Stage::Pinching;
        }
    }

    /// The first finger's last point as `touch`.
    fn last(&self, touch: Touch) -> Vec<Out> {
        self.fingers[0]
            .map(|(_, at)| vec![sent(touch, at)])
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
        self.fingers[slot] = Some((pointer, at));
        if let Stage::Touching { moved } = &mut self.stage {
            *moved = true;
        }
    }

    /// A quick tap's up still to go, now (PR G: before another down, or
    /// when the fingers leave).
    fn tap_up(&mut self) -> Option<Out> {
        self.tap.take().map(|(at, _)| sent(Touch::Up, at))
    }

    /// The frame (`now`: page ms). A quick tap's up goes once [`TAP_MS`]
    /// passed since its down. A waiting finger that waited [`HOLD_MS`] or
    /// slid past [`SLOP`] reaches the editor: its down at its first point
    /// (a tap's up still to go before it), then its move to where it is now
    /// (nothing when it went down off the picture). A finger on the editor
    /// sends the move not sent yet, the newest.
    pub fn frame(&mut self, now: f64) -> Vec<Out> {
        let mut out = Vec::new();
        if self.tap.is_some_and(|(_, since)| tapped(since, now)) {
            out.extend(self.tap_up());
        }
        match (self.stage, self.fingers[0]) {
            (Stage::Waiting { first, on, since }, Some((_, at)))
                if waited(since, now) || slid(first.area, at.area) =>
            {
                if !on {
                    self.stage = Stage::Ignored;
                    return out;
                }
                self.stage = Stage::Touching { moved: false };
                out.extend(self.tap_up());
                out.push(sent(Touch::Down, first));
                if at != first {
                    out.push(sent(Touch::Move, at));
                }
            }
            (Stage::Touching { moved: true }, Some((_, at))) => {
                self.stage = Stage::Touching { moved: false };
                out.push(sent(Touch::Move, at));
            }
            _ => {}
        }
        out
    }

    /// A pointer lifts at `at` (`now`: page ms): a waiting finger's tap
    /// (PR G: its down at its first point at once, its up here on the first
    /// frame [`TAP_MS`] later, a tap's up still to go before it), the first
    /// finger's up here; a pinch's fingers send nothing.
    pub fn up(&mut self, pointer: i32, at: Point, now: f64) -> Vec<Out> {
        let Some(slot) = self.slot(pointer) else {
            return Vec::new();
        };
        let stage = self.stage;
        let out = match stage {
            Stage::Waiting {
                first, on: true, ..
            } => {
                let mut out: Vec<Out> = self.tap_up().into_iter().collect();
                out.push(sent(Touch::Down, first));
                self.tap = Some((at, now));
                out
            }
            Stage::Touching { .. } => vec![sent(Touch::Up, at)],
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
    /// go goes now, a cancel for a finger on the editor; every finger is
    /// forgotten.
    pub fn leave(&mut self) -> Vec<Out> {
        let mut out: Vec<Out> = self.tap_up().into_iter().collect();
        if matches!(self.stage, Stage::Touching { .. }) {
            out.extend(self.last(Touch::Cancel));
        }
        *self = Self::default();
        out
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

    /// The two fingers of a pinch while both are down (in their places'
    /// order), on the area: what moves the view ([`Viewer::follow`]).
    pub fn pair(&self) -> Option<Pair> {
        match self.fingers {
            [Some((a, at_a)), Some((b, at_b))] => Some([(a, at_a.area), (b, at_b.area)]),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A finger on the editor: down at `at` at 0 ms, the down sent at the
    /// hold.
    fn touching(finger: &mut Finger, pointer: i32, at: Point) {
        assert_eq!(finger.down(pointer, at, true, 0.0), vec![]);
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
        assert_eq!(finger.down(6, p(3.0, 3.0), true, 400.0), vec![]);
        assert_eq!(finger.pair(), None);
        assert_eq!(finger.frame(401.0), vec![]);
    }

    #[test]
    fn a_first_finger_waits_then_goes_down_where_it_landed() {
        let mut finger = Finger::default();
        assert_eq!(finger.down(1, p(10.0, 20.0), true, 1000.0), vec![]);
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
        assert_eq!(finger.down(2, p(10.0, 20.0), true, 2000.0), vec![]);
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
        assert_eq!(finger.down(1, p(10.0, 20.0), true, 0.0), vec![]);
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
        finger.down(2, p(10.0, 20.0), true, 0.0);
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
        finger.down(1, p(10.0, 20.0), true, 0.0);
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
        finger.down(2, p(10.0, 20.0), true, 600.0);
        assert_eq!(
            finger.up(2, p(10.0, 20.0), 610.0),
            vec![(Touch::Down, 20.0, 40.0)]
        );
        finger.down(3, p(30.0, 20.0), true, 615.0);
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
        finger.down(4, p(1.0, 1.0), true, 800.0);
        assert_eq!(
            finger.up(4, p(1.0, 1.0), 805.0),
            vec![(Touch::Down, 2.0, 2.0)]
        );
        finger.down(5, p(3.0, 3.0), true, 810.0);
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
        finger.down(11, p(1.0, 1.0), true, 920.0);
        finger.up(11, p(1.0, 1.0), 921.0);
        finger.down(12, p(3.0, 3.0), true, 925.0);
        assert_eq!(finger.up(12, p(3.0, 3.0), 930.0), vec![]);
        finger.down(13, p(5.0, 5.0), true, 935.0);
        assert_eq!(finger.up(13, p(5.0, 5.0), 940.0), vec![]);
        assert_eq!(
            finger.frame(961.0),
            vec![(Touch::Up, 2.0, 2.0), (Touch::Down, 6.0, 6.0)]
        );
        assert_eq!(finger.frame(1001.0), vec![(Touch::Up, 6.0, 6.0)]);
        assert_eq!(finger.frame(1100.0), vec![]);
        // A lift after the up is due, before a frame sent it: that up,
        // then the new down, at once.
        finger.down(14, p(1.0, 1.0), true, 1200.0);
        finger.up(14, p(1.0, 1.0), 1201.0);
        finger.down(15, p(3.0, 3.0), true, 1230.0);
        assert_eq!(
            finger.up(15, p(3.0, 3.0), 1241.0),
            vec![(Touch::Up, 2.0, 2.0), (Touch::Down, 6.0, 6.0)]
        );
        assert_eq!(finger.frame(1280.0), vec![]);
        assert_eq!(finger.frame(1281.0), vec![(Touch::Up, 6.0, 6.0)]);
        // Leaving the screen, or a hidden page, sends a tap's up at once;
        // a tap still waiting for it never went, so nothing of it goes.
        finger.down(6, p(1.0, 1.0), true, 1400.0);
        finger.up(6, p(1.0, 1.0), 1401.0);
        finger.down(16, p(3.0, 3.0), true, 1405.0);
        assert_eq!(finger.up(16, p(3.0, 3.0), 1410.0), vec![]);
        assert_eq!(finger.leave(), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.frame(1500.0), vec![]);
        finger.down(7, p(1.0, 1.0), true, 1600.0);
        finger.up(7, p(1.0, 1.0), 1601.0);
        finger.down(17, p(3.0, 3.0), true, 1605.0);
        assert_eq!(finger.up(17, p(3.0, 3.0), 1610.0), vec![]);
        assert_eq!(finger.visibility(true), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.frame(1700.0), vec![]);
        // A pinch that starts meanwhile: the tap's up goes on its time.
        finger.down(8, p(1.0, 1.0), true, 1800.0);
        finger.up(8, p(1.0, 1.0), 1801.0);
        assert_eq!(finger.down(9, p(5.0, 5.0), true, 1805.0), vec![]);
        assert_eq!(finger.down(10, p(9.0, 9.0), true, 1806.0), vec![]);
        assert_eq!(finger.frame(1840.0), vec![]);
        assert_eq!(finger.frame(1841.0), vec![(Touch::Up, 2.0, 2.0)]);
        assert_eq!(finger.up(9, p(5.0, 5.0), 1850.0), vec![]);
        assert_eq!(finger.up(10, p(9.0, 9.0), 1851.0), vec![]);
        assert!(!finger.held());
        // A waiting finger cancelled (or its capture lost) never reached
        // the editor: nothing goes.
        finger.down(2, p(10.0, 20.0), true, 0.0);
        assert_eq!(finger.cancel(3), vec![], "another pointer");
        assert!(finger.held());
        assert_eq!(finger.cancel(2), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.frame(500.0), vec![]);
        assert_eq!(finger.up(2, p(10.0, 20.0), 500.0), vec![]);
        // Nor when the screen goes or the page hides.
        finger.down(4, p(10.0, 20.0), true, 0.0);
        assert_eq!(finger.leave(), vec![]);
        assert!(!finger.held());
        finger.down(5, p(10.0, 20.0), true, 0.0);
        assert_eq!(finger.visibility(true), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.frame(500.0), vec![]);
    }

    #[test]
    fn a_finger_off_the_picture_never_reaches_the_editor_but_pinches() {
        let mut finger = Finger::default();
        finger.down(1, p(1.0, 1.0), false, 0.0);
        assert!(finger.held());
        assert_eq!(finger.frame(HOLD_MS), vec![], "held: nothing");
        finger.moved(1, p(50.0, 1.0));
        assert_eq!(finger.frame(200.0), vec![]);
        assert_eq!(finger.cancel(1), vec![]);
        assert!(!finger.held());
        // Slid off at once: nothing either.
        finger.down(2, p(1.0, 1.0), false, 0.0);
        finger.moved(2, p(50.0, 1.0));
        assert_eq!(finger.frame(1.0), vec![]);
        assert_eq!(finger.up(2, p(50.0, 1.0), 0.0), vec![]);
        // A tap off the picture: nothing.
        finger.down(3, p(1.0, 1.0), false, 0.0);
        assert_eq!(finger.up(3, p(1.0, 1.0), 0.0), vec![]);
        assert!(!finger.held());
        // Off the picture, then a second finger: a pinch.
        finger.down(4, p(1.0, 1.0), false, 0.0);
        assert_eq!(finger.frame(HOLD_MS), vec![]);
        assert_eq!(finger.down(5, p(9.0, 9.0), true, 130.0), vec![]);
        assert_eq!(finger.pair(), Some([(4, (1.0, 1.0)), (5, (9.0, 9.0))]));
    }

    #[test]
    fn a_second_finger_while_the_first_waits_pinches_and_sends_nothing() {
        let mut finger = Finger::default();
        finger.down(1, p(10.0, 20.0), true, 0.0);
        assert_eq!(finger.pair(), None, "one finger");
        assert_eq!(finger.down(2, p(30.0, 20.0), true, 50.0), vec![]);
        assert!(finger.has(2));
        assert_eq!(finger.pair(), Some([(1, (10.0, 20.0)), (2, (30.0, 20.0))]));
        assert_eq!(finger.frame(500.0), vec![], "no down after the hold");
        finger.moved(1, p(0.0, 20.0));
        finger.moved(2, p(40.0, 21.0));
        assert_eq!(finger.frame(510.0), vec![], "no move");
        assert_eq!(finger.pair(), Some([(1, (0.0, 20.0)), (2, (40.0, 21.0))]));
        // A third finger is nothing.
        assert_eq!(finger.down(3, p(5.0, 5.0), true, 520.0), vec![]);
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
        assert_eq!(finger.down(4, p(7.0, 7.0), true, 610.0), vec![]);
        assert_eq!(finger.pair(), Some([(4, (7.0, 7.0)), (2, (60.0, 20.0))]));
        assert_eq!(finger.cancel(2), vec![]);
        assert_eq!(finger.up(4, p(7.0, 7.0), 0.0), vec![]);
        assert!(!finger.held());
        // The pinch is over: the next finger is a first finger again.
        assert_eq!(finger.down(5, p(10.0, 20.0), true, 700.0), vec![]);
        assert_eq!(finger.frame(820.0), vec![(Touch::Down, 20.0, 40.0)]);
    }

    #[test]
    fn a_second_finger_on_a_touch_cancels_it_where_it_was_then_pinches() {
        let mut finger = Finger::default();
        touching(&mut finger, 1, p(10.0, 20.0));
        finger.moved(1, p(11.0, 22.0));
        assert_eq!(
            finger.down(2, p(40.0, 20.0), true, 130.0),
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
        finger.down(4, p(40.0, 20.0), true, 130.0);
        assert_eq!(finger.cancel(3), vec![]);
        assert_eq!(finger.visibility(true), vec![]);
        assert!(!finger.held());
        assert_eq!(finger.up(4, p(40.0, 20.0), 0.0), vec![]);
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
        ] {
            assert_eq!(said(named), said(word), "{word}");
            assert!(!said(named).starts_with("Neotvoril sa"), "{word}");
        }
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
    fn the_factor_reads_with_a_decimal_comma() {
        assert_eq!(zoom_text(1.0), "1,0×");
        assert_eq!(zoom_text(2.5), "2,5×");
        assert_eq!(zoom_text(2.44), "2,4×");
        assert_eq!(zoom_text(3.96), "4,0×");
        assert_eq!(zoom_text(4.0), "4,0×");
    }

    #[test]
    fn the_screen_draws_a_view_as_the_canvas_transform_and_the_bars_group() {
        // The whole picture: no transform, no group, all of it in sight.
        assert_eq!(
            look(AREA, PICTURE, View::WHOLE),
            Some(Look {
                transform: "translate(0px, 0px) scale(1)".into(),
                zoomed: false,
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
