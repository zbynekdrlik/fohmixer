use super::*;
use fohmixer_proto::lufs::wire::DeviceKind;

const P: u16 = 41_234;
const Q: u16 = 45_678;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Every key counts for 500 ms after it was heard (a meter's window).
fn kept_at(now: Instant) -> impl Fn(&char, Instant) -> bool {
    move |_: &char, heard: Instant| now.saturating_duration_since(heard) < ms(500)
}

fn heard(ports: &mut PollPorts<char>, key: char, port: u16, now: Instant) -> Heard<char> {
    ports.heard(&key, port, now, kept_at(now))
}

fn quiet() -> Heard<char> {
    Heard {
        from: None,
        shared: None,
    }
}

#[test]
fn the_older_key_answering_after_a_newer_one_named_its_port_is_shared() {
    // #264 (the design comment 7228766032): Windows gives a port's
    // datagrams to its first binder only, so the device that loaded later
    // announced once and hears nothing; the one that still answers is the
    // older key, heard after the newer one named the port.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    assert_eq!(heard(&mut ports, 'a', P, t0), quiet());
    assert_eq!(heard(&mut ports, 'a', P, t0 + ms(33)), quiet());
    assert_eq!(
        heard(&mut ports, 'b', P, t0 + ms(50)),
        quiet(),
        "the newer key's own announce proves nothing"
    );
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(66)),
        Heard {
            from: None,
            shared: Some(Shared {
                port: P,
                hearing: 'a',
                deaf: 'b',
            }),
        }
    );
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(99)),
        quiet(),
        "one move a port within its window"
    );
    // The hearing device took another port: its key names it.
    assert_eq!(
        heard(&mut ports, 'a', Q, t0 + ms(120)),
        Heard {
            from: Some(P),
            shared: None,
        }
    );
    assert_eq!(ports.len(), 2);
    assert_eq!(ports.ports(), 2);
}

/// Keys of one device in the tests: the same letter, either case (`a` and
/// `A` stand for one device before and after a rename in Live).
fn one_device(a: &char, b: &char) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[test]
fn a_key_that_replaces_another_of_its_device_on_its_port_is_no_share() {
    // A device moved or renamed in Live answers with a new key (its kind
    // and Max id kept) on the same port; the old key is never heard again.
    let t0 = Instant::now();
    let mut ports = PollPorts::with_same(one_device);
    heard(&mut ports, 'a', P, t0);
    for n in 1..60 {
        assert_eq!(heard(&mut ports, 'A', P, t0 + ms(33 * n)), quiet(), "{n}");
    }
    // Two keys that named the port at the same time: no order, no share.
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0);
    assert_eq!(heard(&mut ports, 'a', P, t0 + ms(33)), quiet());
    assert_eq!(heard(&mut ports, 'b', P, t0 + ms(66)), quiet());
}

#[test]
fn a_key_that_went_stale_while_another_device_answers_on_its_port_is_shared() {
    // The design's second trigger (review of #264): the deaf key's window
    // ended before the other one was heard again, or its announce came in
    // before the first binder's (out of bind order): the one that answers
    // hears, the stale one is deaf.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(510)).shared,
        Some(Shared {
            port: P,
            hearing: 'a',
            deaf: 'b',
        }),
        "the newer key's window ended at 510 ms"
    );
    // Out of bind order: the deaf one's announce came first.
    let mut ports = PollPorts::default();
    heard(&mut ports, 'b', P, t0);
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(5)),
        quiet(),
        "b still kept"
    );
    assert_eq!(heard(&mut ports, 'a', P, t0 + ms(499)), quiet());
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(500))
            .shared
            .map(|s| s.deaf),
        Some('b')
    );
}

#[test]
fn a_remembered_key_is_moved_for_again_each_window_until_it_is_forgotten() {
    // A move that did not take (a device from before the move, a datagram
    // lost): the port is moved again each window while the deaf key is
    // remembered, 15 s after it was last heard.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    let mut moves = Vec::new();
    for n in 1..=1000 {
        let now = t0 + ms(33 * n);
        ports.forget(now, kept_at(now));
        if heard(&mut ports, 'a', P, now).shared.is_some() {
            moves.push(33 * n);
        }
    }
    assert_eq!(
        moves,
        vec![33, 5049, 10_065],
        "then b is forgotten at 15 010 ms"
    );
    assert!(remembered(t0, t0 + ms(14_999)));
    assert!(!remembered(t0, t0 + ms(15_000)));
    assert!(remembered(t0 + ms(5), t0), "a clock that went back");
    assert_eq!(REMEMBER_MS, 15_000);
}

