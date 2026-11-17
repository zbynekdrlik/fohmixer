use super::*;
use fohmixer_proto::listen::wire::parents_in;

fn announce(device: i32, track: &str, set: &str) -> Announce {
    Announce {
        device,
        version: "v1.50.0".into(),
        parents: parents_in(Some(("Listen", "Listen")), track, set),
        sample_rate: 48_000,
        sending: false,
    }
}

fn heard(list: &[(i32, &str, Instant)]) -> BTreeMap<i32, Heard> {
    list.iter()
        .map(|&(device, track, at)| {
            (
                device,
                Heard {
                    announce: announce(device, track, "FOH Band"),
                    at,
                },
            )
        })
        .collect()
}

fn listen() -> (Arc<Listen>, std::sync::mpsc::Receiver<Value>) {
    let (events, records) = EventLog::channel(64);
    (Listen::new(events, no_sets()), records)
}

#[test]
fn a_device_is_fresh_for_three_seconds() {
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    assert!(fresh(t0, t0));
    assert!(fresh(t0, t0 + ms(2999)));
    assert!(!fresh(t0, t0 + ms(3000)));
    assert!(fresh(t0 + ms(5), t0), "a clock that went back");
}

#[test]
fn the_one_fresh_device_on_the_track_is_chosen() {
    let t0 = Instant::now();
    let later = t0 + Duration::from_millis(3500);
    let devices = heard(&[(1, "Vox 1", t0), (2, "Vox 2", later), (3, "Vox 2", t0)]);
    assert_eq!(
        choose(&devices, "FOH Band", "Vox 2", later),
        Choice::Device(2),
        "device 3 is stale"
    );
    assert_eq!(
        choose(&devices, "FOH Band", "Vox 2", t0),
        Choice::TwoDevices
    );
    assert_eq!(
        choose(&devices, "FOH Band", "Vox 1", later),
        Choice::NoDevice
    );
    assert_eq!(choose(&devices, "Other", "Vox 1", t0), Choice::NoDevice);
    assert_eq!(Choice::Device(1).problem(), None);
    assert_eq!(Choice::NoDevice.problem(), Some("no device"));
    assert_eq!(Choice::TwoDevices.problem(), Some("two devices"));
}

#[test]
fn a_start_is_new_a_join_or_a_take() {
    let target = Target {
        instance: "band".into(),
        set: "FOH Band".into(),
        track: "Vox 1".into(),
        listeners: 1,
    };
    assert_eq!(start_of(None, "band", "FOH Band", "Vox 1"), Start::New);
    assert_eq!(
        start_of(Some(&target), "band", "FOH Band", "Vox 1"),
        Start::Join
    );
    assert_eq!(
        start_of(Some(&target), "band", "FOH Band", "Vox 2"),
        Start::Take
    );
    assert_eq!(
        start_of(Some(&target), "master", "FOH Band", "Vox 1"),
        Start::Take
    );
    assert_eq!(
        start_of(Some(&target), "band", "Other", "Vox 1"),
        Start::Take
    );
}

