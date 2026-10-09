//! The Pro-Q 4 instances of a strip's track (#71 PR E, F28), pure: a walk
//! of its devices through the script's generic `get_prop`, one batch per
//! level, carried out by the router (`router/eq.rs`).
//!
//! The first read is the track's `devices` (by its strip's target: a name or
//! an index). The script encodes each device as `{"$ref", "path", "class",
//! "name"}`, its `path` in index form: a `PluginDevice` gets its
//! `class_display_name` read (a Pro-Q 4 says [`PRODUCT`] even when it is
//! renamed), a `RackDevice` its `chains`, and each chain its `devices` again,
//! [`MAX_RACKS`] racks deep at most. A slot that failed (a device deleted
//! meanwhile) is skipped; a failed first read (the track not found) fails
//! the walk with the script's error.

use fohmixer_proto::eq::{PRODUCT, place};
use serde_json::{Value, json};

/// The script's class of a plug-in device.
pub const PLUGIN: &str = "PluginDevice";
/// The script's class of a rack (an audio effect, instrument or drum rack).
pub const RACK: &str = "RackDevice";
/// A device's product name.
pub const DISPLAY_NAME: &str = "class_display_name";
/// A track's or a chain's devices.
pub const DEVICES: &str = "devices";
/// A rack's chains.
pub const CHAINS: &str = "chains";
/// Racks inside racks walked (a rack on the track is the first).
pub const MAX_RACKS: usize = 3;
/// The reads of a whole walk at most: the track's devices; per rack level
/// the chains, then their devices; then the last level's plug-ins' names.
pub const MAX_READS: usize = 2 * MAX_RACKS + 2;

/// One Pro-Q 4 found: its path (index form), where it sits and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub place: String,
    pub name: String,
}

/// One thing a read asks: its path, and the racks and chains above it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// A track's or a chain's devices.
    Devices {
        path: String,
        within: Vec<(String, String)>,
    },
    /// A plug-in's product name.
    Name {
        path: String,
        name: String,
        within: Vec<(String, String)>,
    },
    /// A rack's chains.
    Chains {
        path: String,
        rack: String,
        within: Vec<(String, String)>,
    },
}

/// What a walk does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// These commands, their answer to [`Walk::answer`].
    Read(Vec<Value>),
    /// The Pro-Q 4 instances, track level first.
    Done(Vec<Found>),
    /// The track could not be read: the script's error.
    Failed(String),
}

/// A `get_prop` of `prop` on `path`.
fn get(path: &str, prop: &str) -> Value {
    json!({"target": path, "name": "get_prop", "args": {"prop": prop}})
}

/// A slot's data when it answered.
fn answered(slot: &Value) -> Option<&Value> {
    (slot.get("ok") == Some(&Value::Bool(true)))
        .then(|| slot.get("data"))
        .flatten()
}

/// Why a slot has no answer (none: it has one).
fn refused(slot: Option<&Value>) -> Option<String> {
    let Some(slot) = slot else {
        return Some("no result".to_string());
    };
    if answered(slot).is_some() {
        return None;
    }
    Some(
        slot.get("error")
            .and_then(Value::as_str)
            .unwrap_or("no answer")
            .to_string(),
    )
}

/// The objects of a list answer: each one's class, path and name (one
/// without a path is left out).
fn items(data: &Value) -> Vec<(&str, &str, &str)> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let path = item.get("path").and_then(Value::as_str)?;
            let class = item.get("class").and_then(Value::as_str).unwrap_or("");
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            Some((class, path, name))
        })
        .collect()
}

/// A walk of one track's devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    asks: Vec<Ask>,
    found: Vec<Found>,
    /// The next answer is the track's own read.
    first: bool,
}

impl Walk {
    /// A walk of the track at `target` (its strip's LOM target) and its
    /// first read.
    pub fn start(target: &str) -> (Self, Vec<Value>) {
        let walk = Self {
            asks: vec![Ask::Devices {
                path: target.to_string(),
                within: Vec::new(),
            }],
            found: Vec::new(),
            first: true,
        };
        let commands = walk.commands();
        (walk, commands)
    }

    /// The commands of the asks waiting.
    fn commands(&self) -> Vec<Value> {
        self.asks
            .iter()
            .map(|ask| match ask {
                Ask::Devices { path, .. } => get(path, DEVICES),
                Ask::Name { path, .. } => get(path, DISPLAY_NAME),
                Ask::Chains { path, .. } => get(path, CHAINS),
            })
            .collect()
    }

