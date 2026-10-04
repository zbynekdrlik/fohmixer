use super::*;

/// A watch whose socket said hello at 0, with `rtts` round trips seen.
fn up(rtts: &[f64]) -> DropoutWatch {
    let mut w = DropoutWatch::default();
    w.hello(0.0);
    for (n, rtt) in rtts.iter().enumerate() {
        w.pong(n as u32, *rtt);
    }
    w
}

/// The watch's page tick every 100 ms from `from` to `to`.
fn ticks(w: &mut DropoutWatch, from: f64, to: f64) {
    let steps = ((to - from) / 100.0).round() as u32;
    for k in 0..=steps {
        w.tick(from + 100.0 * f64::from(k));
    }
}

#[test]
fn the_threshold_is_300_ms() {
    assert_eq!(DROPOUT_MS, 300.0);
    assert!(!is_dropout(299.0));
    assert!(!is_dropout(300.0_f64.next_down()));
    assert!(is_dropout(300.0));
    assert!(is_dropout(5_000.0));
    assert_eq!((RTTS_KEPT, PINGS_KEPT, REPORTS_KEPT), (5, 64, 64));
}

#[test]
fn a_pong_after_299_ms_is_no_dropout_after_300_it_is_one() {
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(0, 1_000.0);
    ticks(&mut w, 1_100.0, 1_200.0);
    w.heard(1_299.0);
    w.pong(0, 299.0);
    assert_eq!(w.count(), 0);
    assert!(w.take_reports().is_empty());
    // The next ping waits exactly 300 ms: one dropout, seen only at its end
    // (no tick saw it).
    ticks(&mut w, 1_300.0, 2_000.0);
    w.ping(1, 2_000.0);
    ticks(&mut w, 2_100.0, 2_200.0);
    w.heard(2_300.0);
    w.pong(1, 300.0);
    assert_eq!(w.count(), 1);
    assert!(!w.active());
    assert_eq!(
        w.take_reports(),
        vec![Dropout {
            t: 2_000.0,
            ms: 300.0,
            socket_lost: false,
            rtts: vec![299.0],
        }]
    );
}

#[test]
fn a_long_silence_counts_once_and_is_red_while_it_lasts() {
    let mut w = up(&[12.0, 14.0]);
    ticks(&mut w, 100.0, 900.0);
    w.heard(900.0);
    // Pings every 100 ms, none answered.
    for i in 0..10_u32 {
        let at = 1_000.0 + f64::from(i) * 100.0;
        w.ping(10 + i, at);
        w.tick(at + 50.0);
    }
    // The silence started at the first unanswered ping (1000, after the
    // last message at 900), so the tick at 1350 opened it.
    assert!(w.active());
    assert_eq!(w.count(), 1, "one interval, one count");
    w.tick(2_050.0);
    assert_eq!(w.count(), 1);
    // The burst of pongs ends it.
    w.heard(2_100.0);
    for n in 10..20 {
        w.pong(n, 1_100.0);
    }
    assert!(!w.active());
    let reports = w.take_reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].t, 1_000.0);
    assert_eq!(reports[0].ms, 1_100.0);
    assert_eq!(
        reports[0].rtts,
        vec![12.0, 14.0],
        "the round trips before it"
    );
    // Afterwards the link is quiet again.
    ticks(&mut w, 2_150.0, 2_750.0);
    w.ping(20, 2_700.0);
    w.heard(2_720.0);
    w.pong(20, 20.0);
    assert_eq!(w.count(), 1);
}

#[test]
fn a_message_heard_restarts_the_silence() {
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(0, 1_000.0);
    w.tick(1_100.0);
    // A value push at 1200 shows the link alive: the silence counts from it.
    w.heard(1_200.0);
    w.tick(1_200.0);
    w.tick(1_300.0);
    w.tick(1_400.0);
    w.tick(1_450.0);
    assert!(!w.active(), "250 ms since the last message");
    w.tick(1_500.0);
    assert!(w.active(), "300 ms since the last message");
    w.heard(1_510.0);
    assert_eq!(w.take_reports()[0].t, 1_200.0);
    // Without a ping due, a quiet hub is no dropout.
    let mut idle = up(&[]);
    ticks(&mut idle, 100.0, 10_000.0);
    idle.heard(10_000.0);
    assert_eq!(idle.count(), 0);
    // A ping sent long after the last message: the silence starts at it.
    ticks(&mut idle, 10_100.0, 20_000.0);
    idle.ping(0, 20_000.0);
    ticks(&mut idle, 20_100.0, 20_200.0);
    idle.tick(20_250.0);
    idle.heard(20_260.0);
    idle.pong(0, 260.0);
    assert_eq!(idle.count(), 0);
}

