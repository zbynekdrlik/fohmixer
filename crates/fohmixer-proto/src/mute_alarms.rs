//! The mute alarm (#188; the owner's approved mockup
//! `docs/mockups/mute-alert-v1.html`): a fader whose mute can stop the show
//! (the band's feed at an event) is marked in its channel detail's MUTE
//! ALARM switch; while such a fader is muted its name button blinks red and
//! a red frame blinks round the strip, so a mute that is not the normal
//! state cannot look calm. The hub keeps the marks (`mute_alarms.json`),
//! every page shows them; nothing is written to Live.
//!
//! A mark belongs to a fader, known by its `StripRef` (its Live set and
//! its label, as the sectors and the colours know it), so the same fader in
//! two layouts is marked once. The marks' logic is shared with the mute
//! guard's (#203): [`crate::strip_marks`].
//!
//! The file is `{"mute_alarms": {"<instance>": ["<label>", …]}}`; an
//! unknown field is an error, not ignored (the file can be edited by hand,
//! as the colours' is).

use serde::{Deserialize, Serialize};

use crate::sectors::{Pooled, Refusal, StripRef};
use crate::strip_marks::{self, Marks, StripMarks};

/// The document, `mute_alarms.json`: each Live set's marked faders by
/// label. A fader it does not name has no alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuteAlarmsDoc {
    pub mute_alarms: Marks,
}

impl MuteAlarmsDoc {
    /// The empty document: no fader marked.
    pub fn empty() -> Self {
        Self {
            mute_alarms: Marks::new(),
        }
    }
}

impl StripMarks for MuteAlarmsDoc {
    fn marks(&self) -> &Marks {
        &self.mute_alarms
    }

    fn of_marks(mute_alarms: Marks) -> Self {
        Self { mute_alarms }
    }
}

/// How many marked faders no pool fader carries (a track renamed in Live:
/// its mark waits, the strip shows no alarm).
pub fn unknown(doc: &MuteAlarmsDoc, pool: &[Pooled]) -> usize {
    strip_marks::unknown(doc, pool)
}

/// A page's switch of `strip`'s alarm to `on`: on only for a fader of the
/// pool ([`Refusal::NotInPool`]; off clears any, as COLOR's NONE does);
/// `Ok(None)` when it changes nothing, else the document after.
pub fn set_mute_alarm(
    doc: &MuteAlarmsDoc,
    strip: &StripRef,
    on: bool,
    pool: &[Pooled],
) -> Result<Option<MuteAlarmsDoc>, Refusal> {
    strip_marks::switch(doc, strip, on, pool)
}

#[cfg(test)]
mod tests;
