//! The fader sectors' document on disk (#148): `<data>/sectors.json`,
//! read at start and polled with the layout, written atomically.
//!
//! The hub owns the revision: every change it makes is `rev + 1`, written
//! into the file. A hand edit is served at [`served_rev`] and not written
//! back. A file that does not parse or check keeps the last good document
//! served (at start: the empty one, sectors off) and says why
//! ([`SectorStore::error`]). There is no undo (the owner, 29.10.2026): an
//! older hub's `sectors.json.prev` left in the folder is removed at start.
//!
//! The file mechanics (bytes compared, BOM, unreadable once, removal) are
//! the shared [`crate::doc_store`]; this module is the sectors' document:
//! its checks, the revision and the seed's `never_good`.

use std::io;
use std::path::Path;

use fohmixer_proto::sectors::SectorsDoc;

use crate::doc_store::{DocStore, Document};

pub use crate::doc_store::Polled;

/// The document's file in the data folder.
pub const SECTORS_FILE: &str = "sectors.json";
/// The undo file an older hub kept (removed at start).
pub const OLD_PREV_FILE: &str = "sectors.json.prev";

/// The sectors' store: [`DocStore`] with the seed (a file missing since the
/// start waits for it, `missing`) and the hub's revision.
pub type SectorStore = DocStore<SectorsDoc>;

impl Document for SectorsDoc {
    const FILE: &'static str = SECTORS_FILE;

    fn empty() -> Self {
        SectorsDoc::empty()
    }

    fn problems(&self) -> Vec<String> {
        self.validate()
    }

    /// A layout's trailing empty slots are dropped.
    fn served(self) -> Self {
        self.trimmed()
    }

    /// A hand edit is served at [`served_rev`].
    fn hand_edit(mut self, before: &Self) -> Self {
        self.rev = served_rev(self.rev, before.rev);
        self
    }

    /// Nothing good was ever read: the removal is a missing file, which the
    /// seed may fill.
    fn never_good(refused: bool, served: &Self) -> bool {
        never_good(refused, served)
    }

    /// An older hub's undo file is removed.
    fn on_open(dir: &Path) {
        if let Some(e) = removal_failed(std::fs::remove_file(dir.join(OLD_PREV_FILE))) {
            tracing::warn!(error = %e, "cannot remove an older hub's sectors.json.prev");
        }
    }
}

/// A document's text: one leading UTF-8 BOM skipped, serde, then its checks
/// (`SectorsDoc::validate`) joined with `; `; a layout's trailing empty
/// slots are dropped (`SectorsDoc::trimmed`).
pub fn parse(text: &[u8]) -> Result<SectorsDoc, String> {
    crate::doc_store::parse(text)
}

/// The revision a hand edit is served at: the file's, or one past the
/// served one when that is newer.
pub fn served_rev(file_rev: u64, served_rev: u64) -> u64 {
    file_rev.max(served_rev + 1)
}

/// Whether the store never served a good document from disk: the file
/// it read was refused (`refused`) and the empty document stands in.
fn never_good(refused: bool, served: &SectorsDoc) -> bool {
    refused && *served == SectorsDoc::empty()
}

