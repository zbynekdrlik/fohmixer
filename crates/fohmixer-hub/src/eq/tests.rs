use super::*;
use serde_json::json;

const WALL: f64 = 1_790_000_000_000.0;

fn key(n: u32) -> EditorKey {
    EditorKey::new("band", &format!("live_set tracks {n} devices 0"))
}

/// The device name the lists found.
const NAME: &str = "Vox EQ";

/// `key` as a list found it, with its device's name.
fn found(key: EditorKey) -> (EditorKey, String) {
    (key, NAME.to_string())
}

/// The LOM target of the track the tests list.
const TRACK: &str = "live_set tracks[name=Vox #]";

/// A state where editors 1..=4 were listed.
fn listed() -> Eqs {
    let mut eqs = Eqs::default();
    eqs.listed("band", TRACK, (1..=4).map(key).map(found));
    eqs
}

fn tell(client: ClientId, msg: ServerMsg) -> Act {
    Act::Tell { client, msg }
}

fn rec(
    what: &str,
    key: &EditorKey,
    client: Option<ClientId>,
    session: Option<u32>,
    why: Option<&str>,
) -> Act {
    Act::Record(record(what, key, client, session, why))
}

/// Client 1 opens editor 1 (its picture 1349 × 809); its session.
fn open_one(eqs: &mut Eqs) -> u32 {
    let acts = eqs.open(1, &key(1), WALL);
    let session = acts
        .iter()
        .find_map(|a| match a {
            Act::Open { session, .. } => Some(*session),
            _ => None,
        })
        .expect("an open");
    eqs.opened(&key(1), session, Ok((1349, 809)));
    session
}

#[test]
fn a_page_is_silent_after_2_s() {
    assert!(!silent(2000.0_f64.next_down()));
    assert!(silent(2000.0));
    assert_eq!(GUARD_WAIT, Duration::from_millis(300));
}

#[test]
fn a_point_is_rounded_into_the_picture() {
    assert_eq!(clamp_point(10.4, 20.6, 100, 50), (10, 21));
    assert_eq!(clamp_point(-3.0, -0.4, 100, 50), (0, 0));
    assert_eq!(clamp_point(99.4, 49.4, 100, 50), (99, 49));
    assert_eq!(clamp_point(99.5, 49.5, 100, 50), (99, 49));
    assert_eq!(clamp_point(1e9, 1e9, 100, 50), (99, 49));
    assert_eq!(clamp_point(5.0, 5.0, 0, 0), (0, 0));
    assert_eq!(clamp_point(f64::NAN, 7.0, 100, 50), (0, 7));
}

#[test]
fn a_touch_makes_its_contact_phases() {
    let (a, b) = ((10, 20), (30, 40));
    assert_eq!(phases(Touch::Down, None, a), vec![(Phase::Down, a)]);
    assert_eq!(
        phases(Touch::Down, Some(b), a),
        vec![(Phase::Up, b), (Phase::Down, a)]
    );
    assert_eq!(phases(Touch::Move, Some(b), a), vec![(Phase::Update, a)]);
    assert_eq!(phases(Touch::Move, None, a), Vec::new());
    assert_eq!(phases(Touch::Up, Some(a), a), vec![(Phase::Up, a)]);
    assert_eq!(
        phases(Touch::Up, Some(b), a),
        vec![(Phase::Update, a), (Phase::Up, a)]
    );
    assert_eq!(phases(Touch::Up, None, a), Vec::new());
    assert_eq!(phases(Touch::Cancel, Some(b), a), vec![(Phase::Cancel, b)]);
    assert_eq!(phases(Touch::Cancel, None, a), Vec::new());
}

#[test]
fn the_state_messages_and_records_have_their_shapes() {
    let k = key(1);
    assert_eq!(
        serde_json::to_value(opening_msg(&k)).unwrap(),
        json!({"type": "eq", "instance": "band", "path": "live_set tracks 1 devices 0",
               "state": "opening"})
    );
    assert_eq!(
        serde_json::to_value(open_msg(&k, 3, 1349, 809)).unwrap(),
        json!({"type": "eq", "instance": "band", "path": "live_set tracks 1 devices 0",
               "state": "open", "session": 3, "width": 1349, "height": 809})
    );
    assert_eq!(
        serde_json::to_value(closed_msg(&k, "locked", Some(WALL))).unwrap(),
        json!({"type": "eq", "instance": "band", "path": "live_set tracks 1 devices 0",
               "state": "closed", "reason": "locked", "since": WALL})
    );
    assert_eq!(
        record("open", &k, Some(4), Some(2), None),
        json!({"what": "open", "instance": "band", "path": "live_set tracks 1 devices 0",
               "client": 4, "session": 2, "why": null})
    );
}

