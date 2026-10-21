//! The rack an `+AUTOTUNE` track's autotune is switched by (#104, D19),
//! pure: a bounded walk of the track's devices through the script's
//! generic `get_prop`, one batch per level, carried out by the router
//! (`router/autotune.rs`), with the Pro-Q 4 walk's helpers
//! (`eq/walk.rs`).
//!
//! The first read is the track's `devices` (its LOM target, by index). A
//! `RackDevice` named exactly [`RACK`] is a match, and the find does not
//! descend into it; any other rack gets its `chains` read, then each
//! chain's `devices`, [`MAX_RACKS`] racks deep at most (a rack on the track
//! is the first). When no read is left, one match gets its `parameters`
//! read and its parameter [`MACRO`] (its first macro: the chain selector's
//! mapping) is the find's: its path (index form) and its `$ref`. No match,
//! two or more (at any depth: a name step's rule, never a guess) or a rack
//! without that parameter is [`Missing`]. A failed first read (the track
//! not found, Live offline) fails the find with the script's error; a
//! failed later slot (a chain deleted meanwhile) is skipped.

use serde_json::Value;

use crate::eq::walk::{CHAINS, DEVICES, MAX_RACKS, RACK as RACK_CLASS};
use crate::eq::walk::{answered, get, items, refused};

/// The name of the rack whose first macro switches autotune.
pub const RACK: &str = "Latencies";
/// Its first macro: the chain selector's mapping (LOM parameter 1; 0 is
/// `Device On`).
pub const MACRO: usize = 1;
/// A device's parameters.
pub const PARAMETERS: &str = "parameters";
/// The reads of a whole find at most: the track's devices, per rack level
/// the chains then their devices, then the rack's parameters.
pub const MAX_READS: usize = 2 * MAX_RACKS + 2;

/// The rack's first macro: its path (index form) and its `$ref` (the
/// parameter object, for as long as the script's connection lasts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub id: String,
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
    /// The track (or the matched rack's parameters) could not be read: the
    /// script's error.
    Failed(String),
}

/// One thing a read asks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// A track's or a chain's devices, `depth` racks down.
    Devices { path: String, depth: usize },
    /// A rack's chains, the rack `depth` racks down.
    Chains { path: String, depth: usize },
    /// The matched rack's parameters.
    Parameters { path: String },
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
            .map(|ask| match ask {
                Ask::Devices { path, .. } => get(path, DEVICES),
                Ask::Chains { path, .. } => get(path, CHAINS),
                Ask::Parameters { path } => get(path, PARAMETERS),
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
        let asks = std::mem::take(&mut self.asks);
        if matches!(asks.as_slice(), [Ask::Parameters { .. }]) {
            return macro_of(slots.first());
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
                Ask::Parameters { .. } => {}
            }
        }
        if next.is_empty() {
            return self.matched();
        }
        self.asks = next;
        Step::Read(self.commands())
    }

    /// No read left: one match has its parameters read, none or several are
    /// missing.
    fn matched(&mut self) -> Step {
        match self.matches.as_slice() {
            [] => Step::Done(Err(Missing::NoRack)),
            [path] => {
                self.asks = vec![Ask::Parameters { path: path.clone() }];
                Step::Read(self.commands())
            }
            several => Step::Done(Err(Missing::Twice(several.len()))),
        }
    }
}

/// The rack's parameters answered: its parameter [`MACRO`], none, or the
/// script's error.
fn macro_of(slot: Option<&Value>) -> Step {
    if let Some(why) = refused(slot) {
        return Step::Failed(why);
    }
    let found = slot.and_then(answered).and_then(|data| {
        items(data).get(MACRO).map(|item| Found {
            path: item.path.to_string(),
            id: item.id.to_string(),
        })
    });
    Step::Done(found.ok_or(Missing::NoMacro))
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
