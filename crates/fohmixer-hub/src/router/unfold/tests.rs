use serde_json::json;

use super::*;

fn t(instance: &str, name: &str) -> Track {
    (instance.to_string(), name.to_string())
}

/// A strip's track by name, as `Layout::strip_tracks` gives it: its target.
fn s(instance: &str, name: &str) -> Track {
    (instance.to_string(), target(name))
}

fn key(watch: &Watch) -> String {
    let (instance, target, prop) = watch.target();
    format!("{instance}|{target}|{prop}")
}

/// The router's part: every `Sub` subscribed under its key; the reads'
/// numbers and the other actions returned.
fn apply(keeper: &mut Keeper, actions: Vec<Action>) -> (Vec<u64>, Vec<Action>) {
    let (mut reads, mut rest) = (Vec::new(), Vec::new());
    for action in actions {
        match action {
            Action::Sub(watch) => {
                let key = key(&watch);
                keeper.subscribed(watch, key);
            }
            Action::Read { seq, .. } => reads.push(seq),
            other => rest.push(other),
        }
    }
    (reads, rest)
}

/// A read's answer for one track, step by step up its chain: `(name,
/// fold)` per step, `None` past the top; error slots to `MAX_DEPTH`.
fn chain(steps: &[(&str, Value)]) -> Vec<Value> {
    let mut slots = Vec::new();
    for depth in 0..MAX_DEPTH {
        match steps.get(depth) {
            Some((name, fold)) => {
                slots.push(json!({"ok": true, "data": name}));
                slots.push(json!({"ok": true, "data": fold}));
            }
            None => {
                for _ in 0..2 {
                    slots.push(json!({"ok": false, "error": "None", "errorType": "PathError"}));
                }
            }
        }
    }
    slots
}

fn unfold(group: &str, path: &str) -> Action {
    Action::Unfold {
        instance: "band".into(),
        target: path.into(),
        group: group.into(),
    }
}

#[test]
fn a_read_holds_the_strips_groups_and_unfolds_the_folded_ones() {
    let mut k = Keeper::default();
    let actions = k.set_strips([s("band", "Drums #"), s("band", "Vox")]);
    // Both of the band's lists are watched, and the groups read at once.
    assert_eq!(
        actions[..2],
        [
            Action::Sub(Watch::Tracks("band".into())),
            Action::Sub(Watch::Visible("band".into()))
        ]
    );
    let Action::Read { seq, commands, .. } = &actions[2] else {
        panic!("a read: {actions:?}")
    };
    assert_eq!(commands.len(), 2 * 2 * MAX_DEPTH);
    let seq = *seq;
    apply(&mut k, actions);
    // Drums # sits in Stems grp# (open, as real Live answers it), inside
    // BAND grp# (folded, SimLive's 1); Vox sits in Vocals grp# (folded,
    // real Live's true).
    let mut slots = chain(&[("Stems grp#", json!(false)), ("BAND grp#", json!(1))]);
    slots.extend(chain(&[("Vocals grp#", json!(true))]));
    assert_eq!(
        k.read_done("band", seq, &Ok(slots)),
        vec![
            unfold(
                "BAND grp#",
                "live_set tracks[name=Drums #] group_track group_track"
            ),
            unfold("Vocals grp#", "live_set tracks[name=Vox] group_track"),
        ]
    );
    assert_eq!(
        k.held(),
        vec![
            t("band", "BAND grp#"),
            t("band", "Stems grp#"),
            t("band", "Vocals grp#")
        ]
    );
}

#[test]
fn a_group_two_strips_share_is_unfolded_once_through_the_first_path() {
    let mut k = Keeper::default();
    let (reads, _) = {
        let actions = k.set_strips([s("band", "A"), s("band", "B")]);
        apply(&mut k, actions)
    };
    let mut slots = chain(&[("G", json!(true))]);
    slots.extend(chain(&[("G", json!(true))]));
    assert_eq!(
        k.read_done("band", reads[0], &Ok(slots)),
        vec![unfold("G", "live_set tracks[name=A] group_track")]
    );
}