#[test]
fn only_a_listed_editor_opens() {
    let mut eqs = Eqs::default();
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![
            tell(1, closed_msg(&key(1), reason::UNKNOWN, None)),
            rec("refused", &key(1), Some(1), None, Some(reason::UNKNOWN)),
        ]
    );
    eqs.listed(
        "band",
        TRACK,
        [found(key(1)), (key(2), "Kick EQ".to_string())],
    );
    assert_eq!(eqs.name_of(&key(1)), Some(NAME));
    assert_eq!(eqs.name_of(&key(2)), Some("Kick EQ"));
    assert_eq!(eqs.name_of(&key(3)), None);
    assert_eq!(eqs.open(1, &key(1), WALL).len(), 5);
    // A new connection of the instance: listed again before an open.
    eqs.forget("master");
    assert_eq!(eqs.open(2, &key(1), WALL).len(), 2, "still known: locked");
    eqs.forget("band");
    assert_eq!(
        eqs.open(3, &key(1), WALL),
        vec![
            tell(3, closed_msg(&key(1), reason::UNKNOWN, None)),
            rec("refused", &key(1), Some(3), None, Some(reason::UNKNOWN)),
        ]
    );
}

#[test]
fn a_tracks_new_list_replaces_its_last_one() {
    let mut eqs = Eqs::default();
    let other = "live_set tracks 7";
    assert_eq!(
        eqs.listed("band", TRACK, [found(key(1)), found(key(2))]),
        Vec::new()
    );
    // The same track by another binding names editor 2 too.
    assert_eq!(eqs.listed("band", other, [found(key(2))]), Vec::new());
    // Editor 1 went, editor 3 came: only 1 is no list's now.
    assert_eq!(eqs.listed("band", TRACK, [found(key(3))]), vec![key(1)]);
    assert_eq!(eqs.name_of(&key(1)), None);
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![
            tell(1, closed_msg(&key(1), reason::UNKNOWN, None)),
            rec("refused", &key(1), Some(1), None, Some(reason::UNKNOWN)),
        ]
    );
    assert_eq!(eqs.name_of(&key(2)), Some(NAME));
    assert_eq!(eqs.name_of(&key(3)), Some(NAME));
    // The same answer again drops nothing.
    assert_eq!(eqs.listed("band", TRACK, [found(key(3))]), Vec::new());
    // Another instance's list of the same target is its own; a connect
    // forgets only its own instance's lists.
    let drums = EditorKey::new("drums", "live_set tracks 0 devices 0");
    eqs.listed("drums", TRACK, [(drums.clone(), "Snare EQ".to_string())]);
    assert_eq!(eqs.name_of(&key(3)), Some(NAME));
    eqs.forget("band");
    assert_eq!((eqs.name_of(&key(2)), eqs.name_of(&key(3))), (None, None));
    assert_eq!(eqs.name_of(&drums), Some("Snare EQ"));
}

#[test]
fn an_open_takes_the_lock_and_another_client_is_refused() {
    let mut eqs = listed();
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![
            tell(1, opening_msg(&key(1))),
            Act::Locks,
            rec("take", &key(1), Some(1), None, None),
            rec("open", &key(1), Some(1), Some(1), None),
            Act::Open {
                key: key(1),
                session: 1
            },
        ]
    );
    assert_eq!(
        eqs.locks_for(1),
        vec![EqLock {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            mine: true,
            since: WALL,
        }]
    );
    assert!(!eqs.locks_for(2)[0].mine);
    assert_eq!(eqs.held_by(1), Some((key(1), 1, "opening")));
    assert_eq!(eqs.held_by(2), None);
    // Another client: locked, since when.
    assert_eq!(
        eqs.open(2, &key(1), WALL + 5.0),
        vec![
            tell(2, closed_msg(&key(1), reason::LOCKED, Some(WALL))),
            rec("refused", &key(1), Some(2), None, Some(reason::LOCKED)),
        ]
    );
    // Its own holder hears where it is.
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![tell(1, opening_msg(&key(1)))]
    );
    assert_eq!(
        eqs.opened(&key(1), 1, Ok((1349, 809))),
        vec![
            rec("opened", &key(1), Some(1), Some(1), None),
            tell(1, open_msg(&key(1), 1, 1349, 809)),
            Act::Capture {
                key: key(1),
                session: 1,
                holder: 1
            },
        ]
    );
    assert_eq!(eqs.held_by(1), Some((key(1), 1, "open")));
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![tell(1, open_msg(&key(1), 1, 1349, 809))]
    );
    assert_eq!(eqs.key_of(1), Some(key(1)));
    assert_eq!(eqs.key_of(2), None);
    assert_eq!(eqs.taken(), vec![(key(1), 1)]);
}