/// A removal's failure worth a warning: none when it went or when the file
/// was not there.
fn removal_failed(removed: io::Result<()>) -> Option<io::Error> {
    removed
        .err()
        .filter(|e| e.kind() != io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::sectors::{SectorId, SectorLayout, StripRef};

    /// A document at `rev` whose `lu-1` holds the band's `labels`.
    fn doc(rev: u64, labels: &[&str]) -> SectorsDoc {
        SectorsDoc {
            rev,
            sectors: [(
                SectorId::Lu,
                vec![SectorLayout {
                    id: "lu-1".into(),
                    title: "VOX".into(),
                    strips: labels
                        .iter()
                        .map(|l| {
                            Some(StripRef {
                                instance: "band".into(),
                                label: (*l).into(),
                            })
                        })
                        .collect(),
                }],
            )]
            .into_iter()
            .collect(),
        }
    }

    fn write(dir: &Path, name: &str, doc: &SectorsDoc) {
        std::fs::write(dir.join(name), serde_json::to_string(doc).unwrap()).unwrap();
    }

    fn read(dir: &Path, name: &str) -> SectorsDoc {
        serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap()
    }

    #[test]
    fn open_reads_the_file_and_removes_an_older_hubs_undo_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(3, &["Vox 1"]));
        write(dir.path(), OLD_PREV_FILE, &doc(2, &[]));
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &doc(3, &["Vox 1"]));
        assert!(!store.missing());
        assert_eq!(store.error(), None);
        assert!(!dir.path().join(OLD_PREV_FILE).exists());
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(3, &["Vox 1"]));
    }

    #[test]
    fn a_bad_file_at_start_serves_the_empty_document_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SECTORS_FILE), "{").unwrap();
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert_eq!(store.error(), Some(parse(b"{").unwrap_err().as_str()));
        assert!(store.error().is_some());
        assert!(!store.missing(), "a file is there: no seed over it");
        // A document that parses but does not check is refused too.
        let mut twice = doc(1, &["Vox 1", "Vox 1"]);
        twice.rev = 4;
        write(dir.path(), SECTORS_FILE, &twice);
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert_eq!(
            store.error(),
            Some(r#"layout "lu-1" holds band|Vox 1 twice"#)
        );
    }

    #[test]
    fn parse_joins_every_problem() {
        let mut bad = doc(1, &["Vox 1", "Vox 1"]);
        bad.sectors.get_mut(&SectorId::Lu).unwrap()[0].title = "ABCDEFGHI".into();
        assert_eq!(
            parse(serde_json::to_string(&bad).unwrap().as_bytes()),
            Err(
                r#"layout "lu-1": its title "ABCDEFGHI" is longer than 8 characters; layout "lu-1" holds band|Vox 1 twice"#
                    .to_string()
            )
        );
        assert_eq!(
            parse(br#"{"rev": 0, "sectors": {}}"#),
            Ok(SectorsDoc::empty())
        );
    }

    #[test]
    fn a_hand_edit_with_a_sector_of_no_layout_is_refused() {
        // `"rd": []` would refuse every ARRANGE save `invalid`: the file is
        // refused instead, its error shown, the last good document kept.
        assert_eq!(
            parse(br#"{"rev": 2, "sectors": {"lu": [{"id": "lu-1", "title": "VOX", "strips": []}], "rd": []}}"#),
            Err("sector rd has no layout".to_string())
        );
    }

    #[test]
    fn parse_keeps_a_hand_edits_gaps_and_drops_its_trailing_empty_slots() {
        let text = br#"{"rev": 3, "sectors": {"ru": [{"id": "ru-1", "title": "BAND",
            "strips": [{"instance": "band", "label": "Klavir"}, null,
                       {"instance": "band", "label": "Bass"}, null, null]}]}}"#;
        let read = parse(text).unwrap();
        let strips = &read.layouts(SectorId::Ru)[0].strips;
        assert_eq!(strips.len(), 3);
        assert_eq!(strips[1], None);
        assert_eq!(strips[2].as_ref().map(|s| s.label.as_str()), Some("Bass"));
    }

    #[test]
    fn commit_writes_and_serves_the_change_and_keeps_no_undo_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(1, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        store.commit(doc(2, &["Vox 1", "Bass"])).unwrap();
        assert!(!dir.path().join(OLD_PREV_FILE).exists());
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(2, &["Vox 1", "Bass"]));
        assert_eq!(store.doc(), &doc(2, &["Vox 1", "Bass"]));
        assert!(!dir.path().join("sectors.tmp").exists());
        assert!(!dir.path().join("sectors.json.tmp").exists());
    }

    #[test]
    fn a_hand_edit_is_taken_at_a_newer_rev() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(5, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        write(dir.path(), SECTORS_FILE, &doc(1, &["Klavir"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(6, &["Klavir"]));
        assert!(!dir.path().join(OLD_PREV_FILE).exists());
        assert_eq!(
            read(dir.path(), SECTORS_FILE),
            doc(1, &["Klavir"]),
            "not written back"
        );
        assert_eq!(store.poll(), None, "the same bytes again");
    }

    #[test]
    fn served_rev_is_the_newer_of_the_files_and_one_past_the_served() {
        assert_eq!(served_rev(1, 5), 6);
        assert_eq!(served_rev(9, 5), 9);
        assert_eq!(served_rev(6, 5), 6);
        assert_eq!(served_rev(0, 0), 1);
    }

    #[test]
    fn a_removal_fails_only_for_another_reason_than_a_missing_file() {
        assert!(removal_failed(Ok(())).is_none());
        assert!(removal_failed(Err(io::Error::from(io::ErrorKind::NotFound))).is_none());
        let failed = removal_failed(Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert_eq!(
            failed.map(|e| e.kind()),
            Some(io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn a_bad_file_at_start_then_removed_is_missing_and_never_written_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SECTORS_FILE);
        std::fs::write(&path, "{").unwrap();
        let mut store = SectorStore::open(dir.path());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(store.poll(), None);
        assert!(store.missing(), "the seed may fill it");
        assert!(!path.exists(), "the empty stand-in is not written back");
    }

    #[test]
    fn a_good_file_removed_is_written_back_never_seeded() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(SECTORS_FILE)).unwrap();
        // After the seed only the pages change the document (spec §2): the
        // one served goes back on disk, nothing seeded.
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert!(!store.missing(), "no seed over the owner's arrangement");
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(2, &["Vox 1"]));
    }

    #[test]
    fn never_good_only_when_refused_and_serving_the_empty_document() {
        assert!(never_good(true, &SectorsDoc::empty()));
        assert!(!never_good(false, &SectorsDoc::empty()));
        assert!(!never_good(true, &doc(2, &["Vox 1"])));
    }
}
