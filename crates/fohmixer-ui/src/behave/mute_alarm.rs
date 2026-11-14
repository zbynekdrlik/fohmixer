//! The mute alarm (#188; the owner's approved mockup
//! `docs/mockups/mute-alert-v1.html`): whether a strip is marked, as the
//! hub last told, and what its channel detail's MUTE ALARM shows. The look
//! itself (the name button blinking red and the red frame while a marked
//! strip is muted) is CSS on the strip's `data-mute-alarm` and its mute's
//! `data-muted` (`alarm.css`).

use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::strip_marks::StripMarks;

/// Whether a strip is marked in the hub's document as last told (none
/// told yet, or a strip with no identity: not marked). The mute alarm's
/// and the mute guard's (#203) alike.
pub fn marked<D: StripMarks>(doc: Option<&D>, id: Option<&StripRef>) -> bool {
    doc.zip(id).is_some_and(|(doc, id)| doc.is_on(id))
}

/// The switch's two keys in the detail's order: OFF, then ON.
pub const KEYS: [bool; 2] = [false, true];

/// A key's word, and the state's beside the title.
pub fn word(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
}

/// The strip's `data-mute-alarm`: `"true"` while marked, else none (an
/// unmarked strip carries no attribute).
pub fn strip_attr(marked: bool) -> Option<&'static str> {
    marked.then_some("true")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
    use fohmixer_proto::mute_guards::MuteGuardsDoc;

    fn band(label: &str) -> StripRef {
        StripRef {
            instance: "band".into(),
            label: label.into(),
        }
    }

    #[test]
    fn a_strip_is_marked_only_when_the_hubs_document_names_it() {
        let doc = MuteAlarmsDoc::empty().with(&band("Vox 1"), true);
        assert!(marked(Some(&doc), Some(&band("Vox 1"))));
        assert!(!marked(Some(&doc), Some(&band("Bass"))));
        assert!(!marked(Some(&doc), None), "no identity");
        assert!(
            !marked::<MuteAlarmsDoc>(None, Some(&band("Vox 1"))),
            "nothing told yet"
        );
        assert!(!marked::<MuteAlarmsDoc>(None, None));
        // The mute guard's document reads the same way (#203).
        let guards = MuteGuardsDoc::empty().with(&band("Bass"), true);
        assert!(marked(Some(&guards), Some(&band("Bass"))));
        assert!(!marked(Some(&guards), Some(&band("Vox 1"))));
    }

    #[test]
    fn the_switch_reads_off_then_on() {
        assert_eq!(KEYS, [false, true]);
        assert_eq!(word(false), "OFF");
        assert_eq!(word(true), "ON");
    }

    #[test]
    fn only_a_marked_strip_carries_the_attribute() {
        assert_eq!(strip_attr(true), Some("true"));
        assert_eq!(strip_attr(false), None);
    }
}
