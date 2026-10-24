//! The rack an `+AUTOTUNE` track's autotune is switched by (#104, D19),
//! and the key device inside it (#140 PR 2), pure: a bounded walk of the
//! track's devices through the script's generic `get_prop`, one batch per
//! level, carried out by the router (`router/autotune.rs`), with the Pro-Q
//! 4 walk's helpers (`eq/walk.rs`).
//!
//! The first read is the track's `devices` (its LOM target, by index). A
//! `RackDevice` named exactly [`RACK`] is a match, and the find does not
//! descend into it; any other rack gets its `chains` read, then each
//! chain's `devices`, [`MAX_RACKS`] racks deep at most (a rack on the track
//! is the first). When no read is left, one match gets its `parameters`
//! and its `chains` read in one batch: its parameter [`MACRO`] (its first
//! macro: the chain selector's mapping) is the find's, its path (index
//! form) and its `$ref`. No match, two or more (at any depth: a name step's
//! rule, never a guess) or a rack without that parameter is [`Missing`].
//!
//! Then the key device (spec §5): the devices of the rack's chain 0 (the
//! first chain listed), and when exactly one of them is a plug-in, its
//! `parameters`, whose names make the key device (`keys::device_of`). No
//! chain, other than one plug-in, or a plug-in without Key and Scale is a
//! [`KeyMissing`] in the found macro's [`Found::key`], never a missing
//! rack: the selector is still found.
//!
//! A failed first read (the track not found, Live offline) fails the find
//! with the script's error, and so does a refused read of the matched
//! rack, its chain 0 or its plug-in; a failed slot of the walk (a chain
//! deleted meanwhile) is skipped.

use serde_json::Value;

use crate::eq::walk::{CHAINS, DEVICES, MAX_RACKS, PLUGIN, RACK as RACK_CLASS};
use crate::eq::walk::{Item, answered, get, items, refused};

use super::keys::{KeyMissing, KeyParams, device_of};

/// The name of the rack whose first macro switches autotune.
pub const RACK: &str = "Latencies";
/// Its first macro: the chain selector's mapping (LOM parameter 1; 0 is
/// `Device On`).
pub const MACRO: usize = 1;
/// A device's parameters.
pub const PARAMETERS: &str = "parameters";
/// The reads of a whole find at most: the track's devices, per rack level
/// the chains then their devices, then the rack's parameters and chains,
/// its chain 0's devices and the plug-in's parameters.
pub const MAX_READS: usize = 2 * MAX_RACKS + 4;

/// A parameter the find found: its path (index form) and its `$ref` (for
/// as long as the script's connection lasts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub path: String,
    pub id: String,
}

/// The key device of a rack's chain 0 (#140 PR 2): which device it is and
/// its Key and Scale parameters' indexes, its path (index form), and its
/// Key and Scale parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFound {
    pub params: KeyParams,
    pub device: String,
    pub key: Param,
    pub scale: Param,
}

/// The rack's first macro: its path (index form) and its `$ref` (the
/// parameter object, for as long as the script's connection lasts), and
/// its key device or why it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub id: String,
    pub key: Result<KeyFound, KeyMissing>,
}

/// Why a track has no rack to switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// No rack named [`RACK`].
    NoRack,
    /// This many racks named [`RACK`]: which one is meant is not known.
    Twice(usize),
    /// The rack has no parameter [`MACRO`].
    NoMacro,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Missing::NoRack => write!(f, "no rack named {RACK}"),
            Missing::Twice(n) => write!(f, "{n} racks named {RACK}"),
            Missing::NoMacro => write!(f, "the {RACK} rack has no parameter {MACRO}"),
        }
    }
}

/// What a find does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// These commands, their answer to [`Find::answer`].
    Read(Vec<Value>),
    /// The macro, or why there is none.
    Done(Result<Found, Missing>),
    /// The track (or the matched rack, its chain 0 or its plug-in) could
    /// not be read: the script's error.
    Failed(String),
}

/// One thing a read asks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// A track's or a chain's devices, `depth` racks down.
    Devices { path: String, depth: usize },
    /// A rack's chains, the rack `depth` racks down.
    Chains { path: String, depth: usize },
    /// The matched rack's parameters and chains, in one batch.
    RackParts { path: String },
    /// The devices of the rack's chain 0; `selector` the macro found.
    ChainDevices { path: String, selector: Param },
    /// The one plug-in's parameters.
    PluginParameters { path: String, selector: Param },
}

/// A find of one track's `Latencies` rack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Find {
    asks: Vec<Ask>,
    /// The paths of the racks named [`RACK`] found so far.
    matches: Vec<String>,
    /// The next answer is the track's own read.
    first: bool,
}

impl Find {
    /// A find on the track at `target` (its LOM target) and its first read.
    pub fn start(target: &str) -> (Self, Vec<Value>) {
        let find = Self {
            asks: vec![Ask::Devices {
                path: target.to_string(),
                depth: 0,
            }],
            matches: Vec::new(),
            first: true,
        };
        let commands = find.commands();
        (find, commands)
    }