#[test]
fn listeners_share_a_track_and_the_last_one_ends_it() {
    let (listen, records) = listen();
    let t0 = Instant::now();
    listen.heard(announce(5, "Vox 1", "FOH Band"), t0);
    assert!(!listen.accepts(5, t0), "nobody listens");
    assert!(!listen.select(5, t0));
    let a = listen.start("band", "FOH Band", "Vox 1", 1, "10.0.0.5");
    let b = listen.start("band", "FOH Band", "Vox 1", 2, "10.0.0.6");
    assert_eq!((a.turn(), b.turn()), (1, 1), "a join keeps the turn");
    assert!(listen.accepts(5, t0));
    assert!(!listen.accepts(6, t0));
    let status = listen.status(t0);
    let target = status.target.unwrap();
    assert_eq!((target.listeners, target.device), (2, Some(5)));
    assert!(listen.select(5, t0));
    assert!(!listen.select(6, t0), "each device is told on its own");
    drop(a);
    assert!(listen.accepts(5, t0), "one listener left");
    drop(b);
    assert!(!listen.accepts(5, t0));
    assert_eq!(listen.status(t0).target, None);
    assert!(!listen.select(5, t0), "the end stops it");
    assert_eq!(listen.status(t0).selects, 4, "every question counted");
    let start = records.try_recv().unwrap();
    assert_eq!(start["what"], "start");
    assert_eq!(start["track"], "Vox 1");
    assert_eq!(start["turn"], 1);
    assert_eq!(records.try_recv().unwrap()["what"], "stop");
    assert!(records.try_recv().is_err(), "a join records nothing");
    // The next track is the next turn.
    let c = listen.start("band", "FOH Band", "Vox 2", 3, "10.0.0.7");
    assert_eq!(c.turn(), 2);
    assert!(!listen.select(5, t0), "no device on Vox 2");
    assert_eq!(
        listen.status(t0).target.unwrap().problem.as_deref(),
        Some("no device")
    );
}

#[test]
fn a_stale_device_is_forgotten_at_the_next_announce() {
    let (listen, _records) = listen();
    let t0 = Instant::now();
    listen.heard(announce(1, "Vox 1", "FOH Band"), t0);
    let later = t0 + Duration::from_millis(3000);
    assert!(listen.status(later).devices.is_empty(), "not shown");
    listen.heard(announce(2, "Vox 2", "FOH Band"), later);
    let status = listen.status(later);
    assert_eq!(status.devices.len(), 1);
    assert_eq!(status.devices[0].device, 2);
    assert_eq!(status.devices[0].age_ms, 0);
    assert_eq!(status.devices[0].set, "FOH Band");
    assert_eq!(status.devices[0].sample_rate, 48_000);
}

#[tokio::test]
async fn packets_reach_every_subscriber_and_are_counted() {
    let (listen, _records) = listen();
    let mut rx = listen.subscribe();
    listen.send_packet(Bytes::from_static(b"op"));
    // Bounded: a packet that never comes fails here, never hangs the suite.
    assert_eq!(
        rx.try_recv()
            .expect("the packet is in the channel at once")
            .1,
        Bytes::from_static(b"op")
    );
    assert_eq!(listen.status(Instant::now()).packets, 1);
    listen.counters.refuse("x");
    listen.counters.refuse("x");
    assert_eq!(listen.status(Instant::now()).refused.get("x"), Some(&2));
    assert_eq!(PACKETS, 64);
}

#[tokio::test]
async fn a_port_taken_is_recorded_and_shown() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let (events, records) = EventLog::channel(16);
    let (listen, tasks) = spawn(port, events, no_sets());
    assert!(tasks.is_empty());
    let status = listen.status(Instant::now());
    assert_eq!(status.addr, None);
    assert!(status.error.is_some());
    let record = records.try_recv().unwrap();
    assert_eq!(record["what"], "listening");
    assert!(record["error"].is_string());
    let (events, records) = EventLog::channel(16);
    let (listen, tasks) = spawn(0, events, no_sets());
    assert_eq!(tasks.len(), 2);
    let addr = listen.status(Instant::now()).addr.unwrap();
    assert!(addr.starts_with("127.0.0.1:"), "{addr}");
    assert_eq!(records.try_recv().unwrap()["addr"], addr.as_str());
    for task in tasks {
        task.abort();
    }
}

#[test]
fn at_most_256_devices_are_kept() {
    assert_eq!(MAX_DEVICES, 256);
    assert!(keeps(false, 255));
    assert!(!keeps(false, 256));
    assert!(keeps(true, 256), "a known device always");
    let (listen, _records) = listen();
    let t0 = Instant::now();
    for device in 1..=257 {
        listen.heard(announce(device, "Vox 1", "FOH Band"), t0);
    }
    let status = listen.status(t0);
    assert_eq!(status.devices.len(), 256);
    assert_eq!(status.refused.get("devices"), Some(&1));
    listen.heard(announce(5, "Vox 2", "FOH Band"), t0);
    assert_eq!(listen.status(t0).refused.get("devices"), Some(&1));
}

