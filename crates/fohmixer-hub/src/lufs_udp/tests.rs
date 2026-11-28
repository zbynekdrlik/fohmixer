use super::*;
use fohmixer_proto::lufs::wire::{DeviceKind, Reading, encode};

fn datagram(device: i32) -> Vec<u8> {
    encode(&Reading {
        kind: DeviceKind::Lufs,
        device,
        version: "v1.19.0".to_string(),
        parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
        first: -300,
        second: -253,
        port: None,
    })
}

#[test]
fn the_address_is_localhost_only() {
    assert_eq!(bind_addr(39110).to_string(), "127.0.0.1:39110");
    assert_eq!(*bind_addr(0).ip(), Ipv4Addr::LOCALHOST);
}

#[test]
fn a_datagram_is_taken_bounded_or_refused_and_counted() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let counters = Counters::default();
    let (events, records) = EventLog::channel(16);
    let mut bound = Bound::new();
    let t0 = Instant::now();
    for _ in 0..11 {
        assert!(handle_datagram(
            &datagram(1),
            t0,
            &mut bound,
            None,
            &counters,
            &events,
            &tx
        ));
    }
    assert!(handle_datagram(
        b"junk", t0, &mut bound, None, &counters, &events, &tx
    ));
    let status = counters.status();
    assert_eq!((status.taken, status.dropped, status.full), (10, 1, 0));
    assert_eq!(status.refused.get("truncated"), Some(&1));
    let mut got = 0;
    while let Ok(RouterMsg::Lufs { reading }) = rx.try_recv() {
        assert_eq!(reading.second, -253);
        got += 1;
    }
    assert_eq!(got, 10);
    let record = records.try_recv().expect("one dropping record");
    assert_eq!(record["ev"], "lufs");
    assert_eq!(record["what"], "dropping");
    assert!(records.try_recv().is_err(), "once per key");
    drop(rx);
    assert!(
        !handle_datagram(
            &datagram(1),
            t0 + Duration::from_secs(2),
            &mut bound,
            None,
            &counters,
            &events,
            &tx
        ),
        "the router is gone"
    );
}

#[test]
fn a_linked_datagram_is_parsed_bounded_and_counted_as_the_listeners() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let counters = Arc::new(Counters::default());
    let (events, _records) = EventLog::channel(16);
    let linked = LinkedLufs::new(Arc::clone(&counters), events, tx);
    let t0 = Instant::now();
    assert_eq!(linked.keys(), 0, "nothing heard yet");
    for _ in 0..11 {
        assert!(linked.take(&datagram(2), t0));
    }
    assert!(linked.take(b"junk", t0));
    let status = counters.status();
    assert_eq!((status.taken, status.dropped), (10, 1), "its own bound");
    assert_eq!(status.refused.get("truncated"), Some(&1));
    let mut got = 0;
    while let Ok(RouterMsg::Lufs { reading }) = rx.try_recv() {
        assert_eq!(reading.device, 2);
        got += 1;
    }
    assert_eq!(got, 10);
    // After the sweep's second the key's window is new again.
    assert!(linked.take(&datagram(2), t0 + Duration::from_millis(1500)));
    assert_eq!(counters.status().taken, 11);
    assert_eq!(linked.keys(), 1);
    assert!(linked.take(&datagram(4), t0 + Duration::from_millis(1500)));
    assert_eq!(linked.keys(), 2, "a second device is a second key");
    // A key unheard for FORGET_MS is forgotten at the next sweep.
    let later = t0 + Duration::from_millis(1500 + FORGET_MS + 1);
    assert!(linked.take(&datagram(3), later));
    assert_eq!(linked.keys(), 1, "devices 2 and 4 forgotten, device 3 kept");
    drop(rx);
    assert!(!linked.take(&datagram(2), later + Duration::from_secs(3)));
}