    /// The commands of the asks waiting.
    fn commands(&self) -> Vec<Value> {
        self.asks
            .iter()
            .flat_map(|ask| match ask {
                Ask::Devices { path, .. } => vec![get(path, DEVICES)],
                Ask::Chains { path, .. } => vec![get(path, CHAINS)],
                Ask::RackParts { path } => vec![get(path, PARAMETERS), get(path, CHAINS)],
                Ask::ChainDevices { path, .. } => vec![get(path, DEVICES)],
                Ask::PluginParameters { path, .. } => vec![get(path, PARAMETERS)],
            })
            .collect()
    }

    /// Asks `ask` alone next.
    fn read(&mut self, ask: Ask) -> Step {
        self.asks = vec![ask];
        Step::Read(self.commands())
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
        let asks = std::mem::take(&mut self.asks);
        match asks.as_slice() {
            [Ask::RackParts { .. }] => return self.parts(slots),
            [Ask::ChainDevices { selector, .. }] => {
                return self.chain_devices(selector.clone(), slots.first());
            }
            [Ask::PluginParameters { path, selector }] => {
                return plugin(path, selector.clone(), slots.first());
            }
            _ => {}
        }
        let mut next = Vec::new();
        for (ask, slot) in asks.into_iter().zip(slots) {
            let Some(data) = answered(slot) else {
                continue;
            };
            match ask {
                Ask::Devices { depth, .. } => {
                    for item in items(data).into_iter().filter(|i| i.class == RACK_CLASS) {
                        if item.name == RACK {
                            self.matches.push(item.path.to_string());
                        } else if depth < MAX_RACKS {
                            next.push(Ask::Chains {
                                path: item.path.to_string(),
                                depth,
                            });
                        }
                    }
                }
                Ask::Chains { depth, .. } => {
                    next.extend(items(data).into_iter().map(|item| Ask::Devices {
                        path: item.path.to_string(),
                        depth: depth + 1,
                    }));
                }
                // Read alone, above.
                Ask::RackParts { .. } | Ask::ChainDevices { .. } | Ask::PluginParameters { .. } => {
                }
            }
        }
        if next.is_empty() {
            return self.matched();
        }
        self.asks = next;
        Step::Read(self.commands())
    }

    /// No read left: one match has its parameters and chains read, none or
    /// several are missing.
    fn matched(&mut self) -> Step {
        let path = match self.matches.as_slice() {
            [] => return Step::Done(Err(Missing::NoRack)),
            [path] => path.clone(),
            several => return Step::Done(Err(Missing::Twice(several.len()))),
        };
        self.read(Ask::RackParts { path })
    }

    /// The rack's parameters and chains answered: its macro, then its chain
    /// 0's devices (none without a chain), or the script's error.
    fn parts(&mut self, slots: &[Value]) -> Step {
        let (parameters, chains) = (slots.first(), slots.get(1));
        if let Some(why) = refused(parameters) {
            return Step::Failed(why);
        }
        let Some(selector) = parameters.and_then(answered).and_then(macro_of) else {
            return Step::Done(Err(Missing::NoMacro));
        };
        if let Some(why) = refused(chains) {
            return Step::Failed(why);
        }
        let chain = chains
            .and_then(answered)
            .and_then(|data| items(data).first().map(|item| item.path.to_string()));
        match chain {
            Some(path) => self.read(Ask::ChainDevices { path, selector }),
            None => done(selector, Err(KeyMissing::NoChain)),
        }
    }

    /// Chain 0's devices answered: its one plug-in's parameters are read,
    /// else the key device is missing.
    fn chain_devices(&mut self, selector: Param, slot: Option<&Value>) -> Step {
        if let Some(why) = refused(slot) {
            return Step::Failed(why);
        }
        let plugins: Vec<String> = slot
            .and_then(answered)
            .map(items)
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item.class == PLUGIN)
            .map(|item| item.path.to_string())
            .collect();
        match plugins.as_slice() {
            [path] => self.read(Ask::PluginParameters {
                path: path.clone(),
                selector,
            }),
            several => done(selector, Err(KeyMissing::Plugins(several.len()))),
        }
    }
}

/// A list item as a found parameter.
fn param(item: &Item<'_>) -> Param {
    Param {
        path: item.path.to_string(),
        id: item.id.to_string(),
    }
}

/// The rack's parameter [`MACRO`] in its parameters' list answer.
fn macro_of(data: &Value) -> Option<Param> {
    items(data).get(MACRO).map(param)
}

/// The end of a find: the macro and its key device, or why none.
fn done(selector: Param, key: Result<KeyFound, KeyMissing>) -> Step {
    Step::Done(Ok(Found {
        path: selector.path,
        id: selector.id,
        key,
    }))
}

/// The plug-in at `path`'s parameters answered: its key device from their
/// names, or why none, or the script's error.
fn plugin(path: &str, selector: Param, slot: Option<&Value>) -> Step {
    if let Some(why) = refused(slot) {
        return Step::Failed(why);
    }
    let params = slot.and_then(answered).map(items).unwrap_or_default();
    let names: Vec<&str> = params.iter().map(|item| item.name).collect();
    let key = device_of(&names).and_then(|found| {
        let (Some(key), Some(scale)) = (params.get(found.key), params.get(found.scale)) else {
            return Err(KeyMissing::NoKeyParams);
        };
        Ok(KeyFound {
            params: found,
            device: path.to_string(),
            key: param(key),
            scale: param(scale),
        })
    });
    done(selector, key)
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
