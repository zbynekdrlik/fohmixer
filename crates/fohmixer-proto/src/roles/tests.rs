use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::sectors::{SectorId, Slot};

fn strip(instance: &str, label: &str) -> serde_json::Value {
    json!({"kind": "strip", "strip_kind": "standard", "label": label,
           "binding": {"instance": instance, "anchor": {"kind": "track_at", "index": 0}}})
}

fn alert(instance: &str) -> serde_json::Value {
    json!({"kind": "alert", "period_ms": 500,
           "binding": {"instance": instance, "anchor": {"kind": "track", "name": "Alert"}}})
}

fn layout(value: serde_json::Value) -> Layout {
    serde_json::from_value(value).expect("a test layout")
}

/// A FOH layout: a band page with a rail (a master stage) and a band strip,
/// an MBC view page, the REVERB button, and a band alert in `global`.
fn mixed() -> Layout {
    layout(json!({
        "schema": 2,
        "default_page": "band",
        "pages": [
            {"id": "band", "title": "BAND",
             "rail": [{"kind": "stage", "binding": {"instance": "master", "anchor": {"kind": "track", "name": "Stage"}}},
                      {"kind": "reverb"}],
             "rows": [{"sections": [{"kind": "group", "title": "B", "controls": [strip("band", "Vox 1")]}]}]},
            {"id": "view-mbc", "title": "MBC", "view": true,
             "rows": [{"sections": [{"kind": "group", "controls": [strip("mbc", "Hand 1"), strip("band", "Klavir #")]}]}]}
        ],
        "global": [alert("band")],
        "report": {"dropped": ["a track name"]}
    }))
}

#[test]
fn the_engineer_reaches_every_instance_and_mbc_only_its_own() {
    for instance in ["band", "master", "mbc", ""] {
        assert!(Role::Engineer.reaches(instance), "{instance}");
    }
    assert!(Role::Mbc.reaches("mbc"));
    for instance in ["band", "master", "MBC", "mbc2", ""] {
        assert!(!Role::Mbc.reaches(instance), "{instance}");
    }
    let mbc = StripRef {
        instance: "mbc".into(),
        label: "Hand 1".into(),
    };
    let band = StripRef {
        instance: "band".into(),
        label: "Vox 1".into(),
    };
    assert!(reaches_strip(Role::Mbc, &mbc));
    assert!(!reaches_strip(Role::Mbc, &band));
    assert!(reaches_strip(Role::Engineer, &band));
}

#[test]
fn a_token_subject_names_a_role() {
    assert_eq!(Role::from_sub("engineer"), Some(Role::Engineer));
    assert_eq!(Role::from_sub("mbc"), Some(Role::Mbc));
    for sub in ["hub", "", "Engineer", "MBC"] {
        assert_eq!(Role::from_sub(sub), None, "{sub}");
    }
    for role in [Role::Engineer, Role::Mbc] {
        assert_eq!(Role::from_sub(role.sub()), Some(role));
        assert_eq!(role.as_str(), role.sub());
    }
    assert_eq!(Role::Engineer.sub(), "engineer");
    assert_eq!(Role::Mbc.sub(), "mbc");
    assert_eq!(serde_json::to_value(Role::Mbc).unwrap(), json!("mbc"));
    assert_eq!(
        serde_json::from_value::<Role>(json!("engineer")).unwrap(),
        Role::Engineer
    );
}

#[test]
fn the_engineer_layout_is_the_whole_layout() {
    let layout = mixed();
    assert_eq!(layout_for(Role::Engineer, &layout), layout);
}

#[test]
fn the_mbc_layout_keeps_only_mbc_controls() {
    let got = layout_for(Role::Mbc, &mixed());
    let want = layout(json!({
        "schema": 2,
        "default_page": "view-mbc",
        "pages": [
            {"id": "view-mbc", "title": "MBC", "view": true,
             "rows": [{"sections": [{"kind": "group", "controls": [strip("mbc", "Hand 1")]}]}]}
        ],
        "report": null
    }));
    assert_eq!(got, want);
}