#[tokio::test]
async fn the_tap_gets_every_datagram_as_read() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let counters = Arc::new(Counters::default());
    let (events, _records) = EventLog::channel(16);
    let tap = crate::link::server::tap();
    let mut heard = tap.subscribe();
    let (addr, task) = spawn(0, tx, counters, events, Some(tap.clone()), None).unwrap();
    let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sender.send_to(&datagram(4), addr).unwrap();
    sender.send_to(b"junk", addr).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), heard.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*first, datagram(4).as_slice());
    let second = tokio::time::timeout(Duration::from_secs(5), heard.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*second, b"junk", "before it is parsed");
    let reading = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap();
    assert!(matches!(reading, Some(RouterMsg::Lufs { .. })));
    task.abort();
}

#[test]
fn the_records_fields() {
    assert_eq!(
        listen_fields("127.0.0.1:39110", None),
        json!({"what": "listening", "addr": "127.0.0.1:39110"})
    );
    assert_eq!(
        listen_fields("127.0.0.1:39110", Some("in use"))["warn"],
        json!(true)
    );
    let key = Key::of(&wire::parse(&datagram(3)).unwrap());
    let started = dropping_fields(&key, Dropping::Started);
    assert_eq!(started["what"], "dropping");
    assert_eq!(started["warn"], true);
    assert_eq!(started["device"], 3);
    let ended = dropping_fields(&key, Dropping::Ended);
    assert_eq!(ended["what"], "under_bound");
    assert!(ended.get("warn").is_none());
}

#[test]
fn the_bound_is_swept_once_a_second() {
    // PR #155's mutation gate: the sweep's decision, on its boundary.
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    assert_eq!(SWEEP, ms(1000));
    assert!(!sweep_due(t0, t0));
    assert!(!sweep_due(t0, t0 + ms(999)));
    assert!(sweep_due(t0, t0 + ms(1000)));
    assert!(sweep_due(t0, t0 + ms(2500)));
    assert!(!sweep_due(t0 + ms(1000), t0), "a clock that went back");
}

#[test]
fn counters_count_each_kind() {
    let counters = Counters::default();
    counters.admitted(Admit::Take);
    counters.admitted(Admit::Drop);
    counters.admitted(Admit::Drop);
    counters.admitted(Admit::Full);
    counters.refuse("tags");
    counters.refuse("tags");
    counters.refuse(RECV_ERROR);
    let status = counters.status();
    assert_eq!((status.taken, status.dropped, status.full), (1, 2, 1));
    assert_eq!(status.refused.get("tags"), Some(&2));
    assert_eq!(status.refused.get("recv"), Some(&1));
    assert_eq!((status.addr, status.error), (None, None));
}

fn polled_datagram(device: i32, port: u16) -> Vec<u8> {
    encode(&Reading {
        kind: DeviceKind::Lufs,
        device,
        version: "v1.58.0".to_string(),
        parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
        first: -300,
        second: -253,
        port: Some(port),
    })
}

fn key(kind: DeviceKind, device: i32) -> Key {
    Key {
        kind,
        device,
        parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
    }
}

#[test]
fn a_polled_key_is_polled_until_it_is_lost() {
    // #264: the port is polled while its key is heard; a LUFS key is
    // lost after 1 s, a meter key after 0.5 s (the rule's windows).
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let mut polls = Polls::default();
    assert!(polls.is_empty());
    polls.heard(&key(DeviceKind::Lufs, 1), None, t0);
    assert!(polls.is_empty(), "a pushed-form reading names no port");
    polls.heard(&key(DeviceKind::Lufs, 1), Some(41_000), t0);
    assert!(!polls.is_empty());
    polls.heard(&key(DeviceKind::Lufs, 2), Some(41_000), t0 + ms(300));
    polls.heard(&key(DeviceKind::Meter, 3), Some(42_000), t0);
    assert_eq!(polls.len(), 3);
    assert_eq!(polls.ports(), 2, "a port shared by two keys counts once");
    assert_eq!(
        polls.due(t0 + ms(499), DeviceKind::Lufs),
        BTreeSet::from([41_000]),
        "a port shared by two keys (a device moved) is polled once"
    );
    assert_eq!(
        polls.due(t0 + ms(499), DeviceKind::Meter),
        BTreeSet::from([42_000]),
        "each kind's round polls its own keys only"
    );
    assert!(polls.due(t0 + ms(500), DeviceKind::Meter).is_empty());
    assert_eq!(polls.len(), 2, "the meter key lost at 500 ms");
    assert_eq!(polls.ports(), 1);
    assert_eq!(
        polls.due(t0 + ms(999), DeviceKind::Lufs),
        BTreeSet::from([41_000])
    );
    assert_eq!(
        polls.due(t0 + ms(1000), DeviceKind::Lufs),
        BTreeSet::from([41_000])
    );
    assert_eq!(polls.len(), 1, "key 1 lost at 1000 ms, key 2 heard later");
    // Heard again: kept another window.
    polls.heard(&key(DeviceKind::Lufs, 2), Some(41_000), t0 + ms(1200));
    assert_eq!(
        polls.due(t0 + ms(2199), DeviceKind::Lufs),
        BTreeSet::from([41_000])
    );
    // A round of the other kind drops the lost keys too.
    assert!(polls.due(t0 + ms(2200), DeviceKind::Meter).is_empty());
    assert!(polls.is_empty());
    assert_eq!(polls.ports(), 0);
}

