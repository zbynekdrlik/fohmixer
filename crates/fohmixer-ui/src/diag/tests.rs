//! Tests of `diag.rs`'s pure part: the throttle, the hellos, the report's
//! fields.

use super::*;

const ALL: [Kind; 9] = [
    Kind::Load,
    Kind::Connected,
    Kind::Disconnected,
    Kind::Reconnect,
    Kind::Visibility,
    Kind::Sw,
    Kind::WakeLock,
    Kind::Error,
    Kind::Perf,
];

#[test]
fn the_limits_are_these() {
    assert_eq!(REPORT_GAP_MS, 5000.0);
    assert_eq!(ERROR_MAX_CHARS, 300);
    assert_eq!(ERRORS_REMEMBERED, 32);
    assert_eq!(ERRORS_WAITING, 8);
    assert_eq!(REPORT_URL, "/api/client-report");
}

/// A trailing report that fires without an error.
const TRAILING: Fired = Fired {
    send: true,
    error: None,
    again: None,
};

#[test]
fn every_kind_has_the_name_the_hub_logs() {
    let names: Vec<&str> = ALL.iter().map(|k| k.name()).collect();
    assert_eq!(
        names,
        [
            "load",
            "connected",
            "disconnected",
            "reconnect",
            "visibility",
            "sw",
            "wake-lock",
            "error",
            "perf"
        ]
    );
}

#[test]
fn a_kind_goes_at_most_once_per_five_seconds() {
    let t = 1_000_000.0;
    let sent_at_t = || {
        let mut diag = Diag::default();
        assert_eq!(diag.offer(Kind::Sw, t, None), Verdict::Now);
        diag
    };
    assert_eq!(
        sent_at_t().offer(Kind::Sw, t, None),
        Verdict::Later(t + 5_000.0),
        "not twice at once"
    );
    assert_eq!(
        sent_at_t().offer(Kind::Sw, t + 4_999.999, None),
        Verdict::Later(t + 5_000.0)
    );
    assert_eq!(sent_at_t().offer(Kind::Sw, t + 5_000.0, None), Verdict::Now);
    // A clock that went backwards does not block reports for ever.
    assert_eq!(sent_at_t().offer(Kind::Sw, t - 1.0, None), Verdict::Now);
}

#[test]
fn a_change_within_the_window_goes_once_when_it_ends() {
    // The iPad app hidden and back within 5 s: the hub must not keep
    // "hidden" as its last state.
    let t = 1_000_000.0;
    let mut diag = Diag::default();
    assert_eq!(diag.offer(Kind::Visibility, t, None), Verdict::Now);
    assert_eq!(
        diag.offer(Kind::Visibility, t + 1_000.0, None),
        Verdict::Later(t + 5_000.0)
    );
    // More changes meanwhile ride on the armed report.
    assert_eq!(
        diag.offer(Kind::Visibility, t + 2_000.0, None),
        Verdict::Skip
    );
    assert_eq!(diag.fire(Kind::Visibility, t + 5_000.0), TRAILING);
    // The trailing report opened a new window.
    assert_eq!(
        diag.offer(Kind::Visibility, t + 9_999.0, None),
        Verdict::Later(t + 10_000.0)
    );
    assert_eq!(diag.fire(Kind::Visibility, t + 10_000.0), TRAILING);
    assert_eq!(
        diag.offer(Kind::Visibility, t + 15_000.0, None),
        Verdict::Now
    );
}

#[test]
fn every_kind_has_its_own_window() {
    let mut diag = Diag::default();
    for kind in ALL {
        assert_eq!(diag.offer(kind, 0.0, None), Verdict::Now, "{kind:?}");
    }
    for kind in ALL {
        let later = diag.offer(kind, 1.0, None);
        assert_eq!(later, Verdict::Later(5_000.0), "{kind:?}");
    }
    for kind in ALL {
        assert_eq!(diag.offer(kind, 2.0, None), Verdict::Skip, "{kind:?}");
    }
}