#[test]
fn an_mbc_layout_keeps_its_default_page_and_mbc_rails_pagers_and_globals() {
    let got = layout_for(
        Role::Mbc,
        &layout(json!({
            "schema": 2,
            "default_page": "p2",
            "pages": [
                {"id": "p1", "title": "ONE", "rail": [alert("mbc")]},
                {"id": "p2", "title": "TWO", "rows": [
                    {"sections": [{"kind": "pager", "id": "pg", "default_page": "s1", "pages": [
                        {"id": "s1", "title": "S1", "sections": [{"kind": "group", "controls": [strip("band", "Vox 1")]}]},
                        {"id": "s2", "title": "S2", "sections": [{"kind": "group", "controls": [strip("mbc", "Hand 1")]}]}
                    ]}], "weight": 2.0},
                    {"sections": [{"kind": "group", "controls": [strip("band", "Vox 2")]}]}
                ]}
            ],
            "global": [alert("mbc"), alert("band"), {"kind": "autotune"}]
        })),
    );
    let want = layout(json!({
        "schema": 2,
        "default_page": "p2",
        "pages": [
            {"id": "p1", "title": "ONE", "rail": [alert("mbc")]},
            {"id": "p2", "title": "TWO", "rows": [
                {"sections": [{"kind": "pager", "id": "pg", "default_page": "s2", "pages": [
                    {"id": "s2", "title": "S2", "sections": [{"kind": "group", "controls": [strip("mbc", "Hand 1")]}]}
                ]}], "weight": 2.0}
            ]}
        ],
        "global": [alert("mbc")],
        "report": null
    }));
    assert_eq!(got, want);
}

#[test]
fn a_pager_keeps_its_default_sub_page_when_it_is_left() {
    let got = layout_for(
        Role::Mbc,
        &layout(json!({
            "schema": 2,
            "default_page": "p",
            "pages": [{"id": "p", "title": "P", "rows": [
                {"sections": [{"kind": "pager", "id": "pg", "default_page": "s2", "pages": [
                    {"id": "s1", "title": "S1", "sections": [{"kind": "group", "controls": [strip("mbc", "Hand 1")]}]},
                    {"id": "s2", "title": "S2", "sections": [{"kind": "group", "controls": [strip("mbc", "Hand 2")]}]}
                ]}]}
            ]}]
        })),
    );
    let Section::Pager(pager) = &got.pages[0].rows[0].sections[0] else {
        panic!("a pager");
    };
    assert_eq!(pager.default_page, "s2");
    assert_eq!(pager.pages.len(), 2);
}