#[test]
fn opens_run_one_at_a_time() {
    let mut eqs = listed();
    eqs.open(1, &key(1), WALL);
    assert_eq!(
        eqs.open(2, &key(2), WALL),
        vec![
            tell(2, opening_msg(&key(2))),
            Act::Locks,
            rec("take", &key(2), Some(2), None, None),
        ],
        "queued behind the first"
    );
    assert_eq!(eqs.held_by(2), Some((key(2), 0, "queued")));
    assert_eq!(eqs.key_of(0), None, "a queued editor has no session");
    assert_eq!(eqs.locks_for(3).len(), 2);
    let acts = eqs.opened(&key(1), 1, Ok((100, 50)));
    assert_eq!(
        acts[3..],
        [
            rec("open", &key(2), Some(2), Some(2), None),
            Act::Open {
                key: key(2),
                session: 2
            },
        ]
    );
    // An answer of another session changes nothing but lets the next start.
    eqs.open(3, &key(3), WALL);
    assert_eq!(
        eqs.opened(&key(2), 9, Ok((1, 1))),
        vec![
            rec("open", &key(3), Some(3), Some(3), None),
            Act::Open {
                key: key(3),
                session: 3
            },
        ]
    );
    assert_eq!(eqs.held_by(2), Some((key(2), 2, "opening")));
    // Nothing queued: no open starts.
    assert_eq!(eqs.opened(&key(3), 3, Ok((1, 1))).len(), 3);
}

#[test]
fn a_failed_open_frees_the_editor() {
    let mut eqs = listed();
    eqs.open(1, &key(1), WALL);
    assert_eq!(
        eqs.opened(&key(1), 1, Err("no window".into())),
        vec![
            rec("failed", &key(1), Some(1), Some(1), Some("no window")),
            tell(1, closed_msg(&key(1), "no window", None)),
            Act::Locks,
        ]
    );
    assert_eq!(eqs.held_by(1), None);
    assert!(eqs.locks_for(1).is_empty());
    // It may be opened again at once.
    assert_eq!(eqs.open(2, &key(1), WALL).len(), 5);
}

#[test]
fn an_open_editor_closes_through_the_close_sequence() {
    let mut eqs = listed();
    open_one(&mut eqs);
    assert_eq!(
        eqs.close(1, reason::EXIT, true),
        vec![
            rec("close", &key(1), Some(1), Some(1), Some(reason::EXIT)),
            Act::Close {
                key: key(1),
                session: 1,
                why: reason::EXIT
            },
        ]
    );
    assert_eq!(eqs.held_by(1), Some((key(1), 1, "closing")));
    assert_eq!(eqs.taken(), vec![(key(1), 1)], "its window still taken");
    assert_eq!(
        eqs.close(1, reason::EXIT, true),
        Vec::new(),
        "closing already"
    );
    assert_eq!(
        eqs.open(1, &key(1), WALL),
        vec![tell(1, closed_msg(&key(1), reason::CLOSING, None))]
    );
    assert_eq!(eqs.closed(&key(1), 7), Vec::new(), "another session");
    assert_eq!(
        eqs.closed(&key(1), 1),
        vec![
            rec("closed", &key(1), Some(1), Some(1), Some(reason::EXIT)),
            tell(1, closed_msg(&key(1), reason::EXIT, None)),
            Act::Locks,
        ]
    );
    assert_eq!(eqs.held_by(1), None);
    assert_eq!(eqs.closed(&key(1), 1), Vec::new(), "closed once");
    assert!(eqs.taken().is_empty());
    assert_eq!(eqs.close(1, reason::EXIT, true), Vec::new(), "nothing held");
}