#[test]
fn three_keys_on_a_port_are_moved_one_window_apart_until_one_is_left() {
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    heard(&mut ports, 'c', P, t0 + ms(20));
    assert!(heard(&mut ports, 'a', P, t0 + ms(30)).shared.is_some());
    assert_eq!(heard(&mut ports, 'a', Q, t0 + ms(40)).from, Some(P));
    // b binds first now and hears; c still deaf: moved one window on.
    assert_eq!(heard(&mut ports, 'b', P, t0 + ms(400)), quiet());
    assert_eq!(
        heard(&mut ports, 'b', P, t0 + ms(5030)).shared,
        Some(Shared {
            port: P,
            hearing: 'b',
            deaf: 'c',
        })
    );
}

#[test]
fn a_key_that_moved_onto_another_devices_port_is_newer_there() {
    // Its time on the port starts again: the device already there hears.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'c', Q, t0 + ms(10));
    assert_eq!(heard(&mut ports, 'a', Q, t0 + ms(20)).from, Some(P));
    assert_eq!(
        heard(&mut ports, 'c', Q, t0 + ms(30)).shared,
        Some(Shared {
            port: Q,
            hearing: 'c',
            deaf: 'a',
        })
    );
}

#[test]
fn a_key_heard_more_than_its_announce_is_never_taken_for_a_deaf_one() {
    // The second review of #264: a first binder quiet for a moment (a
    // stalled Live, Device On 0) while a newcomer announces, and a device
    // that left and whose port a new device drew, were heard more than
    // once: their staleness proves nothing.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    for n in 0..10 {
        heard(&mut ports, 'a', P, t0 + ms(33 * n));
    }
    assert_eq!(
        heard(&mut ports, 'b', P, t0 + ms(900)),
        quiet(),
        "the newcomer's announce: a quiet first binder"
    );
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(1000)).shared,
        Some(Shared {
            port: P,
            hearing: 'a',
            deaf: 'b',
        }),
        "the first binder answers again: the newcomer is deaf"
    );
    // A device that left; a new one draws its port within the 15 s.
    let mut ports = PollPorts::default();
    for n in 0..10 {
        heard(&mut ports, 'x', P, t0 + ms(33 * n));
    }
    for n in 0..30 {
        let now = t0 + ms(2000 + 33 * n);
        ports.forget(now, kept_at(now));
        assert_eq!(heard(&mut ports, 'y', P, now), quiet(), "{n}");
    }
}

fn lufs_key(kind: DeviceKind, device: i32, track: &str) -> PortKey {
    PortKey::Lufs(Key {
        kind,
        device,
        parents: [track, "FOH Band", "-", "-"].map(str::to_string),
    })
}

#[test]
fn a_meter_and_a_listen_device_on_one_port_are_one_tables_share() {
    // The kinds share the range (review of #264): a meter that binds first
    // and a Listen device of the other Live on its port. The LUFS side
    // takes the move (the meter hears), the Listen side has none, and the
    // port is polled on for both sides meanwhile.
    let table = PortTable::default();
    let t0 = Instant::now();
    let meter = lufs_key(DeviceKind::Meter, 3, "Vox 1");
    assert_eq!(table.heard(meter.clone(), P, t0), None);
    assert_eq!(table.heard(PortKey::Listen(9), P, t0 + ms(10)), None);
    table.heard(meter.clone(), P, t0 + ms(33));
    assert!(
        table.take(|k| !k.is_lufs()).is_empty(),
        "Listen hears nothing"
    );
    assert_eq!(
        table.take(PortKey::is_lufs),
        vec![Shared {
            port: P,
            hearing: meter.clone(),
            deaf: PortKey::Listen(9),
        }]
    );
    assert!(table.take(PortKey::is_lufs).is_empty(), "taken once");
    assert_eq!(table.heard(meter, Q, t0 + ms(66)), Some(P));
    let later = t0 + ms(4000);
    assert_eq!(
        table.due(later, |k| !k.is_lufs()),
        BTreeSet::from([P]),
        "the Listen device's port polled on past its 3 s"
    );
    assert_eq!(table.due(later, PortKey::is_lufs), BTreeSet::from([P]));
    assert_eq!(table.len_of(|_| true), 0, "both stale at 4 s");
    assert_eq!(table.ports_of(|_| true), 0);
}

