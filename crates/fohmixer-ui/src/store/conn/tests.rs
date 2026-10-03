use super::*;

fn spec(track: &str, prop: &str) -> SubSpec {
    SubSpec::new(
        "band",
        format!("live_set tracks[name={track}]"),
        prop,
        false,
    )
}

fn layout() -> Layout {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/import-tosc/fixtures/expected-layout.json"
    ));
    serde_json::from_str(text).expect("the imported layout parses")
}

fn view(online: bool, set_name: &str) -> InstanceView {
    InstanceView {
        online,
        busy: false,
        set_name: set_name.to_string(),
    }
}

#[test]
fn reconnects_wait_half_a_second_doubling_to_two_until_a_hello() {
    let mut c = Conn::default();
    let waits: Vec<f64> = (0..4).map(|_| c.retry_delay()).collect();
    assert_eq!(waits, vec![500.0, 1000.0, 2000.0, 2000.0]);
    c.opened(0.0);
    c.hello(0.0);
    assert_eq!(c.retry_delay(), 500.0, "a hello starts the schedule again");
}

#[test]
fn a_socket_waits_for_its_hello_then_pings_until_it_falls_silent() {
    let mut c = Conn::default();
    let s = c.opened(10_000.0);
    assert!(!c.ready());
    assert_eq!(c.tick(s, 10_500.0, false), Tick::Wait, "no hello: no ping");
    let hello = c.hello(10_800.0);
    assert!(c.ready());
    assert!(hello.auto_refresh);
    assert_eq!(c.tick(s, 11_000.0, true), Tick::Ping);
    // Silence is counted from the last message heard.
    c.heard(12_000.0);
    for at in [12_000.0, 13_000.0, 14_000.0, 14_999.0] {
        assert_eq!(c.tick(s, at, true), Tick::Ping, "{at}");
    }
    assert_eq!(
        c.tick(s, 15_000.0, true),
        Tick::Silent,
        "3 s without a message"
    );
}

#[test]
fn an_open_socket_without_a_hello_is_the_wrong_protocol_a_connecting_one_is_silent() {
    let mut c = Conn::default();
    let s = c.opened(1_000.0);
    for at in [2_000.0, 3_000.0, 3_999.0] {
        assert_eq!(c.tick(s, at, true), Tick::Wait, "{at}");
    }
    assert_eq!(c.tick(s, 4_000.0, true), Tick::NoHello);
    assert_eq!(
        c.tick(s, 4_000.0, false),
        Tick::Silent,
        "still connecting after 3 s: a dead network, not a wrong hub"
    );
    c.hello(4_100.0);
    for at in [5_100.0, 6_100.0] {
        assert_eq!(c.tick(s, at, true), Tick::Ping, "{at}");
    }
    assert_eq!(
        c.tick(s, 7_100.0, true),
        Tick::Silent,
        "after its hello an open socket that falls silent is half-open"
    );
}

#[test]
fn a_late_tick_gives_the_socket_a_new_silence_window() {
    let mut c = Conn::default();
    let s = c.opened(0.0);
    c.hello(0.0);
    assert_eq!(c.tick(s, 1_000.0, true), Tick::Ping);
    // The next tick fires a minute late (a throttled tab, a blocked main
    // thread): that says nothing about the socket. A ping, and 3 s for its
    // answer from now.
    assert_eq!(c.tick(s, 61_000.0, true), Tick::Ping);
    for at in [62_000.0, 63_000.0] {
        assert_eq!(c.tick(s, at, true), Tick::Ping, "{at}");
    }
    assert_eq!(
        c.tick(s, 64_000.0, true),
        Tick::Silent,
        "3 s after the late tick, still nothing heard"
    );
}

#[test]
fn a_tick_two_periods_after_the_last_is_on_time() {
    let mut c = Conn::default();
    let s = c.opened(0.0);
    c.hello(0.0);
    assert_eq!(c.tick(s, 1_000.0, true), Tick::Ping);
    assert_eq!(
        c.tick(s, 3_000.0, true),
        Tick::Silent,
        "2 s after the last tick is not late: nothing heard for 3 s"
    );
}

#[test]
fn a_watchdog_ends_with_its_socket() {
    let mut c = Conn::default();
    let first = c.opened(0.0);
    let second = c.opened(0.0);
    assert_ne!(first, second);
    assert_eq!(c.tick(first, 1.0, true), Tick::Done, "replaced");
    assert_eq!(c.tick(second, 1.0, true), Tick::Wait);
    assert!(c.closed().reconnect, "an unstopped store reconnects");
    assert_eq!(c.tick(second, 1.0, true), Tick::Done, "closed");
    let third = c.opened(0.0);
    assert_ne!(third, first);
    assert_ne!(third, second);
    assert_eq!(c.tick(third, 1.0, true), Tick::Wait);
    assert!(!c.stopped());
    c.stop();
    assert_eq!(c.tick(third, 1.0, true), Tick::Done, "stopped");
    assert!(c.stopped());
    assert!(!c.closed().reconnect, "a stopped store does not reconnect");
}

