//! REVERB (#130, spec D18, F30): the reverb faders (the `+REVERB` marker
//! strips, `Layout::reverb_faders`) and, from PR 4, the hub's REVERB AUT
//! rule. Pure: the router hands it each layout's faders and carries out
//! what it returns ([`Act`]); every step is an event-log record (`reverb`)
//! with one hub log line ([`step_line`]).

use fohmixer_proto::client::{Low, ReverbFader, ReverbState, ReverbStatus, set_key};
use fohmixer_proto::layout::RuleFader;
use fohmixer_proto::volume::{FLOOR_DB, value2db, volume_of};
use serde::{Deserialize, Serialize};

/// How far REVERB lowers a fader (dB).
pub const DROP_DB: f64 = 10.0;
/// After a stop, how long REVERB AUT waits (the last chord's tail), ms.
pub const STOP_WAIT_MS: f64 = 2000.0;
/// The stop's lowering glide, ms.
pub const LOWER_MS: f64 = 2000.0;
/// A tap's lowering glide, ms.
pub const TAP_MS: f64 = 500.0;
/// The return glide (a start or a tap), ms.
pub const RETURN_MS: f64 = 500.0;
/// A glide's write period, ms (40 a second per fader; a page sends 60).
pub const STEP_MS: f64 = 25.0;
/// Two volumes this close are one level (Live keeps a float32).
pub const SAME_VOLUME: f64 = 1e-6;

/// Whether two volumes are one level.
pub fn same_volume(a: f64, b: f64) -> bool {
    (a - b).abs() <= SAME_VOLUME
}

/// The volume [`DROP_DB`] under `v` on the strips' law (−∞ stays).
pub fn lowered(v: f64) -> f64 {
    volume_of(value2db(v) - DROP_DB)
}

/// Whether a time `at` has come at `now`.
pub fn due(at: f64, now: f64) -> bool {
    now >= at
}

/// Whether a volume is −∞ (nothing to lower).
pub fn silent(v: f64) -> bool {
    v <= 0.0
}

/// A glide of one fader, linear in dB (each end at the −70 dB floor at
/// least), its end exactly `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    from_db: f64,
    to_db: f64,
    to: f64,
    t0: f64,
    ms: f64,
}

impl Glide {
    pub fn new(from: f64, to: f64, t0: f64, ms: f64) -> Self {
        Self {
            from_db: value2db(from).max(FLOOR_DB),
            to_db: value2db(to).max(FLOOR_DB),
            to,
            t0,
            ms,
        }
    }

    /// Its volume at `now` and whether it arrived.
    pub fn at(&self, now: f64) -> (f64, bool) {
        if due(self.t0 + self.ms, now) {
            return (self.to, true);
        }
        let p = ((now - self.t0) / self.ms).max(0.0);
        (
            volume_of(self.from_db + (self.to_db - self.from_db) * p),
            false,
        )
    }
}

/// What `hub-state.json` keeps of REVERB (flattened into `HubState`; the
/// defaults write no key, so a file of STAGE AUT alone stays as it was).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(
        default,
        rename = "reverb_aut",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub aut: bool,
    /// The faders the rule holds lowered, and why (PR 4).
    #[serde(
        default,
        rename = "reverb_low",
        skip_serializing_if = "Option::is_none"
    )]
    pub low: Option<SavedLow>,
}

/// The lowered state across a hub restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLow {
    pub by: Low,
    pub faders: Vec<SavedFader>,
}

/// A held fader across a restart: who (its set and label), where it
/// returns, where the rule left it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedFader {
    pub instance: String,
    pub label: String,
    pub back: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<f64>,
}

/// A step of REVERB: the `reverb` record's fields (`step` and its own).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// A layout named these faders.
    Faders { labels: Vec<String> },
}

/// What the router carries out.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// A record and its log line.
    Step(Step),
    /// Every page gets the new state.
    Tell,
}

/// One reverb fader.
#[derive(Debug, Clone, PartialEq)]
struct Fader {
    spec: RuleFader,
    /// Its volume's write key (a page's `set` of it has this key).
    key: String,
}

impl Fader {
    fn of(spec: RuleFader) -> Self {
        let key = set_key(&spec.instance, &spec.target, "value");
        Self { spec, key }
    }
}

/// REVERB's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Reverb {
    aut: bool,
    faders: Vec<Fader>,
}

impl Reverb {
    /// From `hub-state.json`.
    pub fn new(saved: &Saved) -> Self {
        Self {
            aut: saved.aut,
            faders: Vec::new(),
        }
    }

    fn labels(&self) -> Vec<String> {
        self.faders.iter().map(|f| f.spec.label.clone()).collect()
    }

    /// A served layout's faders.
    pub fn set_faders(&mut self, specs: Vec<RuleFader>) -> Vec<Act> {
        if self.faders.iter().map(|f| &f.spec).eq(specs.iter()) {
            return Vec::new();
        }
        self.faders = specs.into_iter().map(Fader::of).collect();
        vec![
            Act::Step(Step::Faders {
                labels: self.labels(),
            }),
            Act::Tell,
        ]
    }