#[test]
fn parameter_controls_belong_to_their_link_or_first_target() {
    let target = |instance: &str| {
        json!({"binding": {"instance": instance, "anchor": {"kind": "track", "name": "X"}},
               "prop": "mixer_device volume value"})
    };
    let got = layout_for(
        Role::Mbc,
        &layout(json!({
            "schema": 2,
            "default_page": "p",
            "pages": [{"id": "p", "title": "P", "rail": [
                {"kind": "param_toggle", "label": "T1", "press": "toggle", "targets": [target("mbc"), target("band")]},
                {"kind": "param_toggle", "label": "T2", "press": "toggle", "targets": [target("band"), target("mbc")]},
                {"kind": "param_toggle", "label": "T3", "press": "toggle", "targets": []},
                {"kind": "param_fader", "label": "F1", "targets": [target("mbc")]},
                {"kind": "param_fader", "label": "F2", "link": {"instance": "band", "id": "x"}, "targets": [target("mbc")]},
                {"kind": "param_fader", "label": "F3", "link": {"instance": "mbc", "id": "y"}},
                {"kind": "solo", "binding": {"instance": "mbc", "anchor": {"kind": "track", "name": "G"}}},
                {"kind": "solo", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "G"}}}
            ]}]
        })),
    );
    let labels: Vec<String> = got.pages[0]
        .rail
        .iter()
        .map(|c| match c {
            Control::ParamToggle { label, .. } | Control::ParamFader { label, .. } => label.clone(),
            Control::Solo { binding, .. } => format!("solo {}", binding.instance),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(labels, ["T1", "F1", "F3", "solo mbc"]);
}

#[test]
fn an_mbc_layout_with_nothing_left_has_one_empty_page() {
    let got = layout_for(
        Role::Mbc,
        &layout(json!({
            "schema": 2,
            "default_page": "band",
            "pages": [{"id": "band", "title": "BAND", "rail": [alert("band"), {"kind": "reverb"}],
                       "rows": [{"sections": [{"kind": "group", "controls": [strip("band", "Vox 1")]}]}]}],
            "config": {"fader_shaping": true}
        })),
    );
    let want = layout(json!({
        "schema": 2,
        "default_page": "mbc",
        "pages": [{"id": "mbc", "title": "MBC"}],
        "config": {"fader_shaping": true},
        "report": null
    }));
    assert_eq!(got, want);
}

fn r(instance: &str, label: &str) -> Slot {
    Some(StripRef {
        instance: instance.into(),
        label: label.into(),
    })
}

fn sl(id: &str, strips: Vec<Slot>) -> SectorLayout {
    SectorLayout {
        id: id.into(),
        title: id.to_uppercase(),
        strips,
    }
}

fn doc(sectors: Vec<(SectorId, Vec<SectorLayout>)>) -> SectorsDoc {
    SectorsDoc {
        rev: 7,
        sectors: sectors.into_iter().collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn the_mbc_sectors_keep_only_mbc_faders() {
    let given = doc(vec![
        (
            SectorId::Lu,
            vec![sl(
                "lu1",
                vec![
                    r("band", "A"),
                    r("mbc", "X"),
                    None,
                    r("mbc", "Y"),
                    r("band", "B"),
                ],
            )],
        ),
        (SectorId::Ld, vec![sl("ld1", vec![r("band", "A")])]),
        (
            SectorId::Ru,
            vec![
                sl("ru1", vec![r("master", "M")]),
                sl("ru2", vec![]),
                sl("ru3", vec![r("mbc", "Z")]),
            ],
        ),
        (
            SectorId::Rd,
            vec![
                sl("rd1", vec![r("band", "A")]),
                sl("rd2", vec![r("band", "C")]),
            ],
        ),
    ]);
    let want = doc(vec![
        (
            SectorId::Lu,
            vec![sl("lu1", vec![None, r("mbc", "X"), None, r("mbc", "Y")])],
        ),
        (SectorId::Ld, vec![sl("ld1", vec![])]),
        (
            SectorId::Ru,
            vec![sl("ru2", vec![]), sl("ru3", vec![r("mbc", "Z")])],
        ),
        (SectorId::Rd, vec![sl("rd1", vec![])]),
    ]);
    assert_eq!(sectors_for(Role::Mbc, &given), want);
    assert_eq!(sectors_for(Role::Engineer, &given), given);
    assert_eq!(
        sectors_for(Role::Mbc, &SectorsDoc::empty()),
        SectorsDoc::empty()
    );
}

#[test]
fn mbc_may_edit_only_a_document_of_its_own() {
    let own = doc(vec![
        (
            SectorId::Lu,
            vec![sl("lu1", vec![r("mbc", "X"), None, r("mbc", "Y")])],
        ),
        (SectorId::Ld, vec![sl("ld1", vec![]), sl("ld2", vec![])]),
    ]);
    assert!(may_edit_sectors(Role::Mbc, &own));
    assert!(may_edit_sectors(Role::Mbc, &SectorsDoc::empty()));
    let mixed = doc(vec![(
        SectorId::Lu,
        vec![sl("lu1", vec![r("mbc", "X"), r("band", "A")])],
    )]);
    assert!(!may_edit_sectors(Role::Mbc, &mixed));
    assert!(may_edit_sectors(Role::Engineer, &mixed));
    assert!(may_edit_sectors(Role::Engineer, &own));
}

#[test]
fn mbc_may_arrange_only_on_a_hub_of_its_own_faders() {
    let band = StripRef {
        instance: "band".into(),
        label: "A".into(),
    };
    let mbc = StripRef {
        instance: "mbc".into(),
        label: "X".into(),
    };
    let empty = SectorsDoc::empty();
    // The MBC hub: its pool and its document are mbc's.
    assert!(may_arrange(Role::Mbc, &empty, [&mbc]));
    assert!(may_arrange(Role::Mbc, &empty, std::iter::empty()));
    // The FOH hub: a band fader in the pool, even with an empty document.
    assert!(!may_arrange(Role::Mbc, &empty, [&mbc, &band]));
    assert!(!may_arrange(Role::Mbc, &empty, [&band]));
    // A document of another set's fader, whatever the pool.
    let mixed = doc(vec![(SectorId::Lu, vec![sl("lu1", vec![r("band", "A")])])]);
    assert!(!may_arrange(Role::Mbc, &mixed, [&mbc]));
    // The engineer everywhere.
    assert!(may_arrange(Role::Engineer, &mixed, [&mbc, &band]));
    assert!(may_arrange(Role::Engineer, &empty, [&band]));
}

#[test]
fn an_mbc_layout_keeps_an_mbc_readout_and_drops_a_band_one() {
    let readout = |instance: &str| {
        json!({"kind": "lufs_readout", "label": "STREAM LUFS", "target": -14,
               "binding": {"instance": instance, "anchor": {"kind": "ctrl", "id": "stream"}}})
    };
    let mut whole = mixed();
    whole.global.push(layout_control(readout("mbc")));
    whole.global.push(layout_control(readout("band")));
    let got = layout_for(Role::Mbc, &whole);
    assert_eq!(got.global, vec![layout_control(readout("mbc"))]);
    assert_eq!(layout_for(Role::Engineer, &whole), whole);
}

fn layout_control(value: serde_json::Value) -> Control {
    serde_json::from_value(value).expect("a test control")
}
