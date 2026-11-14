//! The mute guard chosen on the tablet (#203): whether a strip's mute acts
//! only by the 1 s hold of #162, and what its channel detail's MUTE GUARD
//! shows. Three sources guard a strip: the frame's `double_click_mute`
//! list and a Tuner's `+MG` (both the layout's `Strip.mute_guard`) and the
//! hub's mark set in the detail; any of them guards it.

/// Whether the strip's mute is guarded: by the layout or by the hub's mark.
pub fn guarded(layout: bool, marked: bool) -> bool {
    layout || marked
}

/// The switch's two keys in the detail's order: OFF, then HOLD.
pub const KEYS: [bool; 2] = [false, true];

/// A key's word, and the state's beside the title.
pub fn word(on: bool) -> &'static str {
    if on { "HOLD" } else { "OFF" }
}

/// The note under the switch while the layout guards the strip (its
/// `+MG` tag or the frame's list), which no switch here lifts.
pub const LAYOUT_NOTE: &str =
    "Its tag (+MG) or the layout guards this mute too; OFF here keeps HOLD.";

/// The note shown while the layout guards the strip, else none.
pub fn layout_note(layout: bool) -> Option<&'static str> {
    layout.then_some(LAYOUT_NOTE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_source_guards_the_strip() {
        assert!(!guarded(false, false));
        assert!(guarded(true, false), "the layout alone");
        assert!(guarded(false, true), "the hub's mark alone");
        assert!(guarded(true, true));
    }

    #[test]
    fn the_switch_reads_off_then_hold() {
        assert_eq!(KEYS, [false, true]);
        assert_eq!(word(false), "OFF");
        assert_eq!(word(true), "HOLD");
    }

    #[test]
    fn the_note_shows_only_while_the_layout_guards() {
        assert_eq!(layout_note(true), Some(LAYOUT_NOTE));
        assert_eq!(layout_note(false), None);
        assert!(LAYOUT_NOTE.contains("+MG"));
    }
}