#[test]
fn each_kind_is_polled_at_its_own_rate() {
    // #264: the meter every 33 ms (30 a second), LUFS every 200 ms.
    assert_eq!(poll_every(DeviceKind::Meter), Duration::from_millis(33));
    assert_eq!(poll_every(DeviceKind::Lufs), Duration::from_millis(200));
    assert_eq!(POLLED_KINDS, [DeviceKind::Lufs, DeviceKind::Meter]);
}

#[test]
fn a_sweep_sends_each_port_each_kinds_poll_once_and_counts_it_apart() {
    // A device already running (a hub restarted under a running Live:
    // since #264 both kinds send only when polled) answers its kind's
    // sweep poll and is learned; the sweep's sends count apart from the
    // rounds' polls. Port 0 stands for a refused send.
    let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    device
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let port = device.local_addr().unwrap().port();
    let poller = poll_socket().unwrap();
    let counters = Counters::default();
    let mut ports = vec![0, port];
    ports.sort_unstable();
    assert_eq!(sweep(&poller, ports, &counters), (2, 2));
    let mut buf = [0u8; 64];
    for kind in POLLED_KINDS {
        let (n, from) = device.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], wire::poll_datagram(kind).as_slice());
        assert_eq!(from, poller.local_addr().unwrap());
    }
    let status = counters.status();
    assert_eq!((status.swept, status.sweep_failed), (2, 2));
    assert_eq!((status.polled, status.poll_failed), (0, 0));
    counters.swept(2, 3);
    let status = counters.status();
    assert_eq!((status.swept, status.sweep_failed), (4, 5));
}

#[test]
fn a_send_reset_by_an_earlier_ports_answer_is_tried_once_more() {
    // Windows reports an earlier datagram's ICMP port unreachable on the
    // socket's next send (WSAECONNRESET): that send went nowhere, so it
    // is tried again; any other refusal is a failed send.
    assert!(send_again(io::ErrorKind::ConnectionReset));
    assert!(!send_again(io::ErrorKind::WouldBlock));
    assert!(!send_again(io::ErrorKind::ConnectionRefused));
    assert!(!send_again(io::ErrorKind::InvalidInput));
}

#[tokio::test]
async fn the_poll_socket_is_named_in_the_status() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let counters = Arc::new(Counters::default());
    let (events, _records) = EventLog::channel(16);
    let (_, task) = spawn(0, tx, Arc::clone(&counters), events, None, None).unwrap();
    let from = counters.status().poll_from.expect("the poll socket");
    assert!(from.starts_with("127.0.0.1:"), "{from}");
    assert_eq!(counters.status().swept, 0, "no sweep asked");
    task.abort();
}

#[tokio::test]
async fn a_rounds_clock_keeps_its_grid_after_a_late_tick() {
    // #264's review: on Windows a timer wakes up to ~15.6 ms late; with
    // `Delay` every late tick would push the next one out (33 ms then
    // reads ~47 ms, about 21 polls a second). `Skip` keeps the grid
    // (30 a second on average) and still never sends a burst.
    for kind in POLLED_KINDS {
        let clock = round_clock(kind);
        assert_eq!(clock.period(), poll_every(kind));
        assert_eq!(
            clock.missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Skip
        );
    }
}

