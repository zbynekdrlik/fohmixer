//! The fader sectors' document on disk (#148): `<data>/sectors.json`, the
//! version before the last change as `sectors.json.prev` (the undo), read
//! at start and polled with the layout, written atomically.
//!
//! The hub owns the revision: every change it makes is `rev + 1`, written
//! into the file. A hand edit (there is no editor for the layouts' names
//! yet) is served at [`served_rev`] and not written back; it is a change
//! too, so the version before it becomes the undo. A file that does not
//! parse or check keeps the last good document served (at start: the empty
//! one, sectors off) and says why ([`SectorStore::error`]).

use std::io;
use std::path::{Path, PathBuf};

use fohmixer_proto::sectors::{Refusal, SectorsDoc};

/// The document's file in the data folder.
pub const SECTORS_FILE: &str = "sectors.json";
/// The version before the last change.
pub const PREV_FILE: &str = "sectors.json.prev";

/// What a poll found: a new document taken, or a file refused (why).
#[derive(Debug, Clone, PartialEq)]
pub enum Polled {
    Taken,
    Refused(String),
}

/// The document as served, the version before the last change, and what
/// the file on disk held when the store last read or wrote it.
#[derive(Debug)]
pub struct SectorStore {
    dir: PathBuf,
    doc: SectorsDoc,
    prev: Option<SectorsDoc>,
    /// No `sectors.json` on disk (the seed may write the first one).
    missing: bool,
    /// The file's bytes as last read or written: a poll that reads them
    /// again takes nothing (the store's own write, a rewrite in place).
    known: Option<Vec<u8>>,
    error: Option<String>,
}

/// A document's text: serde, then its checks (`SectorsDoc::validate`)
/// joined with `; `.
pub fn parse(text: &[u8]) -> Result<SectorsDoc, String> {
    let doc: SectorsDoc = serde_json::from_slice(text).map_err(|e| e.to_string())?;
    let errors = doc.validate();
    if errors.is_empty() {
        Ok(doc)
    } else {
        Err(errors.join("; "))
    }
}

/// The revision a hand edit is served at: the file's, or one past the
/// served one when that is newer.
pub fn served_rev(file_rev: u64, served_rev: u64) -> u64 {
    file_rev.max(served_rev + 1)
}

/// A document as the files hold it.
fn text_of(doc: &SectorsDoc) -> io::Result<String> {
    serde_json::to_string_pretty(doc).map_err(io::Error::other)
}

