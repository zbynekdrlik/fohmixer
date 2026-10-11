//! The part of the Pro-Q 4 picture the page asks the hub for (#71 PR L),
//! pure: at 2× and more ([`super::eq::edits`]) the screen shows a part of
//! the picture, so the hub needs to send only that part, with a margin for
//! a small pan ([`crop_of`]); the page tells the hub once the view rested
//! ([`ViewWatch`]). On a link slower than the frames, a part a quarter of
//! the picture is a quarter of each frame's transmit time, and of a pong's
//! wait behind it (the design on #71, comment 6107067321).

use fohmixer_proto::eq::Crop;

use super::eq::Look;

/// The margin a crop keeps around the part in sight on each side, as a
/// fraction of that part's size.
pub const CROP_MARGIN: f64 = 0.25;

/// A crop's sides lie on this grid of picture pixels (JPEG's 16 px blocks:
/// a small pan within the margin asks for nothing new).
pub const CROP_GRID: f64 = 16.0;

/// How long a changed view rests before the hub hears its crop (ms).
pub const VIEW_SETTLE_MS: f64 = 150.0;

/// A side's part from `start`, `size` px long, with the margin, on the
/// grid, within `0..=whole` (px): its first and its end pixel.
fn span(start: f64, size: f64, whole: f64) -> (u32, u32) {
    let margin = size * CROP_MARGIN;
    let from = ((start - margin).max(0.0) / CROP_GRID).floor() * CROP_GRID;
    let to = ((start + size + margin) / CROP_GRID).ceil() * CROP_GRID;
    // `as` saturates; the picture's sides are whole pixels.
    (from as u32, to.min(whole) as u32)
}

/// The part of a picture of `picture` px the hub sends for `look`: none
/// under 2× (the whole picture: one finger does not edit there, and the
/// overview shows it all) and when the part with its margin is the whole
/// picture.
pub fn crop_of(picture: (u32, u32), look: &Look) -> Option<Crop> {
    if !look.edits {
        return None;
    }
    let (width, height) = (f64::from(picture.0), f64::from(picture.1));
    let (left, top, seen_w, seen_h) = look.seen;
    let (x, right) = span(left * width, seen_w * width, width);
    let (y, bottom) = span(top * height, seen_h * height, height);
    let crop = Crop {
        x,
        y,
        w: right.saturating_sub(x),
        h: bottom.saturating_sub(y),
    };
    let whole = (crop.w, crop.h) == picture;
    (!whole && crop.w.min(crop.h) > 0).then_some(crop)
}

/// The part of a picture of `picture` px in sight for `look` (no margin, on
/// whole pixels: its start floored, its end ceiled; within the picture).
pub fn sight(picture: (u32, u32), look: &Look) -> Crop {
    let (width, height) = (f64::from(picture.0), f64::from(picture.1));
    let (left, top, seen_w, seen_h) = look.seen;
    let side = |start: f64, size: f64, whole: f64| {
        let from = start.floor().clamp(0.0, whole);
        let to = (start + size).ceil().clamp(from, whole);
        // `as` saturates; both lie within 0..=whole.
        (from as u32, (to - from) as u32)
    };
    let (x, w) = side(left * width, seen_w * width, width);
    let (y, h) = side(top * height, seen_h * height, height);
    Crop { x, y, w, h }
}

/// Whether a frame's `part` (the picture drawn last) covers the part in
/// `sight` (PR L review: after a pan past the margin, the strip that came
/// into sight is old until the hub's new part is drawn, and a finger there
/// must not act).
pub fn covers(part: Crop, sight: Crop) -> bool {
    part.x <= sight.x
        && part.y <= sight.y
        && sight.x + sight.w <= part.x + part.w
        && sight.y + sight.h <= part.y + part.h
}

