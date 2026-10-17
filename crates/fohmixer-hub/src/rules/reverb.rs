//! REVERB (#130, spec D18, F30): the reverb faders (the `+REVERB` marker
//! strips, `Layout::reverb_faders`) and, from PR 4, the hub's REVERB AUT
//! rule. Pure: the router hands it each layout's faders and carries out
//! what it returns ([`Act`]); every step is an event-log record (`reverb`)
//! with one hub log line ([`step_line`]).

use fohmixer_proto::client::{Low, ReverbFader, ReverbState, ReverbStatus, set_key};
use fohmixer_proto::layout::RuleFader;
use serde::{Deserialize, Serialize};

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
}