#[test]
fn an_editor_left_while_it_opens_closes_once_open() {
    let mut eqs = listed();
    eqs.open(1, &key(1), WALL);
    assert_eq!(eqs.close(1, reason::EXIT, true), Vec::new());
    assert_eq!(
        eqs.opened(&key(1), 1, Ok((1349, 809))),
        vec![
            rec("opened", &key(1), Some(1), Some(1), None),
            rec("close", &key(1), Some(1), Some(1), Some(reason::EXIT)),
            Act::Close {
                key: key(1),
                session: 1,
                why: reason::EXIT
            },
        ]
    );
    // A queued one is dropped at once, and never opens.
    eqs.open(2, &key(2), WALL);
    eqs.open(3, &key(3), WALL);
    assert_eq!(
        eqs.close(3, reason::EXIT, true),
        vec![tell(3, closed_msg(&key(3), reason::EXIT, None)), Act::Locks]
    );
    let acts = eqs.opened(&key(2), 2, Ok((1, 1)));
    assert!(
        !acts.iter().any(|a| matches!(a, Act::Open { .. })),
        "{acts:?}"
    );
}

#[test]
fn opening_another_editor_closes_the_held_one_first() {
    let mut eqs = listed();
    open_one(&mut eqs);
    assert_eq!(
        eqs.open(1, &key(2), WALL + 1.0),
        vec![
            tell(1, opening_msg(&key(2))),
            Act::Locks,
            rec("take", &key(2), Some(1), None, None),
            rec("close", &key(1), Some(1), Some(1), Some(reason::SWITCH)),
            Act::Close {
                key: key(1),
                session: 1,
                why: reason::SWITCH
            },
        ]
    );
    // Both are locked meanwhile: the closing one and the waiting one.
    assert_eq!(eqs.locks_for(2).len(), 2);
    assert_eq!(
        eqs.open(2, &key(2), WALL),
        vec![
            tell(2, closed_msg(&key(2), reason::LOCKED, Some(WALL + 1.0))),
            rec("refused", &key(2), Some(2), None, Some(reason::LOCKED)),
        ]
    );
    assert_eq!(
        eqs.closed(&key(1), 1),
        vec![
            rec("closed", &key(1), Some(1), Some(1), Some(reason::SWITCH)),
            tell(1, closed_msg(&key(1), reason::SWITCH, None)),
            Act::Locks,
            rec("open", &key(2), Some(1), Some(2), None),
            Act::Open {
                key: key(2),
                session: 2
            },
        ]
    );
    assert_eq!(eqs.held_by(1), Some((key(2), 2, "opening")));
}

#[test]
fn a_newer_open_replaces_the_one_waiting_and_leaving_drops_it() {
    let mut eqs = listed();
    open_one(&mut eqs);
    eqs.open(1, &key(2), WALL);
    assert_eq!(
        eqs.open(1, &key(3), WALL),
        vec![
            Act::Locks,
            tell(1, opening_msg(&key(3))),
            Act::Locks,
            rec("take", &key(3), Some(1), None, None),
        ],
        "key 2 dropped; key 1 is closing already"
    );
    assert_eq!(eqs.locks_for(1).len(), 2, "keys 1 and 3");
    // The page leaves the screen: the waiting one goes too.
    assert_eq!(eqs.close(1, reason::EXIT, true), vec![Act::Locks]);
    assert_eq!(eqs.locks_for(1).len(), 1, "only the closing one");
    // A switch's own close keeps the waiting one.
    let mut eqs = listed();
    open_one(&mut eqs);
    eqs.open(1, &key(2), WALL);
    eqs.closed(&key(1), 1);
    assert_eq!(eqs.held_by(1), Some((key(2), 2, "opening")));
}

#[test]
fn a_closed_socket_closes_its_editor_and_drops_the_waiting_one() {
    let mut eqs = listed();
    open_one(&mut eqs);
    eqs.open(1, &key(2), WALL);
    eqs.heard(1, 5.0);
    assert_eq!(eqs.detach(1), vec![Act::Locks]);
    let mut eqs = listed();
    open_one(&mut eqs);
    assert_eq!(
        eqs.detach(1),
        vec![
            rec("close", &key(1), Some(1), Some(1), Some(reason::DETACH)),
            Act::Close {
                key: key(1),
                session: 1,
                why: reason::DETACH
            },
        ]
    );
}

