//! Tests of the recorder's backlog (#43 PR E): a long drag keeps every move
//! (the sizing case of the design note §5.2), what the bound drops first,
//! and the drop markers with the time span they leave.

use super::moves::{Axis, Press, Trail, touch_start};
use super::*;
use crate::behave::Start;

/// Volume keys of the 90th-percentile length at the service of 2026-10-04
/// (69 characters), with invented names.
const KEYS: [&str; 2] = [
    "band|live_set tracks[name=Klavir # Vox 1 L] mixer_device volume|value",
    "band|live_set tracks[name=Klavir # Vox 2 R] mixer_device volume|value",
];
/// A page clock like the Ableton PC's (ms since the epoch).
const T0: f64 = 1_791_125_456_604.6;
/// One frame at 60 Hz (ms).
const FRAME_MS: f64 = 1000.0 / 60.0;
/// The link's tick (ms): a batch may go, then a ping, whose pong comes 2 ms
/// later.
const TICK_MS: f64 = 100.0;
/// 30 s at 60 Hz.
const DRAG_FRAMES: u32 = 1_800;

/// An event of kind `kind` at page time `t` whose JSON is exactly `len`
/// bytes.
fn sized(kind: &str, t: f64, len: usize) -> Value {
    let base = json!({"ev": kind, "t": t, "pad": ""}).to_string().len();
    json!({"ev": kind, "t": t, "pad": "a".repeat(len - base)})
}

/// The events of a batch.
fn events_of(batch: &str) -> Vec<Value> {
    let message: Value = serde_json::from_str(batch).expect("a batch is JSON");
    message["events"].as_array().expect("events").clone()
}

/// The kind and page time of each event the ring holds, oldest first.
fn held(r: &Recorder) -> Vec<(String, f64)> {
    r.events
        .iter()
        .map(|e| {
            let event: Value = serde_json::from_str(&e.text).expect("an event is JSON");
            (e.kind.clone(), event["t"].as_f64().expect("a t"))
        })
        .collect()
}

/// Essential `touch` events at page time `t` filling `bytes` exactly (1 000
/// bytes each, the rest in the last).
fn fill_essential(r: &mut Recorder, t: f64, bytes: usize) {
    let mut left = bytes;
    while left > 0 {
        let len = if left >= 2_000 { 1_000 } else { left };
        r.push(&sized("touch", t, len));
        left -= len;
    }
}

/// The page's link as the recorder sees it: a tick every 100 ms, each with
/// the recorder's batch when one goes, then a ping whose pong comes 2 ms
/// later and proves the batches before it.
struct Link {
    r: Recorder,
    next_tick: f64,
    ping: u32,
    batches: Vec<(f64, String)>,
}

impl Link {
    fn new() -> Self {
        Self {
            r: Recorder::default(),
            next_tick: T0 + 50.0,
            ping: 1,
            batches: Vec::new(),
        }
    }

    /// Runs the ticks due by `now`.
    fn run_to(&mut self, now: f64) {
        while self.next_tick <= now {
            let tick = self.next_tick;
            if let Some(batch) = self.r.upload(tick, true, 0, self.ping) {
                self.batches.push((tick, batch));
            }
            self.r.pong(tick + 2.0, 2.0);
            self.r.proved(self.ping);
            self.ping += 1;
            self.next_tick += TICK_MS;
        }
    }

    /// Every event that went up, in order.
    fn events(&self) -> Vec<Value> {
        self.batches
            .iter()
            .flat_map(|(_, b)| events_of(b))
            .collect()
    }
}

