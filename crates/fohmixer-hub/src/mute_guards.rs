//! The mute guard's marks on disk (#203): `<data>/mute_guards.json`, kept
//! exactly as the mute alarm's `mute_alarms.json` ([`crate::mute_alarms`]:
//! the shared file store, [`crate::doc_store`], no seed, no revision, a
//! missing file is the empty document, no fader guarded by the tablet).

use fohmixer_proto::mute_guards::MuteGuardsDoc;
use fohmixer_proto::strip_marks::StripMarks;

use crate::doc_store::{DocStore, Document};

/// The document's file in the data folder.
pub const MUTE_GUARDS_FILE: &str = "mute_guards.json";

/// The marks' store: [`DocStore`] as it is (no seed, no revision).
pub type MuteGuardStore = DocStore<MuteGuardsDoc>;

impl Document for MuteGuardsDoc {
    const FILE: &'static str = MUTE_GUARDS_FILE;

    fn empty() -> Self {
        MuteGuardsDoc::empty()
    }

    fn problems(&self) -> Vec<String> {
        self.validate()
    }
}

/// A document's text: one leading UTF-8 BOM skipped, serde, then its checks
/// joined with `; `.
pub fn parse(text: &[u8]) -> Result<MuteGuardsDoc, String> {
    crate::doc_store::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    use fohmixer_proto::sectors::StripRef;

    fn guarded(label: &str) -> MuteGuardsDoc {
        MuteGuardsDoc::empty().with(
            &StripRef {
                instance: "band".into(),
                label: label.into(),
            },
            true,
        )
    }

    #[test]
    fn the_file_is_mute_guards_json() {
        assert_eq!(MUTE_GUARDS_FILE, "mute_guards.json");
        assert_eq!(<MuteGuardsDoc as Document>::FILE, MUTE_GUARDS_FILE);
        assert_eq!(<MuteGuardsDoc as Document>::empty(), MuteGuardsDoc::empty());
    }

    #[test]
    fn parse_checks_the_document() {
        assert_eq!(
            parse(br#"{"mute_guards": {"band": [" "]}}"#),
            Err(r#"a fader of "band" has an empty label"#.to_string())
        );
        assert!(parse(br#"{"mute_alarms": {}}"#).is_err());
        let text = serde_json::to_string(&guarded("Vox 1")).unwrap();
        assert_eq!(parse(text.as_bytes()), Ok(guarded("Vox 1")));
    }

    #[test]
    fn a_change_is_written_and_a_hand_edit_taken() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MuteGuardStore::open(dir.path());
        assert_eq!(store.doc(), &MuteGuardsDoc::empty());
        assert!(store.missing());
        store.commit(guarded("Vox 1")).unwrap();
        let on_disk: MuteGuardsDoc =
            serde_json::from_slice(&std::fs::read(dir.path().join(MUTE_GUARDS_FILE)).unwrap())
                .unwrap();
        assert_eq!(on_disk, guarded("Vox 1"));
        assert_eq!(store.poll(), None);
        std::fs::write(
            dir.path().join(MUTE_GUARDS_FILE),
            serde_json::to_string(&guarded("Bass")).unwrap(),
        )
        .unwrap();
        assert_eq!(store.poll(), Some(crate::doc_store::Polled::Taken));
        assert_eq!(store.doc(), &guarded("Bass"));
    }
}