/// Where a frame's part lands on a canvas of `canvas` px that draws the
/// whole picture of `whole` px (the overview): the part's corner `at` and
/// size `part` in picture px, scaled (left, top, width, height).
pub fn scaled_part(
    at: (u32, u32),
    part: (u32, u32),
    whole: (u32, u32),
    canvas: (u32, u32),
) -> (f64, f64, f64, f64) {
    let scale = |v: u32, of: u32, to: u32| f64::from(v) * f64::from(to) / f64::from(of.max(1));
    (
        scale(at.0, whole.0, canvas.0),
        scale(at.1, whole.1, canvas.1),
        scale(part.0, whole.0, canvas.0),
        scale(part.1, whole.1, canvas.1),
    )
}

/// Whether a view changed at `since` rested its [`VIEW_SETTLE_MS`] at
/// `now` (page ms).
fn settled(since: f64, now: f64) -> bool {
    now - since >= VIEW_SETTLE_MS
}

/// The crop as the hub knows it (none: the whole picture, as every open
/// starts) and a changed one resting, looked at each frame (no timer).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewWatch {
    sent: Option<Crop>,
    resting: Option<(Option<Crop>, f64)>,
}

impl ViewWatch {
    /// The frame's crop (`open`: the page's editor is open; `now`: page
    /// ms): one that differs from the one sent goes once it rested
    /// [`VIEW_SETTLE_MS`] while the editor is open (each change starts the
    /// rest again; one back where it was sends nothing).
    /// The picture took another size (PR L review): the hub dropped its crop,
    /// so the whole picture is what it sends now; a crop for the new size
    /// goes once the view rests.
    pub fn resized(&mut self) {
        self.sent = None;
        self.resting = None;
    }