    /// The answer of the last read (its slots in command order): the next
    /// read, or the end.
    pub fn answer(&mut self, slots: &[Value]) -> Step {
        if self.first
            && let Some(why) = refused(slots.first())
        {
            return Step::Failed(why);
        }
        self.first = false;
        let mut next = Vec::new();
        for (ask, slot) in std::mem::take(&mut self.asks).into_iter().zip(slots) {
            let Some(data) = answered(slot) else {
                continue;
            };
            match ask {
                Ask::Devices { within, .. } => {
                    for (class, path, name) in items(data) {
                        if class == PLUGIN {
                            next.push(Ask::Name {
                                path: path.to_string(),
                                name: name.to_string(),
                                within: within.clone(),
                            });
                        } else if class == RACK && within.len() < MAX_RACKS {
                            next.push(Ask::Chains {
                                path: path.to_string(),
                                rack: name.to_string(),
                                within: within.clone(),
                            });
                        }
                    }
                }
                Ask::Chains { rack, within, .. } => {
                    for (_, path, chain) in items(data) {
                        let mut inside = within.clone();
                        inside.push((rack.clone(), chain.to_string()));
                        next.push(Ask::Devices {
                            path: path.to_string(),
                            within: inside,
                        });
                    }
                }
                Ask::Name { path, name, within } => {
                    if data.as_str() == Some(PRODUCT) {
                        self.found.push(Found {
                            path,
                            place: place(&within),
                            name,
                        });
                    }
                }
            }
        }
        if next.is_empty() {
            return Step::Done(std::mem::take(&mut self.found));
        }
        self.asks = next;
        Step::Read(self.commands())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(data: Value) -> Value {
        json!({"ok": true, "data": data})
    }

    fn device(class: &str, path: &str, name: &str) -> Value {
        json!({"$ref": "r", "path": path, "class": class, "name": name})
    }

    fn targets(step: &Step) -> Vec<(String, String)> {
        let Step::Read(commands) = step else {
            panic!("a read: {step:?}")
        };
        commands
            .iter()
            .map(|c| {
                assert_eq!(c["name"], "get_prop");
                (
                    c["target"].as_str().unwrap().to_string(),
                    c["args"]["prop"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    const TRACK: &str = "live_set tracks[name=Hand2 #]";

    #[test]
    fn a_walk_reads_the_track_then_its_plug_ins_and_racks_down_to_their_pro_qs() {
        let (mut walk, first) = Walk::start(TRACK);
        assert_eq!(
            first,
            vec![json!({"target": TRACK, "name": "get_prop", "args": {"prop": "devices"}})]
        );
        let step = walk.answer(&[ok(json!([
            device("PluginDevice", "live_set tracks 1 devices 0", "Pro-Q 4"),
            device("Device", "live_set tracks 1 devices 1", "EQ Eight"),
            device("RackDevice", "live_set tracks 1 devices 2", "Vocal FX"),
            device("PluginDevice", "live_set tracks 1 devices 3", "Pro-C 2"),
            json!({"class": "PluginDevice", "name": "no path"}),
        ]))]);
        assert_eq!(
            targets(&step),
            vec![
                (
                    "live_set tracks 1 devices 0".into(),
                    "class_display_name".into()
                ),
                ("live_set tracks 1 devices 2".into(), "chains".into()),
                (
                    "live_set tracks 1 devices 3".into(),
                    "class_display_name".into()
                ),
            ]
        );
        let step = walk.answer(&[
            ok(json!("Pro-Q 4")),
            ok(json!([
                {"$ref": "c0", "path": "live_set tracks 1 devices 2 chains 0", "class": "Chain", "name": "Main"},
                {"$ref": "c1", "path": "live_set tracks 1 devices 2 chains 1", "class": "Chain", "name": "Air"},
            ])),
            ok(json!("Pro-C 2")),
        ]);
        assert_eq!(
            targets(&step),
            vec![
                (
                    "live_set tracks 1 devices 2 chains 0".into(),
                    "devices".into()
                ),
                (
                    "live_set tracks 1 devices 2 chains 1".into(),
                    "devices".into()
                ),
            ]
        );
        let step = walk.answer(&[
            ok(json!([device(
                "PluginDevice",
                "live_set tracks 1 devices 2 chains 0 devices 0",
                "De-ess"
            )])),
            // A failed slot (the chain deleted meanwhile) is skipped.
            json!({"ok": false, "error": "not found: chains 1", "errorType": "PathError"}),
        ]);
        assert_eq!(
            targets(&step),
            vec![(
                "live_set tracks 1 devices 2 chains 0 devices 0".into(),
                "class_display_name".into()
            )]
        );
        // A renamed Pro-Q is still one: its product name says so.
        let step = walk.answer(&[ok(json!("Pro-Q 4"))]);
        assert_eq!(
            step,
            Step::Done(vec![
                Found {
                    path: "live_set tracks 1 devices 0".into(),
                    place: "na tracku".into(),
                    name: "Pro-Q 4".into(),
                },
                Found {
                    path: "live_set tracks 1 devices 2 chains 0 devices 0".into(),
                    place: "Vocal FX › Main".into(),
                    name: "De-ess".into(),
                },
            ])
        );
    }

    #[test]
    fn a_track_without_plug_ins_is_done_at_once_and_a_missing_one_fails() {
        let (mut walk, _) = Walk::start(TRACK);
        assert_eq!(
            walk.answer(&[ok(json!([device(
                "Device",
                "live_set tracks 1 devices 0",
                "Tuner"
            )]))]),
            Step::Done(Vec::new())
        );
        let (mut walk, _) = Walk::start(TRACK);
        assert_eq!(
            walk.answer(&[json!({"ok": false, "error": "not found: tracks[name=Hand2 #]"})]),
            Step::Failed("not found: tracks[name=Hand2 #]".into())
        );
        let (mut walk, _) = Walk::start(TRACK);
        assert_eq!(
            walk.answer(&[json!({"ok": false})]),
            Step::Failed("no answer".into())
        );
        let (mut walk, _) = Walk::start(TRACK);
        assert_eq!(walk.answer(&[]), Step::Failed("no result".into()));
        // Only the track's own read fails the walk.
        let (mut walk, _) = Walk::start(TRACK);
        let step = walk.answer(&[ok(json!([device(
            "PluginDevice",
            "live_set tracks 1 devices 0",
            "Pro-Q 4"
        )]))]);
        assert!(matches!(step, Step::Read(_)));
        assert_eq!(
            walk.answer(&[json!({"ok": false, "error": "gone"})]),
            Step::Done(Vec::new())
        );
    }

    /// A list answer of one chain.
    fn chain(path: &str, name: &str) -> Value {
        ok(json!([{"path": path, "class": "Chain", "name": name}]))
    }

    #[test]
    fn racks_are_walked_three_deep_in_at_most_eight_reads() {
        assert_eq!(MAX_RACKS, 3);
        assert_eq!(MAX_READS, 8);
        let (mut walk, _) = Walk::start(TRACK);
        // 1: the track holds a rack.
        let step = walk.answer(&[ok(json!([device("RackDevice", "r1", "R1")]))]);
        assert_eq!(targets(&step), vec![("r1".into(), "chains".into())]);
        // 2: its chain.
        let step = walk.answer(&[chain("r1 chains 0", "C1")]);
        assert_eq!(
            targets(&step),
            vec![("r1 chains 0".into(), "devices".into())]
        );
        // 3: a second rack and a plug-in in it.
        let step = walk.answer(&[ok(json!([
            device("RackDevice", "r2", "R2"),
            device("PluginDevice", "q1", "Comp"),
        ]))]);
        assert_eq!(
            targets(&step),
            vec![
                ("r2".into(), "chains".into()),
                ("q1".into(), "class_display_name".into())
            ]
        );
        // 4: its chain; the plug-in is no Pro-Q.
        let step = walk.answer(&[chain("r2 chains 0", "C2"), ok(json!("Pro-C 2"))]);
        assert_eq!(
            targets(&step),
            vec![("r2 chains 0".into(), "devices".into())]
        );
        // 5: a third rack and a Pro-Q.
        let step = walk.answer(&[ok(json!([
            device("RackDevice", "r3", "R3"),
            device("PluginDevice", "q2", "Low"),
        ]))]);
        assert_eq!(
            targets(&step),
            vec![
                ("r3".into(), "chains".into()),
                ("q2".into(), "class_display_name".into())
            ]
        );
        // 6: its chain.
        let step = walk.answer(&[chain("r3 chains 0", "C3"), ok(json!("Pro-Q 4"))]);
        assert_eq!(
            targets(&step),
            vec![("r3 chains 0".into(), "devices".into())]
        );
        // 7: a fourth rack is not walked; the Pro-Q beside it is asked.
        let step = walk.answer(&[ok(json!([
            device("RackDevice", "r4", "R4"),
            device("PluginDevice", "q3", "High"),
        ]))]);
        assert_eq!(
            targets(&step),
            vec![("q3".into(), "class_display_name".into())]
        );
        // 8: the end.
        assert_eq!(
            walk.answer(&[ok(json!("Pro-Q 4"))]),
            Step::Done(vec![
                Found {
                    path: "q2".into(),
                    place: "R1 › C1 › R2 › C2".into(),
                    name: "Low".into(),
                },
                Found {
                    path: "q3".into(),
                    place: "R1 › C1 › R2 › C2 › R3 › C3".into(),
                    name: "High".into(),
                },
            ])
        );
    }
}