#[test]
fn only_a_close_after_a_hello_is_a_lost_connection() {
    let mut c = Conn::default();
    // An attempt that never said hello (the hub down, a reload close).
    c.opened(0.0);
    let attempt = c.closed();
    assert_eq!(
        attempt,
        Closed {
            reconnect: true,
            lost: false
        }
    );
    // A connection that said hello, then closed.
    c.opened(0.0);
    c.hello(0.0);
    let lost = c.closed();
    assert_eq!(
        lost,
        Closed {
            reconnect: true,
            lost: true
        }
    );
    // Its close counts once: a second close of the same socket is no news.
    assert!(!c.closed().lost);
    // After a stop nothing reconnects and nothing is lost.
    c.opened(0.0);
    c.hello(0.0);
    c.stop();
    assert_eq!(
        c.closed(),
        Closed {
            reconnect: false,
            lost: false
        }
    );
}

#[test]
fn a_close_or_a_stop_ends_sending() {
    let mut c = Conn::default();
    c.want(vec![spec("Hand1 #", "mute")]);
    c.opened(0.0);
    c.hello(0.0);
    assert_eq!(c.refresh(), Some(vec![spec("Hand1 #", "mute")]));
    c.closed();
    assert!(!c.ready());
    assert_eq!(c.refresh(), None, "no refresh without a socket");
    c.opened(0.0);
    c.hello(0.0);
    c.stop();
    assert!(!c.ready());
    assert_eq!(c.refresh(), None);
}

#[test]
fn only_the_first_hello_of_the_page_refreshes_and_every_hello_resubscribes() {
    let mut c = Conn::default();
    let (change, ready) = c.want(vec![spec("Hand1 #", "mute"), spec("Hand2 #", "mute")]);
    assert!(!ready, "before the hello the set waits");
    assert_eq!(change.added.len(), 2);
    assert_eq!(c.wanted_len(), 2);
    c.opened(0.0);
    let first = c.hello(0.0);
    assert!(first.auto_refresh);
    assert_eq!(
        first.specs,
        vec![spec("Hand1 #", "mute"), spec("Hand2 #", "mute")]
    );
    c.closed();
    c.opened(0.0);
    let again = c.hello(0.0);
    assert!(!again.auto_refresh);
    assert_eq!(again.specs, first.specs);
    let (change, ready) = c.want(vec![spec("Hand2 #", "mute")]);
    assert!(ready);
    assert_eq!(change.removed, vec![spec("Hand1 #", "mute").key()]);
    assert_eq!(c.keys_of(Some("band")), vec![spec("Hand2 #", "mute").key()]);
    assert_eq!(c.keys_of(Some("master")), Vec::<String>::new());
    assert_eq!(c.keys_of(None), vec![spec("Hand2 #", "mute").key()]);
}

#[test]
fn command_ids_count_up() {
    let mut c = Conn::default();
    assert_eq!([c.next_id(), c.next_id()], ["c1", "c2"]);
}

#[test]
fn the_layout_on_screen_is_replaced_only_by_another_one() {
    let mut c = Conn::default();
    assert!(!c.shows(1));
    c.showing(1);
    assert!(c.shows(1));
    assert!(!c.shows(2));
    let served = layout();
    assert!(replaces(None, &served));
    assert!(!replaces(Some(&served), &served.clone()));
    let mut edited = served.clone();
    edited.pages.truncate(1);
    assert!(replaces(Some(&served), &edited));
}

#[test]
fn an_instance_back_online_or_on_another_set_reads_its_ranges_again() {
    let first = instance_change(None, &view(true, "Show"));
    assert_eq!(
        first,
        InstanceChange {
            pending: false,
            ranges: true
        }
    );
    let same = instance_change(Some(&view(true, "Show")), &view(true, "Show"));
    assert_eq!(
        same,
        InstanceChange::default(),
        "the same state changes nothing"
    );
    let back = instance_change(Some(&view(false, "Show")), &view(true, "Show"));
    assert!(back.ranges && !back.pending);
    let other_set = instance_change(Some(&view(true, "Show")), &view(true, "Rehearsal"));
    assert!(other_set.ranges && !other_set.pending);
    let mut busy = view(true, "Show");
    busy.busy = true;
    let idle = instance_change(Some(&busy), &view(true, "Show"));
    assert!(idle.ranges && !idle.pending, "a stall ended: read again");
    let stalled = instance_change(Some(&view(true, "Show")), &busy);
    assert_eq!(
        stalled,
        InstanceChange::default(),
        "going busy changes nothing"
    );
    let gone = instance_change(Some(&view(true, "Show")), &view(false, "Show"));
    assert_eq!(
        gone,
        InstanceChange {
            pending: true,
            ranges: false
        }
    );
    let never = instance_change(None, &view(false, ""));
    assert!(never.pending && !never.ranges);
}

