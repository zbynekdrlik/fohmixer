//! The shape of a Pro-Q 4 editor on the PC (#71 PR G): the page's picture
//! area's aspect at about Pro-Q 4's own pixel count at 100 %, so the
//! editor lays itself out for the device that opens it (Pro-Q 4 resizes
//! freely by its window: FabFilter's help; in Carla's bridge on the PC a
//! 760 × 1271 picture laid itself out upright, `.claude/rules/plugin-window.md`).
//! Pure: the window worker (`plugwin`) asks the backend for the room and
//! the minimum, and posts the size.

use fohmixer_proto::eq::Area;

/// Pro-Q 4's picture at 100 % (Live's `FF_UIWindow`, 1349 × 809): the
/// pixels an editor keeps whatever its shape.
pub const PIXELS: f64 = 1349.0 * 809.0;

/// The picture size (px) for a page's `area` (CSS px): its aspect at
/// [`PIXELS`], scaled down to fit `room` (the picture's room on the PC's
/// screen, where the window stands), each side at least `min` (Pro-Q's own
/// minimum: a side `min` holds up bends the aspect). None for an area with
/// no shape (a side 0, below 0 or not a number, an aspect past any number):
/// the editor keeps its size.
pub fn editor_size(area: Area, room: (u32, u32), min: (u32, u32)) -> Option<(u32, u32)> {
    let aspect = area.w / area.h;
    let shaped = area.w > 0.0 && area.h > 0.0 && aspect.is_finite();
    if !shaped {
        return None;
    }
    let width = (PIXELS * aspect).sqrt();
    let height = (PIXELS / aspect).sqrt();
    let scale = (f64::from(room.0) / width)
        .min(f64::from(room.1) / height)
        .min(1.0);
    let side = |pixels: f64, least: u32| ((pixels * scale).round() as u32).max(least);
    Some((side(width, min.0), side(height, min.1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PC's room: its 2560 × 1440 screen less a 40 px taskbar, less
    /// Live's window frame (16 × 39) for a window in the top-left corner.
    const ROOM: (u32, u32) = (2544, 1361);
    const MIN: (u32, u32) = (600, 400);

    fn size(w: f64, h: f64) -> Option<(u32, u32)> {
        editor_size(Area { w, h }, ROOM, MIN)
    }

    #[test]
    fn the_pixels_are_pro_q_4s_at_100_percent() {
        assert_eq!(PIXELS, 1_091_341.0);
        // Pro-Q's own shape keeps its own size.
        assert_eq!(size(1349.0, 809.0), Some((1349, 809)));
        assert_eq!(size(1.0, 1.0), Some((1045, 1045)));
    }

    #[test]
    fn a_device_gets_its_aspect_at_pro_qs_pixel_count() {
        // A tablet on its side (1.53) and a phone on its side (2.5): both
        // fit the room whole.
        assert_eq!(size(1194.0, 780.0), Some((1293, 844)));
        assert_eq!(size(800.0, 320.0), Some((1652, 661)));
        // The desktop project's area (1.89).
        assert_eq!(size(1280.0, 676.0), Some((1438, 759)));
    }

    #[test]
    fn an_upright_phone_is_scaled_down_into_the_room() {
        // 0.49: 731 × 1492 at Pro-Q's pixel count, too high for the room's
        // 1361: scaled down, the aspect kept.
        assert_eq!(size(392.0, 800.0), Some((667, 1361)));
        // A window lower on the screen has less room: the width then stops
        // at the minimum and the aspect bends.
        assert_eq!(
            editor_size(Area { w: 392.0, h: 800.0 }, (2544, 1065), MIN),
            Some((600, 1065))
        );
    }

    #[test]
    fn a_side_never_goes_below_the_minimum() {
        // 30:1 fits the room's width; its height 85 stops at 400.
        assert_eq!(size(3000.0, 100.0), Some((2544, 400)));
        assert_eq!(size(100.0, 3000.0), Some((600, 1361)));
        // A room smaller than the minimum: the minimum.
        assert_eq!(
            editor_size(
                Area {
                    w: 1349.0,
                    h: 809.0
                },
                (300, 200),
                MIN
            ),
            Some((600, 400))
        );
    }

    #[test]
    fn an_area_with_no_shape_keeps_the_editors_size() {
        for (w, h) in [
            (0.0, 500.0),
            (500.0, 0.0),
            (0.0, 0.0),
            (-390.0, 800.0),
            (390.0, -800.0),
            (-390.0, -800.0),
            (f64::NAN, 800.0),
            (390.0, f64::NAN),
            (f64::INFINITY, 800.0),
            (1e300, 1e-300),
        ] {
            assert_eq!(size(w, h), None, "{w} × {h}");
        }
        // The smallest areas with a shape have one.
        assert_eq!(
            size(f64::MIN_POSITIVE, f64::MIN_POSITIVE),
            Some((1045, 1045))
        );
    }
}
