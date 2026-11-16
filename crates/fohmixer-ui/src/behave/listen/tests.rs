use fohmixer_proto::layout::{Anchor, Binding, CtrlId};
use fohmixer_proto::listen::{AudioClientMsg, AudioState};
use serde_json::json;

use super::*;

fn target(track: &str) -> Target {
    Target {
        instance: "band".to_string(),
        track: track.to_string(),
        label: track.to_uppercase(),
    }
}

fn start(track: &str) -> Act {
    Act::Send(AudioClientMsg::ListenStart {
        instance: "band".to_string(),
        track: track.to_string(),
    })
}

fn binding(anchor: Anchor) -> Binding {
    Binding {
        instance: "band".to_string(),
        anchor,
        path: None,
    }
}

#[test]
fn a_track_by_name_is_known_and_one_by_place_is_read() {
    assert_eq!(
        track_name(&binding(Anchor::Track {
            name: "Vox 1".into()
        })),
        TrackName::Known("Vox 1".into())
    );
    assert_eq!(
        track_name(&binding(Anchor::Return {
            name: "A-Reverb".into()
        })),
        TrackName::Known("A-Reverb".into())
    );
    assert_eq!(
        track_name(&binding(Anchor::TrackAt { index: 3 })),
        TrackName::Read("live_set tracks 3".into())
    );
    assert_eq!(
        track_name(&binding(Anchor::ReturnAt { index: 1 })),
        TrackName::Read("live_set return_tracks 1".into())
    );
    assert_eq!(
        track_name(&binding(Anchor::Master)),
        TrackName::Read("live_set master_track".into())
    );
    assert_eq!(track_name(&binding(Anchor::Song)), TrackName::None);
    assert_eq!(
        track_name(&binding(Anchor::Ctrl { id: CtrlId::Stage })),
        TrackName::None
    );
}

#[test]
fn a_read_name_is_its_first_slots_text() {
    let ok = Ok(vec![json!({"ok": true, "data": "Vox 1"})]);
    assert_eq!(name_of(&ok), Some("Vox 1".to_string()));
    assert_eq!(name_of(&Ok(vec![json!({"ok": true, "data": ""})])), None);
    assert_eq!(name_of(&Ok(vec![json!({"ok": false, "data": "x"})])), None);
    assert_eq!(name_of(&Ok(vec![json!({"data": "x"})])), None);
    assert_eq!(name_of(&Ok(vec![json!({"ok": true, "data": 3})])), None);
    assert_eq!(name_of(&Ok(Vec::new())), None);
    assert_eq!(name_of(&Err("not connected".into())), None);
}

#[test]
fn a_tap_starts_by_opening_the_socket_and_asks_once_it_opened() {
    let mut l = Listen::new();
    assert_eq!(l.phase(), Phase::Off);
    assert!(!l.lit());
    assert_eq!(l.tap(Some(target("Vox 1")), true), vec![Act::Open]);
    assert_eq!(l.phase(), Phase::Opening);
    assert!(l.lit());
    assert_eq!(l.want(), Some(&target("Vox 1")));
    assert_eq!(l.opened(), vec![start("Vox 1")]);
    assert_eq!(
        l.status(AudioState::Listening, Some("band"), Some("Vox 1")),
        Vec::new()
    );
    assert_eq!(l.phase(), Phase::Listening);
    assert_eq!(l.label(), Some("VOX 1"));
}

#[test]
fn a_second_tap_stops_and_closes() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    assert_eq!(
        l.tap(Some(target("Vox 1")), true),
        vec![Act::Close, Act::Silence]
    );
    assert_eq!(l.phase(), Phase::Off);
    assert!(!l.lit());
    // The socket is gone: the next start opens it again.
    assert_eq!(l.tap(Some(target("Vox 2")), true), vec![Act::Open]);
}

#[test]
fn a_tap_while_the_socket_is_open_asks_at_once() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    // The hub's taken closes it; an open socket only while one is wanted.
    let mut open = Listen::new();
    open.open = true;
    assert_eq!(open.tap(Some(target("Vox 2")), true), vec![start("Vox 2")]);
}