    pub fn frame(&mut self, crop: Option<Crop>, open: bool, now: f64) -> Option<Option<Crop>> {
        let newest = self.resting.map_or(self.sent, |(resting, _)| resting);
        if newest != crop {
            self.resting = Some((crop, now));
        }
        let (want, since) = self.resting?;
        if !(open && settled(since, now)) {
            return None;
        }
        self.resting = None;
        (want != self.sent).then(|| {
            self.sent = want;
            want
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A look at `zoom` (edits from 2×) of the part `seen` (fractions).
    fn look(edits: bool, seen: (f64, f64, f64, f64)) -> Look {
        Look {
            transform: String::new(),
            zoomed: edits,
            edits,
            factor: String::new(),
            seen,
        }
    }

    fn crop(x: u32, y: u32, w: u32, h: u32) -> Crop {
        Crop { x, y, w, h }
    }

    #[test]
    fn a_crop_is_the_part_in_sight_with_its_margin_on_the_grid() {
        assert_eq!(
            (CROP_MARGIN, CROP_GRID, VIEW_SETTLE_MS),
            (0.25, 16.0, 150.0)
        );
        // 1600 × 800: 640 px in sight from 640, a margin of 160 each side:
        // 480..1440; 266.67 px from 240, a margin of 66.67: 173.3..573.3,
        // 160..576 on the grid.
        let third = 1.0 / 3.0;
        assert_eq!(
            crop_of((1600, 800), &look(true, (0.4, 0.3, 0.4, third))),
            Some(crop(480, 160, 960, 416))
        );
        // Under 2×: the whole picture.
        assert_eq!(
            crop_of((1600, 800), &look(false, (0.4, 0.3, 0.4, third))),
            None
        );
        // At an edge: clamped to the picture.
        assert_eq!(
            crop_of((1600, 800), &look(true, (0.0, 0.75, 0.25, 0.25))),
            Some(crop(0, 544, 512, 256))
        );
        // A part whose margin covers the whole picture: none.
        assert_eq!(
            crop_of((1600, 800), &look(true, (0.1, 0.1, 0.8, 0.8))),
            None
        );
        // Nothing in sight on a side: none.
        assert_eq!(
            crop_of((1600, 800), &look(true, (0.4, 0.3, 0.0, 0.25))),
            None
        );
        // An empty picture: none.
        assert_eq!(crop_of((0, 0), &look(true, (0.0, 0.0, 1.0, 1.0))), None);
    }

    #[test]
    fn a_side_is_widened_by_its_margin_and_kept_within_the_picture() {
        assert_eq!(span(100.0, 400.0, 1349.0), (0, 608));
        assert_eq!(span(800.0, 400.0, 1349.0), (688, 1312));
        assert_eq!(span(1000.0, 400.0, 1349.0), (896, 1349));
        assert_eq!(span(0.0, 0.0, 100.0), (0, 0));
    }

    #[test]
    fn the_part_in_sight_and_whether_a_drawn_part_covers_it() {
        // 1600 × 800, a quarter of each side from (0.5, 0.25): 400 × 200
        // from (800, 200).
        assert_eq!(
            sight((1600, 800), &look(true, (0.5, 0.25, 0.25, 0.25))),
            crop(800, 200, 400, 200)
        );
        // Fractions of a pixel widen it outwards; it stays in the picture.
        assert_eq!(
            sight((1349, 809), &look(true, (0.1, 0.1, 0.5, 0.5))),
            crop(134, 80, 676, 406)
        );
        assert_eq!(
            sight((100, 80), &look(true, (0.9, 0.9, 0.5, 0.5))),
            crop(90, 72, 10, 8)
        );
        let part = crop(100, 50, 300, 200);
        assert!(covers(part, crop(100, 50, 300, 200)), "exactly");
        assert!(covers(part, crop(150, 100, 100, 100)));
        assert!(!covers(part, crop(99, 100, 100, 100)), "left");
        assert!(!covers(part, crop(150, 49, 100, 100)), "top");
        assert!(!covers(part, crop(301, 100, 100, 100)), "right");
        assert!(!covers(part, crop(150, 151, 100, 100)), "bottom");
    }

    #[test]
    fn a_part_lands_scaled_on_the_overview() {
        assert_eq!(
            scaled_part((400, 200), (800, 400), (1600, 800), (160, 80)),
            (40.0, 20.0, 80.0, 40.0)
        );
        // The whole picture fills it.
        assert_eq!(
            scaled_part((0, 0), (1349, 809), (1349, 809), (133, 80)),
            (0.0, 0.0, 133.0, 80.0)
        );
        // An empty picture never divides by 0.
        assert_eq!(
            scaled_part((0, 0), (0, 0), (0, 0), (10, 10)),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn a_changed_crop_goes_once_it_rested_while_the_editor_is_open() {
        let mut watch = ViewWatch::default();
        let a = Some(crop(0, 0, 320, 160));
        let b = Some(crop(16, 0, 320, 160));
        // The whole picture is what the hub has: nothing to send.
        assert_eq!(watch.frame(None, true, 0.0), None);
        // A crop: once it rested 150 ms.
        assert_eq!(watch.frame(a, true, 10.0), None);
        assert_eq!(watch.frame(a, true, 160.0_f64.next_down()), None);
        assert_eq!(watch.frame(a, true, 160.0), Some(a));
        assert_eq!(watch.frame(a, true, 400.0), None, "sent");
        // Each change starts the rest again.
        assert_eq!(watch.frame(b, true, 500.0), None);
        assert_eq!(watch.frame(a, true, 600.0), None);
        assert_eq!(watch.frame(b, true, 700.0), None);
        assert_eq!(watch.frame(b, true, 849.0), None);
        assert_eq!(watch.frame(b, true, 850.0), Some(b));
        // Back where it was before resting: nothing.
        assert_eq!(watch.frame(a, true, 900.0), None);
        assert_eq!(watch.frame(b, true, 950.0), None);
        assert_eq!(watch.frame(b, true, 2000.0), None);
        // Not while the editor is not open; once it is, the rest counts.
        assert_eq!(watch.frame(None, false, 3000.0), None);
        assert_eq!(watch.frame(None, false, 3500.0), None);
        assert_eq!(watch.frame(None, true, 3600.0), Some(None));
        // A picture of another size: the hub sends it whole, so the same
        // crop goes again once it rests.
        assert_eq!(watch.frame(a, true, 4000.0), None);
        assert_eq!(watch.frame(a, true, 4150.0), Some(a));
        watch.resized();
        assert_eq!(watch.frame(a, true, 4200.0), None);
        assert_eq!(watch.frame(a, true, 4350.0), Some(a));
    }
}
