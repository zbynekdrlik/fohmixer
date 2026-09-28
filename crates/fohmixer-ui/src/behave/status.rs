//! The strip's status pill (spec F5; TouchOSC `group_init.lua`): red while
//! the strip is not bound; yellow for 150 ms after a volume or meter value
//! arrived, fading to green by 500 ms.

use super::Rgb;

pub const RED: Rgb = [255.0, 0.0, 0.0];
pub const YELLOW: Rgb = [255.0, 255.0, 0.0];
pub const GREEN: Rgb = [0.0, 255.0, 0.0];
/// Yellow this long after a value…
const YELLOW_MS: f64 = 150.0;
/// …then fading to green until this long after it.
const GREEN_MS: f64 = 500.0;

/// The pill's colour: `bound` or not, `since_ms` after the last value.
pub fn status_color(bound: bool, since_ms: f64) -> Rgb {
    if !bound {
        return RED;
    }
    // 0 until 150 ms (yellow), 1 from 500 ms (green), linear between.
    let fade = ((since_ms - YELLOW_MS) / (GREEN_MS - YELLOW_MS)).clamp(0.0, 1.0);
    [255.0 * (1.0 - fade), 255.0, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_unbound_yellow_after_a_value_then_green() {
        assert_eq!(status_color(false, 0.0), RED);
        assert_eq!(status_color(false, 10_000.0), RED);
        assert_eq!(status_color(true, 0.0), YELLOW);
        assert_eq!(status_color(true, 149.0), YELLOW);
        assert_eq!(status_color(true, 150.0), YELLOW);
        assert_eq!(status_color(true, 325.0), [127.5, 255.0, 0.0]);
        assert_eq!(status_color(true, 237.5), [191.25, 255.0, 0.0]);
        assert_eq!(status_color(true, 500.0), GREEN);
        assert_eq!(status_color(true, f64::INFINITY), GREEN);
    }
}
