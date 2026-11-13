//! The mute alarm's marks on disk (#188): `<data>/mute_alarms.json`, read at
//! start and polled with the layout, written atomically (the shared file
//! store, [`crate::doc_store`], as the colours' `colors.json`: no seed, no
//! revision, a missing file is the empty document, no fader marked).
//!
//! A hand edit is taken as it is and not written back. A file that does
//! not parse or check keeps the last good document served (at start: the
//! empty one) and says why ([`MuteAlarmStore::error`]). A file removed
//! while the hub runs is written back from the document served, once a file
//! was read or written.

use fohmixer_proto::mute_alarms::MuteAlarmsDoc;

use crate::doc_store::{DocStore, Document};

pub use crate::doc_store::Polled;

/// The document's file in the data folder.
pub const MUTE_ALARMS_FILE: &str = "mute_alarms.json";

/// The marks' store: [`DocStore`] as it is (no seed, no revision).
pub type MuteAlarmStore = DocStore<MuteAlarmsDoc>;

impl Document for MuteAlarmsDoc {
    const FILE: &'static str = MUTE_ALARMS_FILE;

    fn empty() -> Self {
        MuteAlarmsDoc::empty()
    }

    fn problems(&self) -> Vec<String> {
        self.validate()
    }
}

/// A document's text: one leading UTF-8 BOM skipped, serde, then its checks
/// (`MuteAlarmsDoc::validate`) joined with `; `.
pub fn parse(text: &[u8]) -> Result<MuteAlarmsDoc, String> {
    crate::doc_store::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use fohmixer_proto::sectors::StripRef;

    /// A document with these band faders marked.
    fn doc(labels: &[&str]) -> MuteAlarmsDoc {
        labels.iter().fold(MuteAlarmsDoc::empty(), |doc, l| {
            let strip = StripRef {
                instance: "band".into(),
                label: (*l).into(),
            };
            doc.with(&strip, true)
        })
    }

    fn read(dir: &Path) -> MuteAlarmsDoc {
        serde_json::from_slice(&std::fs::read(dir.join(MUTE_ALARMS_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn the_file_is_mute_alarms_json() {
        assert_eq!(MUTE_ALARMS_FILE, "mute_alarms.json");
        assert_eq!(<MuteAlarmsDoc as Document>::FILE, MUTE_ALARMS_FILE);
    }

    #[test]
    fn a_missing_file_is_the_empty_document_and_a_change_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MuteAlarmStore::open(dir.path());
        assert_eq!(store.doc(), &MuteAlarmsDoc::empty());
        assert!(store.missing());
        assert_eq!(store.error(), None);
        store.commit(doc(&["Vox 1"])).unwrap();
        assert_eq!(read(dir.path()), doc(&["Vox 1"]));
        assert_eq!(store.doc(), &doc(&["Vox 1"]));
        // The store's own write is no hand edit.
        assert_eq!(store.poll(), None);
    }

    #[test]
    fn a_bad_file_at_start_serves_the_empty_document_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MUTE_ALARMS_FILE), "{").unwrap();
        let store = MuteAlarmStore::open(dir.path());
        assert_eq!(store.doc(), &MuteAlarmsDoc::empty());
        assert_eq!(store.error(), Some(parse(b"{").unwrap_err().as_str()));
        // An unknown field is refused too.
        std::fs::write(
            dir.path().join(MUTE_ALARMS_FILE),
            r#"{"mute_alarms": {"band": ["Vox 1"]}, "on": true}"#,
        )
        .unwrap();
        let store = MuteAlarmStore::open(dir.path());
        assert_eq!(store.doc(), &MuteAlarmsDoc::empty());
        assert!(store.error().is_some_and(|e| e.contains("on")), "{store:?}");
    }

    #[test]
    fn parse_joins_every_problem() {
        assert_eq!(
            parse(br#"{"mute_alarms": {"band": ["", " "]}}"#),
            Err(
                r#"a fader of "band" has an empty label; a fader of "band" has an empty label"#
                    .to_string()
            )
        );
        assert_eq!(parse(br#"{"mute_alarms": {}}"#), Ok(MuteAlarmsDoc::empty()));
        let text = serde_json::to_string(&doc(&["Vox 1"])).unwrap();
        assert_eq!(parse(text.as_bytes()), Ok(doc(&["Vox 1"])));
    }

    #[test]
    fn a_hand_edit_is_taken_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MUTE_ALARMS_FILE),
            serde_json::to_string(&doc(&["Vox 1"])).unwrap(),
        )
        .unwrap();
        let mut store = MuteAlarmStore::open(dir.path());
        assert_eq!(store.doc(), &doc(&["Vox 1"]));
        std::fs::write(
            dir.path().join(MUTE_ALARMS_FILE),
            serde_json::to_string(&doc(&["Bass"])).unwrap(),
        )
        .unwrap();
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(&["Bass"]));
    }

    /// No seed fills a missing file: a bad file at start, then removed, is
    /// written back as the empty document served.
    #[test]
    fn a_bad_file_at_start_then_removed_is_written_back_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MUTE_ALARMS_FILE), "{").unwrap();
        let mut store = MuteAlarmStore::open(dir.path());
        std::fs::remove_file(dir.path().join(MUTE_ALARMS_FILE)).unwrap();
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert_eq!(read(dir.path()), MuteAlarmsDoc::empty());
    }
}
