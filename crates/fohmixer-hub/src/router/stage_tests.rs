//! STAGE AUT against the marker keeper (#104): a write made while the track
//! places are unsure, through the keeper's own read and quiet time.

use std::sync::Arc;

use fohmixer_proto::client::HUB_STAGE_AUT;
use fohmixer_proto::layout::CtrlId;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::reverb::tests::{answer, next_read, pushed, read_answered, settle, slot};
use super::tests::{attach, offline_router, records};
use super::{Router, RouterMsg};

const STAGE_MARK: &str = "+CTRL:stage";
const OTHER_MARK: &str = r#""Other" +G:FX"#;

struct Staged {
    router: Router,
    rx: std::sync::mpsc::Receiver<Value>,
    own: UnboundedReceiver<RouterMsg>,
    _dir: tempfile::TempDir,
}

/// A router whose marker keeper follows the band (tracks `[1, 2]`), the
/// STAGE control on `+CTRL:stage` on track 1, composed from the keeper's own
/// read, STAGE AUT on, Live standing: the rule's first write made.
async fn staged() -> Staged {
    let dir = tempfile::tempdir().unwrap();
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "stage", "aut": true,
                  "binding": {"instance": "band", "anchor": {"kind": "ctrl", "id": "stage"}}}]}]});
    std::fs::write(dir.path().join("layout.json"), frame.to_string()).unwrap();
    let store = Arc::new(crate::layout::store_in(
        dir.path(),
        std::path::Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, mut own, rx) = offline_router(dir.path());
    let mut router = router.with_layout(store);
    let _page = attach(&mut router, 1);
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "return_tracks") => slot("live_1.returns", json!([])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let (seq, _) = next_read(&mut own).await;
    read_answered(&mut router, &mut own, seq, 2, &[(1, STAGE_MARK)]).await;
    settle(&mut router, &mut own).await;
    let target = router.stage_target.as_ref().expect("the stage bound");
    assert_eq!(
        (target.target.as_str(), target.ctrl),
        ("live_set tracks 1", Some(CtrlId::Stage))
    );
    router.handle(RouterMsg::SetHub {
        client: 1,
        key: HUB_STAGE_AUT.into(),
        value: json!(true),
    });
    assert_eq!(router.stage.writes(), 1);
    let _ = records(&rx);
    Staged {
        router,
        rx,
        own,
        _dir: dir,
    }
}

/// Live's new list of tracks in a flush of its own.
fn list(router: &mut Router, tracks: Value) {
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", tracks)]);
    router.flush();
}

/// Live starts, in a flush of its own.
fn start(router: &mut Router) {
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
}

/// A track inserted above the stage track once the note is gone: the move
/// must write nothing over a mute set by hand.
async fn a_later_move_writes_nothing(s: &mut Staged, tracks: u32) {
    let writes = s.router.stage.writes();
    let list_now: Vec<u32> = (0..=tracks).collect();
    list(&mut s.router, json!(list_now));
    let (seq, _) = next_read(&mut s.own).await;
    let tuners = [(tracks, STAGE_MARK)];
    read_answered(&mut s.router, &mut s.own, seq, tracks + 1, &tuners).await;
    settle(&mut s.router, &mut s.own).await;
    assert_eq!(
        s.router.stage_target.as_ref().map(|t| t.target.as_str()),
        Some(format!("live_set tracks {tracks}").as_str())
    );
    assert_eq!(s.router.stage.writes(), writes, "a hand's mute stays");
}

#[tokio::test]
async fn a_list_change_and_a_start_in_one_flush_write_the_moved_track_too() {
    // The rule hears the transport after the keeper's values: the read runs
    // when it writes, so that write counts as unsure, and the move writes
    // the mute again on the new index.
    let mut s = staged().await;
    s.router.subs.on_values(
        "band",
        &[
            pushed("live_1.is_playing", json!(true)),
            pushed("live_1.tracks", json!([9, 1, 2])),
        ],
    );
    s.router.flush();
    assert_eq!(s.router.stage.writes(), 2, "the old index");
    let (seq, _) = next_read(&mut s.own).await;
    read_answered(&mut s.router, &mut s.own, seq, 3, &[(2, STAGE_MARK)]).await;
    settle(&mut s.router, &mut s.own).await;
    assert_eq!(s.router.stage.writes(), 3, "again, on the moved track");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let got = records(&s.rx);
    let failed: Vec<&Value> = got
        .iter()
        .filter(|r| r["ev"] == "stage_aut" && r["step"] == "failed")
        .map(|r| &r["target"])
        .collect();
    assert_eq!(failed.last(), Some(&&json!("live_set tracks 2")));
}

#[tokio::test]
async fn an_unchanged_read_forgets_a_write_made_while_it_ran() {
    // `markers_apply`: the read ends with nothing found anew.
    let mut s = staged().await;
    list(&mut s.router, json!([1, 2, 9]));
    let (seq, _) = next_read(&mut s.own).await;
    start(&mut s.router);
    assert_eq!(s.router.stage.writes(), 2);
    read_answered(&mut s.router, &mut s.own, seq, 3, &[(1, STAGE_MARK)]).await;
    a_later_move_writes_nothing(&mut s, 3).await;
}

#[tokio::test]
async fn a_composition_that_leaves_the_stage_forgets_a_write_made_before_it() {
    // `markers_settled`: another Tuner changed, the stage track did not.
    let mut s = staged().await;
    list(&mut s.router, json!([1, 2, 9]));
    let (seq, _) = next_read(&mut s.own).await;
    start(&mut s.router);
    assert_eq!(s.router.stage.writes(), 2);
    let tuners = [(1, STAGE_MARK), (2, OTHER_MARK)];
    read_answered(&mut s.router, &mut s.own, seq, 3, &tuners).await;
    assert_eq!(s.router.stage.writes(), 2);
    settle(&mut s.router, &mut s.own).await;
    a_later_move_writes_nothing(&mut s, 3).await;
}
