use super::*;

fn key(kind: DeviceKind, device: i32) -> Key {
    Key {
        kind,
        device,
        parents: [
            "Vox 1".to_string(),
            "FOH Band".to_string(),
            "-".to_string(),
            "-".to_string(),
        ],
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn the_bounds_constants() {
    assert_eq!((MAX_KEYS, LUFS_PER_SECOND, METER_PER_SECOND), (64, 10, 60));
    assert_eq!((WINDOW, QUIET, FORGET_MS), (ms(1000), ms(10_000), 10_000));
    assert_eq!(per_second(DeviceKind::Lufs), 10);
    assert_eq!(per_second(DeviceKind::Meter), 60);
}

#[test]
fn ten_in_a_second_then_drop() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    let k = key(DeviceKind::Lufs, 1);
    for i in 0..10 {
        assert_eq!(bound.admit(&k, t0 + ms(i * 10)).0, Admit::Take, "{i}");
    }
    assert_eq!(
        bound.admit(&k, t0 + ms(999)),
        (Admit::Drop, Some(Dropping::Started))
    );
    assert_eq!(bound.admit(&k, t0 + ms(999)), (Admit::Drop, None));
    assert_eq!(bound.admit(&k, t0 + ms(1000)), (Admit::Take, None));
}

#[test]
fn meter_keys_take_sixty() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    let k = key(DeviceKind::Meter, 1);
    for i in 0..60 {
        assert_eq!(bound.admit(&k, t0 + ms(i)).0, Admit::Take, "{i}");
    }
    assert_eq!(bound.admit(&k, t0 + ms(500)).0, Admit::Drop);
}

#[test]
fn keys_do_not_share_a_window() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    let (a, b) = (key(DeviceKind::Lufs, 1), key(DeviceKind::Lufs, 2));
    for i in 0..11 {
        bound.admit(&a, t0 + ms(i));
    }
    assert_eq!(bound.admit(&a, t0 + ms(30)).0, Admit::Drop);
    assert_eq!(bound.admit(&b, t0 + ms(30)).0, Admit::Take);
    // The kind is part of the key: a meter of the same id and place too.
    assert_eq!(
        bound.admit(&key(DeviceKind::Meter, 1), t0 + ms(30)).0,
        Admit::Take
    );
    assert_eq!(bound.len(), 3);
}

#[test]
fn the_65th_key_is_full() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    for device in 0..64 {
        assert_eq!(
            bound.admit(&key(DeviceKind::Lufs, device), t0).0,
            Admit::Take
        );
    }
    assert_eq!(
        bound.admit(&key(DeviceKind::Lufs, 64), t0),
        (Admit::Full, None)
    );
    assert_eq!(bound.len(), 64);
    // A kept key still goes.
    assert_eq!(bound.admit(&key(DeviceKind::Lufs, 3), t0).0, Admit::Take);
}

#[test]
fn forgotten_keys_free_room() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    for device in 0..64 {
        bound.admit(&key(DeviceKind::Lufs, device), t0);
    }
    bound.admit(&key(DeviceKind::Lufs, 0), t0 + ms(5000));
    bound.forget_before(t0 + ms(5000));
    assert_eq!(bound.len(), 1);
    assert!(!bound.is_empty());
    assert_eq!(
        bound.admit(&key(DeviceKind::Lufs, 64), t0 + ms(5000)).0,
        Admit::Take
    );
    // A key heard exactly at the cutoff stays.
    bound.forget_before(t0 + ms(5000));
    assert_eq!(bound.len(), 2);
    bound.forget_before(t0 + ms(5001));
    assert!(bound.is_empty());
}

#[test]
fn drop_warning_once_per_change() {
    assert_eq!(drop_warning(false, true), Some(Dropping::Started));
    assert_eq!(drop_warning(true, false), Some(Dropping::Ended));
    assert_eq!(drop_warning(true, true), None);
    assert_eq!(drop_warning(false, false), None);
}

#[test]
fn a_dropping_key_ends_after_ten_quiet_seconds() {
    let t0 = Instant::now();
    let mut bound = Bound::new();
    let k = key(DeviceKind::Lufs, 1);
    for i in 0..10 {
        bound.admit(&k, t0 + ms(i));
    }
    assert_eq!(bound.admit(&k, t0 + ms(100)).1, Some(Dropping::Started));
    // Under the bound but not yet quiet for 10 s: still dropping.
    assert_eq!(bound.admit(&k, t0 + ms(2000)), (Admit::Take, None));
    assert_eq!(bound.admit(&k, t0 + ms(10_099)), (Admit::Take, None));
    assert_eq!(
        bound.admit(&k, t0 + ms(10_100)),
        (Admit::Take, Some(Dropping::Ended))
    );
    assert!(!quiet_since(t0 + ms(100), t0 + ms(10_099)));
    assert!(quiet_since(t0 + ms(100), t0 + ms(10_100)));
}

#[test]
fn a_key_of_a_reading() {
    let reading = Reading {
        kind: DeviceKind::Meter,
        device: 9,
        version: "v1.19.0".to_string(),
        parents: [
            "LUFS".to_string(),
            "LUFS".to_string(),
            "Vox 1".to_string(),
            "FOH Band".to_string(),
        ],
        first: -120,
        second: -180,
    };
    let k = Key::of(&reading);
    assert_eq!((k.kind, k.device), (DeviceKind::Meter, 9));
    assert_eq!(k.parents, reading.parents);
}