#[test]
fn a_frozen_or_hidden_page_does_not_count_its_own_silence() {
    // The page's main thread stalls 520 ms with a ping due: the pong waits in
    // the page's queue. Whether the message or the late tick runs first,
    // nothing counts.
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(0, 1_000.0);
    w.heard(1_520.0);
    w.pong(0, 520.0);
    assert_eq!(w.count(), 0, "the message ran first: no tick for 520 ms");
    w.tick(1_525.0);
    w.ping(1, 1_525.0);
    w.tick(1_625.0);
    w.tick(1_725.0);
    w.tick(2_300.0);
    assert!(!w.active(), "the late tick starts the silence over");
    w.heard(2_310.0);
    assert_eq!(w.count(), 0);
    // A hidden page pings once a second and its timers come late: nothing
    // counts either.
    w.ping(2, 3_300.0);
    w.tick(3_300.0);
    w.tick(4_300.0);
    w.heard(4_310.0);
    w.pong(2, 1_010.0);
    assert_eq!(w.count(), 0);
    assert!(w.take_reports().is_empty());
    // Ticking again, the watch counts again.
    ticks(&mut w, 4_400.0, 4_800.0);
    w.ping(3, 4_800.0);
    ticks(&mut w, 4_900.0, 5_000.0);
    assert!(!w.active());
    w.tick(5_100.0);
    assert!(w.active());
    assert_eq!(w.count(), 1);
}

#[test]
fn a_lost_socket_is_one_dropout_until_the_next_hello() {
    let mut w = up(&[20.0]);
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(1, 1_000.0);
    ticks(&mut w, 1_100.0, 1_400.0);
    assert!(w.active());
    // The silence went on and the watchdog dropped the socket.
    ticks(&mut w, 1_500.0, 4_000.0);
    w.lost(4_000.0);
    assert!(w.active());
    ticks(&mut w, 4_100.0, 4_500.0);
    w.heard(4_600.0);
    assert!(w.active(), "a socket's first message before its hello");
    w.hello(5_000.0);
    assert!(!w.active());
    assert_eq!(w.count(), 1);
    assert_eq!(
        w.take_reports(),
        vec![Dropout {
            t: 1_000.0,
            ms: 4_000.0,
            socket_lost: true,
            rtts: vec![20.0],
        }]
    );
    // A socket that closes with nothing due: the dropout starts at the
    // close, however short.
    ticks(&mut w, 5_100.0, 6_000.0);
    w.heard(6_000.0);
    w.tick(6_100.0);
    w.lost(6_100.0);
    assert_eq!(w.count(), 2);
    ticks(&mut w, 6_200.0, 6_400.0);
    w.hello(6_400.0);
    let report = &w.take_reports()[0];
    assert_eq!(
        (report.t, report.ms, report.socket_lost),
        (6_100.0, 300.0, true)
    );
    // A close with a ping due, not yet 300 ms: from that ping.
    ticks(&mut w, 6_500.0, 7_000.0);
    w.ping(5, 7_000.0);
    w.tick(7_100.0);
    w.lost(7_100.0);
    w.hello(7_600.0);
    assert_eq!(w.take_reports()[0].t, 7_000.0);
    assert_eq!(w.count(), 3);
}

