//! The strips' colours on disk (#164): `<data>/colors.json`, read at start
//! and polled with the layout, written atomically (the sectors' file
//! store, [`crate::doc_store`], without the sectors' seed and revision: a
//! missing file is the empty document, every strip NONE).
//!
//! A hand edit is taken as it is and not written back. A file that does
//! not parse or check keeps the last good document served (at start: the
//! empty one) and says why ([`ColorStore::error`]). A file removed while
//! the hub runs is written back from the document served, once a file was
//! read or written (a bad file at start: the empty document).

use fohmixer_proto::colors::ColorsDoc;

use crate::doc_store::{DocStore, Document};

pub use crate::doc_store::Polled;

/// The document's file in the data folder.
pub const COLORS_FILE: &str = "colors.json";

/// The colours' store: [`DocStore`] as it is (no seed, no revision).
pub type ColorStore = DocStore<ColorsDoc>;

impl Document for ColorsDoc {
    const FILE: &'static str = COLORS_FILE;

    fn empty() -> Self {
        ColorsDoc::empty()
    }

    fn problems(&self) -> Vec<String> {
        self.validate()
    }
}

/// A document's text: one leading UTF-8 BOM skipped, serde, then its checks
/// (`ColorsDoc::validate`) joined with `; `.
pub fn parse(text: &[u8]) -> Result<ColorsDoc, String> {
    crate::doc_store::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use fohmixer_proto::colors::ColorKey;
    use fohmixer_proto::sectors::StripRef;

    /// A document with these band faders pink.
    fn doc(labels: &[&str]) -> ColorsDoc {
        labels.iter().fold(ColorsDoc::empty(), |doc, l| {
            let strip = StripRef {
                instance: "band".into(),
                label: (*l).into(),
            };
            doc.with(&strip, Some(ColorKey::Pink))
        })
    }

    fn read(dir: &Path) -> ColorsDoc {
        serde_json::from_slice(&std::fs::read(dir.join(COLORS_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn a_bad_file_at_start_serves_the_empty_document_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(COLORS_FILE), "{").unwrap();
        let store = ColorStore::open(dir.path());
        assert_eq!(store.doc(), &ColorsDoc::empty());
        assert_eq!(store.error(), Some(parse(b"{").unwrap_err().as_str()));
        // An unknown key is refused too.
        std::fs::write(
            dir.path().join(COLORS_FILE),
            r#"{"colors": {"band": {"Vox 1": "gold"}}}"#,
        )
        .unwrap();
        let store = ColorStore::open(dir.path());
        assert_eq!(store.doc(), &ColorsDoc::empty());
        assert!(
            store.error().is_some_and(|e| e.contains("gold")),
            "{store:?}"
        );
    }

    #[test]
    fn parse_joins_every_problem() {
        assert_eq!(
            parse(br#"{"colors": {"band": {"": "red", " ": "blue"}}}"#),
            Err(
                r#"a fader of "band" has an empty label; a fader of "band" has an empty label"#
                    .to_string()
            )
        );
        assert_eq!(parse(br#"{"colors": {}}"#), Ok(ColorsDoc::empty()));
        let text = serde_json::to_string(&doc(&["Vox 1"])).unwrap();
        assert_eq!(parse(text.as_bytes()), Ok(doc(&["Vox 1"])));
    }

    #[test]
    fn a_hand_edit_is_taken_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(COLORS_FILE),
            serde_json::to_string(&doc(&["Vox 1"])).unwrap(),
        )
        .unwrap();
        let mut store = ColorStore::open(dir.path());
        std::fs::write(
            dir.path().join(COLORS_FILE),
            serde_json::to_string(&doc(&["Bass"])).unwrap(),
        )
        .unwrap();
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(&["Bass"]));
    }

    /// No seed fills a missing colours file: a bad file at start, then
    /// removed, is written back as the empty document served.
    #[test]
    fn a_bad_file_at_start_then_removed_is_written_back_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(COLORS_FILE), "{").unwrap();
        let mut store = ColorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(COLORS_FILE)).unwrap();
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert_eq!(read(dir.path()), ColorsDoc::empty());
    }
}
