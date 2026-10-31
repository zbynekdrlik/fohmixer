use super::*;

fn key(kind: DeviceKind, device: i32, track: &str) -> Key {
    Key {
        kind,
        device,
        parents: [track, "FOH Set", "-", "-"].map(str::to_string),
    }
}

fn meter(device: i32, track: &str) -> Key {
    key(DeviceKind::Meter, device, track)
}

#[test]
fn the_period_and_silence_on_their_boundaries() {
    assert_eq!(PERIOD_MS, 10_000);
    assert_eq!(SILENT_TENTHS, -700);
    assert!(!above_silence(-700));
    assert!(above_silence(-699));
    assert!(!period_over(100, 10_099));
    assert!(period_over(100, 10_100));
    assert!(!period_over(100, 50), "a clock that went back");
}

#[test]
fn the_peak_is_kept_across_datagrams_on_each_side() {
    let mut peaks = Peaks::default();
    let vox = meter(1, "Vox 1");
    peaks.note(&vox, -200, -100, true, 0);
    peaks.note(&vox, -120, -300, true, 30);
    peaks.note(&vox, -500, -400, true, 60);
    let line = peaks.take(PERIOD_MS).expect("the period is over");
    assert!(!line.warn);
    assert_eq!(
        line.fields,
        json!({
            "what": "meter_peak",
            "ms": 10_000,
            "meters": [{
                "device": 1,
                "parents": ["Vox 1", "FOH Set", "-", "-"],
                "matched": true,
                "left_db": -12.0,
                "right_db": -10.0,
            }],
        })
    );
    assert!(!line.text.is_empty());
}

#[test]
fn each_key_keeps_its_own_peak() {
    let mut peaks = Peaks::default();
    peaks.note(&meter(1, "Vox 1"), -100, -110, true, 0);
    peaks.note(&meter(2, "Vox 2"), -300, -310, false, 10);
    let line = peaks.take(PERIOD_MS).unwrap();
    assert_eq!(
        line.fields["meters"],
        json!([
            {
                "device": 1,
                "parents": ["Vox 1", "FOH Set", "-", "-"],
                "matched": true,
                "left_db": -10.0,
                "right_db": -11.0,
            },
            {
                "device": 2,
                "parents": ["Vox 2", "FOH Set", "-", "-"],
                "matched": false,
                "left_db": -30.0,
                "right_db": -31.0,
            },
        ])
    );
}

#[test]
fn a_key_says_whether_its_latest_value_was_matched() {
    let mut peaks = Peaks::default();
    let vox = meter(1, "Vox 1");
    peaks.note(&vox, -100, -100, false, 0);
    peaks.note(&vox, -200, -200, true, 10);
    let line = peaks.take(PERIOD_MS).unwrap();
    assert_eq!(line.fields["meters"][0]["matched"], true);
}

#[test]
fn the_period_ends_only_after_its_ten_seconds_and_starts_again_empty() {
    let mut peaks = Peaks::default();
    let vox = meter(1, "Vox 1");
    peaks.note(&vox, -100, -100, true, 500);
    assert!(peaks.take(10_499).is_none());
    let line = peaks.take(10_600).expect("over");
    assert_eq!(line.fields["ms"], 10_100);
    assert!(peaks.take(30_000).is_none(), "nothing held any more");
    // A quieter next period shows its own peak, timed from its own start.
    peaks.note(&vox, -400, -410, true, 40_000);
    assert!(peaks.take(49_999).is_none(), "a period of its own");
    let line = peaks.take(50_000).expect("over");
    assert_eq!(line.fields["ms"], 10_000);
    assert_eq!(line.fields["meters"][0]["left_db"], -40.0);
    assert_eq!(line.fields["meters"][0]["right_db"], -41.0);
}

#[test]
fn silence_writes_nothing_and_starts_no_period() {
    let mut peaks = Peaks::default();
    let vox = meter(1, "Vox 1");
    peaks.note(&vox, -700, -700, true, 0);
    assert!(peaks.take(100_000).is_none());
    // The first value above silence starts the period, on either side.
    peaks.note(&vox, -700, -699, true, 100_000);
    assert!(peaks.take(109_999).is_none());
    let line = peaks.take(110_000).expect("over");
    assert_eq!(line.fields["meters"][0]["left_db"], -70.0);
    assert_eq!(line.fields["meters"][0]["right_db"], -69.9);
    peaks.note(&vox, -699, -700, true, 200_000);
    assert!(peaks.take(210_000).is_some());
}

#[test]
fn a_silent_reading_inside_a_period_leaves_its_peak_alone() {
    let mut peaks = Peaks::default();
    let vox = meter(1, "Vox 1");
    peaks.note(&vox, -100, -200, true, 0);
    peaks.note(&vox, -700, -700, false, 10);
    let line = peaks.take(PERIOD_MS).unwrap();
    assert_eq!(line.fields["meters"][0]["left_db"], -10.0);
    assert_eq!(line.fields["meters"][0]["matched"], true);
}

#[test]
fn a_lufs_device_holds_nothing() {
    let mut peaks = Peaks::default();
    peaks.note(&key(DeviceKind::Lufs, 1, "Vox 1"), -100, -100, true, 0);
    assert!(peaks.take(PERIOD_MS).is_none());
}