#[test]
fn a_list_change_reads_again_and_a_late_answer_is_void() {
    let mut k = Keeper::default();
    let actions = k.set_strips([s("band", "Drums #")]);
    let (reads, _) = apply(&mut k, actions);
    k.read_done(
        "band",
        reads[0],
        &Ok(chain(&[("Stems grp#", json!(false))])),
    );
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    // A group folded (visible_tracks) or a track moved (tracks): a new read.
    for watch in [Watch::Visible("band".into()), Watch::Tracks("band".into())] {
        let actions = k.value(&key(&watch), Some(&json!([])));
        let (again, rest) = apply(&mut k, actions);
        assert_eq!((again.len(), rest), (1, vec![]));
    }
    // A failing list (no value) is no news; nor is a key never subscribed.
    assert_eq!(k.value(&key(&Watch::Tracks("band".into())), None), vec![]);
    assert_eq!(k.value("band|live_set|other", Some(&json!(1))), vec![]);
    // The first read's late answer is void: the groups stay as read.
    assert_eq!(
        k.read_done("band", reads[0], &Ok(chain(&[("Old grp#", json!(true))]))),
        vec![]
    );
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    // A read of an instance never read is void too.
    assert_eq!(
        k.read_done("master", reads[0], &Ok(chain(&[("G", json!(true))]))),
        vec![]
    );
}

#[test]
fn the_track_leaving_its_group_lets_the_group_go() {
    let mut k = Keeper::default();
    let actions = k.set_strips([s("band", "Drums #")]);
    let (reads, _) = apply(&mut k, actions);
    k.read_done(
        "band",
        reads[0],
        &Ok(chain(&[("Stems grp#", json!(false))])),
    );
    let actions = k.value(&key(&Watch::Tracks("band".into())), Some(&json!([])));
    let (again, _) = apply(&mut k, actions);
    assert_eq!(k.read_done("band", again[0], &Ok(chain(&[]))), vec![]);
    assert_eq!(k.held(), vec![]);
}

#[test]
fn a_new_layout_reads_each_instance_and_drops_one_without_strips() {
    let mut k = Keeper::default();
    let actions = k.set_strips([s("band", "Drums #"), s("master", "Hand1 #")]);
    let (reads, _) = apply(&mut k, actions);
    assert_eq!(reads.len(), 2, "one read per instance");
    k.read_done(
        "band",
        reads[0],
        &Ok(chain(&[("Stems grp#", json!(false))])),
    );
    k.read_done("master", reads[1], &Ok(chain(&[("M grp#", json!(false))])));
    assert_eq!(
        k.held(),
        vec![t("band", "Stems grp#"), t("master", "M grp#")]
    );
    // The master's strips are gone: its lists are let go, and its read in
    // flight is void.
    let master_read = {
        let actions = k.value(&key(&Watch::Visible("master".into())), Some(&json!([])));
        apply(&mut k, actions).0[0]
    };
    let actions = k.set_strips([s("band", "Drums #")]);
    let mut gone: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            Action::Unsub { key } => Some(key.clone()),
            _ => None,
        })
        .collect();
    gone.sort();
    assert_eq!(
        gone,
        vec![
            key(&Watch::Tracks("master".into())),
            key(&Watch::Visible("master".into()))
        ]
    );
    assert_eq!(actions.len(), 3, "two removals and the band's read");
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    assert_eq!(
        k.read_done(
            "master",
            master_read,
            &Ok(chain(&[("M grp#", json!(true))]))
        ),
        vec![]
    );
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
}

