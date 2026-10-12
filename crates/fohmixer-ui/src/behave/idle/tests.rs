use super::*;

/// A watch made at 0 that went idle for no touch at `at` (2 s countdown).
fn touch_idle(at: f64) -> IdleWatch {
    let mut w = IdleWatch::new(at - 2_000.0);
    assert_eq!(
        w.tick(at, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
    w
}

#[test]
fn touch_idle_waits_the_layouts_seconds_absent_or_0_never() {
    assert_eq!(idle_after_ms(None), None);
    assert_eq!(idle_after_ms(Some(0)), None);
    assert_eq!(idle_after_ms(Some(1)), Some(1_000.0));
    assert_eq!(idle_after_ms(Some(120)), Some(120_000.0));
}

#[test]
fn a_hidden_page_asks_for_no_meters_an_untouched_one_for_paced_ones() {
    assert_eq!(IDLE_METER_MS, 250);
    for setting in [None, Some(0), Some(800)] {
        assert_eq!(meter_ms(Sleep::Hidden, setting), 0, "{setting:?}");
    }
    assert_eq!(meter_ms(Sleep::Touch, None), 250);
    assert_eq!(meter_ms(Sleep::Touch, Some(0)), 250, "0 would pause them");
    assert_eq!(meter_ms(Sleep::Touch, Some(1)), 1, "the hub clamps it");
    assert_eq!(meter_ms(Sleep::Touch, Some(800)), 800);
}

#[test]
fn an_idle_page_draws_ten_frames_a_second() {
    assert_eq!(IDLE_FRAME_MS, 100.0);
}

#[test]
fn a_finger_an_open_eq_screen_or_an_eq_input_not_shown_holds_the_page() {
    assert!(!holds(false, false, true), "nothing holds it");
    assert!(holds(true, false, true), "a finger down");
    assert!(holds(false, true, true), "the Pro-Q 4 screen open");
    assert!(holds(false, false, false), "an EQ input not shown yet");
    assert!(holds(true, true, false));
}

#[test]
fn the_countdown_is_over_at_exactly_its_length() {
    assert!(!due(2_999.0, 1_000.0, 2_000.0));
    assert!(!due(3_000.0_f64.next_down(), 1_000.0, 2_000.0));
    assert!(due(3_000.0, 1_000.0, 2_000.0));
    assert!(due(9_000.0, 1_000.0, 2_000.0));
}

#[test]
fn a_hidden_page_is_idle_at_once_and_wakes_when_shown() {
    let mut w = IdleWatch::new(0.0);
    assert!(!w.is_idle());
    assert_eq!(w.sleeping(), None);
    assert_eq!(
        w.visibility(100.0, false),
        None,
        "shown and active: nothing"
    );
    assert_eq!(w.visibility(500.0, true), Some(Turn::Sleep(Sleep::Hidden)));
    assert!(w.is_idle());
    assert_eq!(w.sleeping(), Some(Sleep::Hidden));
    assert_eq!(w.visibility(600.0, true), None, "still hidden: nothing");
    assert_eq!(
        w.visibility(2_500.0, false),
        Some(Turn::Wake {
            why: Wake::Visible,
            idle_ms: 2_000.0,
        })
    );
    assert!(!w.is_idle());
    assert_eq!(w.visibility(2_600.0, false), None);
}

#[test]
fn shown_again_the_countdown_starts_over() {
    let mut w = IdleWatch::new(0.0);
    w.visibility(1_000.0, true);
    assert_eq!(w.tick(60_000.0, Some(2_000.0), false), None, "hidden");
    w.visibility(70_000.0, false);
    assert_eq!(w.tick(71_999.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(72_000.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
}

#[test]
fn an_untouched_page_goes_idle_after_the_countdown_once() {
    let mut w = IdleWatch::new(1_000.0);
    assert_eq!(w.tick(2_999.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(3_000.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
    assert_eq!(w.sleeping(), Some(Sleep::Touch));
    assert_eq!(w.tick(3_100.0, Some(2_000.0), false), None, "idle already");
    assert_eq!(w.tick(3_200.0, Some(2_000.0), true), None);
    assert!(w.is_idle());
}

#[test]
fn something_that_holds_the_page_starts_the_countdown_over() {
    let mut w = IdleWatch::new(0.0);
    assert_eq!(w.tick(100_000.0, Some(2_000.0), true), None, "held: never");
    assert!(!w.is_idle());
    assert_eq!(w.tick(101_999.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(102_000.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch)),
        "2 s after the last tick that held it"
    );
}

#[test]
fn touch_idle_off_never_sleeps_and_switched_on_counts_from_then() {
    let mut w = IdleWatch::new(0.0);
    assert_eq!(w.tick(10_000.0, None, false), None);
    assert_eq!(w.tick(11_000.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(12_000.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
}

#[test]
fn a_touch_starts_the_countdown_over_and_wakes_an_untouched_page() {
    let mut w = IdleWatch::new(0.0);
    assert_eq!(w.touched(1_500.0), None, "active: nothing to wake");
    assert_eq!(w.tick(3_499.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(3_500.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
    assert_eq!(
        w.touched(4_750.0),
        Some(Turn::Wake {
            why: Wake::Touch,
            idle_ms: 1_250.0,
        })
    );
    assert!(!w.is_idle());
    assert_eq!(w.tick(6_749.0, Some(2_000.0), false), None);
    assert_eq!(
        w.tick(6_750.0, Some(2_000.0), false),
        Some(Turn::Sleep(Sleep::Touch))
    );
}

#[test]
fn a_hidden_page_stays_idle_until_it_is_shown() {
    let mut w = IdleWatch::new(0.0);
    w.visibility(100.0, true);
    assert_eq!(w.touched(200.0), None);
    assert_eq!(w.tick(300.0, None, false), None, "touch-idle off");
    assert_eq!(w.tick(400.0, Some(2_000.0), true), None);
    assert_eq!(w.sleeping(), Some(Sleep::Hidden));
}

#[test]
fn an_untouched_page_hidden_goes_idle_again_paused_and_shown_wakes() {
    let mut w = touch_idle(3_000.0);
    assert_eq!(w.visibility(3_500.0, false), None, "shown: no change");
    assert_eq!(w.sleeping(), Some(Sleep::Touch));
    assert_eq!(
        w.visibility(4_000.0, true),
        Some(Turn::Sleep(Sleep::Hidden))
    );
    assert_eq!(w.sleeping(), Some(Sleep::Hidden));
    assert_eq!(
        w.visibility(6_000.0, false),
        Some(Turn::Wake {
            why: Wake::Visible,
            idle_ms: 3_000.0,
        }),
        "idle since it went idle for no touch"
    );
}

#[test]
fn touch_idle_switched_off_wakes_an_untouched_page() {
    let mut w = touch_idle(3_000.0);
    assert_eq!(
        w.tick(3_500.0, None, false),
        Some(Turn::Wake {
            why: Wake::Config,
            idle_ms: 500.0,
        })
    );
    assert!(!w.is_idle());
    assert_eq!(w.tick(3_600.0, None, false), None);
}

#[test]
fn each_turn_is_an_idle_event() {
    assert_eq!(
        Turn::Sleep(Sleep::Hidden).event(5.0),
        json!({"ev": "idle", "t": 5.0, "on": true, "why": "hidden"})
    );
    assert_eq!(
        Turn::Sleep(Sleep::Touch).event(6.0),
        json!({"ev": "idle", "t": 6.0, "on": true, "why": "touch"})
    );
    for (why, name) in [
        (Wake::Touch, "touch"),
        (Wake::Visible, "visible"),
        (Wake::Config, "config"),
    ] {
        assert_eq!(
            Turn::Wake {
                why,
                idle_ms: 1_234.4,
            }
            .event(9.0),
            json!({"ev": "idle", "t": 9.0, "on": false, "why": name, "ms": 1_234.0})
        );
    }
}