#[test]
fn a_30_s_drag_of_two_faders_at_60_hz_keeps_every_move_at_the_real_cap() {
    // Two fingers drag two faders for 30 s; every frame of each sends a set
    // (so no batch goes while they move: PR D's gate) and records its move.
    let mut link = Link::new();
    let mut trails = [Trail::default(), Trail::default()];
    let pointers = [5, 6];
    let press = Press {
        at: T0 - 8.0,
        c: 600.0,
        travel: 320.0,
    };
    let start = Start {
        shown: 0.3,
        live: 0.3,
        local: false,
        from: 0.3,
    };
    for (f, trail) in trails.iter_mut().enumerate() {
        let keys = [KEYS[f].to_string()];
        link.r
            .push(&touch_start(T0, &keys, pointers[f], press, start));
        trail.start(pointers[f], Axis::Up, press, start.from);
    }
    let mut seq = 0;
    let mut generated = 0;
    for i in 1..=DRAG_FRAMES {
        let t = T0 + FRAME_MS * f64::from(i);
        link.run_to(t);
        for (f, trail) in trails.iter_mut().enumerate() {
            let c = 600.0 - 0.1 * f64::from(i);
            trail.moved(pointers[f], t - 6.3, c);
            seq += 1;
            let sent = trail.raw(c);
            let (record, essential) = trail
                .take(t, KEYS[f], sent, Some(seq))
                .expect("a frame that sent");
            generated += record.to_string().len();
            if essential {
                link.r.push_essential(&record);
            } else {
                link.r.push(&record);
            }
        }
        link.r.set_went();
    }
    // The fingers lift and rest: the backlog drains at the cap.
    let lift = T0 + FRAME_MS * f64::from(DRAG_FRAMES + 1);
    for (f, trail) in trails.iter_mut().enumerate() {
        link.r
            .push(&touch(lift, "up", &[KEYS[f].to_string()], pointers[f]));
        trail.end(pointers[f]);
    }
    let mut now = lift;
    while !link.r.is_empty() && now < lift + 600_000.0 {
        now += TICK_MS;
        link.run_to(now);
    }
    assert!(
        generated > 600_000,
        "the drag's moves are {generated} bytes, far over PR D's 48 KB"
    );
    let events = link.events();
    let dropped: Vec<&Value> = events.iter().filter(|e| e["ev"] == "overflow").collect();
    assert!(dropped.is_empty(), "nothing went: {dropped:?}");
    let moves: Vec<u64> = events
        .iter()
        .filter(|e| e["ev"] == "mv")
        .map(|e| e["q"].as_u64().expect("a q"))
        .collect();
    assert_eq!(
        moves,
        (1..=2 * u64::from(DRAG_FRAMES)).collect::<Vec<u64>>(),
        "every frame's move, in order"
    );
    assert!(link.r.is_empty(), "all of it logged");
    // About 5 records of ~170 bytes fill a 1 000-byte batch, one a tick:
    // 3 600 records drain in about 72 s once the fingers rest.
    let (last, _) = link.batches.last().expect("batches");
    assert!(
        last - lift < 80_000.0,
        "drained {} ms after the lift",
        last - lift
    );
}

#[test]
fn past_the_bound_round_trips_and_long_frames_go_first_then_moves_never_the_essential() {
    let mut r = Recorder::default();
    r.push(&sized("mv", 1.0, 1_000));
    r.push(&sized("rtt", 2.0, 1_000));
    r.push(&sized("frame", 3.0, 1_000));
    r.push(&sized("mv", 4.0, 1_000));
    fill_essential(&mut r, 5.0, BACKLOG_BYTES - 4_000);
    assert_eq!(r.backlog(), BACKLOG_BYTES, "exactly at the bound");
    let kinds = |r: &Recorder| -> Vec<(String, f64)> {
        held(r)
            .into_iter()
            .filter(|(kind, _)| kind != "touch")
            .collect()
    };
    let at = |kind: &str, t: f64| (kind.to_string(), t);
    // Each 1 000 bytes more drop one event: the round trip, then the long
    // frame (rank 0, oldest first), then the moves, oldest first.
    r.push(&sized("touch", 6.0, 1_000));
    assert_eq!(kinds(&r), [at("mv", 1.0), at("frame", 3.0), at("mv", 4.0)]);
    r.push(&sized("touch", 6.0, 1_000));
    assert_eq!(kinds(&r), [at("mv", 1.0), at("mv", 4.0)]);
    r.push(&sized("touch", 6.0, 1_000));
    assert_eq!(kinds(&r), [at("mv", 4.0)]);
    r.push(&sized("touch", 6.0, 1_000));
    assert_eq!(kinds(&r), Vec::<(String, f64)>::new());
    assert_eq!(r.backlog(), BACKLOG_BYTES);
    // Nothing left to drop: the essential events stay over the bound.
    r.push(&sized("touch", 6.0, 1_000));
    assert_eq!(r.backlog(), BACKLOG_BYTES + 1_000);
    // The next batch says, per kind, how many went and from when to when
    // (page ms), before anything else.
    let batch = r.upload(500.0, true, 0, 1).expect("a batch");
    assert_eq!(
        events_of(&batch),
        vec![
            json!({"ev": "overflow", "t": 500.0, "n": 1, "kinds": {"frame": 1}, "from": 3.0, "to": 3.0}),
            json!({"ev": "overflow", "t": 500.0, "n": 2, "kinds": {"mv": 2}, "from": 1.0, "to": 4.0}),
            json!({"ev": "overflow", "t": 500.0, "n": 1, "kinds": {"rtt": 1}, "from": 2.0, "to": 2.0}),
        ],
        "the markers fill the batch"
    );
}