#[test]
fn a_window_that_went_away_frees_its_editor() {
    let mut eqs = listed();
    open_one(&mut eqs);
    eqs.input(1, Touch::Down, 1.0, 1.0, 10.0);
    assert_eq!(eqs.lost(&key(1), 5), Vec::new(), "another session");
    assert_eq!(
        eqs.lost(&key(1), 1),
        vec![
            rec("lost", &key(1), Some(1), Some(1), Some(reason::GONE)),
            tell(1, closed_msg(&key(1), reason::GONE, None)),
            Act::Locks,
        ]
    );
    assert_eq!(eqs.held_by(1), None);
    assert_eq!(eqs.tick(20.0), Vec::new(), "its contact is gone too");
    // A closing editor finishes its close.
    assert_eq!(open_one(&mut eqs), 2);
    eqs.close(1, reason::EXIT, true);
    assert_eq!(eqs.lost(&key(1), 2), Vec::new());
    assert_eq!(eqs.held_by(1), Some((key(1), 2, "closing")));
}

#[test]
fn a_finger_makes_one_contact_on_the_open_editor() {
    let mut eqs = listed();
    assert_eq!(
        eqs.input(1, Touch::Down, 1.0, 1.0, 0.0),
        Vec::new(),
        "none held"
    );
    eqs.open(1, &key(1), WALL);
    assert_eq!(
        eqs.input(1, Touch::Down, 1.0, 1.0, 0.0),
        Vec::new(),
        "not open yet"
    );
    eqs.opened(&key(1), 1, Ok((100, 50)));
    // Each down starts a contact of the next number.
    let touch = |contact: u32, phase: Phase, at: (i32, i32)| Act::Touch {
        session: 1,
        contact,
        phase,
        at,
    };
    assert_eq!(
        eqs.input(1, Touch::Move, 5.0, 5.0, 1.0),
        Vec::new(),
        "no contact"
    );
    assert_eq!(
        eqs.input(1, Touch::Down, 10.4, 200.0, 2.0),
        vec![
            rec("touch", &key(1), Some(1), Some(1), Some("down")),
            touch(1, Phase::Down, (10, 49)),
        ]
    );
    assert_eq!(
        eqs.input(1, Touch::Move, 12.0, 20.0, 3.0),
        vec![touch(1, Phase::Update, (12, 20))]
    );
    assert_eq!(
        eqs.input(1, Touch::Up, 14.0, 21.0, 4.0),
        vec![
            rec("touch", &key(1), Some(1), Some(1), Some("up")),
            touch(1, Phase::Update, (14, 21)),
            touch(1, Phase::Up, (14, 21)),
        ]
    );
    assert_eq!(
        eqs.input(1, Touch::Up, 14.0, 21.0, 5.0),
        Vec::new(),
        "ended"
    );
    eqs.input(1, Touch::Down, 1.0, 1.0, 6.0);
    assert_eq!(
        eqs.input(1, Touch::Up, 1.0, 1.0, 7.0),
        vec![
            rec("touch", &key(1), Some(1), Some(1), Some("up")),
            touch(2, Phase::Up, (1, 1)),
        ],
        "at its last point: no move first"
    );
    eqs.input(1, Touch::Down, 1.0, 1.0, 8.0);
    assert_eq!(
        eqs.input(1, Touch::Down, 2.0, 2.0, 9.0),
        vec![
            rec("touch", &key(1), Some(1), Some(1), Some("down")),
            touch(3, Phase::Up, (1, 1)),
            touch(4, Phase::Down, (2, 2)),
        ],
        "a new down ends the old contact first"
    );
    assert_eq!(
        eqs.input(1, Touch::Cancel, 30.0, 30.0, 10.0),
        vec![
            rec("touch", &key(1), Some(1), Some(1), Some("cancel")),
            touch(4, Phase::Cancel, (2, 2)),
        ],
        "a cancel ends it where it was"
    );
    assert_eq!(eqs.input(1, Touch::Cancel, 3.0, 3.0, 11.0), Vec::new());
    // Another client's editor while this contact is down: busy.
    eqs.input(1, Touch::Down, 1.0, 1.0, 12.0);
    eqs.open(2, &key(2), WALL);
    eqs.opened(&key(2), 2, Ok((100, 50)));
    assert_eq!(
        eqs.input(2, Touch::Down, 1.0, 1.0, 13.0),
        vec![rec("busy", &key(2), Some(2), Some(2), Some("down"))]
    );
    // A closing editor takes no more touches, and its contact is gone.
    eqs.close(1, reason::EXIT, true);
    assert_eq!(eqs.input(1, Touch::Move, 2.0, 2.0, 14.0), Vec::new());
    assert_eq!(eqs.input(2, Touch::Down, 1.0, 1.0, 15.0).len(), 2);
}