#[test]
fn a_tap_with_no_name_or_no_decoder_starts_nothing() {
    let mut l = Listen::new();
    assert_eq!(l.tap(None, true), Vec::new());
    assert_eq!(l.phase(), Phase::Off);
    assert_eq!(l.tap(Some(target("Vox 1")), false), Vec::new());
    assert_eq!(l.phase(), Phase::NoDecoder);
    assert!(!l.lit());
    assert_eq!(l.label(), Some("VOX 1"));
}

#[test]
fn another_channel_is_followed_while_listening() {
    let mut l = Listen::new();
    // Not listening: a channel change does nothing.
    assert_eq!(l.follow(target("Vox 2")), Vec::new());
    assert!(!l.lit());
    l.tap(Some(target("Vox 1")), true);
    // Still opening: the start goes once the socket opened.
    assert_eq!(l.follow(target("Vox 2")), Vec::new());
    assert_eq!(l.opened(), vec![start("Vox 2")]);
    l.status(AudioState::Listening, Some("band"), Some("Vox 2"));
    assert_eq!(l.follow(target("Keys")), vec![start("Keys")]);
    assert_eq!(l.phase(), Phase::Opening);
    // The old track's status, late, changes nothing.
    l.status(AudioState::Listening, Some("band"), Some("Vox 2"));
    assert_eq!(l.phase(), Phase::Opening);
    l.status(AudioState::Taken, Some("band"), Some("Vox 2"));
    assert!(l.lit());
    l.status(AudioState::Listening, Some("band"), Some("Keys"));
    assert_eq!(l.phase(), Phase::Listening);
}

#[test]
fn the_same_track_again_keeps_listening_with_its_new_label() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    l.status(AudioState::Listening, Some("band"), Some("Vox 1"));
    let renamed = Target {
        label: "LEAD".to_string(),
        ..target("Vox 1")
    };
    assert_eq!(l.follow(renamed), Vec::new());
    assert_eq!(l.phase(), Phase::Listening);
    assert_eq!(l.label(), Some("LEAD"));
    // Another instance's track of the same name is another track.
    let master = Target {
        instance: "master".to_string(),
        ..target("Vox 1")
    };
    assert_eq!(
        l.follow(master),
        vec![Act::Send(AudioClientMsg::ListenStart {
            instance: "master".to_string(),
            track: "Vox 1".to_string(),
        })]
    );
}

#[test]
fn no_source_is_shown_and_listening_comes_back() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    l.status(AudioState::NoSource, Some("band"), Some("Vox 1"));
    assert_eq!(l.phase(), Phase::NoSource);
    assert!(l.lit());
    l.status(AudioState::Listening, Some("band"), Some("Vox 1"));
    assert_eq!(l.phase(), Phase::Listening);
    // Another instance's status is not ours.
    l.status(AudioState::NoSource, Some("master"), Some("Vox 1"));
    assert_eq!(l.phase(), Phase::Listening);
}

#[test]
fn taken_stops_and_names_the_track() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    assert_eq!(
        l.status(AudioState::Taken, Some("band"), Some("Vox 1")),
        vec![Act::Close, Act::Silence]
    );
    assert_eq!(l.phase(), Phase::Taken);
    assert!(!l.lit());
    assert_eq!(l.label(), Some("VOX 1"));
    // A later status (the socket's last words) changes nothing.
    assert_eq!(l.status(AudioState::Stopped, None, None), Vec::new());
    assert_eq!(l.phase(), Phase::Taken);
    // The next tap starts afresh, opening the socket.
    assert_eq!(l.tap(Some(target("Vox 1")), true), vec![Act::Open]);
}

#[test]
fn refused_stops_whatever_it_names() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    assert_eq!(
        l.status(AudioState::Refused, None, None),
        vec![Act::Close, Act::Silence]
    );
    assert_eq!(l.phase(), Phase::Refused);
    assert!(!l.lit());
}

#[test]
fn stopped_changes_nothing_while_listening() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    l.status(AudioState::Listening, Some("band"), Some("Vox 1"));
    assert_eq!(l.status(AudioState::Stopped, None, None), Vec::new());
    assert_eq!(l.phase(), Phase::Listening);
}