#[test]
fn a_failed_read_is_tried_again_three_times_in_a_row() {
    let mut k = Keeper::default();
    let actions = k.set_strips([s("band", "Drums #")]);
    let (mut reads, _) = apply(&mut k, actions);
    let retry = vec![Action::Retry {
        instance: "band".into(),
    }];
    for _ in 0..MAX_RETRIES {
        let seq = *reads.last().unwrap();
        assert_eq!(k.read_done("band", seq, &Err("timeout".into())), retry);
        let actions = k.retry("band");
        let (again, _) = apply(&mut k, actions);
        reads.extend(again);
    }
    // A fourth failure in a row waits for news (a list change, a connect).
    let seq = *reads.last().unwrap();
    assert_eq!(k.read_done("band", seq, &Err("timeout".into())), vec![]);
    // News starts a new run of retries.
    let actions = k.value(&key(&Watch::Tracks("band".into())), Some(&json!([])));
    let (fresh, _) = apply(&mut k, actions);
    assert_eq!(k.read_done("band", fresh[0], &Err("timeout".into())), retry);
    // A success ends a run: the next failure is the first again.
    let actions = k.retry("band");
    let (next, _) = apply(&mut k, actions);
    k.read_done("band", next[0], &Ok(chain(&[("Stems grp#", json!(false))])));
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    let actions = k.retry("band");
    let (after, _) = apply(&mut k, actions);
    for _ in 0..MAX_RETRIES {
        assert_eq!(k.read_done("band", after[0], &Err("timeout".into())), retry);
    }
    // A failed read keeps the groups held.
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    // An instance without strips is not tried again.
    assert_eq!(k.retry("master"), vec![]);
}

#[test]
fn reads_climb_the_chain_of_groups() {
    let commands = read_commands(&[target("A]b")]);
    assert_eq!(commands.len(), 2 * MAX_DEPTH);
    assert_eq!(
        commands[0],
        json!({"target": r"live_set tracks[name=A\]b] group_track", "name": "get_prop", "args": {"prop": "name"}})
    );
    assert_eq!(
        commands[1],
        json!({"target": r"live_set tracks[name=A\]b] group_track", "name": "get_prop", "args": {"prop": "fold_state"}})
    );
    assert_eq!(
        commands[2]["target"],
        r"live_set tracks[name=A\]b] group_track group_track"
    );
    assert_eq!(read_commands(&[]), Vec::<Value>::new());
    // A name that is not a string, a slot not ok or missing: no group.
    let tracks = vec![target("T")];
    let (names, folded_at) = read_groups(
        &tracks,
        &[
            json!({"ok": true, "data": null}),
            json!({"ok": true, "data": true}),
            json!({"ok": "yes", "data": "H"}),
            json!({"ok": true, "data": true}),
            json!({"ok": true, "data": "G"}),
        ],
    );
    assert_eq!(names, BTreeSet::from(["G".to_string()]));
    assert_eq!(folded_at, BTreeMap::new(), "G's fold slot is missing");
}

#[test]
fn fold_values_targets_and_listings() {
    assert!(folded(&json!(1)));
    assert!(folded(&json!(true)));
    assert!(!folded(&json!(0)));
    assert!(!folded(&json!(false)));
    assert!(!folded(&json!(null)));
    assert!(!folded(&json!(2)));
    assert_eq!(target("Stems grp#"), "live_set tracks[name=Stems grp#]");
    assert_eq!(
        group_path(&target("A"), 2),
        "live_set tracks[name=A] group_track group_track"
    );
    assert_eq!(
        Watch::Tracks("band".into()).target(),
        ("band", "live_set".to_string(), TRACKS)
    );
    assert_eq!(
        Watch::Visible("band".into()).target(),
        ("band", "live_set".to_string(), VISIBLE)
    );
    assert_eq!(
        by_instance(vec![t("band", "A"), t("master", "C"), t("band", "B")]),
        BTreeMap::from([
            ("band".to_string(), vec!["A".to_string(), "B".to_string()]),
            ("master".to_string(), vec!["C".to_string()]),
        ])
    );
    assert_eq!(by_instance(Vec::new()), BTreeMap::new());
}
