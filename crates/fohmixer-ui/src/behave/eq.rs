//! The Pro-Q 4 screen's finger and picture (#71 PR E, F28), pure: where the
//! editor's picture sits fitted into the screen ([`fit`], the stylesheet's
//! `object-fit: contain`), a point of the screen in the picture's pixels
//! ([`to_picture`]), the one finger the screen takes ([`Finger`]: a second
//! finger is ignored, a move goes at most once per animation frame, the
//! newest one, and the end always goes, a lost capture as a cancel, a
//! hidden page's finger lifted), a card's lock ([`card_lock`]: one editor on
//! the PC's screen at a time, so another page's editor locks every card)
//! and its text, whether its open is offered ([`can_open`]), when the cards
//! are listed ([`lists_now`]), what names an editor ([`place_text`]), a
//! card's note and the cards' list note in Slovak ([`failure_text`],
//! [`list_text`]), which frames the screen shows ([`shows_frame`]) and a
//! card's picture URL ([`picture_url`]).

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

/// What a finger sends: its phase and its point (picture pixels).
pub type Out = (Touch, f64, f64);

/// The one finger on the picture.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Finger {
    pointer: Option<i32>,
    at: (f64, f64),
    /// A move not sent yet.
    moved: bool,
}

impl Finger {
    /// A pointer's down at `at` (`on`: on the picture): the finger, unless
    /// one is down already (a second finger) or it missed the picture.
    pub fn down(&mut self, pointer: i32, at: (f64, f64), on: bool) -> Option<Out> {
        if self.pointer.is_some() || !on {
            return None;
        }
        *self = Self {
            pointer: Some(pointer),
            at,
            moved: false,
        };
        Some((Touch::Down, at.0, at.1))
    }

    /// The finger moved to `at` (another pointer's move is nothing): it goes
    /// at the next frame, the newest one only.
    pub fn moved(&mut self, pointer: i32, at: (f64, f64)) {
        if self.pointer == Some(pointer) {
            self.at = at;
            self.moved = true;
        }
    }

    /// The frame: the move not sent yet, if any.
    pub fn frame(&mut self) -> Option<Out> {
        if !self.moved {
            return None;
        }
        self.moved = false;
        Some((Touch::Move, self.at.0, self.at.1))
    }

    /// The finger lifts at `at`: its up, there.
    pub fn up(&mut self, pointer: i32, at: (f64, f64)) -> Option<Out> {
        if self.pointer != Some(pointer) {
            return None;
        }
        *self = Self::default();
        Some((Touch::Up, at.0, at.1))
    }

    /// The finger's touch is cancelled (or its capture lost): a cancel
    /// where it was.
    pub fn cancel(&mut self, pointer: i32) -> Option<Out> {
        if self.pointer != Some(pointer) {
            return None;
        }
        let at = self.at;
        *self = Self::default();
        Some((Touch::Cancel, at.0, at.1))
    }

    /// The screen goes away under the finger: a cancel, if one is down.
    pub fn leave(&mut self) -> Option<Out> {
        let pointer = self.pointer?;
        self.cancel(pointer)
    }

    /// The page went hidden (`hidden`) or was shown again: a hidden page
    /// lifts its finger (a cancel, if one is down), as the Stream Deck's
    /// keys go up. A hidden page still pings, so the hub's 2 s silence would
    /// never end the PC's contact.
    pub fn visibility(&mut self, hidden: bool) -> Option<Out> {
        if hidden { self.leave() } else { None }
    }

    /// Whether a finger is down.
    pub fn held(&self) -> bool {
        self.pointer.is_some()
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

    #[test]
    fn one_finger_moves_once_a_frame_and_always_ends() {
        let mut finger = Finger::default();
        assert_eq!(finger.down(1, (5.0, 6.0), false), None, "off the picture");
        assert!(!finger.held());
        assert_eq!(
            finger.down(1, (5.0, 6.0), true),
            Some((Touch::Down, 5.0, 6.0))
        );
        assert!(finger.held());
        assert_eq!(finger.down(2, (9.0, 9.0), true), None, "a second finger");
        assert_eq!(finger.frame(), None, "nothing moved");
        finger.moved(2, (9.0, 9.0));
        assert_eq!(finger.frame(), None, "another finger's move");
        finger.moved(1, (7.0, 6.0));
        finger.moved(1, (8.0, 6.0));
        assert_eq!(finger.frame(), Some((Touch::Move, 8.0, 6.0)), "the newest");
        assert_eq!(finger.frame(), None, "sent once");
        assert_eq!(finger.up(2, (1.0, 1.0)), None);
        finger.moved(1, (9.0, 6.0));
        assert_eq!(finger.up(1, (10.0, 6.0)), Some((Touch::Up, 10.0, 6.0)));
        assert_eq!(
            finger.frame(),
            None,
            "a move before the up is not sent after it"
        );
        assert_eq!(finger.cancel(1), None, "its lost capture after the up");
        assert_eq!(finger.leave(), None);
        // A cancel ends it where it was.
        finger.down(3, (1.0, 2.0), true);
        finger.moved(3, (4.0, 5.0));
        assert_eq!(finger.cancel(4), None);
        assert_eq!(finger.cancel(3), Some((Touch::Cancel, 4.0, 5.0)));
        assert!(!finger.held());
        // Leaving the screen with a finger down cancels it.
        finger.down(5, (1.0, 1.0), true);
        assert_eq!(finger.leave(), Some((Touch::Cancel, 1.0, 1.0)));
        assert_eq!(finger.leave(), None);
        assert_eq!(
            finger.down(6, (2.0, 2.0), true),
            Some((Touch::Down, 2.0, 2.0))
        );
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
        assert!(!locked(CardLock::Free) && !locked(CardLock::Mine));
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
        assert!(!lock_refusal("exit") && !lock_refusal("moved"));
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
        assert_eq!(finger.visibility(true), None, "no finger down");
        finger.down(4, (10.0, 20.0), true);
        finger.moved(4, (11.0, 21.0));
        assert_eq!(finger.visibility(false), None, "shown: it stays down");
        assert!(finger.held());
        assert_eq!(
            finger.visibility(true),
            Some((Touch::Cancel, 11.0, 21.0)),
            "hidden: a cancel where it was"
        );
        assert!(!finger.held());
        assert_eq!(finger.frame(), None, "its last move is not sent after");
        assert_eq!(finger.up(4, (12.0, 22.0)), None, "its lift sends nothing");
        assert_eq!(finger.visibility(true), None);
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
}