#[test]
fn a_clock_that_went_back_loses_no_key() {
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    // The LUFS rule's stale window, through the hub's port table (#264).
    let lufs = PortKey::Lufs(key(DeviceKind::Lufs, 1));
    let meter = PortKey::Lufs(key(DeviceKind::Meter, 1));
    assert!(lufs.kept(t0 + ms(5000), t0));
    assert!(lufs.kept(t0, t0 + ms(999)));
    assert!(!lufs.kept(t0, t0 + ms(1000)));
    assert!(meter.kept(t0, t0 + ms(499)));
    assert!(!meter.kept(t0, t0 + ms(500)));
    assert_eq!(poll_addr(41_234).to_string(), "127.0.0.1:41234");
}

#[test]
fn a_kept_datagram_teaches_its_port_and_a_full_table_teaches_none() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let counters = Counters::default();
    let (events, _records) = EventLog::channel(16);
    let mut bound = Bound::new();
    let mut polls = Polls::default();
    let t0 = Instant::now();
    assert!(handle_datagram(
        &polled_datagram(1, 41_001),
        t0,
        &mut bound,
        Some(&mut polls),
        &counters,
        &events,
        &tx
    ));
    assert_eq!(polls.due(t0, DeviceKind::Lufs), BTreeSet::from([41_001]));
    // Over its bound (dropped, still kept): the port stays known.
    for _ in 0..12 {
        handle_datagram(
            &polled_datagram(1, 41_001),
            t0,
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx,
        );
    }
    assert!(counters.status().dropped > 0);
    assert_eq!(polls.len(), 1);
    // Fill the key table, then a new key finds no room: no port.
    for device in 2..=64 {
        handle_datagram(
            &datagram(device),
            t0,
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx,
        );
    }
    handle_datagram(
        &polled_datagram(99, 41_099),
        t0,
        &mut bound,
        Some(&mut polls),
        &counters,
        &events,
        &tx,
    );
    assert_eq!(counters.status().full, 1);
    assert_eq!(polls.due(t0, DeviceKind::Lufs), BTreeSet::from([41_001]));
    // A link's datagram (no polls) teaches nothing and still goes on.
    assert!(handle_datagram(
        &polled_datagram(1, 41_002),
        t0 + Duration::from_secs(2),
        &mut bound,
        None,
        &counters,
        &events,
        &tx
    ));
}

#[test]
fn a_round_polls_each_due_port_of_its_kind_once_and_counts_it() {
    let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    device
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let port = device.local_addr().unwrap().port();
    let meter = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    meter
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let meter_port = meter.local_addr().unwrap().port();
    let poller = poll_socket().unwrap();
    let counters = Counters::default();
    let mut polls = Polls::default();
    let t0 = Instant::now();
    polls.heard(&key(DeviceKind::Lufs, 1), Some(port), t0);
    polls.heard(&key(DeviceKind::Lufs, 2), Some(port), t0);
    polls.heard(&key(DeviceKind::Meter, 5), Some(meter_port), t0);
    poll_round(&poller, &mut polls, DeviceKind::Lufs, t0, &counters);
    let mut buf = [0u8; 64];
    let n = device.recv(&mut buf).unwrap();
    assert_eq!(&buf[..n], wire::poll_datagram(DeviceKind::Lufs).as_slice());
    let status = counters.status();
    assert_eq!(
        (status.polled, status.poll_failed, status.polling),
        (1, 0, 2),
        "one LUFS poll sent; both kinds' ports polled now"
    );
    // The meter's round: its own poll, to its own port only.
    poll_round(&poller, &mut polls, DeviceKind::Meter, t0, &counters);
    let n = meter.recv(&mut buf).unwrap();
    assert_eq!(&buf[..n], wire::poll_datagram(DeviceKind::Meter).as_slice());
    assert_eq!(counters.status().polled, 2);
    poll_round(
        &poller,
        &mut polls,
        DeviceKind::Lufs,
        t0 + Duration::from_millis(1000),
        &counters,
    );
    let status = counters.status();
    assert_eq!((status.polled, status.polling), (2, 0), "every key lost");
    // A send the socket refuses (port 0) is counted failed, the good
    // one sent: two ports polled.
    polls.heard(
        &key(DeviceKind::Lufs, 3),
        Some(port),
        t0 + Duration::from_secs(2),
    );
    polls.heard(
        &key(DeviceKind::Lufs, 4),
        Some(0),
        t0 + Duration::from_secs(2),
    );
    poll_round(
        &poller,
        &mut polls,
        DeviceKind::Lufs,
        t0 + Duration::from_secs(2),
        &counters,
    );
    let status = counters.status();
    assert_eq!(
        (status.polled, status.poll_failed, status.polling),
        (3, 1, 2)
    );
    assert_eq!(round_counts(3, 1), (2, 1));
    assert_eq!(round_counts(2, 0), (2, 0));
    assert_eq!(round_counts(0, 0), (0, 0));
    counters.polled(2, 3, 4);
    let status = counters.status();
    assert_eq!(
        (status.polled, status.poll_failed, status.polling),
        (5, 4, 4)
    );
}