#[test]
fn a_resting_contact_is_left_to_the_worker_and_a_silent_pages_ends() {
    let mut eqs = listed();
    open_one(&mut eqs);
    assert_eq!(eqs.tick(1.0), Vec::new(), "no contact");
    eqs.input(1, Touch::Down, 5.0, 6.0, 1000.0);
    // The window worker keeps a resting contact alive on its own clock:
    // the router's tick sends nothing for it.
    assert_eq!(eqs.tick(1100.0), Vec::new());
    assert_eq!(eqs.tick(1250.0), Vec::new());
    eqs.input(1, Touch::Move, 7.0, 6.0, 1300.0);
    // Heard (a ping) keeps it; silent for 2 s ends it.
    eqs.heard(1, 2500.0);
    assert_eq!(eqs.tick(4499.0), Vec::new());
    assert_eq!(
        eqs.tick(4500.0),
        vec![
            rec("silent", &key(1), Some(1), Some(1), None),
            Act::Touch {
                session: 1,
                contact: 1,
                phase: Phase::Cancel,
                at: (7, 6)
            },
        ]
    );
    assert_eq!(eqs.tick(4600.0), Vec::new(), "ended");
    // A client never heard is silent at once.
    let mut eqs = listed();
    open_one(&mut eqs);
    eqs.input(1, Touch::Down, 5.0, 6.0, 0.0);
    eqs.detach(1);
    assert_eq!(eqs.tick(1.0), Vec::new(), "its close forgot the contact");
}

#[test]
fn a_contact_the_worker_ended_is_forgotten_never_a_newer_one() {
    let mut eqs = listed();
    open_one(&mut eqs);
    let moved = |contact: u32, at: (i32, i32)| {
        vec![Act::Touch {
            session: 1,
            contact,
            phase: Phase::Update,
            at,
        }]
    };
    eqs.input(1, Touch::Down, 5.0, 6.0, 0.0);
    // Another session's contact, or another number: this one is kept.
    eqs.contact_ended(2, 1);
    eqs.contact_ended(1, 2);
    assert_eq!(eqs.input(1, Touch::Move, 6.0, 6.0, 10.0), moved(1, (6, 6)));
    eqs.contact_ended(1, 1);
    assert_eq!(eqs.tick(20.0), Vec::new());
    assert_eq!(
        eqs.input(1, Touch::Move, 7.0, 7.0, 30.0),
        Vec::new(),
        "no contact to move"
    );
    // A late word of an older contact (its last update refused after the
    // router had sent its up and a new down) leaves the newer one alone.
    eqs.input(1, Touch::Down, 5.0, 6.0, 40.0);
    eqs.input(1, Touch::Up, 5.0, 6.0, 50.0);
    eqs.input(1, Touch::Down, 8.0, 8.0, 60.0);
    eqs.contact_ended(1, 2);
    assert_eq!(eqs.input(1, Touch::Move, 9.0, 9.0, 70.0), moved(3, (9, 9)));
}

#[test]
fn the_last_pictures_are_kept_per_editor() {
    let pictures = Pictures::default();
    assert!(!pictures.has(&key(1)));
    assert_eq!(pictures.get(&key(1)), None);
    pictures.put(&key(1), Bytes::from_static(b"one"));
    pictures.put(&key(1), Bytes::from_static(b"two"));
    assert!(pictures.has(&key(1)));
    assert!(!pictures.has(&key(2)));
    assert_eq!(pictures.get(&key(1)), Some(Bytes::from_static(b"two")));
    // Dropped by key, and by instance.
    let drums = EditorKey::new("drums", "live_set tracks 0 devices 0");
    for k in [key(2), key(3), drums.clone()] {
        pictures.put(&k, Bytes::from_static(b"jpeg"));
    }
    pictures.remove(&[key(1), key(2)]);
    assert_eq!(
        [key(1), key(2), key(3)].map(|k| pictures.has(&k)),
        [false, false, true]
    );
    pictures.forget("band");
    assert!(!pictures.has(&key(3)));
    assert!(pictures.has(&drums));
}
