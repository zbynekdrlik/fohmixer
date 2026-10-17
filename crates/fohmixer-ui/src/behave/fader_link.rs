//! The fader link on the surface (#130, D18; the approved mockup
//! reverb-link-v2): whether a leader's touch moves its partners, the corner
//! marks of a leader and a partner, the detail's texts, and a linked
//! touch's start and volumes on the backing-tracks fader's same-dB drag
//! (`vca::Drag::around`). Pure.

use super::vca::Drag;

/// Whether a leader's fader moves its partners: the hub's switch is on
/// (`None` before its value) and it has partners.
pub fn linked(on: Option<bool>, partners: usize) -> bool {
    on == Some(true) && partners > 0
}

/// Whether a partner's leaders move it now: each `(its switch, fixed)`, a
/// frame's linked fader (Podklady All) always.
pub fn follows_on(follows: &[(Option<bool>, bool)]) -> bool {
    follows.iter().any(|&(on, fixed)| fixed || on == Some(true))
}

/// The fader mark in a strip's corner line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Flink {
    #[default]
    Off,
    /// A leader whose link is on: the chain with `dB`, filled.
    Lead,
    /// A partner a leader moves: the same chain, outlined (Q1).
    Follow,
}

impl Flink {
    /// The strip's `data-flink` and the mark's `data-role`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Lead => "lead",
            Self::Follow => "follow",
        }
    }

    /// Whether the corner shows a mark.
    pub fn shown(self) -> bool {
        self != Self::Off
    }

    /// Whether the mark is a partner's (outlined).
    pub fn follows(self) -> bool {
        self == Self::Follow
    }
}

/// A strip's mark: a leader's while its link moves others (it matters more
/// on a drag), else a partner's while a leader moves it.
pub fn flink_of(lead_on: bool, follow_on: bool) -> Flink {
    if lead_on {
        Flink::Lead
    } else if follow_on {
        Flink::Follow
    } else {
        Flink::Off
    }
}

/// The mark draws the chain alone beside a problem's `!` and a mute chain
/// (v1's crowded case): it never yields.
pub fn flink_compact(flag: bool, mute_chain: bool) -> bool {
    flag && mute_chain
}

/// A leader's switch (the button's capitals are the stylesheet's).
pub fn link_text(labels: &[String]) -> String {
    if labels.is_empty() {
        "SPOJENÝ".to_string()
    } else {
        format!("SPOJENÝ S {}", labels.join(", "))
    }
}

/// A partner's note: its leader.
pub fn follow_text(leader: &str) -> String {
    format!("IDE S {leader}")
}

/// A partner's note: whether its leader moves it, and where it is switched.
pub fn follow_state(on: Option<bool>, fixed: bool, leader: &str) -> String {
    if fixed {
        return "vždy spojené".to_string();
    }
    match on {
        Some(true) => "spojenie zapnuté".to_string(),
        Some(false) => format!("spojenie vypnuté · zapína sa v detaile {leader}"),
        None => String::new(),
    }
}

/// A linked touch's start: the own track's shown volume first, then each
/// partner that shows one (its index kept); none without the own track's
/// value or without a partner's.
pub fn link_drag(own: Option<f64>, partners: &[Option<f64>]) -> Option<(Drag, Vec<usize>)> {
    let own = own?;
    let (idx, starts): (Vec<usize>, Vec<f64>) = partners
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .unzip();
    if idx.is_empty() {
        return None;
    }
    let mut start = vec![own];
    start.extend(starts);
    Some((Drag::around(start, own), idx))
}

/// Each moved partner's volume with the own track at `own`: (its index, its
/// volume).
pub fn link_volumes(drag: &Drag, idx: &[usize], own: f64) -> Vec<(usize, f64)> {
    idx.iter()
        .copied()
        .zip(drag.volumes(own).into_iter().skip(1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::volume::value2db;

    #[test]
    fn a_leader_moves_its_partners_while_its_switch_is_on() {
        assert!(linked(Some(true), 1));
        assert!(!linked(Some(false), 1));
        assert!(!linked(None, 1), "before the hub's value");
        assert!(!linked(Some(true), 0));
    }

    #[test]
    fn a_partner_is_moved_while_any_leader_is_on_or_fixed() {
        assert!(follows_on(&[(Some(true), false)]));
        assert!(
            follows_on(&[(None, true)]),
            "the frame's linked fader is always on"
        );
        assert!(follows_on(&[(Some(false), false), (Some(true), false)]));
        assert!(!follows_on(&[(Some(false), false), (None, false)]));
        assert!(!follows_on(&[]));
    }

    #[test]
    fn the_corner_shows_the_leaders_mark_before_the_partners() {
        assert_eq!(flink_of(true, true), Flink::Lead);
        assert_eq!(flink_of(true, false), Flink::Lead);
        assert_eq!(flink_of(false, true), Flink::Follow);
        assert_eq!(flink_of(false, false), Flink::Off);
        assert_eq!(
            [Flink::Off, Flink::Lead, Flink::Follow].map(Flink::name),
            ["", "lead", "follow"]
        );
        assert_eq!(
            [Flink::Off, Flink::Lead, Flink::Follow].map(Flink::shown),
            [false, true, true]
        );
        assert_eq!(
            [Flink::Off, Flink::Lead, Flink::Follow].map(Flink::follows),
            [false, false, true]
        );
        assert_eq!(Flink::default(), Flink::Off);
        assert!(flink_compact(true, true));
        assert!(!flink_compact(true, false));
        assert!(!flink_compact(false, true));
    }

    #[test]
    fn the_details_texts_name_the_partners_and_the_leader() {
        assert_eq!(link_text(&["Rev ST".to_string()]), "SPOJENÝ S Rev ST");
        assert_eq!(
            link_text(&["#11".to_string(), "Rev ST".to_string()]),
            "SPOJENÝ S #11, Rev ST"
        );
        assert_eq!(link_text(&[]), "SPOJENÝ");
        assert_eq!(follow_text("Rev PA"), "IDE S Rev PA");
        assert_eq!(
            follow_state(Some(true), false, "Rev PA"),
            "spojenie zapnuté"
        );
        assert_eq!(
            follow_state(Some(false), false, "Rev PA"),
            "spojenie vypnuté · zapína sa v detaile Rev PA"
        );
        assert_eq!(follow_state(None, false, "Rev PA"), "");
        assert_eq!(
            follow_state(Some(false), true, "Podklady All"),
            "vždy spojené"
        );
    }

    #[test]
    fn a_linked_touch_starts_from_the_own_and_every_partner_with_a_value() {
        assert!(
            link_drag(None, &[Some(0.6)]).is_none(),
            "no own value: no touch"
        );
        assert!(
            link_drag(Some(0.85), &[None]).is_none(),
            "no partner value: unlinked"
        );
        let (drag, idx) = link_drag(Some(0.85), &[None, Some(0.6)]).unwrap();
        assert_eq!(idx, vec![1]);
        assert_eq!(
            link_volumes(&drag, &idx, 0.85),
            vec![(1, 0.6)],
            "no move: the start exactly"
        );
        let moved = link_volumes(&drag, &idx, 0.7);
        assert_eq!(moved[0].0, 1);
        assert!(
            ((value2db(moved[0].1) - value2db(0.6)) - (value2db(0.7) - value2db(0.85))).abs()
                < 0.01
        );
    }
}