#[test]
fn a_socket_lost_while_the_page_is_away_counts_from_its_next_tick() {
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    // Hidden: the timers stop, the socket closes meanwhile.
    w.lost(9_000.0);
    assert!(!w.active());
    assert_eq!(w.count(), 0);
    // Shown again: the first tick is late, the next one sees the socket
    // down.
    w.tick(20_000.0);
    assert!(!w.active());
    w.tick(20_100.0);
    assert!(w.active());
    assert_eq!(w.count(), 1);
    w.tick(20_200.0);
    assert_eq!(w.count(), 1);
    w.hello(20_600.0);
    let report = &w.take_reports()[0];
    assert_eq!(
        (report.t, report.ms, report.socket_lost),
        (20_100.0, 500.0, true)
    );
    // A tick while connected never counts a lost socket.
    ticks(&mut w, 20_700.0, 21_000.0);
    assert_eq!(w.count(), 1);
}

#[test]
fn a_socket_lost_during_a_page_stall_counts_from_the_next_tick() {
    // The watch's rule for a stall (the glue's part, that the tick goes on
    // between sockets, is `link.spec.ts`'s stall test): the page's main
    // thread stalls and the socket closes meanwhile (its close runs before
    // the late tick). The ticks go on without a socket, so the dropout
    // starts at the first on-time tick after the stall and lasts until the
    // next socket's hello.
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    w.lost(1_450.0);
    assert!(!w.active(), "no tick lived through the close");
    w.tick(1_455.0);
    assert!(!w.active(), "the late tick starts over");
    ticks(&mut w, 1_555.0, 1_955.0);
    assert!(w.active());
    w.hello(2_000.0);
    assert!(!w.active());
    assert_eq!(w.count(), 1);
    assert_eq!(
        w.take_reports(),
        vec![Dropout {
            t: 1_555.0,
            ms: 445.0,
            socket_lost: true,
            rtts: vec![],
        }]
    );
}

#[test]
fn nothing_counts_before_the_first_hello() {
    let mut w = DropoutWatch::default();
    ticks(&mut w, 0.0, 1_000.0);
    w.ping(0, 1_000.0);
    ticks(&mut w, 1_100.0, 5_000.0);
    w.heard(5_000.0);
    ticks(&mut w, 5_100.0, 6_000.0);
    w.lost(6_000.0);
    ticks(&mut w, 6_100.0, 6_900.0);
    assert_eq!(w.count(), 0);
    assert!(!w.active());
    w.hello(7_000.0);
    assert!(w.take_reports().is_empty(), "the page's first connection");
}

#[test]
fn a_pong_answers_every_ping_up_to_it() {
    let mut w = up(&[]);
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(1, 1_000.0);
    w.tick(1_100.0);
    w.ping(2, 1_100.0);
    w.tick(1_200.0);
    w.ping(3, 1_200.0);
    w.heard(1_250.0);
    w.pong(2, 250.0);
    assert_eq!(w.pings.len(), 1, "ping 3 is still due");
    assert_eq!(w.pings.front(), Some(&(3, 1_200.0)));
    // Ping 3 is due since 1200, but the last message (1250) is later.
    w.tick(1_300.0);
    w.tick(1_400.0);
    w.tick(1_549.0);
    assert!(!w.active());
    w.tick(1_550.0);
    assert!(w.active());
    // At most 64 unanswered pings are kept: the oldest stays.
    let mut flood = up(&[]);
    for n in 0..=64_u32 {
        flood.ping(n, f64::from(n));
    }
    assert_eq!(flood.pings.len(), 64);
    assert_eq!(flood.pings.front(), Some(&(0, 0.0)));
    assert_eq!(flood.pings.back(), Some(&(63, 63.0)));
}