#[test]
fn an_error_message_goes_once() {
    let mut diag = Diag::default();
    assert_eq!(diag.offer(Kind::Error, 0.0, Some("boom")), Verdict::Now);
    assert_eq!(
        diag.offer(Kind::Error, 10_000.0, Some("boom")),
        Verdict::Skip
    );
    assert_eq!(
        diag.offer(Kind::Error, 20_000.0, Some("bang")),
        Verdict::Now
    );
    // Two more within the window wait their turn, one per 5 s.
    assert_eq!(
        diag.offer(Kind::Error, 21_000.0, Some("crash")),
        Verdict::Later(25_000.0)
    );
    assert_eq!(
        diag.offer(Kind::Error, 22_000.0, Some("oops")),
        Verdict::Skip
    );
    assert_eq!(
        diag.offer(Kind::Error, 23_000.0, Some("crash")),
        Verdict::Skip
    );
    assert_eq!(
        diag.fire(Kind::Error, 25_000.0),
        Fired {
            send: true,
            error: Some("crash".into()),
            again: Some(30_000.0),
        }
    );
    assert_eq!(
        diag.fire(Kind::Error, 30_000.0),
        Fired {
            send: true,
            error: Some("oops".into()),
            again: None,
        }
    );
    // Sent: never again.
    assert_eq!(
        diag.offer(Kind::Error, 40_000.0, Some("oops")),
        Verdict::Skip
    );
    // A timer with nothing waiting sends nothing.
    assert_eq!(
        Diag::default().fire(Kind::Error, 0.0),
        Fired {
            send: false,
            error: None,
            again: None,
        }
    );
}

#[test]
fn at_most_eight_errors_wait() {
    let mut diag = Diag::default();
    assert_eq!(diag.offer(Kind::Error, 0.0, Some("first")), Verdict::Now);
    for n in 0..ERRORS_WAITING + 2 {
        let message = format!("w{n}");
        diag.offer(Kind::Error, 1.0, Some(message.as_str()));
    }
    let mut sent = Vec::new();
    let mut at = Some(5_000.0);
    for _ in 0..ERRORS_WAITING + 5 {
        let Some(time) = at else { break };
        let fired = diag.fire(Kind::Error, time);
        sent.extend(fired.error);
        at = fired.again;
    }
    let expected: Vec<String> = (0..ERRORS_WAITING).map(|n| format!("w{n}")).collect();
    assert_eq!(sent, expected);
    assert_eq!(at, None);
}

#[test]
fn only_the_latest_error_messages_are_remembered() {
    let mut diag = Diag::default();
    let at = |n: usize| n as f64 * REPORT_GAP_MS;
    for n in 0..=ERRORS_REMEMBERED {
        let message = format!("e{n}");
        let verdict = diag.offer(Kind::Error, at(n), Some(message.as_str()));
        assert_eq!(verdict, Verdict::Now, "{n}");
    }
    let later = at(ERRORS_REMEMBERED + 1);
    // e1 is still remembered; e0, the oldest, was forgotten.
    assert_eq!(diag.offer(Kind::Error, later, Some("e1")), Verdict::Skip);
    assert_eq!(diag.offer(Kind::Error, later, Some("e0")), Verdict::Now);
}

#[test]
fn the_first_hello_connects_the_later_ones_reconnect() {
    let mut diag = Diag::default();
    assert_eq!(diag.reconnects(), 0);
    assert_eq!(diag.hello(), Kind::Connected);
    assert_eq!(diag.reconnects(), 0);
    assert_eq!(diag.hello(), Kind::Reconnect);
    assert_eq!(diag.hello(), Kind::Reconnect);
    assert_eq!(diag.reconnects(), 2);
}

#[test]
fn a_home_screen_app_is_standalone() {
    assert_eq!(display_mode(false, false), "browser");
    assert_eq!(display_mode(true, false), "standalone");
    assert_eq!(display_mode(false, true), "standalone");
    assert_eq!(display_mode(true, true), "standalone");
}

#[test]
fn the_screen_and_the_visibility_read_plainly() {
    assert_eq!(screen_text(1194.0, 834.0, 2.0), "1194x834@2");
    assert_eq!(screen_text(1280.0, 720.0, 1.5), "1280x720@1.5");
    assert_eq!(visibility(true), "hidden");
    assert_eq!(visibility(false), "visible");
}

