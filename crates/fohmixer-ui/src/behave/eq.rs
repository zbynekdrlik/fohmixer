//! The Pro-Q 4 screen's finger and picture (#71 PR E, F28), pure: where the
//! editor's picture sits fitted into the screen ([`fit`], the stylesheet's
//! `object-fit: contain`), a point of the screen in the picture's pixels
//! ([`to_picture`]), the one finger the screen takes ([`Finger`]: a second
//! finger is ignored, a move goes at most once per animation frame, the
//! newest one, and the end always goes, a lost capture as a cancel), a
//! card's lock ([`card_lock`]) and its text, which frames the screen shows
//! ([`shows_frame`]) and a card's picture URL ([`picture_url`]).

use fohmixer_proto::eq::{EqLock, Touch};

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
}

/// The lock of the editor at `path` on `instance`.
pub fn card_lock(locks: &[EqLock], instance: &str, path: &str) -> CardLock {
    match locks
        .iter()
        .find(|lock| lock.instance == instance && lock.path == path)
    {
        None => CardLock::Free,
        Some(lock) if lock.mine => CardLock::Mine,
        Some(lock) => CardLock::Other(lock.since),
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

/// The line under a card after its open failed (`reason`, the hub's); none
/// for a close the page or the hub made on purpose.
pub fn failure_text(reason: &str) -> Option<String> {
    let quiet = ["exit", "switch", "detach", "socket"];
    (!quiet.contains(&reason)).then(|| format!("Neotvoril sa: {reason}"))
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
        let locks = [lock("a", true), lock("b", false)];
        assert_eq!(card_lock(&locks, "band", "a"), CardLock::Mine);
        assert_eq!(
            card_lock(&locks, "band", "b"),
            CardLock::Other(1_790_000_000_000.0)
        );
        assert_eq!(card_lock(&locks, "band", "c"), CardLock::Free);
        assert_eq!(card_lock(&locks, "master", "a"), CardLock::Free);
        assert_eq!(hh_mm(9, 5), "09:05");
        assert_eq!(hh_mm(12, 41), "12:41");
        assert_eq!(locked_text("12:41"), "Upravuje ho iný zvukár (od 12:41)");
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
        for quiet in ["exit", "switch", "detach", "socket"] {
            assert_eq!(failure_text(quiet), None, "{quiet}");
        }
        assert_eq!(
            failure_text("no window"),
            Some("Neotvoril sa: no window".to_string())
        );
        assert_eq!(
            failure_text("locked"),
            Some("Neotvoril sa: locked".to_string())
        );
    }
}