#[test]
fn reports_keep_the_last_five_round_trips_and_at_most_64_wait() {
    let w = up(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    assert_eq!(w.rtts, [2.0, 3.0, 4.0, 5.0, 6.0]);
    let mut w = up(&[]);
    for i in 0..65_u32 {
        let at = 10_000.0 * f64::from(i + 1);
        ticks(&mut w, at - 1_000.0, at);
        w.ping(i, at);
        ticks(&mut w, at + 100.0, at + 300.0);
        w.heard(at + 400.0);
        w.pong(i, 400.0);
    }
    assert_eq!(w.count(), 65);
    let reports = w.take_reports();
    assert_eq!(reports.len(), 64, "the oldest went");
    assert_eq!(reports[0].t, 20_000.0);
    assert!(w.take_reports().is_empty(), "taken");
}

#[test]
fn a_report_is_a_dropout_event_of_a_trace() {
    let report = Dropout {
        t: 5_000.5,
        ms: 420.0,
        socket_lost: true,
        rtts: vec![12.0, 14.5],
    };
    assert_eq!(
        report.event(),
        json!({"ev": "dropout", "t": 5_000.5, "ms": 420.0, "socket_lost": true,
               "rtts": [12.0, 14.5]})
    );
}

/// One whole dropout from `at`: a ping unanswered for 400 ms, the page
/// ticking, then the pong.
fn dropout(w: &mut DropoutWatch, at: f64, n: u32) {
    ticks(w, at - 500.0, at);
    w.ping(n, at);
    ticks(w, at + 100.0, at + 300.0);
    w.heard(at + 400.0);
    w.pong(n, 400.0);
}

#[test]
fn the_counter_counts_each_dropout_once_and_is_red_only_while_it_lasts() {
    let mut w = up(&[]);
    assert_eq!(w.counter(), Counter::default(), "0, neutral");
    // A long silence: red while it lasts, counted once.
    ticks(&mut w, 100.0, 1_000.0);
    w.ping(0, 1_000.0);
    ticks(&mut w, 1_100.0, 1_200.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 0,
            active: false
        }
    );
    w.tick(1_300.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 1,
            active: true
        }
    );
    ticks(&mut w, 1_400.0, 2_500.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 1,
            active: true
        }
    );
    w.heard(2_550.0);
    w.pong(0, 1_550.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 1,
            active: false
        }
    );
    // A lost socket: one more, red until the next hello.
    ticks(&mut w, 2_600.0, 3_000.0);
    w.lost(3_000.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 2,
            active: true
        }
    );
    ticks(&mut w, 3_100.0, 3_600.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 2,
            active: true
        }
    );
    w.hello(3_650.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 2,
            active: false
        }
    );
    // A silence of exactly 300 ms that no tick saw: counted when it ends,
    // never red.
    ticks(&mut w, 3_700.0, 4_000.0);
    w.ping(1, 4_000.0);
    ticks(&mut w, 4_100.0, 4_200.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 2,
            active: false
        }
    );
    w.heard(4_300.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 3,
            active: false
        }
    );
}

#[test]
fn a_tap_resets_the_counter_to_0_and_its_event_says_what_it_showed() {
    let mut w = up(&[]);
    for (i, at) in [10_000.0, 20_000.0, 30_000.0, 40_000.0, 50_000.0]
        .into_iter()
        .enumerate()
    {
        dropout(&mut w, at, i as u32);
    }
    assert_eq!(
        w.counter(),
        Counter {
            count: 5,
            active: false
        }
    );
    assert_eq!(
        w.reset(51_000.5),
        json!({"ev": "reset", "t": 51_000.5, "count": 5, "active": false})
    );
    assert_eq!(
        w.counter(),
        Counter {
            count: 0,
            active: false
        }
    );
    for (i, at) in [60_000.0, 70_000.0, 80_000.0].into_iter().enumerate() {
        dropout(&mut w, at, 10 + i as u32);
    }
    assert_eq!(
        w.counter(),
        Counter {
            count: 3,
            active: false
        },
        "since the tap"
    );
    assert_eq!(w.count(), 8, "since the page loaded");
    // A tap while a dropout lasts: 0, still red until it ends.
    ticks(&mut w, 80_500.0, 90_000.0);
    w.ping(20, 90_000.0);
    ticks(&mut w, 90_100.0, 90_400.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 4,
            active: true
        }
    );
    assert_eq!(
        w.reset(90_450.0),
        json!({"ev": "reset", "t": 90_450.0, "count": 4, "active": true})
    );
    assert_eq!(
        w.counter(),
        Counter {
            count: 0,
            active: true
        }
    );
    w.heard(90_600.0);
    assert_eq!(
        w.counter(),
        Counter {
            count: 0,
            active: false
        }
    );
    assert_eq!(w.count(), 9);
    // A second tap with nothing new: 0 again.
    assert_eq!(
        w.reset(91_000.0),
        json!({"ev": "reset", "t": 91_000.0, "count": 0, "active": false})
    );
}
