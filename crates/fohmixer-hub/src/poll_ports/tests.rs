use super::*;

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

#[test]
fn a_key_that_replaces_another_on_its_port_is_no_share() {
    // A device moved or renamed in Live answers with a new key on the same
    // port; the old key is never heard again.
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    for n in 1..10 {
        assert_eq!(heard(&mut ports, 'b', P, t0 + ms(33 * n)), quiet());
    }
    // Two keys that named the port at the same time: no order, no share.
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0);
    assert_eq!(heard(&mut ports, 'a', P, t0 + ms(33)), quiet());
    assert_eq!(heard(&mut ports, 'b', P, t0 + ms(66)), quiet());
}

#[test]
fn a_newer_key_no_longer_kept_is_no_share() {
    let t0 = Instant::now();
    let mut ports = PollPorts::default();
    heard(&mut ports, 'a', P, t0);
    heard(&mut ports, 'b', P, t0 + ms(10));
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(510)),
        quiet(),
        "the newer key's window ended at 510 ms"
    );
    assert_eq!(
        heard(&mut ports, 'a', P, t0 + ms(509)).shared.map(|s| s.deaf),
        Some('b'),
        "a clock read late still judges by its own now"
    );
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