#[tokio::test]
async fn the_newest_start_takes_the_track_and_the_old_leases_leave_nothing() {
    let (listen, records) = listen();
    let t0 = Instant::now();
    listen.heard(announce(5, "Vox 1", "FOH Band"), t0);
    listen.heard(announce(6, "Vox 2", "FOH Band"), t0);
    let mut turns = listen.turns();
    let a = listen.start("band", "FOH Band", "Vox 1", 1, "10.0.0.5");
    let b = listen.start("band", "FOH Band", "Vox 1", 2, "10.0.0.6");
    assert_eq!(*turns.borrow_and_update(), 1);
    let c = listen.start("band", "FOH Band", "Vox 2", 3, "10.0.0.7");
    assert_eq!(c.turn(), 2);
    assert!(turns.has_changed().unwrap());
    assert_eq!(
        *turns.borrow_and_update(),
        2,
        "the old pages see a new turn"
    );
    assert!(listen.accepts(6, t0) && !listen.accepts(5, t0));
    // A packet carries the turn it was sent in.
    let mut rx = listen.subscribe();
    listen.send_packet(Bytes::from_static(b"x"));
    assert_eq!(rx.try_recv().unwrap().0, 2);
    // The taken pages leaving changes nothing.
    drop(a);
    drop(b);
    let target = listen.status(t0).target.unwrap();
    assert_eq!((target.track.as_str(), target.listeners), ("Vox 2", 1));
    drop(c);
    assert_eq!(listen.status(t0).target, None);
    let all: Vec<Value> = records.try_iter().collect();
    let mut taken: Vec<Value> = all
        .iter()
        .filter(|r| r["what"] == "taken")
        .cloned()
        .collect();
    assert_eq!(taken.len(), 1);
    let record = taken[0].as_object_mut().unwrap();
    assert_eq!(record.remove("ev"), Some(serde_json::json!("listen")));
    assert!(record.remove("ts").is_some());
    assert_eq!(
        taken[0],
        serde_json::json!({"what": "taken",
            "from": {"instance": "band", "set": "FOH Band", "track": "Vox 1", "listeners": 2},
            "to": {"instance": "band", "set": "FOH Band", "track": "Vox 2", "listeners": 1},
            "client": 3, "peer": "10.0.0.7"})
    );
    let whats: Vec<&str> = all.iter().filter_map(|r| r["what"].as_str()).collect();
    assert_eq!(whats, ["start", "taken", "start", "stop"]);
}

#[test]
fn a_set_loaded_while_listening_is_followed() {
    let (events, _records) = EventLog::channel(16);
    let set = Arc::new(Mutex::new(Some("FOH Band".to_string())));
    let read = Arc::clone(&set);
    let sets: SetNames = Arc::new(move |instance: &str| {
        assert_eq!(instance, "band");
        read.lock().unwrap().clone()
    });
    let listen = Listen::new(events, sets);
    let t0 = Instant::now();
    listen.heard(announce(5, "Vox 1", "FOH Band"), t0);
    listen.heard(announce(9, "Vox 1", "Other Set"), t0);
    let _lease = listen.start("band", "FOH Band", "Vox 1", 1, "10.0.0.5");
    assert!(listen.accepts(5, t0));
    assert!(listen.select(5, t0));
    *set.lock().unwrap() = Some("Other Set".to_string());
    assert!(listen.accepts(9, t0) && !listen.accepts(5, t0));
    assert!(listen.select(9, t0) && !listen.select(5, t0));
    assert_eq!(listen.status(t0).target.unwrap().set, "Other Set");
    // An instance whose set is unknown for a moment keeps the last one.
    *set.lock().unwrap() = None;
    assert!(listen.accepts(9, t0));
}
