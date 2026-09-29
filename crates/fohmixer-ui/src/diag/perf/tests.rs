//! Tests of `perf.rs`: the frame window, the pointers at once, when a
//! periodic perf report is due and what a perf report carries.

use super::*;

#[test]
fn the_limits_are_these() {
    assert_eq!(WINDOW_MS, 10_000.0);
    assert_eq!(PERF_GAP_MS, 60_000.0);
    assert_eq!(TOUCHES_REPORTED, 2);
    assert_eq!(POINTERS_MAX, 20);
}

#[test]
fn a_window_is_full_after_ten_seconds_of_frames() {
    assert!(!window_full(10_000.0_f64.next_down()));
    assert!(window_full(10_000.0));
    assert!(window_full(10_000.5));
    assert!(!window_full(0.0));
}

#[test]
fn the_frame_rate_is_the_window_s_frames_per_second() {
    assert_eq!(fps(600, 10_000.0), 60.0);
    assert_eq!(fps(220, 10_000.0), 22.0);
    assert_eq!(fps(1, 250.0), 4.0);
}

#[test]
fn a_periodic_report_is_due_at_most_once_a_minute() {
    let t = 1_000_000.0;
    assert!(perf_due(t, None), "none sent yet");
    assert!(!perf_due(t, Some(t)));
    assert!(!perf_due((t + 60_000.0).next_down(), Some(t)));
    assert!(perf_due(t + 60_000.0, Some(t)));
    // A clock that went backwards does not block the reports for ever.
    assert!(perf_due(t - 1.0, Some(t)));
}

/// `count` frames `gap` ms apart, the first at `start` + `gap`: the times
/// a periodic report was due.
fn frames(perf: &mut Perf, start: f64, count: usize, gap: f64) -> Vec<f64> {
    let mut due = Vec::new();
    for n in 1..=count {
        let now = start + gap * n as f64;
        if perf.frame(now, gap) {
            due.push(now);
        }
    }
    due
}

#[test]
fn ten_seconds_of_frames_give_their_rate_and_their_longest_gap() {
    let mut perf = Perf::default();
    // 599 frames 16 ms apart: 9584 ms, not a window yet.
    assert_eq!(frames(&mut perf, 0.0, 599, 16.0), Vec::<f64>::new());
    assert_eq!(perf.clone().report(0.0).window, None);
    // A 416 ms stall fills it: 600 frames in 10 s.
    assert!(
        perf.frame(10_000.0, 416.0),
        "the first window: a report is due"
    );
    assert_eq!(
        perf.report(10_000.0).window,
        Some(Window {
            fps: 60.0,
            long_ms: 416.0
        })
    );
}

#[test]
fn the_longest_gap_is_the_longest_not_the_last() {
    let mut perf = Perf::default();
    assert!(!perf.frame(250.0, 250.0));
    // 375 more frames 26 ms apart: 250 + 9750 = 10_000 ms.
    let due = frames(&mut perf, 250.0, 375, 26.0);
    assert_eq!(due, [10_000.0]);
    assert_eq!(
        perf.report(10_000.0).window,
        Some(Window {
            fps: 37.6,
            long_ms: 250.0
        })
    );
}

#[test]
fn every_window_counts_its_own_frames() {
    let mut perf = Perf::default();
    frames(&mut perf, 0.0, 500, 20.0);
    // The next window: 250 frames 40 ms apart (a slow stretch).
    frames(&mut perf, 10_000.0, 250, 40.0);
    assert_eq!(
        perf.report(20_000.0).window,
        Some(Window {
            fps: 25.0,
            long_ms: 40.0
        })
    );
}

#[test]
fn a_negative_gap_counts_as_none() {
    // The loop's first frame can carry a time a little before the moment
    // the loop started (the frame's own start).
    let mut perf = Perf::default();
    assert!(!perf.frame(0.0, -5.0));
    assert!(perf.frame(10_000.0, 10_000.0));
    assert_eq!(
        perf.report(10_000.0).window,
        Some(Window {
            fps: 0.2,
            long_ms: 10_000.0
        })
    );
}