fn lufs_key(device: i32) -> Key {
    Key {
        kind: DeviceKind::Lufs,
        device,
        parents: ["Vox 2", "FOH Band", "-", "-"].map(str::to_string),
    }
}

#[test]
fn a_port_two_keys_name_gets_one_move_of_the_hearing_kind_and_is_polled_on() {
    // #264 (the design comment 7228766032): a meter answers on its port
    // after a LUFS device of the other Live named it at its load: the
    // LUFS device is deaf (Windows gives the port's datagrams to its
    // first binder). The hub sends the port one meter move, the meter
    // takes another port, and the port is still polled for the LUFS
    // device, which hears from then on. Its socket lies outside the
    // devices' range (a start sweep of another test polls all of it).
    let range = wire::POLL_PORT_MIN..wire::POLL_PORT_MIN + wire::POLL_PORT_SPAN;
    let device = (0..64)
        .map(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap())
        .find(|s| !range.contains(&s.local_addr().unwrap().port()))
        .expect("a port outside the devices' range");
    device
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let port = device.local_addr().unwrap().port();
    let poller = poll_socket().unwrap();
    let (events, records) = EventLog::channel(16);
    let mut polls = Polls::default();
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let meter = key(DeviceKind::Meter, 1);
    let lufs = lufs_key(2);
    polls.heard(&meter, Some(port), t0);
    polls.heard(&lufs, Some(port), t0 + ms(10));
    assert_eq!(send_moves(&poller, &mut polls, &events), 0, "no share yet");
    polls.heard(&meter, Some(port), t0 + ms(33));
    assert_eq!(send_moves(&poller, &mut polls, &events), 1);
    let mut buf = [0u8; 64];
    let n = device.recv(&mut buf).unwrap();
    assert_eq!(&buf[..n], wire::move_datagram(DeviceKind::Meter).as_slice());
    let record = records.try_recv().expect("a shared_port record");
    assert_eq!(record["ev"], "lufs");
    assert_eq!(record["what"], "shared_port");
    assert_eq!(record["port"], port);
    assert_eq!(record["warn"], true);
    assert_eq!(record["sent"], true);
    assert_eq!(record["moved"]["kind"], "meter");
    assert_eq!(record["moved"]["device"], 1);
    assert_eq!(record["stays"]["kind"], "lufs");
    assert_eq!(record["stays"]["device"], 2);
    // Within the window: no second move.
    polls.heard(&meter, Some(port), t0 + ms(66));
    assert_eq!(send_moves(&poller, &mut polls, &events), 0);
    // The meter names its new port: one moved record from the caller.
    assert_eq!(polls.heard(&meter, Some(41_999), t0 + ms(99)), Some(port));
    assert_eq!(polls.heard(&meter, Some(41_999), t0 + ms(132)), None);
    // The LUFS device's window ends at 1010 ms, yet its port is polled
    // in its kind's round until the move's window ends.
    let late = t0 + ms(2000);
    polls.heard(&meter, Some(41_999), late);
    assert_eq!(
        polls.due(late, DeviceKind::Lufs),
        BTreeSet::from([port]),
        "the port the LUFS device stays on"
    );
    assert_eq!(
        polls.due(late, DeviceKind::Meter),
        BTreeSet::from([port, 41_999])
    );
    assert!(polls.due(t0 + ms(5033), DeviceKind::Lufs).is_empty());
}

