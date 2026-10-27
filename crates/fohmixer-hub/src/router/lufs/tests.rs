use super::*;
use serde_json::json;

fn lists() -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            hub_key("band", "live_set", "tracks", false),
            json!([
                {"$ref": "1", "path": "live_set tracks 0", "class": "Track", "name": "Hand1 #"},
                {"$ref": "2", "path": "live_set tracks 1", "class": "Track", "name": "Vox 1"},
            ]),
        ),
        (
            hub_key("band", "live_set", "return_tracks", false),
            json!([{"$ref": "3", "path": "live_set return_tracks 0", "class": "Track", "name": "A-Reverb #"}]),
        ),
    ])
}

#[test]
fn the_constants() {
    assert_eq!(LEVELS_CLIENT, ClientId::MAX - 5);
    assert_eq!(LUFS_TICK, Duration::from_millis(100));
}

#[test]
fn a_points_track_is_named_from_its_anchor_or_the_hubs_lists() {
    let masters = BTreeMap::from([("band".to_string(), "Main".to_string())]);
    let name = |anchor: Anchor| track_name(&lists(), &masters, "band", &anchor);
    assert_eq!(name(Anchor::TrackAt { index: 1 }).as_deref(), Some("Vox 1"));
    assert_eq!(name(Anchor::TrackAt { index: 2 }), None);
    assert_eq!(
        name(Anchor::ReturnAt { index: 0 }).as_deref(),
        Some("A-Reverb #")
    );
    assert_eq!(name(Anchor::ReturnAt { index: 1 }), None);
    assert_eq!(
        name(Anchor::Track {
            name: "Vocals".into()
        })
        .as_deref(),
        Some("Vocals")
    );
    assert_eq!(
        name(Anchor::Return {
            name: "PA Ret".into()
        })
        .as_deref(),
        Some("PA Ret")
    );
    assert_eq!(name(Anchor::Master).as_deref(), Some("Main"));
    assert_eq!(name(Anchor::Song), None);
    // Another instance's lists name nothing here.
    assert_eq!(
        track_name(&lists(), &masters, "master", &Anchor::TrackAt { index: 1 }),
        None
    );
    assert_eq!(
        track_name(&lists(), &masters, "master", &Anchor::Master),
        None
    );
}

#[test]
fn an_item_without_a_name_names_nothing() {
    assert_eq!(item_name(&json!([{"$ref": "1"}]), 0), None);
    assert_eq!(item_name(&json!("not a list"), 0), None);
    assert_eq!(item_name(&json!([{"name": "A"}]), 0).as_deref(), Some("A"));
}

#[test]
fn mute_reads_a_bool_or_a_number() {
    assert_eq!(mute_of(&json!(true)), Some(true));
    assert_eq!(mute_of(&json!(false)), Some(false));
    assert_eq!(mute_of(&json!(1)), Some(true));
    assert_eq!(mute_of(&json!(0)), Some(false));
    assert_eq!(mute_of(&json!("on")), None);
}

#[test]
fn the_names_read_answers_both_names_or_why_not() {
    let ok = |name: &str| json!({"ok": true, "data": name});
    assert_eq!(
        names_of(&[ok("FOH Band"), ok("Main")]),
        Ok(("FOH Band".to_string(), "Main".to_string()))
    );
    assert!(names_of(&[ok("FOH Band")]).is_err());
    assert!(names_of(&[json!({"ok": false, "error": "x"}), ok("Main")]).is_err());
    assert!(names_of(&[json!({"ok": true, "data": 3}), ok("Main")]).is_err());
}

#[test]
fn a_names_read_is_tried_again_only_when_live_did_not_answer() {
    assert!(read_again("instance offline", 1));
    assert!(read_again("no result within 3 s", CONNECT_RETRIES));
    assert!(!read_again("no result within 3 s", CONNECT_RETRIES + 1));
    assert!(!read_again("not found: live_set", 1));
}
