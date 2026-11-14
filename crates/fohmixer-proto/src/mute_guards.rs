//! The mute guard chosen on the tablet (#203; the owner, 2026-11-14: which
//! fader's mute is protected by a hold is set through its SEL): a fader
//! marked in its channel detail's MUTE GUARD switch mutes only by the 1 s
//! hold of #162, so a stray tap cannot mute it. The hub keeps the marks
//! (`mute_guards.json`), every page follows them; nothing is written to
//! Live. The layout's own guards (the frame's `double_click_mute`, a
//! Tuner's `+MG`) keep guarding: a page guards a strip that either marks.
//!
//! The file is `{"mute_guards": {"<instance>": ["<label>", …]}}`; an
//! unknown field is an error, not ignored (the file can be edited by hand,
//! as the mute alarm's is). Its logic is the mute alarm's,
//! [`crate::strip_marks`].

use serde::{Deserialize, Serialize};

use crate::strip_marks::{Marks, StripMarks};

/// The document, `mute_guards.json`: each Live set's guarded faders by
/// label. A fader it does not name is guarded only by the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuteGuardsDoc {
    pub mute_guards: Marks,
}

impl MuteGuardsDoc {
    /// The empty document: no fader marked.
    pub fn empty() -> Self {
        Self {
            mute_guards: Marks::new(),
        }
    }
}

impl StripMarks for MuteGuardsDoc {
    fn marks(&self) -> &Marks {
        &self.mute_guards
    }

    fn of_marks(mute_guards: Marks) -> Self {
        Self { mute_guards }
    }
}

#[cfg(test)]
mod tests;