#[test]
fn a_move_the_socket_refuses_is_recorded_unsent() {
    let poller = poll_socket().unwrap();
    let (events, records) = EventLog::channel(16);
    let mut polls = Polls::default();
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    // Port 0 stands for a refused send.
    polls.heard(&key(DeviceKind::Lufs, 1), Some(0), t0);
    polls.heard(&lufs_key(2), Some(0), t0 + ms(10));
    polls.heard(&key(DeviceKind::Lufs, 1), Some(0), t0 + ms(200));
    assert_eq!(send_moves(&poller, &mut polls, &events), 1);
    let record = records.try_recv().unwrap();
    assert_eq!(record["sent"], false);
    assert_eq!(record["moved"]["kind"], "lufs");
}

#[test]
fn the_move_records_fields() {
    let shared = Shared {
        port: 41_234,
        hearing: PortKey::Lufs(key(DeviceKind::Meter, 3)),
        deaf: PortKey::Listen(4),
    };
    assert_eq!(
        shared_fields(&shared, true),
        json!({
            "what": "shared_port",
            "port": 41_234,
            "moved": {"kind": "meter", "device": 3, "parents": ["Vox 1", "FOH Band", "-", "-"]},
            "stays": {"kind": "listen", "device": 4},
            "move": "/fohmixer/meter/move",
            "sent": true,
            "warn": true,
        })
    );
    assert_eq!(
        moved_fields(&key(DeviceKind::Lufs, 5), 41_234, 45_678),
        json!({
            "what": "moved",
            "kind": "lufs",
            "device": 5,
            "parents": ["Vox 1", "FOH Band", "-", "-"],
            "from": 41_234,
            "to": 45_678,
        })
    );
}

#[test]
fn a_kept_datagram_on_a_new_port_is_one_moved_record() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let counters = Counters::default();
    let (events, records) = EventLog::channel(16);
    let mut bound = Bound::new();
    let mut polls = Polls::default();
    let t0 = Instant::now();
    for (port, at) in [(41_001, 0), (41_001, 200), (41_002, 400)] {
        handle_datagram(
            &polled_datagram(1, port),
            t0 + Duration::from_millis(at),
            &mut bound,
            Some(&mut polls),
            &counters,
            &events,
            &tx,
        );
    }
    let record = records.try_recv().expect("a moved record");
    assert_eq!(record["what"], "moved");
    assert_eq!(
        (record["from"].clone(), record["to"].clone()),
        (json!(41_001), json!(41_002))
    );
    assert!(records.try_recv().is_err(), "one record");
}

/// A device socket outside the devices' range (a start sweep of another
/// test polls all of it), reading for 5 s.
fn outside_device() -> std::net::UdpSocket {
    let range = wire::POLL_PORT_MIN..wire::POLL_PORT_MIN + wire::POLL_PORT_SPAN;
    let device = (0..64)
        .map(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap())
        .find(|s| !range.contains(&s.local_addr().unwrap().port()))
        .expect("a port outside the devices' range");
    device
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    device
}

#[test]
fn every_kinds_move_is_the_lufs_the_meter_and_the_listen_one() {
    use fohmixer_proto::listen::wire as listen_wire;
    assert_eq!(
        every_move(),
        vec![
            (
                wire::LUFS_MOVE_ADDRESS,
                wire::move_datagram(DeviceKind::Lufs)
            ),
            (
                wire::METER_MOVE_ADDRESS,
                wire::move_datagram(DeviceKind::Meter)
            ),
            (listen_wire::MOVE_ADDRESS, listen_wire::move_datagram()),
        ]
    );
    assert_eq!(OS_TABLE_EVERY, Duration::from_secs(30));
}

