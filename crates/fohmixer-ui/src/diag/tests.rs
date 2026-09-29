//! Tests of `diag.rs`'s pure part: the throttle, the hellos, the report's
//! fields.

use super::*;

const ALL: [Kind; 8] = [
    Kind::Load,
    Kind::Connected,
    Kind::Disconnected,
    Kind::Reconnect,
    Kind::Visibility,
    Kind::Sw,
    Kind::WakeLock,
    Kind::Error,
];

#[test]
fn the_limits_are_these() {
    assert_eq!(REPORT_GAP_MS, 5000.0);
    assert_eq!(ERROR_MAX_CHARS, 300);
    assert_eq!(ERRORS_REMEMBERED, 32);
    assert_eq!(REPORT_URL, "/api/client-report");
}

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
            "error"
        ]
    );
}

#[test]
fn a_kind_goes_at_most_once_per_five_seconds() {
    let t = 1_000_000.0;
    let mut diag = Diag::default();
    assert!(diag.allow(Kind::Visibility, t, None));
    assert!(!diag.allow(Kind::Visibility, t, None), "not twice at once");
    assert!(!diag.allow(Kind::Visibility, t + 4_999.999, None));
    assert!(diag.allow(Kind::Visibility, t + 5_000.0, None));
    // A refused report does not move the window: 5 s after the last sent.
    assert!(!diag.allow(Kind::Visibility, t + 9_999.0, None));
    assert!(diag.allow(Kind::Visibility, t + 10_000.0, None));
    // A clock that went backwards does not block reports for ever.
    assert!(diag.allow(Kind::Visibility, t, None));
}

#[test]
fn every_kind_has_its_own_window() {
    let mut diag = Diag::default();
    for kind in ALL {
        assert!(diag.allow(kind, 0.0, None), "{kind:?}");
    }
    for kind in ALL {
        assert!(!diag.allow(kind, 1.0, None), "{kind:?}");
    }
}

#[test]
fn an_error_message_goes_once() {
    let mut diag = Diag::default();
    assert!(diag.allow(Kind::Error, 0.0, Some("boom")));
    assert!(!diag.allow(Kind::Error, 10_000.0, Some("boom")));
    assert!(diag.allow(Kind::Error, 20_000.0, Some("bang")));
    // Two different errors within 5 s: the second waits for the window.
    assert!(!diag.allow(Kind::Error, 21_000.0, Some("crash")));
    assert!(diag.allow(Kind::Error, 25_000.0, Some("crash")));
}

#[test]
fn only_the_latest_error_messages_are_remembered() {
    let mut diag = Diag::default();
    let at = |n: usize| n as f64 * REPORT_GAP_MS;
    for n in 0..=ERRORS_REMEMBERED {
        let message = format!("e{n}");
        assert!(
            diag.allow(Kind::Error, at(n), Some(message.as_str())),
            "{n}"
        );
    }
    let later = at(ERRORS_REMEMBERED + 1);
    // e1 is still remembered; e0, the oldest, was forgotten.
    assert!(!diag.allow(Kind::Error, later, Some("e1")));
    assert!(diag.allow(Kind::Error, later, Some("e0")));
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
