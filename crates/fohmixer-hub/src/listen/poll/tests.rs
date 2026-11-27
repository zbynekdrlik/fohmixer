use super::*;
use crate::events::EventLog;
use fohmixer_proto::listen::wire::{Announce, parents_in};

/// A device's socket on 127.0.0.1 (any port: `heard` takes the announce as
/// given, the range is the wire's check) and its announce on `track`.
fn device(id: i32, track: &str) -> (std::net::UdpSocket, Announce) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let announce = Announce {
        device: id,
        version: "v1.60.4".into(),
        parents: parents_in(None, track, "FOH Band"),
        sample_rate: 48_000,
        sending: false,
        port,
    };
    (socket, announce)
}

fn read(socket: &std::net::UdpSocket) -> Vec<u8> {
    let mut buf = [0u8; 64];
    let (n, _) = socket.recv_from(&mut buf).expect("a datagram within 0.5 s");
    buf[..n].to_vec()
}

fn nothing(socket: &std::net::UdpSocket) -> bool {
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut buf = [0u8; 64];
    socket.recv_from(&mut buf).is_err()
}

fn listen() -> Arc<Listen> {
    let (events, _records) = EventLog::channel(16);
    Listen::new(events, super::super::no_sets())
}

#[test]
fn a_round_tells_each_device_its_own_select_and_counts_the_polls() {
    let listen = listen();
    let poller = crate::lufs_udp::poll_socket().unwrap();
    let (vox, vox_announce) = device(1, "Vox 1");
    let (other, other_announce) = device(2, "Vox 2");
    let now = Instant::now();
    listen.heard(vox_announce, now);
    listen.heard(other_announce, now);
    poll_round(&poller, &listen, now);
    assert_eq!(read(&vox), poll_datagram(false));
    assert_eq!(read(&other), poll_datagram(false));
    let _lease = listen.start("band", "FOH Band", "Vox 1", 1, "10.0.0.5");
    poll_round(&poller, &listen, now);
    assert_eq!(read(&vox), poll_datagram(true), "the target's device");
    assert_eq!(read(&other), poll_datagram(false));
    let status = listen.status(now);
    assert_eq!((status.polls, status.poll_failed), (4, 0));
    assert_eq!(status.frame_polls, 0, "a round sends no frame poll");
}

#[test]
fn a_frame_round_drives_the_one_selected_device_only() {
    let listen = listen();
    let poller = crate::lufs_udp::poll_socket().unwrap();
    let (vox, vox_announce) = device(1, "Vox 1");
    let (other, other_announce) = device(2, "Vox 2");
    let now = Instant::now();
    listen.heard(vox_announce, now);
    listen.heard(other_announce, now);
    frame_round(&poller, &listen, now);
    assert!(
        nothing(&vox) && nothing(&other),
        "nobody listens: no frame poll"
    );
    let _lease = listen.start("band", "FOH Band", "Vox 2", 1, "10.0.0.5");
    frame_round(&poller, &listen, now);
    frame_round(&poller, &listen, now);
    other
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    assert_eq!(read(&other), frame_datagram());
    assert_eq!(read(&other), frame_datagram());
    assert!(nothing(&vox));
    let status = listen.status(now);
    assert_eq!(
        (status.frame_polls, status.polls, status.poll_failed),
        (2, 0, 0)
    );
}

#[test]
fn the_sweep_sends_off_to_every_port_of_its_range_once() {
    let listen = listen();
    let poller = crate::lufs_udp::poll_socket().unwrap();
    let (first, _) = device(1, "Vox 1");
    let at = first.local_addr().unwrap().port();
    assert_eq!(sweep(&poller, at..at + 1, &listen), (1, 0));
    assert_eq!(read(&first), poll_datagram(false));
    assert!(nothing(&first), "once");
    let status = listen.status(Instant::now());
    assert_eq!((status.swept, status.sweep_failed), (1, 0));
    assert_eq!(status.polls, 0, "the sweep counts apart");
    assert_eq!(sweep(&poller, at..at, &listen), (0, 0), "an empty range");
}