#[test]
fn a_hidden_page_s_pause_is_no_frame_gap() {
    let mut perf = Perf::default();
    // A closed window, then half of the next.
    frames(&mut perf, 0.0, 625, 16.0);
    frames(&mut perf, 10_000.0, 300, 16.0);
    perf.pause();
    assert_eq!(
        perf.clone().report(0.0).window,
        None,
        "the page's rate before it was hidden is gone"
    );
    // The first frame after the pause carries the minute the page was
    // hidden: not counted.
    assert!(!perf.frame(74_800.0, 60_000.0));
    // Then a whole window of its own (the open half was dropped).
    frames(&mut perf, 74_800.0, 624, 16.0);
    assert!(
        perf.clone().report(0.0).window.is_none(),
        "624 frames: 9984 ms"
    );
    assert!(perf.frame(84_800.0, 16.0));
    assert_eq!(
        perf.report(84_800.0).window,
        Some(Window {
            fps: 62.5,
            long_ms: 16.0
        })
    );
}

#[test]
fn the_periodic_report_goes_once_a_minute_as_windows_close() {
    let mut perf = Perf::default();
    // One frame a second: a window closes every 10 s.
    let mut due = Vec::new();
    for n in 1..=140 {
        let now = 1_000.0 * f64::from(n);
        if perf.frame(now, 1_000.0) {
            due.push(now);
            perf.report(now);
        }
    }
    assert_eq!(due, [10_000.0, 70_000.0, 130_000.0]);
}

#[test]
fn a_due_report_not_sent_is_due_again_at_the_next_window() {
    // The throttle held the report back and nothing went: the next window
    // asks again.
    let mut perf = Perf::default();
    let due = frames(&mut perf, 0.0, 30, 1_000.0);
    assert_eq!(due, [10_000.0, 20_000.0, 30_000.0]);
}

#[test]
fn the_pointer_types_are_touch_mouse_and_pen() {
    assert_eq!(Pointer::parse("touch"), Some(Pointer::Touch));
    assert_eq!(Pointer::parse("mouse"), Some(Pointer::Mouse));
    assert_eq!(Pointer::parse("pen"), Some(Pointer::Pen));
    assert_eq!(Pointer::parse(""), None);
    assert_eq!(Pointer::parse("Touch"), None);
    assert_eq!(Pointer::Touch.name(), "touch");
    assert_eq!(Pointer::Mouse.name(), "mouse");
    assert_eq!(Pointer::Pen.name(), "pen");
}

#[test]
fn two_fingers_or_more_are_reported_when_the_page_sees_more_than_ever() {
    let mut perf = Perf::default();
    assert!(!perf.down(1, "touch", true), "one finger is no report");
    assert!(perf.down(2, "touch", false), "two at once: the most yet");
    assert!(perf.down(3, "touch", false), "three");
    perf.up(3);
    assert!(
        !perf.down(4, "touch", false),
        "three again: not more than ever"
    );
    assert!(perf.down(5, "touch", false), "four");
    // A report does not lower the page's most.
    perf.report(0.0);
    perf.up(5);
    assert!(!perf.down(6, "touch", false));
}

#[test]
fn the_touches_count_since_the_last_report_from_the_fingers_still_down() {
    let mut perf = Perf::default();
    for id in 1..=4 {
        perf.down(id, "touch", id == 1);
    }
    perf.up(1);
    perf.up(2);
    perf.up(3);
    let report = perf.report(0.0);
    assert_eq!(
        report.touches_max, 4,
        "lifted before the report: still counted"
    );
    assert_eq!(report.pointer, Some(Pointer::Touch));
    // One finger is still down: the next report starts from it.
    assert_eq!(perf.report(1.0).touches_max, 1);
    perf.up(4);
    assert_eq!(
        perf.report(2.0).touches_max,
        1,
        "down when the last report went"
    );
    assert_eq!(perf.report(3.0).touches_max, 0);
    // An unknown pointer going up changes nothing.
    perf.down(7, "touch", true);
    perf.up(8);
    assert_eq!(perf.report(4.0).touches_max, 1);
    assert_eq!(perf.report(5.0).touches_max, 1);
}

#[test]
fn a_pointer_down_twice_counts_once() {
    let mut perf = Perf::default();
    assert!(!perf.down(1, "touch", true));
    assert!(!perf.down(1, "touch", false));
    assert_eq!(perf.report(0.0).touches_max, 1);
}

