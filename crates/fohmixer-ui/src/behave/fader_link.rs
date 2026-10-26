//! The fader link on the surface (#130, D18; the approved mockup
//! reverb-link-v2): whether a leader's touch moves its partners, the
//! marks-line marks of a leader and a partner, the detail's texts, and a linked
//! touch's start and volumes on the backing-tracks fader's same-dB drag
//! (`vca::Drag::around`; a leader at −∞ moves alone). Pure.

use fohmixer_proto::volume::value2db;

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

/// The fader mark on a strip's marks line (#151; it was the corner line).
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

/// A leader's switch (the button's capitals are the stylesheet's).
pub fn link_text(labels: &[String]) -> String {
    if labels.is_empty() {
        "LINKED".to_string()
    } else {
        format!("LINKED TO {}", labels.join(", "))
    }
}

/// A partner's note: its leader.
pub fn follow_text(leader: &str) -> String {
    format!("FOLLOWS {leader}")
}

/// A partner's note: whether its leader moves it, and where it is switched.
pub fn follow_state(on: Option<bool>, fixed: bool, leader: &str) -> String {
    if fixed {
        return "always linked".to_string();
    }
    match on {
        Some(true) => "link on".to_string(),
        Some(false) => format!("link off · switch it in {leader}'s detail"),
        None => String::new(),
    }
}

/// A linked touch's start: the own track's shown volume first, then each
/// partner that shows one (its index kept); none without the own track's
/// value or without a partner's. A leader at −∞ has no dB to move its
/// partners by (the main session's ruling on #130: a move up would throw
/// them to +6 dB, a write at the bottom give NaN): it moves alone.
pub fn link_drag(own: Option<f64>, partners: &[Option<f64>]) -> Option<(Drag, Vec<usize>)> {
    let own = own.filter(|&v| value2db(v).is_finite())?;
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
    }

    #[test]
    fn the_details_texts_name_the_partners_and_the_leader() {
        assert_eq!(link_text(&["Rev ST".to_string()]), "LINKED TO Rev ST");
        assert_eq!(
            link_text(&["#11".to_string(), "Rev ST".to_string()]),
            "LINKED TO #11, Rev ST"
        );
        assert_eq!(link_text(&[]), "LINKED");
        assert_eq!(follow_text("Rev PA"), "FOLLOWS Rev PA");
        assert_eq!(follow_state(Some(true), false, "Rev PA"), "link on");
        assert_eq!(
            follow_state(Some(false), false, "Rev PA"),
            "link off · switch it in Rev PA's detail"
        );
        assert_eq!(follow_state(None, false, "Rev PA"), "");
        assert_eq!(
            follow_state(Some(false), true, "Podklady All"),
            "always linked"
        );
    }

    #[test]
    fn a_leader_at_minus_infinity_moves_alone() {
        // Same dB is undefined from −∞ (the main session's ruling on #130):
        // a move up would throw every partner to +6 dB, a write at the
        // bottom give NaN. The leader moves alone.
        assert!(link_drag(Some(0.0), &[Some(0.6)]).is_none());
        // The lowest volume above the floor (the law's bisection step,
        // `volume_of(-70.0.next_up())`) is a level: it leads.
        let lowest = 3.0517578125e-05;
        assert!(value2db(lowest).is_finite());
        let (drag, idx) = link_drag(Some(lowest), &[Some(0.6)]).expect("a level leads");
        assert_eq!(idx, vec![0]);
        assert_eq!(link_volumes(&drag, &idx, lowest), vec![(0, 0.6)]);
    }

    #[test]
    fn a_partner_at_minus_infinity_stays_there_when_the_leader_moves() {
        let (drag, idx) = link_drag(Some(0.6), &[Some(0.0), Some(0.36)]).unwrap();
        assert_eq!(idx, vec![0, 1]);
        let up = link_volumes(&drag, &idx, 0.7);
        assert_eq!(up[0], (0, 0.0));
        assert!(up[1].1 > 0.36);
        assert_eq!(link_volumes(&drag, &idx, 0.5)[0], (0, 0.0));
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
