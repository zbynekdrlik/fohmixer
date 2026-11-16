//! A strip's home level (#225): its Tuner's `+HOME:<dB>` moves the unity
//! mark and the double tap's glide from 0 dB to that level.

use super::*;

const TRAVEL: f64 = 100.0;

#[track_caller]
fn close(got: f64, want: f64, tolerance: f64) {
    assert!((got - want).abs() <= tolerance, "{got} is not {want}");
}

#[test]
fn a_strip_without_a_home_level_calls_0_db_home() {
    assert_eq!(home_volume(None), UNITY);
}

#[test]
fn a_home_level_is_its_live_volume_on_the_strips_law() {
    let minus_6 = home_volume(Some(-6.0));
    close(value2db(minus_6), -6.0, 0.005);
    assert!(minus_6 < UNITY, "{minus_6}");
    close(value2db(home_volume(Some(-2.5))), -2.5, 0.005);
    close(value2db(home_volume(Some(-60.0))), -60.0, 0.005);
    assert_eq!(home_volume(Some(6.0)), 1.0, "+6 dB is the top");
    // The bisection's own resolution (a hair over 0.85, 0.0006 dB).
    close(home_volume(Some(0.0)), UNITY, 1e-4);
}

/// A plain fader whose double tap glides to `to`, double tapped at Live's
/// position `live` (two taps 60 ms apart), its glide's arrival time.
fn double_tapped(to: f64, live: f64) -> (FaderCtl, f64) {
    let mut f = FaderCtl::new(false, Some(to));
    f.down(1, true, 500.0, TRAVEL, 0.0, live);
    f.up(1, 20.0);
    f.down(1, true, 500.0, TRAVEL, 60.0, live);
    f.up(1, 80.0);
    let arrive = 80.0 + (to - live).abs() / GLIDE_SPEED * 1000.0 + 1.0;
    (f, arrive)
}

#[test]
fn a_double_tap_glides_to_the_home_level_up_and_down() {
    for law in [VolumeLaw::Live, VolumeLaw::TouchOsc] {
        let home = law.to_pos(home_volume(Some(-6.0)));
        assert!(home < law.to_pos(UNITY), "{law:?}");
        for live in [0.2, 0.95] {
            let (mut f, arrive) = double_tapped(home, live);
            let sent = f.frame(arrive - 200.0, Some(live)).send.unwrap();
            assert!(
                (sent - home).abs() < (live - home).abs(),
                "{law:?} from {live}: on its way, {sent}"
            );
            assert!(!f.take_ended(), "still gliding");
            assert_eq!(
                f.frame(arrive, Some(live)).send,
                Some(home),
                "{law:?} from {live}: lands on the home level exactly"
            );
            assert!(f.take_ended());
        }
    }
}

#[test]
fn the_home_level_on_live_s_law_reads_minus_6_db_and_is_not_0_db() {
    let home = VolumeLaw::Live.to_pos(home_volume(Some(-6.0)));
    close(value2db(VolumeLaw::Live.to_live(home)), -6.0, 0.005);
    let (mut f, arrive) = double_tapped(home, 0.2);
    assert_ne!(f.frame(arrive, Some(0.2)).send, Some(to_pos(UNITY)));
}
