//! The scene rules on disk (#207): `<data>/scenes.json`, read at start and
//! polled with the layout, written atomically (the colours' store,
//! [`crate::doc_store`]: no seed, no revision; a missing file is the empty
//! document, no scene heard, no rule).
//!
//! A hand edit is taken as it is and not written back. A file that does
//! not parse or check keeps the last good document served and says why
//! ([`SceneStore::error`]). A file removed while the hub runs is written
//! back from the document served.

use fohmixer_proto::scenes::ScenesDoc;

use crate::doc_store::{DocStore, Document};

pub use crate::doc_store::Polled;

/// The document's file in the data folder.
pub const SCENES_FILE: &str = "scenes.json";

/// The scene rules' store: [`DocStore`] as it is.
pub type SceneStore = DocStore<ScenesDoc>;

impl Document for ScenesDoc {
    const FILE: &'static str = SCENES_FILE;

    fn empty() -> Self {
        ScenesDoc::empty()
    }

    fn problems(&self) -> Vec<String> {
        self.validate()
    }
}

/// A document's text: one leading UTF-8 BOM skipped, serde, then its checks
/// joined with `; `.
pub fn parse(text: &[u8]) -> Result<ScenesDoc, String> {
    crate::doc_store::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    use fohmixer_proto::scenes::{SceneMute, SceneRule, heard};
    use fohmixer_proto::sectors::StripRef;

    #[test]
    fn the_file_is_scenes_json() {
        assert_eq!(SCENES_FILE, "scenes.json");
        assert_eq!(<ScenesDoc as Document>::FILE, SCENES_FILE);
    }

    #[test]
    fn a_missing_file_is_the_empty_document_and_a_change_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SceneStore::open(dir.path());
        assert_eq!(store.doc(), &ScenesDoc::empty());
        assert!(!dir.path().join(SCENES_FILE).exists());
        let doc = heard(store.doc(), "intro").unwrap().with(
            &StripRef {
                instance: "band".into(),
                label: "Vox 1".into(),
            },
            "intro",
            Some(SceneRule {
                db: Some(-3.0),
                mute: Some(SceneMute::Unmute),
                time_ms: 2000,
            }),
        );
        store.commit(doc.clone()).unwrap();
        let read: ScenesDoc =
            serde_json::from_slice(&std::fs::read(dir.path().join(SCENES_FILE)).unwrap()).unwrap();
        assert_eq!(read, doc);
        assert_eq!(SceneStore::open(dir.path()).doc(), &doc);
    }

    #[test]
    fn a_file_that_does_not_check_keeps_the_empty_document_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(SCENES_FILE),
            r#"{"known": ["intro", "intro"]}"#,
        )
        .unwrap();
        let store = SceneStore::open(dir.path());
        assert_eq!(store.doc(), &ScenesDoc::empty());
        assert_eq!(
            store.error(),
            Some(
                parse(br#"{"known": ["intro", "intro"]}"#)
                    .unwrap_err()
                    .as_str()
            )
        );
        assert!(store.error().unwrap().contains("listed twice"));
    }
}
