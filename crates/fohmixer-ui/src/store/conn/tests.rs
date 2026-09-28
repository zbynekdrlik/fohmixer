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
    assert_eq!(c.tick(s, 14_999.0, true), Tick::Ping);
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
    assert_eq!(c.tick(s, 3_999.0, true), Tick::Wait);
    assert_eq!(c.tick(s, 4_000.0, true), Tick::NoHello);
    assert_eq!(
        c.tick(s, 4_000.0, false),
        Tick::Silent,
        "still connecting after 3 s: a dead network, not a wrong hub"
    );
    c.hello(4_100.0);
    assert_eq!(
        c.tick(s, 7_100.0, true),
        Tick::Silent,
        "after its hello an open socket that falls silent is half-open"
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
    assert!(c.closed(), "an unstopped store reconnects");
    assert_eq!(c.tick(second, 1.0, true), Tick::Done, "closed");
    let third = c.opened(0.0);
    assert_ne!(third, first);
    assert_ne!(third, second);
    assert_eq!(c.tick(third, 1.0, true), Tick::Wait);
    assert!(!c.stopped());
    c.stop();
    assert_eq!(c.tick(third, 1.0, true), Tick::Done, "stopped");
    assert!(c.stopped());
    assert!(!c.closed(), "a stopped store does not reconnect");
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
        "a busy flip changes nothing"
    );
    let back = instance_change(Some(&view(false, "Show")), &view(true, "Show"));
    assert!(back.ranges && !back.pending);
    let other_set = instance_change(Some(&view(true, "Show")), &view(true, "Rehearsal"));
    assert!(other_set.ranges && !other_set.pending);
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