#[test]
fn a_port_two_processes_bind_gets_every_kinds_move_and_is_polled_on() {
    // #264 (the design comment 7231753561): a collision from before the
    // hub's start: the sweep reached the first binder only, so the hub
    // knows no key of the deaf device. The OS's table names the port bound
    // by two Lives; the hub moves whichever device hears there and polls
    // the port on for every kind, so the deaf one is learned.
    let device = outside_device();
    let port = device.local_addr().unwrap().port();
    let range = port..port + 1;
    let poller = poll_socket().unwrap();
    let (events, records) = EventLog::channel(16);
    let mut polls = Polls::default();
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let rows = vec![
        (port, 10),
        (port, 20),
        (port, 20),
        (port + 1, 10),
        (port + 1, 30),
    ];
    assert_eq!(
        os_moves(&poller, &mut polls, Ok(rows.clone()), &range, t0, &events),
        1
    );
    let mut buf = [0u8; 64];
    for (_, datagram) in every_move() {
        let n = device.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], datagram.as_slice());
    }
    let record = records.try_recv().expect("a shared_port record");
    assert_eq!(record["ev"], "lufs");
    assert_eq!(record["what"], "shared_port");
    assert_eq!(record["found"], "os_table");
    assert_eq!(record["port"], port);
    assert_eq!(record["pids"], 2);
    assert_eq!(record["sent"], true);
    assert_eq!(record["warn"], true);
    assert!(
        records.try_recv().is_err(),
        "one record: the other port is outside the range"
    );
    for kind in POLLED_KINDS {
        assert_eq!(polls.due(t0 + ms(4999), kind), BTreeSet::from([port]));
    }
    // Within the window nothing goes again; after it, a share still there
    // is moved again.
    assert_eq!(
        os_moves(
            &poller,
            &mut polls,
            Ok(rows.clone()),
            &range,
            t0 + ms(4999),
            &events
        ),
        0
    );
    assert!(records.try_recv().is_err());
    assert!(polls.due(t0 + ms(5000), DeviceKind::Lufs).is_empty());
    assert_eq!(
        os_moves(
            &poller,
            &mut polls,
            Ok(rows),
            &range,
            t0 + ms(5000),
            &events
        ),
        1
    );
    assert!(records.try_recv().is_ok());
}

#[test]
fn a_table_that_could_not_be_read_moves_nothing() {
    let poller = poll_socket().unwrap();
    let (events, records) = EventLog::channel(16);
    let mut polls = Polls::default();
    let t0 = Instant::now();
    let failed = Err(io::Error::other("the table"));
    assert_eq!(
        os_moves(&poller, &mut polls, failed, &(40_000..50_000), t0, &events),
        0
    );
    assert!(records.try_recv().is_err());
    assert!(polls.due(t0, DeviceKind::Lufs).is_empty());
}

#[test]
fn an_os_found_move_the_socket_refuses_is_recorded_unsent() {
    let poller = poll_socket().unwrap();
    let (events, records) = EventLog::channel(16);
    let mut polls = Polls::default();
    // Port 0 stands for a refused send.
    let rows = vec![(0, 1), (0, 2), (0, 3)];
    assert_eq!(
        os_moves(
            &poller,
            &mut polls,
            Ok(rows),
            &(0..1),
            Instant::now(),
            &events
        ),
        1
    );
    let record = records.try_recv().unwrap();
    assert_eq!(record["sent"], false);
    assert_eq!(record["pids"], 3);
}

#[test]
fn the_os_found_records_fields() {
    assert_eq!(
        os_shared_fields(41_234, 2, true),
        json!({
            "what": "shared_port",
            "found": "os_table",
            "port": 41_234,
            "pids": 2,
            "moves": ["/fohmixer/lufs/move", "/fohmixer/meter/move", "/fohmixer/listen/move"],
            "sent": true,
            "warn": true,
        })
    );
}