#[test]
fn a_primary_pointer_forgets_the_missed_ups_of_its_type() {
    // A primary pointer is the first of its type: the others still counted
    // lost their `pointerup`.
    let mut perf = Perf::default();
    perf.down(1, "touch", true);
    perf.down(2, "touch", false);
    assert_eq!(perf.report(0.0).touches_max, 2);
    perf.down(3, "touch", true);
    assert_eq!(
        perf.report(1.0).touches_max,
        2,
        "two were down at the report"
    );
    assert_eq!(perf.report(2.0).touches_max, 1, "now only the new one");
    // A mouse is its own type: the fingers stay.
    let mut mixed = Perf::default();
    mixed.down(1, "touch", true);
    mixed.down(2, "touch", false);
    mixed.down(9, "mouse", true);
    assert_eq!(mixed.report(0.0).touches_max, 3);
    assert_eq!(mixed.report(1.0).touches_max, 3);
}

#[test]
fn at_most_twenty_pointers_are_counted() {
    let mut perf = Perf::default();
    for id in 0..25 {
        perf.down(id, "touch", id == 0);
    }
    assert_eq!(perf.report(0.0).touches_max, 20);
    // The ones not counted have no up to miss.
    for id in 0..20 {
        perf.up(id);
    }
    assert_eq!(perf.report(1.0).touches_max, 20);
    assert_eq!(perf.report(2.0).touches_max, 0);
}

#[test]
fn the_pointer_is_the_latest_one_s_type() {
    let mut perf = Perf::default();
    assert_eq!(
        perf.clone().report(0.0).pointer,
        None,
        "nothing touched yet"
    );
    perf.down(1, "mouse", true);
    perf.up(1);
    assert_eq!(perf.clone().report(0.0).pointer, Some(Pointer::Mouse));
    perf.down(2, "pen", true);
    assert_eq!(perf.report(0.0).pointer, Some(Pointer::Pen));
    // It stays until another pointer goes down.
    assert_eq!(perf.report(1.0).pointer, Some(Pointer::Pen));
    perf.down(3, "", true);
    assert_eq!(perf.report(2.0).pointer, None);
}

#[test]
fn a_hidden_page_has_no_fingers_down() {
    let mut perf = Perf::default();
    perf.down(1, "touch", true);
    perf.down(2, "touch", false);
    perf.pause();
    assert_eq!(perf.report(0.0).touches_max, 2, "seen before the pause");
    assert_eq!(perf.report(1.0).touches_max, 0);
}

#[test]
fn a_perf_report_s_fields_are_short_words() {
    let mut fields = ReportFields {
        kind: Some("perf".into()),
        ..ReportFields::default()
    };
    PerfReport {
        window: Some(Window {
            fps: 59.94,
            long_ms: 33.6,
        }),
        touches_max: 4,
        pointer: Some(Pointer::Touch),
    }
    .fill(&mut fields);
    assert_eq!(
        fields,
        ReportFields {
            kind: Some("perf".into()),
            fps: Some("59.9".into()),
            long_frame_ms: Some("34".into()),
            touches_max: Some("4".into()),
            pointer: Some("touch".into()),
            ..ReportFields::default()
        }
    );
    // No window measured yet, nothing touched.
    PerfReport {
        window: None,
        touches_max: 0,
        pointer: None,
    }
    .fill(&mut fields);
    assert_eq!(fields.fps, None);
    assert_eq!(fields.long_frame_ms, None);
    assert_eq!(fields.touches_max.as_deref(), Some("0"));
    assert_eq!(fields.pointer, None);
    // One decimal, whole milliseconds.
    PerfReport {
        window: Some(Window {
            fps: 21.96,
            long_ms: 112.2,
        }),
        touches_max: 11,
        pointer: Some(Pointer::Pen),
    }
    .fill(&mut fields);
    assert_eq!(fields.fps.as_deref(), Some("22.0"));
    assert_eq!(fields.long_frame_ms.as_deref(), Some("112"));
    assert_eq!(fields.touches_max.as_deref(), Some("11"));
    assert_eq!(fields.pointer.as_deref(), Some("pen"));
}