impl SectorStore {
    /// The store of `data_dir`: its file and its previous version as read
    /// now. A missing file is the empty document; one that does not parse
    /// or check is the empty document with its error; a previous version
    /// that does not is none.
    pub fn open(data_dir: &Path) -> Self {
        let mut store = Self {
            dir: data_dir.to_path_buf(),
            doc: SectorsDoc::empty(),
            prev: None,
            missing: false,
            known: None,
            error: None,
        };
        store.prev = std::fs::read(store.path(PREV_FILE))
            .ok()
            .and_then(|bytes| parse(&bytes).ok());
        match std::fs::read(store.path(SECTORS_FILE)) {
            Ok(bytes) => {
                match parse(&bytes) {
                    Ok(doc) => store.doc = doc,
                    Err(why) => store.error = Some(why),
                }
                store.known = Some(bytes);
            }
            Err(e) => {
                store.missing = e.kind() == io::ErrorKind::NotFound;
                if !store.missing {
                    store.error = Some(e.to_string());
                }
            }
        }
        store
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The document served.
    pub fn doc(&self) -> &SectorsDoc {
        &self.doc
    }

    /// Whether the store holds the version before the last change.
    pub fn has_prev(&self) -> bool {
        self.prev.is_some()
    }

    /// Whether no `sectors.json` is on disk.
    pub fn missing(&self) -> bool {
        self.missing
    }

    /// Why the file on disk is not the document served.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the file again: a new document (a hand edit) is taken at
    /// [`served_rev`] and the one before it kept as the previous version;
    /// a bad one is refused and the last good one stays. Bytes the store
    /// already knows take nothing; a missing file only marks it missing.
    pub fn poll(&mut self) -> Option<Polled> {
        let bytes = match std::fs::read(self.path(SECTORS_FILE)) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.missing = e.kind() == io::ErrorKind::NotFound;
                self.known = None;
                return None;
            }
        };
        self.missing = false;
        if self.known.as_deref() == Some(bytes.as_slice()) {
            return None;
        }
        let parsed = parse(&bytes);
        self.known = Some(bytes);
        match parsed {
            Ok(mut doc) => {
                doc.rev = served_rev(doc.rev, self.doc.rev);
                let before = std::mem::replace(&mut self.doc, doc);
                if let Err(e) = text_of(&before)
                    .and_then(|text| crate::atomic_write(&self.path(PREV_FILE), &text))
                {
                    tracing::warn!(error = %e, "cannot keep the sectors' previous version on disk (sectors.json.prev); the undo stays in memory");
                }
                self.prev = Some(before);
                self.error = None;
                Some(Polled::Taken)
            }
            Err(why) => {
                self.error = Some(why.clone());
                Some(Polled::Refused(why))
            }
        }
    }

    /// A change: the document served now becomes the previous version
    /// (written first), then `next` is written and served.
    pub fn commit(&mut self, next: SectorsDoc) -> io::Result<()> {
        let before = text_of(&self.doc)?;
        let text = text_of(&next)?;
        crate::atomic_write(&self.path(PREV_FILE), &before)?;
        crate::atomic_write(&self.path(SECTORS_FILE), &text)?;
        self.prev = Some(std::mem::replace(&mut self.doc, next));
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.error = None;
        Ok(())
    }

    /// The first document: written and served, with no previous version
    /// (an old `sectors.json.prev` is removed).
    pub fn seed(&mut self, seeded: SectorsDoc) -> io::Result<()> {
        let text = text_of(&seeded)?;
        crate::atomic_write(&self.path(SECTORS_FILE), &text)?;
        if let Err(e) = std::fs::remove_file(self.path(PREV_FILE))
            && e.kind() != io::ErrorKind::NotFound
        {
            return Err(e);
        }
        self.doc = seeded;
        self.prev = None;
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.error = None;
        Ok(())
    }

    /// The undo: the previous version comes back at the next revision, and
    /// the one served becomes the previous (a second undo redoes it).
    /// Refused without a previous version; else the write's outcome.
    pub fn undo(&mut self) -> Result<io::Result<()>, Refusal> {
        let mut back = self.prev.clone().ok_or(Refusal::NothingToUndo)?;
        back.rev = self.doc.rev + 1;
        Ok(self.commit(back))
    }
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
                        .map(|l| StripRef {
                            instance: "band".into(),
                            label: (*l).into(),
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
    fn a_missing_file_is_the_empty_document_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert!(store.missing());
        assert!(!store.has_prev());
        assert_eq!(store.error(), None);
    }

    #[test]
    fn open_reads_the_file_and_its_prev() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(3, &["Vox 1"]));
        write(dir.path(), PREV_FILE, &doc(2, &[]));
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &doc(3, &["Vox 1"]));
        assert!(store.has_prev());
        assert!(!store.missing());
        assert_eq!(store.error(), None);
        // A previous version that does not parse is none.
        std::fs::write(dir.path().join(PREV_FILE), "{").unwrap();
        assert!(!SectorStore::open(dir.path()).has_prev());
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
    fn commit_keeps_the_version_before_as_prev_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(1, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        store.commit(doc(2, &["Vox 1", "Bass"])).unwrap();
        assert_eq!(read(dir.path(), PREV_FILE), doc(1, &["Vox 1"]));
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(2, &["Vox 1", "Bass"]));
        assert_eq!(store.doc(), &doc(2, &["Vox 1", "Bass"]));
        assert!(store.has_prev());
        assert!(!dir.path().join("sectors.tmp").exists());
        assert!(!dir.path().join("sectors.json.tmp").exists());
    }

    #[test]
    fn the_stores_own_write_is_not_polled_as_a_change() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(1, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        assert_eq!(store.poll(), None, "the file as opened");
        store.commit(doc(2, &["Bass"])).unwrap();
        assert_eq!(store.poll(), None);
        assert_eq!(store.doc(), &doc(2, &["Bass"]));
    }

    #[test]
    fn a_hand_edit_is_taken_at_a_newer_rev_and_keeps_prev() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(5, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        write(dir.path(), SECTORS_FILE, &doc(1, &["Klavir"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(6, &["Klavir"]));
        assert!(store.has_prev());
        assert_eq!(read(dir.path(), PREV_FILE), doc(5, &["Vox 1"]));
        assert_eq!(
            read(dir.path(), SECTORS_FILE),
            doc(1, &["Klavir"]),
            "not written back"
        );
        assert_eq!(store.poll(), None, "the same bytes again");
        // Its undo brings back the version before it.
        store.undo().unwrap().unwrap();
        assert_eq!(store.doc(), &doc(7, &["Vox 1"]));
    }

    #[test]
    fn a_bad_hand_edit_keeps_the_last_good_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        std::fs::write(dir.path().join(SECTORS_FILE), "{").unwrap();
        let why = parse(b"{").unwrap_err();
        assert_eq!(store.poll(), Some(Polled::Refused(why.clone())));
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
        assert_eq!(store.error(), Some(why.as_str()));
        assert!(!store.has_prev());
        assert_eq!(store.poll(), None, "refused once");
        write(dir.path(), SECTORS_FILE, &doc(1, &["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.error(), None);
        assert_eq!(store.doc(), &doc(3, &["Bass"]));
    }

    #[test]
    fn a_file_removed_is_missing_and_a_new_one_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(SECTORS_FILE)).unwrap();
        assert_eq!(store.poll(), None);
        assert!(store.missing());
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]), "still served");
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        assert_eq!(store.poll(), Some(Polled::Taken), "the same bytes, back");
        assert!(!store.missing());
        assert_eq!(store.doc(), &doc(3, &["Vox 1"]));
    }

    #[test]
    fn a_file_that_cannot_be_read_keeps_its_bytes_known_and_says_so_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        write(dir.path(), PREV_FILE, &doc(1, &[]));
        let mut store = SectorStore::open(dir.path());
        let path = dir.path().join(SECTORS_FILE);
        let bytes = std::fs::read(&path).unwrap();
        // A read that fails for another reason than a missing file (on the
        // PC an editor or the antivirus holding it): here a folder at its
        // path, which no read takes.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            matches!(store.poll(), Some(Polled::Unreadable(_))),
            "said once"
        );
        assert_eq!(store.poll(), None, "and not again while it lasts");
        assert!(!store.missing());
        // The same bytes back are no change: no new revision, no undo step.
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(store.poll(), None);
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
        assert_eq!(read(dir.path(), PREV_FILE), doc(1, &[]), "the undo kept");
        // A failure after a good read is said again.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(store.poll(), Some(Polled::Unreadable(_))));
    }

    #[test]
    fn undo_swaps_and_a_second_undo_redoes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(1, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        store.commit(doc(2, &["Vox 1", "Bass"])).unwrap();
        store.undo().unwrap().unwrap();
        assert_eq!(store.doc(), &doc(3, &["Vox 1"]));
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(3, &["Vox 1"]));
        assert_eq!(read(dir.path(), PREV_FILE), doc(2, &["Vox 1", "Bass"]));
        store.undo().unwrap().unwrap();
        assert_eq!(store.doc(), &doc(4, &["Vox 1", "Bass"]));
        assert_eq!(read(dir.path(), PREV_FILE), doc(3, &["Vox 1"]));
    }

    #[test]
    fn undo_without_prev_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SectorStore::open(dir.path());
        assert!(matches!(store.undo(), Err(Refusal::NothingToUndo)));
        assert!(!dir.path().join(SECTORS_FILE).exists());
    }

    #[test]
    fn served_rev_is_the_newer_of_the_files_and_one_past_the_served() {
        assert_eq!(served_rev(1, 5), 6);
        assert_eq!(served_rev(9, 5), 9);
        assert_eq!(served_rev(6, 5), 6);
        assert_eq!(served_rev(0, 0), 1);
    }

    #[test]
    fn seed_writes_the_file_with_no_prev() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), PREV_FILE, &doc(7, &["Old"]));
        let mut store = SectorStore::open(dir.path());
        assert!(store.missing());
        store.seed(doc(1, &["Vox 1"])).unwrap();
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(1, &["Vox 1"]));
        assert!(!dir.path().join(PREV_FILE).exists());
        assert!(!store.has_prev());
        assert!(!store.missing());
        assert_eq!(store.doc(), &doc(1, &["Vox 1"]));
        assert_eq!(store.poll(), None, "its own write");
        assert!(matches!(store.undo(), Err(Refusal::NothingToUndo)));
    }

    #[test]
    fn a_write_into_a_missing_folder_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let mut store = SectorStore::open(&gone);
        assert!(store.commit(doc(1, &["Vox 1"])).is_err());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert!(!store.has_prev());
        assert!(store.seed(doc(1, &["Vox 1"])).is_err());
        assert_eq!(store.doc(), &SectorsDoc::empty());
    }
}