#[test]
fn a_lost_socket_opens_again_while_listening() {
    let mut l = Listen::new();
    assert_eq!(l.closed(), Vec::new());
    l.tap(Some(target("Vox 1")), true);
    l.opened();
    assert_eq!(l.closed(), vec![Act::Reopen]);
    assert_eq!(l.phase(), Phase::Lost);
    assert!(l.lit());
    // A follow meanwhile waits for the socket.
    assert_eq!(l.follow(target("Vox 2")), Vec::new());
    assert_eq!(l.opened(), vec![start("Vox 2")]);
}

#[test]
fn a_socket_opened_after_a_stop_closes_again() {
    let mut l = Listen::new();
    l.tap(Some(target("Vox 1")), true);
    l.tap(None, true);
    assert_eq!(l.opened(), vec![Act::Close]);
    assert_eq!(l.closed(), Vec::new());
    assert_eq!(l.phase(), Phase::Off);
}

#[test]
fn the_phases_have_their_names() {
    let all = [
        (Phase::Off, "off"),
        (Phase::Opening, "opening"),
        (Phase::Listening, "listening"),
        (Phase::NoSource, "no_source"),
        (Phase::Lost, "lost"),
        (Phase::Taken, "taken"),
        (Phase::Refused, "refused"),
        (Phase::NoDecoder, "no_decoder"),
    ];
    for (phase, name) in all {
        assert_eq!(phase.name(), name);
    }
}

#[test]
fn the_foot_says_what_is_heard_or_why_not() {
    assert_eq!(foot_text(Phase::Off, Some("VOX 1")), None);
    assert_eq!(
        foot_text(Phase::Opening, Some("VOX 1")).as_deref(),
        Some("listening VOX 1")
    );
    assert_eq!(
        foot_text(Phase::Listening, Some("VOX 1")).as_deref(),
        Some("listening VOX 1")
    );
    assert_eq!(
        foot_text(Phase::NoSource, Some("VOX 1")).as_deref(),
        Some("VOX 1: no source")
    );
    assert_eq!(
        foot_text(Phase::Lost, Some("VOX 1")).as_deref(),
        Some("VOX 1: reconnecting")
    );
    assert_eq!(
        foot_text(Phase::Taken, Some("VOX 1")).as_deref(),
        Some("VOX 1: taken by another page")
    );
    assert_eq!(
        foot_text(Phase::Refused, Some("VOX 1")).as_deref(),
        Some("VOX 1: refused")
    );
    assert_eq!(
        foot_text(Phase::NoDecoder, None).as_deref(),
        Some("no audio decoder in this browser")
    );
    assert_eq!(
        foot_text(Phase::Listening, None).as_deref(),
        Some("listening ")
    );
}

#[test]
fn the_volume_is_silent_at_zero_and_a_straight_db_line_above() {
    assert_eq!(gain_db(0.0), None);
    assert_eq!(gain_db(-0.5), None);
    assert_eq!(gain_db(f64::MIN_POSITIVE), Some(-40.0));
    assert_eq!(gain_db(1.0), Some(12.0));
    assert_eq!(gain_db(2.0), Some(12.0));
    assert_eq!(gain_db(0.5), Some(-14.0));
    assert_eq!(gain_db(0.25), Some(-27.0));
    assert_eq!((MIN_DB, MAX_DB, VOLUME_DEFAULT), (-40.0, 12.0, 0.7));
    assert_eq!(RECONNECT_MS, 1000);
    assert_eq!(gain(0.0), 0.0);
    assert!((gain(1.0) - 3.981_071_705_534_973).abs() < 1e-12);
    assert!((gain(40.0 / 52.0) - 1.0).abs() < 1e-12);
    assert!((gain(0.5) - 0.199_526_231_496_888).abs() < 1e-12);
}

#[test]
fn the_volume_under_a_finger_is_its_place_on_the_slider() {
    assert_eq!(volume_at(50.0, 0.0, 100.0), 0.5);
    assert_eq!(volume_at(30.0, 10.0, 80.0), 0.25);
    assert_eq!(volume_at(-5.0, 0.0, 100.0), 0.0);
    assert_eq!(volume_at(150.0, 0.0, 100.0), 1.0);
    assert_eq!(volume_at(5.0, 0.0, 0.0), 0.0);
    assert_eq!(volume_at(5.0, 0.0, -1.0), 0.0);
    assert_eq!(volume_at(5.0, 0.0, f64::MIN_POSITIVE), 1.0);
}
