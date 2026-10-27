//! The confirming walk of a key in one Live (#151, spec §4.4; the main
//! session's ruling 3 of 27.10), pure: the hub reads the candidate strip's
//! track through that Live's script and counts the Max devices named as
//! the key's kind (`FohMixer LUFS` / `FohMixer Meter`, `class_name`
//! [`MAX_CLASS`]) at the key's place: on the track itself for a key whose
//! parents are `[track, set, -, -]`, inside the chain named `parents[0]` of
//! the rack named `parents[1]` on the track for `[chain, rack, track,
//! set]`. Exactly one is [`Answer::Ours`], none [`Answer::NotOurs`], more
//! [`Answer::Two`]; a first read Live refused or did not answer is
//! [`Answer::Failed`]. No path or index is taken from the device: the walk
//! is the hub's own, as AUTOTUNE's find (`rules/autotune/find.rs`).
//!
//! The reads, one batch each: the track's `devices`; the matching rack's
//! `chains` and the class of each device of the name on the track; the
//! matching chain's `devices`; their class. [`MAX_READS`] at most.

use fohmixer_proto::lufs::wire::{DeviceKind, NO_PARENT, PARENTS};
use serde_json::Value;

use crate::eq::walk::{CHAINS, DEVICES, RACK, answered, get, items, refused};

/// The LOM's `class_name` of a Max audio effect (read on the PC, #151 P1).
pub const MAX_CLASS: &str = "MxDeviceAudioEffect";
/// The LUFS device's name.
pub const LUFS_NAME: &str = "FohMixer LUFS";
/// The meter device's name.
pub const METER_NAME: &str = "FohMixer Meter";
/// A device's class property.
pub const CLASS_NAME: &str = "class_name";
/// The batches of a walk at most.
pub const MAX_READS: usize = 4;

/// The device name of a kind.
pub fn device_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Lufs => LUFS_NAME,
        DeviceKind::Meter => METER_NAME,
    }
}

/// Where a key's device sits, from its parents' names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// On the track (`[track, set, -, -]`).
    Track,
    /// In a rack's chain on the track (`[chain, rack, track, set]`).
    Chain { rack: String, chain: String },
}

/// The place a key's parents name: in a chain when four are named, on the
/// track otherwise.
pub fn place_of(parents: &[String; PARENTS]) -> Place {
    if parents[2] == NO_PARENT {
        Place::Track
    } else {
        Place::Chain {
            rack: parents[1].clone(),
            chain: parents[0].clone(),
        }
    }
}

/// A walk's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Exactly one device of the kind at the key's place.
    Ours,
    /// None.
    NotOurs,
    /// Two or more.
    Two,
    /// The track could not be read (Live's words).
    Failed(String),
}

impl Answer {
    /// Its word for the status and the log.
    pub fn word(&self) -> &'static str {
        match self {
            Answer::Ours => "ours",
            Answer::NotOurs => "not_ours",
            Answer::Two => "two",
            Answer::Failed(_) => "failed",
        }
    }
}

/// What a walk does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// These commands; their slots to [`Confirm::answer`].
    Read(Vec<Value>),
    /// The answer.
    Done(Answer),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// A track's (none) or the matching chain's (`true`) devices.
    Devices { path: String, in_chain: bool },
    /// The matching rack's chains.
    Chains { path: String },
    /// A device's class.
    Class { path: String },
}

/// One walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    name: &'static str,
    place: Place,
    asks: Vec<Ask>,
    found: usize,
    first: bool,
}

impl Confirm {
    /// A walk for a key of `kind` at `parents`, of the track at `track`
    /// (the candidate strip's target), and its first read.
    pub fn start(kind: DeviceKind, parents: &[String; PARENTS], track: &str) -> (Self, Vec<Value>) {
        let walk = Self {
            name: device_name(kind),
            place: place_of(parents),
            asks: vec![Ask::Devices {
                path: track.to_string(),
                in_chain: false,
            }],
            found: 0,
            first: true,
        };
        let commands = walk.commands();
        (walk, commands)
    }

    fn commands(&self) -> Vec<Value> {
        self.asks
            .iter()
            .map(|ask| match ask {
                Ask::Devices { path, .. } => get(path, DEVICES),
                Ask::Chains { path } => get(path, CHAINS),
                Ask::Class { path } => get(path, CLASS_NAME),
            })
            .collect()
    }

    /// Whether a device list at this level holds the key's devices.
    fn at_place(&self, in_chain: bool) -> bool {
        in_chain == matches!(self.place, Place::Chain { .. })
    }

    /// The answer of the last read (its slots in command order): the next
    /// read, or the answer. A slot refused after the first read (a device
    /// deleted meanwhile) counts nothing.
    pub fn answer(&mut self, slots: &[Value]) -> Step {
        if self.first
            && let Some(why) = refused(slots.first())
        {
            return Step::Done(Answer::Failed(why));
        }
        self.first = false;
        let mut next = Vec::new();
        for (ask, slot) in std::mem::take(&mut self.asks).into_iter().zip(slots) {
            let Some(data) = answered(slot) else {
                continue;
            };
            match ask {
                Ask::Devices { in_chain, .. } => {
                    for item in items(data) {
                        if item.name == self.name && self.at_place(in_chain) {
                            next.push(Ask::Class {
                                path: item.path.to_string(),
                            });
                        } else if !in_chain
                            && item.class == RACK
                            && matches!(&self.place, Place::Chain { rack, .. } if rack == item.name)
                        {
                            next.push(Ask::Chains {
                                path: item.path.to_string(),
                            });
                        }
                    }
                }
                Ask::Chains { .. } => {
                    for item in items(data) {
                        if matches!(&self.place, Place::Chain { chain, .. } if chain == item.name) {
                            next.push(Ask::Devices {
                                path: item.path.to_string(),
                                in_chain: true,
                            });
                        }
                    }
                }
                Ask::Class { .. } => {
                    if data.as_str() == Some(MAX_CLASS) {
                        self.found += 1;
                    }
                }
            }
        }
        if next.is_empty() {
            return Step::Done(answer_of(self.found));
        }
        self.asks = next;
        Step::Read(self.commands())
    }
}

/// The answer of a count of devices found.
pub fn answer_of(found: usize) -> Answer {
    match found {
        0 => Answer::NotOurs,
        1 => Answer::Ours,
        _ => Answer::Two,
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod tests;