#[test]
fn a_keys_window_is_its_kinds_and_one_device_is_its_kind_and_id() {
    let t0 = Instant::now();
    let meter = lufs_key(DeviceKind::Meter, 3, "Vox 1");
    let lufs = lufs_key(DeviceKind::Lufs, 3, "Vox 1");
    assert!(meter.kept(t0, t0 + ms(499)));
    assert!(!meter.kept(t0, t0 + ms(500)));
    assert!(lufs.kept(t0, t0 + ms(999)));
    assert!(!lufs.kept(t0, t0 + ms(1000)));
    assert!(PortKey::Listen(1).kept(t0, t0 + ms(2999)));
    assert!(!PortKey::Listen(1).kept(t0, t0 + ms(3000)));
    assert!(meter.kept(t0 + ms(5), t0), "a clock that went back");
    let renamed = lufs_key(DeviceKind::Meter, 3, "Vox 9");
    assert!(PortKey::same_device(&meter, &renamed));
    assert!(!PortKey::same_device(&meter, &lufs), "another kind");
    assert!(!PortKey::same_device(
        &meter,
        &lufs_key(DeviceKind::Meter, 4, "Vox 1")
    ));
    assert!(PortKey::same_device(
        &PortKey::Listen(2),
        &PortKey::Listen(2)
    ));
    assert!(!PortKey::same_device(
        &PortKey::Listen(2),
        &PortKey::Listen(3)
    ));
    assert!(!PortKey::same_device(&PortKey::Listen(3), &meter));
    assert!(meter.is_lufs() && !PortKey::Listen(3).is_lufs());
    // Two Lives count Max ids alike: the set tells their devices apart.
    let other_live = PortKey::Lufs(Key {
        kind: DeviceKind::Meter,
        device: 3,
        parents: ["Vox 1", "FOH Master", "-", "-"].map(str::to_string),
    });
    assert!(!PortKey::same_device(&meter, &other_live));
    // A rename on the port is no share in the hub's table.
    let table = PortTable::default();
    table.heard(meter.clone(), P, t0);
    for n in 1..40 {
        table.heard(renamed.clone(), P, t0 + ms(33 * n));
    }
    assert!(table.take(|_| true).is_empty());
}

#[test]
fn a_moved_port_is_polled_for_its_window_with_its_keys() {
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    assert!(heard(&mut ports, 'a', P, t0 + ms(20)).shared.is_some());
    heard(&mut ports, 'a', Q, t0 + ms(30));
    let moved = t0 + ms(20);
    // Both keys lost: the moved port is still polled, for the move's keys.
    ports.forget(moved + ms(4999), kept_at(moved + ms(4999)));
    assert!(ports.is_empty());
    assert_eq!(
        ports.due(moved + ms(4999), |k| *k == 'b'),
        BTreeSet::from([P])
    );
    assert_eq!(
        ports.due(moved + ms(4999), |k| *k == 'a'),
        BTreeSet::from([P])
    );
    assert!(ports.due(moved + ms(4999), |k| *k == 'c').is_empty());
    ports.forget(moved + ms(5000), kept_at(moved + ms(5000)));
    assert!(ports.due(moved + ms(5000), |_| true).is_empty());
}

#[test]
fn due_names_the_kept_keys_ports_of_the_pick_each_once() {
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0);
    heard(&mut ports, 'c', Q, t0);
    assert_eq!(ports.due(t0, |_| true), BTreeSet::from([P, Q]));
    assert_eq!(ports.due(t0, |k| *k == 'c'), BTreeSet::from([Q]));
    ports.forget(t0 + ms(500), kept_at(t0 + ms(500)));
    assert!(ports.is_empty());
    assert_eq!(ports.ports(), 0);
}

#[test]
fn a_port_is_moved_again_only_after_its_window() {
    let t0 = Instant::now();
    assert!(move_due(None, t0));
    assert!(!move_due(Some(t0), t0 + ms(4999)));
    assert!(move_due(Some(t0), t0 + ms(5000)));
    assert!(!move_due(Some(t0 + ms(10)), t0), "a clock that went back");
    assert!(moving(t0, t0 + ms(4999)));
    assert!(!moving(t0, t0 + ms(5000)));
    // The same pair still on one port after the window: moved again.
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    assert!(heard(&mut ports, 'a', P, t0 + ms(20)).shared.is_some());
    let later = t0 + ms(5020);
    heard(&mut ports, 'b', P, later - ms(10));
    assert!(heard(&mut ports, 'a', P, later - ms(1)).shared.is_none());
    assert_eq!(
        heard(&mut ports, 'a', P, later).shared,
        Some(Shared {
            port: P,
            hearing: 'a',
            deaf: 'b',
        })
    );
}