#[test]
fn an_instance_back_online_gets_its_writes_again() {
    let (on, off) = (view(true, "Show"), view(false, "Show"));
    assert!(
        back_online(Some(&off), &on),
        "after a reconnect or a restart"
    );
    assert!(back_online(None, &on), "its first report");
    assert!(!back_online(Some(&on), &on), "the same state");
    assert!(!back_online(Some(&on), &off), "gone");
    assert!(!back_online(Some(&off), &off), "still away");
    assert!(!back_online(None, &off), "never there");
    assert!(
        !back_online(Some(&on), &view(true, "Rehearsal")),
        "another set: the old writes are not its own"
    );
    let mut busy = view(true, "Show");
    busy.busy = true;
    assert!(
        !back_online(Some(&busy), &on),
        "a stall ended: nothing was dropped"
    );
}

#[test]
fn pings_carry_their_number_the_page_time_and_the_latest_round_trip() {
    // #43: the hub logs every ping; 100 ms apart they resolve a short stall.
    assert_eq!(PING_MS, 100);
    let mut c = Conn::default();
    c.opened(0.0);
    assert_eq!(
        c.ping(10.0, 1_000_010.0),
        ClientMsg::Ping {
            n: 0,
            t: 1_000_010.0,
            rtt: None,
            rtt_n: None
        }
    );
    assert_eq!(c.pong(0, 1_000_010.0, 1_000_034.5), 24.5);
    assert_eq!(
        c.ping(110.0, 1_000_110.0),
        ClientMsg::Ping {
            n: 1,
            t: 1_000_110.0,
            rtt: Some(24.5),
            rtt_n: Some(0)
        }
    );
    // Ping 1's pong is late: ping 2 still carries ping 0's round trip, named
    // as ping 0's (the hub pairs it once).
    assert_eq!(
        c.ping(210.0, 1_000_210.0),
        ClientMsg::Ping {
            n: 2,
            t: 1_000_210.0,
            rtt: Some(24.5),
            rtt_n: Some(0)
        }
    );
    assert_eq!(c.pong(1, 1_000_110.0, 1_000_240.0), 130.0);
    assert!(matches!(
        c.ping(310.0, 1_000_310.0),
        ClientMsg::Ping {
            n: 3,
            rtt: Some(130.0),
            rtt_n: Some(1),
            ..
        }
    ));
    // A new socket starts without a round trip.
    c.opened(400.0);
    assert!(matches!(
        c.ping(400.0, 1_000_400.0),
        ClientMsg::Ping {
            n: 4,
            rtt: None,
            rtt_n: None,
            ..
        }
    ));
    // The number wraps instead of overflowing.
    c.next_ping = u32::MAX;
    assert!(matches!(
        c.ping(0.0, 0.0),
        ClientMsg::Ping { n: u32::MAX, .. }
    ));
    assert!(matches!(c.ping(0.0, 0.0), ClientMsg::Ping { n: 0, .. }));
}

#[test]
fn a_hidden_page_pings_once_a_second() {
    assert_eq!(PING_HIDDEN_MS, 1000.0);
    let mut c = Conn::default();
    let s = c.opened(0.0);
    c.hello(0.0);
    assert_eq!(c.tick(s, 100.0, true), Tick::Ping, "visible: every tick");
    c.ping(100.0, 100.0);
    assert_eq!(c.tick(s, 200.0, true), Tick::Ping);
    c.ping(200.0, 200.0);
    c.set_hidden(true);
    c.heard(300.0);
    for at in [300.0, 700.0, 1_199.0] {
        assert_eq!(c.tick(s, at, true), Tick::Wait, "{at}");
    }
    assert_eq!(
        c.tick(s, 1_200.0, true),
        Tick::Ping,
        "1 s after the last ping"
    );
    c.ping(1_200.0, 1_200.0);
    assert_eq!(c.tick(s, 1_300.0, true), Tick::Wait);
    c.set_hidden(false);
    assert_eq!(
        c.tick(s, 1_400.0, true),
        Tick::Ping,
        "shown again: every tick"
    );
    // Hidden or not, the silence still drops the socket.
    c.set_hidden(true);
    assert_eq!(c.tick(s, 3_300.0, true), Tick::Silent);
}