#[test]
fn an_error_event_says_where_when_it_knows() {
    assert_eq!(
        error_event_text("x is undefined", "https://foh.example.org/app.js", 12, 7),
        "x is undefined at https://foh.example.org/app.js:12:7"
    );
    assert_eq!(error_event_text("Script error.", "", 0, 0), "Script error.");
}

#[test]
fn the_pwa_attributes_are_their_reports() {
    assert_eq!(attribute_kind("data-sw"), Some(Kind::Sw));
    assert_eq!(attribute_kind("data-wake-lock"), Some(Kind::WakeLock));
    assert_eq!(attribute_kind("data-pwa"), None);
    assert_eq!(attribute_kind("class"), None);
}

fn ipad() -> Env {
    Env {
        standalone_media: false,
        navigator_standalone: true,
        ua: "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)".into(),
        host: "foh.example.org".into(),
        screen: (1194.0, 834.0, 2.0),
        sw: Some("registered".into()),
        wake_lock: Some("held".into()),
        hidden: false,
    }
}

#[test]
fn a_report_carries_the_page_s_state() {
    assert_eq!(
        fields(Kind::Reconnect, &ipad(), 3, None),
        ReportFields {
            kind: Some("reconnect".into()),
            display: Some("standalone".into()),
            ua: Some("Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)".into()),
            build: Some(fohmixer_proto::VERSION.into()),
            host: Some("foh.example.org".into()),
            screen: Some("1194x834@2".into()),
            sw: Some("registered".into()),
            wake_lock: Some("held".into()),
            visibility: Some("visible".into()),
            reconnects: Some("3".into()),
            error: None,
            fps: None,
            long_frame_ms: None,
            touches_max: None,
            pointer: None,
        }
    );
    // Off https: no service worker, no wake lock; a hidden browser tab.
    let tab = Env {
        standalone_media: false,
        navigator_standalone: false,
        sw: None,
        wake_lock: None,
        hidden: true,
        ..ipad()
    };
    let report = fields(Kind::Visibility, &tab, 0, None);
    assert_eq!(report.display.as_deref(), Some("browser"));
    assert_eq!(report.sw, None);
    assert_eq!(report.wake_lock, None);
    assert_eq!(report.visibility.as_deref(), Some("hidden"));
}

#[test]
fn an_error_report_carries_its_text_cut_short() {
    let report = fields(Kind::Error, &ipad(), 0, Some("boom"));
    assert_eq!(report.kind.as_deref(), Some("error"));
    assert_eq!(report.error.as_deref(), Some("boom"));
    let long = "e".repeat(ERROR_MAX_CHARS + 1);
    let cut = fields(Kind::Error, &ipad(), 0, Some(long.as_str()))
        .error
        .unwrap();
    assert_eq!(cut, format!("{}…", "e".repeat(ERROR_MAX_CHARS)));
}

#[test]
fn a_perf_report_is_a_report_of_the_page_with_its_numbers() {
    // `fields` gives the page's state; `PerfReport::fill` adds the frame
    // rate and the touches (`perf.rs`).
    let mut report = fields(Kind::Perf, &ipad(), 0, None);
    assert_eq!(report.kind.as_deref(), Some("perf"));
    assert_eq!(report.visibility.as_deref(), Some("visible"));
    assert_eq!(report.fps, None);
    perf::PerfReport {
        window: Some(perf::Window {
            fps: 120.0,
            long_ms: 16.0,
        }),
        touches_max: 4,
        pointer: Some(perf::Pointer::Touch),
    }
    .fill(&mut report);
    assert_eq!(report.fps.as_deref(), Some("120.0"));
    assert_eq!(report.long_frame_ms.as_deref(), Some("16"));
    assert_eq!(report.touches_max.as_deref(), Some("4"));
    assert_eq!(report.pointer.as_deref(), Some("touch"));
    assert_eq!(report.screen.as_deref(), Some("1194x834@2"));
}