#[test]
fn the_round_clocks_keep_their_grid() {
    // #264: Windows wakes a timer up to ~15.6 ms late; `Skip` keeps the
    // grid (`hub-rust.md`).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let round = clock(POLL_MS);
        assert_eq!(round.period(), Duration::from_millis(250));
        assert_eq!(
            round.missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Skip
        );
        let frames = clock(FRAME_POLL_MS);
        assert_eq!(frames.period(), Duration::from_millis(20));
        assert_eq!(
            frames.missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Skip
        );
    });
}

#[tokio::test]
async fn the_task_polls_a_device_it_heard_and_names_its_socket() {
    let listen = listen();
    let task = spawn(socket().unwrap(), Arc::clone(&listen), None);
    let (vox, vox_announce) = device(1, "Vox 1");
    listen.heard(vox_announce, Instant::now());
    let from = listen
        .status(Instant::now())
        .poll_from
        .expect("the poll socket");
    let read = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 64];
        let (n, sender) = vox.recv_from(&mut buf).expect("a poll within 0.5 s");
        (buf[..n].to_vec(), sender.to_string())
    })
    .await
    .unwrap();
    assert_eq!(read, (poll_datagram(false), from));
    task.abort();
}

#[test]
fn a_refused_send_is_counted_failed() {
    let listen = listen();
    let now = Instant::now();
    listen.counters.polled(3, 1);
    listen.counters.frame_polled(5, 2);
    listen.counters.swept(7, 4);
    let status = listen.status(now);
    assert_eq!(
        (
            status.polls,
            status.frame_polls,
            status.poll_failed,
            status.swept,
            status.sweep_failed
        ),
        (3, 5, 3, 7, 4)
    );
}

#[test]
fn a_port_two_devices_name_gets_one_move_and_is_polled_for_the_one_that_stays() {
    // #264 (the design comment 7228766032): a device answers on its port
    // after another device named the same port at its load (that one is
    // deaf: Windows gives the port's datagrams to its first binder). The
    // round sends the port one move before its poll; the device that hears
    // takes another port, and the old port is polled on for the other.
    let (events, records) = EventLog::channel(16);
    let listen = Listen::new(events, super::super::no_sets());
    let poller = crate::lufs_udp::poll_socket().unwrap();
    let (vox, vox_announce) = device(1, "Vox 1");
    let port = vox_announce.port;
    let deaf = Announce {
        device: 2,
        version: "v1.60.4".into(),
        parents: parents_in(None, "Vox 2", "FOH Band"),
        sample_rate: 48_000,
        sending: false,
        port,
    };
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    listen.heard(vox_announce.clone(), t0);
    listen.heard(deaf, t0 + ms(10));
    poll_round(&poller, &listen, t0 + ms(10));
    assert_eq!(read(&vox), poll_datagram(false), "no share yet: a poll");
    listen.heard(vox_announce.clone(), t0 + ms(250));
    poll_round(&poller, &listen, t0 + ms(250));
    assert_eq!(read(&vox), move_datagram(), "the move first");
    assert_eq!(read(&vox), poll_datagram(false), "then the port's one poll");
    let record = records.try_recv().expect("a shared_port record");
    assert_eq!(record["ev"], "listen");
    assert_eq!(record["what"], "shared_port");
    assert_eq!(record["port"], port);
    assert_eq!(record["moved"]["device"], 1);
    assert_eq!(record["stays"]["device"], 2);
    assert_eq!(record["stays"]["track"], "Vox 2");
    assert_eq!(record["sent"], true);
    assert_eq!(record["warn"], true);
    // The device that heard names its new port.
    let (_moved_to, moved_announce) = device(1, "Vox 1");
    let to = moved_announce.port;
    listen.heard(moved_announce, t0 + ms(300));
    let record = records.try_recv().expect("a moved record");
    assert_eq!(record["what"], "moved");
    assert_eq!(
        (record["from"].clone(), record["to"].clone()),
        (serde_json::json!(port), serde_json::json!(to))
    );
    poll_round(&poller, &listen, t0 + ms(500));
    assert_eq!(read(&vox), poll_datagram(false), "the old port polled on");
    assert!(nothing(&vox), "no second move within the window");
    // Device 2 unheard past its 3 s, the old port still polled until the
    // move's 5 s end.
    poll_round(&poller, &listen, t0 + ms(4000));
    vox.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    assert_eq!(read(&vox), poll_datagram(false));
    poll_round(&poller, &listen, t0 + ms(5250));
    assert!(nothing(&vox), "the window ended");
}
