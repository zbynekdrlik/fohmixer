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
    (Listen::new(events), records)
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
fn a_start_is_new_a_join_or_busy() {
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
        Start::Busy
    );
    assert_eq!(
        start_of(Some(&target), "master", "FOH Band", "Vox 1"),
        Start::Busy
    );
    assert_eq!(
        start_of(Some(&target), "band", "Other", "Vox 1"),
        Start::Busy
    );
}

#[test]
fn the_select_names_the_device_stops_or_says_nothing() {
    assert_eq!(select_now(Some(Choice::Device(4)), None), Some(4));
    assert_eq!(select_now(Some(Choice::NoDevice), Some(4)), Some(0));
    assert_eq!(select_now(Some(Choice::TwoDevices), None), Some(0));
    assert_eq!(select_now(None, Some(4)), Some(0), "once after the end");
    assert_eq!(select_now(None, Some(0)), None);
    assert_eq!(select_now(None, None), None);
}

#[test]
fn listeners_share_a_track_and_the_last_one_ends_it() {
    let (listen, records) = listen();
    let t0 = Instant::now();
    listen.heard(announce(5, "Vox 1", "FOH Band"), t0);
    assert!(!listen.accepts(5, t0), "nobody listens");
    assert_eq!(listen.next_select(t0), None);
    let a = listen.start("band", "FOH Band", "Vox 1").unwrap();
    let b = listen.start("band", "FOH Band", "Vox 1").unwrap();
    assert!(listen.start("band", "FOH Band", "Vox 2").is_err());
    assert!(listen.accepts(5, t0));
    assert!(!listen.accepts(6, t0));
    let status = listen.status(t0);
    let target = status.target.unwrap();
    assert_eq!((target.listeners, target.device), (2, Some(5)));
    assert_eq!(listen.next_select(t0), Some(Select { device: 5, turn: 1 }));
    drop(a);
    assert!(listen.accepts(5, t0), "one listener left");
    drop(b);
    assert!(!listen.accepts(5, t0));
    assert_eq!(listen.status(t0).target, None);
    assert_eq!(listen.next_select(t0), Some(Select { device: 0, turn: 1 }));
    assert_eq!(listen.next_select(t0), None, "the stop goes once");
    assert_eq!(listen.status(t0).selects, 2);
    let start = records.try_recv().unwrap();
    assert_eq!(start["what"], "start");
    assert_eq!(start["track"], "Vox 1");
    assert_eq!(start["turn"], 1);
    assert_eq!(records.try_recv().unwrap()["what"], "stop");
    assert!(records.try_recv().is_err(), "a join records nothing");
    // The next track is the next turn.
    let _c = listen.start("band", "FOH Band", "Vox 2").unwrap();
    assert_eq!(
        listen.next_select(t0),
        Some(Select { device: 0, turn: 2 }),
        "no device on Vox 2"
    );
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
    assert_eq!(rx.recv().await.unwrap(), Bytes::from_static(b"op"));
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
    let (listen, tasks) = spawn(port, 1, events);
    assert!(tasks.is_empty());
    let status = listen.status(Instant::now());
    assert_eq!(status.addr, None);
    assert!(status.error.is_some());
    let record = records.try_recv().unwrap();
    assert_eq!(record["what"], "listening");
    assert!(record["error"].is_string());
    let (events, records) = EventLog::channel(16);
    let (listen, tasks) = spawn(0, 1, events);
    assert_eq!(tasks.len(), 2);
    let addr = listen.status(Instant::now()).addr.unwrap();
    assert!(addr.starts_with("127.0.0.1:"), "{addr}");
    assert_eq!(records.try_recv().unwrap()["addr"], addr.as_str());
    for task in tasks {
        task.abort();
    }
}
