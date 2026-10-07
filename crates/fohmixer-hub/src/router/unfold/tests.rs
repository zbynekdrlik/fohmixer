use serde_json::json;

use super::*;

fn t(instance: &str, name: &str) -> Track {
    (instance.to_string(), name.to_string())
}

fn fold(instance: &str, name: &str) -> Watch {
    Watch::Fold(t(instance, name))
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

/// A read's answer: one name slot per entry, `None` for an error slot.
fn answer(names: &[Option<&str>]) -> Result<Vec<Value>, String> {
    Ok(names
        .iter()
        .map(|n| match n {
            Some(name) => json!({"ok": true, "data": name}),
            None => json!({"ok": false, "error": "None", "errorType": "PathError"}),
        })
        .collect())
}

#[test]
fn a_read_finds_the_strips_groups_and_their_folds_are_watched() {
    let mut k = Keeper::default();
    let actions = k.set_strips([t("band", "Vocal 1 repro#"), t("band", "Drums #")]);
    // The track list is watched and the groups read at once.
    assert_eq!(actions[0], Action::Sub(Watch::Tracks("band".into())));
    let Action::Read {
        instance,
        seq,
        commands,
    } = &actions[1]
    else {
        panic!("a read: {actions:?}")
    };
    assert_eq!((instance.as_str(), actions.len()), ("band", 2));
    assert_eq!(commands.len(), 2 * MAX_DEPTH);
    let seq = *seq;
    apply(&mut k, actions);
    // Drums # sits in Stems grp#, inside BAND grp#; Vocal 1 repro# in none.
    let actions = k.read_done(
        "band",
        seq,
        &answer(&[Some("Stems grp#"), Some("BAND grp#"), None]),
    );
    assert_eq!(
        actions,
        vec![
            Action::Sub(fold("band", "BAND grp#")),
            Action::Sub(fold("band", "Stems grp#"))
        ]
    );
    apply(&mut k, actions);
    assert_eq!(
        k.held(),
        vec![t("band", "BAND grp#"), t("band", "Stems grp#")]
    );
    // Folded: unfolded at once, every time; open, or an error: nothing.
    let stems = key(&fold("band", "Stems grp#"));
    for _ in 0..2 {
        assert_eq!(
            k.value(&stems, Some(&json!(1))),
            vec![Action::Unfold(t("band", "Stems grp#"))]
        );
    }
    assert_eq!(k.value(&stems, Some(&json!(0))), vec![]);
    assert_eq!(k.value(&stems, None), vec![]);
    assert_eq!(k.value("band|unknown|fold_state", Some(&json!(1))), vec![]);
}

#[test]
fn a_track_list_change_reads_again_and_lets_go_of_a_group_left() {
    let mut k = Keeper::default();
    let actions = k.set_strips([t("band", "Drums #")]);
    let (reads, _) = apply(&mut k, actions);
    let actions = k.read_done("band", reads[0], &answer(&[Some("Stems grp#")]));
    let (_, rest) = apply(&mut k, actions);
    assert_eq!(rest, vec![]);
    // The list changed (or Live connected): a new read.
    let actions = k.value(&key(&Watch::Tracks("band".into())), Some(&json!([])));
    let (again, rest) = apply(&mut k, actions);
    assert_eq!((again.len(), rest), (1, vec![]));
    assert!(again[0] > reads[0]);
    // The old read's late answer is void.
    assert_eq!(
        k.read_done("band", reads[0], &answer(&[Some("Old grp#")])),
        vec![]
    );
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
    // The track left its group: the group is let go.
    assert_eq!(
        k.read_done("band", again[0], &answer(&[None])),
        vec![Action::Unsub {
            key: key(&fold("band", "Stems grp#"))
        }]
    );
    assert_eq!(k.held(), vec![]);
    // A failed read changes nothing.
    let actions = k.value(&key(&Watch::Tracks("band".into())), None);
    let (third, _) = apply(&mut k, actions);
    assert_eq!(
        k.read_done("band", third[0], &Err("instance offline".into())),
        vec![]
    );
    // A read of an instance never read is void.
    assert_eq!(
        k.read_done("master", third[0], &answer(&[Some("G")])),
        vec![]
    );
}

#[test]
fn a_new_layout_reads_each_instance_and_drops_one_without_strips() {
    let mut k = Keeper::default();
    let actions = k.set_strips([t("band", "Drums #"), t("master", "Hand1 #")]);
    let (reads, _) = apply(&mut k, actions);
    assert_eq!(reads.len(), 2, "one read per instance");
    let actions = k.read_done("band", reads[0], &answer(&[Some("Stems grp#")]));
    apply(&mut k, actions);
    let actions = k.read_done("master", reads[1], &answer(&[Some("M grp#")]));
    apply(&mut k, actions);
    assert_eq!(
        k.held(),
        vec![t("band", "Stems grp#"), t("master", "M grp#")]
    );
    // The master's strips are gone: its list and its group are let go.
    let actions = k.set_strips([t("band", "Drums #")]);
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
            key(&fold("master", "M grp#")),
            key(&Watch::Tracks("master".into()))
        ]
    );
    assert_eq!(
        actions
            .iter()
            .filter(|a| matches!(a, Action::Read { instance, .. } if instance == "band"))
            .count(),
        1
    );
    assert_eq!(actions.len(), 3, "two removals and the band's read");
    assert_eq!(k.held(), vec![t("band", "Stems grp#")]);
}

#[test]
fn reads_climb_the_chain_of_groups() {
    let commands = read_commands(&["A]b".to_string()]);
    assert_eq!(commands.len(), MAX_DEPTH);
    assert_eq!(
        commands[0],
        json!({"target": r"live_set tracks[name=A\]b] group_track", "name": "get_prop", "args": {"prop": "name"}})
    );
    assert_eq!(
        commands[1]["target"],
        r"live_set tracks[name=A\]b] group_track group_track"
    );
    assert_eq!(read_commands(&[]), Vec::<Value>::new());
    assert_eq!(
        read_groups(&[
            json!({"ok": true, "data": "G"}),
            json!({"ok": true, "data": "G"}),
            json!({"ok": false, "error": "None"}),
            json!({"ok": true, "data": null}),
            json!({"ok": "yes", "data": "H"}),
        ]),
        BTreeSet::from(["G".to_string()])
    );
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
        Watch::Tracks("band".into()).target(),
        ("band", "live_set".to_string(), TRACKS)
    );
    assert_eq!(
        fold("band", "G").target(),
        ("band", "live_set tracks[name=G]".to_string(), FOLD)
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