#[test]
fn a_moves_span_is_its_oldest_and_newest_dropped_move_and_a_touchs_first_moves_stay() {
    let mut r = Recorder::default();
    r.push_essential(&sized("mv", 10.0, 1_000));
    r.push(&sized("mv", 20.0, 1_000));
    r.push(&sized("mv", 30.0, 1_000));
    r.push(&sized("mv", 40.0, 1_000));
    fill_essential(&mut r, 50.0, BACKLOG_BYTES - 4_000);
    r.push(&sized("touch", 60.0, 2_000));
    let left: Vec<(String, f64)> = held(&r)
        .into_iter()
        .filter(|(kind, _)| kind == "mv")
        .collect();
    assert_eq!(
        left,
        [("mv".to_string(), 10.0), ("mv".to_string(), 40.0)],
        "the first move (essential) stays, the two oldest others went"
    );
    let batch = r.upload(900.0, true, 0, 1).expect("a batch");
    assert_eq!(
        events_of(&batch)[0],
        json!({"ev": "overflow", "t": 900.0, "n": 2, "kinds": {"mv": 2}, "from": 20.0, "to": 30.0})
    );
    // The next drops start a new marker.
    r.push(&sized("touch", 70.0, 1_000));
    let batch = r.upload(2_000.0, true, 0, 2).expect("a batch");
    assert_eq!(
        events_of(&batch)[0],
        json!({"ev": "overflow", "t": 2_000.0, "n": 1, "kinds": {"mv": 1}, "from": 40.0, "to": 40.0})
    );
}

#[test]
fn moves_go_last_round_trips_long_frames_and_other_kinds_first() {
    for ev in [
        "touch",
        "dropout",
        "reset",
        "sock",
        "visibility",
        "overflow",
        "intent",
    ] {
        assert_eq!(drop_rank(ev), None, "{ev}");
    }
    assert_eq!(drop_rank("mv"), Some(1));
    for ev in ["rtt", "frame", "x", ""] {
        assert_eq!(drop_rank(ev), Some(0), "{ev}");
    }
}

#[test]
fn a_span_covers_a_requeued_move_older_than_the_one_that_went_before_it() {
    // A move went up; a newer one went for the bound; then a lost socket
    // put the older one back and it went too: the span runs from the older
    // to the newer, whatever order they went in.
    let mut r = Recorder::default();
    r.push(&sized("mv", 10.0, 1_000));
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    r.push(&sized("mv", 20.0, 1_000));
    fill_essential(&mut r, 30.0, BACKLOG_BYTES - 1_000);
    r.push(&sized("touch", 40.0, 1_000));
    r.requeue();
    let left: Vec<(String, f64)> = held(&r)
        .into_iter()
        .filter(|(kind, _)| kind == "mv")
        .collect();
    assert_eq!(left, Vec::<(String, f64)>::new(), "both went");
    let batch = r.upload(500.0, true, 0, 2).expect("a batch");
    assert_eq!(
        events_of(&batch)[0],
        json!({"ev": "overflow", "t": 500.0, "n": 2, "kinds": {"mv": 2}, "from": 10.0, "to": 20.0})
    );
}