    /// What every page is told.
    pub fn state(&self) -> ReverbState {
        ReverbState {
            aut: self.aut,
            low: None,
            playing: None,
            waiting: false,
            faders: self
                .faders
                .iter()
                .map(|f| ReverbFader {
                    key: f.key.clone(),
                    label: f.spec.label.clone(),
                    back: None,
                    hand: false,
                })
                .collect(),
        }
    }

    /// `/api/status` `reverb`.
    pub fn status(&self) -> ReverbStatus {
        ReverbStatus {
            aut: self.aut,
            low: None,
            faders: self.faders.len(),
            writes: 0,
            hands: 0,
        }
    }
}

/// A step's hub log line.
pub fn step_line(step: &Step) -> String {
    match step {
        Step::Faders { labels } if labels.is_empty() => {
            "REVERB has no faders (no +REVERB strip)".to_string()
        }
        Step::Faders { labels } => format!("REVERB's faders: {}", labels.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fader(label: &str, index: u32) -> RuleFader {
        RuleFader {
            instance: "band".into(),
            label: label.into(),
            target: format!("live_set tracks {index} mixer_device volume"),
        }
    }

    #[test]
    fn the_faders_come_from_the_layout_each_with_its_write_key() {
        let mut rule = Reverb::new(&Saved::default());
        assert_eq!(
            rule.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]),
            vec![
                Act::Step(Step::Faders {
                    labels: vec!["Rev ST".into(), "Rev PA".into()]
                }),
                Act::Tell,
            ]
        );
        assert_eq!(
            rule.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]),
            vec![],
            "the same faders: nothing"
        );
        let state = rule.state();
        assert_eq!(
            state
                .faders
                .iter()
                .map(|f| (f.key.as_str(), f.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("band|live_set tracks 4 mixer_device volume|value", "Rev ST"),
                ("band|live_set tracks 5 mixer_device volume|value", "Rev PA"),
            ]
        );
        assert!(!state.aut && state.low.is_none() && state.playing.is_none() && !state.waiting);
        assert_eq!(rule.status().faders, 2);
        // A layout without them: told, and the line says so.
        assert_eq!(
            rule.set_faders(vec![]),
            vec![Act::Step(Step::Faders { labels: vec![] }), Act::Tell]
        );
        assert_eq!(rule.status().faders, 0);
    }

    #[test]
    fn each_step_has_its_line() {
        assert_eq!(
            step_line(&Step::Faders {
                labels: vec!["Rev ST".into(), "Rev PA".into()]
            }),
            "REVERB's faders: Rev ST, Rev PA"
        );
        assert_eq!(
            step_line(&Step::Faders { labels: vec![] }),
            "REVERB has no faders (no +REVERB strip)"
        );
        assert_eq!(
            serde_json::to_value(Step::Faders {
                labels: vec!["A".into()]
            })
            .unwrap(),
            serde_json::json!({"step": "faders", "labels": ["A"]})
        );
        assert!(
            Reverb::new(&Saved {
                aut: true,
                low: None
            })
            .state()
            .aut
        );
        assert!(
            Reverb::new(&Saved {
                aut: true,
                low: None
            })
            .status()
            .aut
        );
    }

    #[test]
    fn the_rules_numbers() {
        assert_eq!(
            (
                DROP_DB,
                STOP_WAIT_MS,
                LOWER_MS,
                TAP_MS,
                RETURN_MS,
                STEP_MS,
                SAME_VOLUME
            ),
            (10.0, 2000.0, 2000.0, 500.0, 500.0, 25.0, 0.000_001)
        );
    }

    #[test]
    fn the_thresholds_are_exact() {
        assert!(same_volume(0.0, 1e-6));
        assert!(!same_volume(0.0, 1e-6_f64.next_up()));
        assert!(same_volume(1e-6, 0.0));
        assert!(same_volume(0.5, 0.5), "a level is its own");
        assert!(due(1000.0, 1000.0));
        assert!(!due(1000.0, 1000.0_f64.next_down()));
        assert!(due(1000.0, 1000.0_f64.next_up()));
        assert!(silent(0.0));
        assert!(!silent(f64::MIN_POSITIVE));
        assert!((value2db(lowered(0.85)) + 10.0).abs() < 0.01);
        assert!((value2db(lowered(0.6)) - (value2db(0.6) - 10.0)).abs() < 0.01);
        assert_eq!(lowered(0.0), 0.0);
    }

    #[test]
    fn a_glide_is_linear_in_db_and_lands_exactly() {
        let g = Glide::new(0.85, 0.6, 1000.0, 2000.0);
        assert_eq!(g.at(1000.0), (volume_of(value2db(0.85)), false));
        let (half, done) = g.at(2000.0);
        assert!(!done);
        assert!((value2db(half) - (value2db(0.85) + value2db(0.6)) / 2.0).abs() < 0.01);
        assert!(!g.at(3000.0_f64.next_down()).1);
        assert_eq!(g.at(3000.0), (0.6, true), "exactly its end");
        assert_eq!(g.at(9000.0), (0.6, true));
        // From −∞ (the floor) up: no NaN, the end exact.
        let up = Glide::new(0.0, 0.3, 0.0, 500.0);
        assert!(up.at(250.0).0.is_finite() && up.at(250.0).0 > 0.0);
        assert_eq!(up.at(500.0), (0.3, true));
    }
}
