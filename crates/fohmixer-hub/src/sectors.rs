//! The fader sectors' document on disk (#148): `<data>/sectors.json`,
//! read at start and polled with the layout, written atomically.
//!
//! The hub owns the revision: every change it makes is `rev + 1`, written
//! into the file. A hand edit is served at [`served_rev`] and not written
//! back. A file that does not parse or check keeps the last good document
//! served (at start: the empty one, sectors off) and says why
//! ([`SectorStore::error`]). There is no undo (the owner, 29.10.2026): an
//! older hub's `sectors.json.prev` left in the folder is removed at start.

use std::io;
use std::path::{Path, PathBuf};

use fohmixer_proto::sectors::SectorsDoc;

/// The document's file in the data folder.
pub const SECTORS_FILE: &str = "sectors.json";
/// The undo file an older hub kept (removed at start).
pub const OLD_PREV_FILE: &str = "sectors.json.prev";

/// What a poll found: a new document taken, a file refused (why), or a
/// file that could not be read (why; said once while it lasts).
#[derive(Debug, Clone, PartialEq)]
pub enum Polled {
    Taken,
    Refused(String),
    Unreadable(String),
    /// The file was removed and could not be written back (why; said once
    /// while it lasts).
    Unwritable(String),
    /// The file was removed and is written back from the document served.
    Restored,
}

/// The document as served, and what the file on disk held when the store
/// last read or wrote it.
#[derive(Debug)]
pub struct SectorStore {
    dir: PathBuf,
    doc: SectorsDoc,
    /// No `sectors.json` on disk since the start (the seed may write the
    /// first one); a file removed later is written back, never seeded.
    missing: bool,
    /// The file's bytes as last read or written: a poll that reads them
    /// again takes nothing (the store's own write, a rewrite in place).
    known: Option<Vec<u8>>,
    error: Option<String>,
    /// The last poll could not read the file (its failure was said).
    failing: bool,
}

/// The UTF-8 byte order mark a Windows editor may write first (Windows
/// PowerShell 5's `Set-Content -Encoding UTF8` does).
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// A document's text: one leading [`BOM`] skipped, serde, then its checks
/// (`SectorsDoc::validate`) joined with `; `; a layout's trailing empty
/// slots are dropped (`SectorsDoc::trimmed`).
pub fn parse(text: &[u8]) -> Result<SectorsDoc, String> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let doc: SectorsDoc = serde_json::from_slice(text).map_err(|e| e.to_string())?;
    let errors = doc.validate();
    if errors.is_empty() {
        Ok(doc.trimmed())
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
    /// The store of `data_dir`: its file as read now. A missing file is the
    /// empty document; one that does not parse or check is the empty
    /// document with its error. An older hub's undo file is removed.
    pub fn open(data_dir: &Path) -> Self {
        let mut store = Self {
            dir: data_dir.to_path_buf(),
            doc: SectorsDoc::empty(),
            missing: false,
            known: None,
            error: None,
            failing: false,
        };
        if let Some(e) = removal_failed(std::fs::remove_file(store.path(OLD_PREV_FILE))) {
            tracing::warn!(error = %e, "cannot remove an older hub's sectors.json.prev");
        }
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

    /// Whether no `sectors.json` is on disk.
    pub fn missing(&self) -> bool {
        self.missing
    }

    /// Why the file on disk is not the document served.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the file again: a new document (a hand edit) is taken at
    /// [`served_rev`]; a bad one is refused and the last good one stays. Bytes the store
    /// already knows take nothing. A file missing since the start waits for
    /// the seed; one removed later is written back from the document served
    /// ([`Polled::Restored`]: after the seed only the pages change it). A
    /// file that cannot be read (on the PC another program holding it)
    /// keeps the bytes known, so the same bytes read later are no change:
    /// [`Polled::Unreadable`] once, then nothing until a read succeeds.
    pub fn poll(&mut self) -> Option<Polled> {
        let bytes = match std::fs::read(self.path(SECTORS_FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return self.gone(),
            Err(e) => return self.unreadable(e.to_string()),
        };
        self.failing = false;
        self.missing = false;
        if self.known.as_deref() == Some(bytes.as_slice()) {
            return None;
        }
        let parsed = parse(&bytes);
        self.known = Some(bytes);
        match parsed {
            Ok(mut doc) => {
                doc.rev = served_rev(doc.rev, self.doc.rev);
                self.doc = doc;
                self.error = None;
                Some(Polled::Taken)
            }
            Err(why) => {
                self.error = Some(why.clone());
                Some(Polled::Refused(why))
            }
        }
    }

    /// No `sectors.json`: missing since the start (nothing read or written
    /// yet), it waits for the seed; removed since, the document served is
    /// written back.
    fn gone(&mut self) -> Option<Polled> {
        self.known.as_ref()?;
        if never_good(self.error.is_some(), &self.doc) {
            // Nothing good was ever read: the removal is a missing file,
            // which the seed may fill.
            self.known = None;
            self.missing = true;
            return None;
        }
        let written = text_of(&self.doc).and_then(|text| {
            crate::atomic_write(&self.path(SECTORS_FILE), &text)?;
            Ok(text)
        });
        match written {
            Ok(text) => {
                self.known = Some(text.into_bytes());
                self.failing = false;
                Some(Polled::Restored)
            }
            Err(e) => self.once(Polled::Unwritable(e.to_string())),
        }
    }

    /// A read or write of the file failed (`why`): said only when the
    /// last poll did not fail too.
    fn unreadable(&mut self, why: String) -> Option<Polled> {
        self.once(Polled::Unreadable(why))
    }

    /// A failure (`polled`), said only when the last poll did not fail too.
    fn once(&mut self, polled: Polled) -> Option<Polled> {
        let first = !self.failing;
        self.failing = true;
        first.then_some(polled)
    }

    /// A change: `next` is written and served. A failed write changes
    /// nothing.
    pub fn commit(&mut self, next: SectorsDoc) -> io::Result<()> {
        let text = text_of(&next)?;
        crate::atomic_write(&self.path(SECTORS_FILE), &text)?;
        self.doc = next;
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.failing = false;
        self.error = None;
        Ok(())
    }

    /// The first document: written and served.
    pub fn seed(&mut self, seeded: SectorsDoc) -> io::Result<()> {
        let text = text_of(&seeded)?;
        crate::atomic_write(&self.path(SECTORS_FILE), &text)?;
        self.doc = seeded;
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.error = None;
        Ok(())
    }
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
    fn a_missing_file_is_the_empty_document_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert!(store.missing());
        assert_eq!(store.error(), None);
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
    fn parse_reads_past_one_leading_utf8_bom() {
        // Windows PowerShell 5's `Set-Content -Encoding UTF8` writes one.
        let text = serde_json::to_string(&doc(2, &["Vox 1"])).unwrap();
        let mut bommed = b"\xEF\xBB\xBF".to_vec();
        bommed.extend_from_slice(text.as_bytes());
        assert_eq!(parse(&bommed), Ok(doc(2, &["Vox 1"])));
        // Only one: a second is no JSON.
        let mut twice = b"\xEF\xBB\xBF".to_vec();
        twice.extend_from_slice(&bommed);
        assert!(parse(&twice).is_err());
        // A file with a BOM opens as the document it holds.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SECTORS_FILE), &bommed).unwrap();
        let store = SectorStore::open(dir.path());
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
        assert_eq!(store.error(), None);
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
    fn a_bad_hand_edit_keeps_the_last_good_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        std::fs::write(dir.path().join(SECTORS_FILE), "{").unwrap();
        let why = parse(b"{").unwrap_err();
        assert_eq!(store.poll(), Some(Polled::Refused(why.clone())));
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
        assert_eq!(store.error(), Some(why.as_str()));
        assert_eq!(store.poll(), None, "refused once");
        write(dir.path(), SECTORS_FILE, &doc(1, &["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.error(), None);
        assert_eq!(store.doc(), &doc(3, &["Bass"]));
    }

    #[test]
    fn a_file_removed_is_written_back_from_the_served_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(SECTORS_FILE)).unwrap();
        // After the seed only the pages change the document (spec §2): the
        // one served goes back on disk, nothing seeded.
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert!(!store.missing(), "no seed over the owner's arrangement");
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(2, &["Vox 1"]));
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]), "still served");
        assert_eq!(store.poll(), None, "its own write");
        // A new file after it is a hand edit as any.
        write(dir.path(), SECTORS_FILE, &doc(2, &["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(3, &["Bass"]));
    }

    #[test]
    fn a_file_missing_at_start_stays_missing_until_the_seed() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SectorStore::open(dir.path());
        assert_eq!(store.poll(), None);
        assert!(store.missing());
        assert!(!dir.path().join(SECTORS_FILE).exists(), "nothing written");
    }

    #[test]
    fn a_file_that_cannot_be_read_keeps_its_bytes_known_and_says_so_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
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
        // The same bytes back are no change: no new revision.
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(store.poll(), None);
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
        // A failure after a good read is said again.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(store.poll(), Some(Polled::Unreadable(_))));
    }

    #[test]
    fn served_rev_is_the_newer_of_the_files_and_one_past_the_served() {
        assert_eq!(served_rev(1, 5), 6);
        assert_eq!(served_rev(9, 5), 9);
        assert_eq!(served_rev(6, 5), 6);
        assert_eq!(served_rev(0, 0), 1);
    }

    #[test]
    fn seed_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SectorStore::open(dir.path());
        assert!(store.missing());
        store.seed(doc(1, &["Vox 1"])).unwrap();
        assert_eq!(read(dir.path(), SECTORS_FILE), doc(1, &["Vox 1"]));
        assert!(!store.missing());
        assert_eq!(store.doc(), &doc(1, &["Vox 1"]));
        assert_eq!(store.poll(), None, "its own write");
    }

    #[test]
    fn a_write_into_a_missing_folder_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let mut store = SectorStore::open(&gone);
        assert!(store.commit(doc(1, &["Vox 1"])).is_err());
        assert_eq!(store.doc(), &SectorsDoc::empty());
        assert!(store.seed(doc(1, &["Vox 1"])).is_err());
        assert_eq!(store.doc(), &SectorsDoc::empty());
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

    /// A file that is there but cannot be read is never taken as removed:
    /// nothing is written over it (the mutation gate's survivor of #148,
    /// `poll`'s `NotFound` guard). Unix only: the read is refused by the
    /// file's mode, which the test's own (non-root) user honours.
    #[cfg(unix)]
    #[test]
    fn a_file_refused_by_its_mode_is_unreadable_and_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        let path = dir.path().join(SECTORS_FILE);
        let hand = serde_json::to_string(&doc(9, &["Vox 2"])).unwrap();
        std::fs::write(&path, &hand).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let polled = store.poll();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(polled, Some(Polled::Unreadable(_))), "{polled:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            hand,
            "not written over"
        );
        assert_eq!(store.doc(), &doc(2, &["Vox 1"]));
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
    fn never_good_only_when_refused_and_serving_the_empty_document() {
        assert!(never_good(true, &SectorsDoc::empty()));
        assert!(!never_good(false, &SectorsDoc::empty()));
        assert!(!never_good(true, &doc(2, &["Vox 1"])));
    }

    #[test]
    fn a_removed_file_that_cannot_be_written_back_is_unwritable_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SECTORS_FILE);
        write(dir.path(), SECTORS_FILE, &doc(2, &["Vox 1"]));
        let mut store = SectorStore::open(dir.path());
        // The write back needs the folder: gone, every write fails.
        std::fs::remove_file(&path).unwrap();
        let folder = dir.path().to_path_buf();
        drop(dir);
        assert!(!folder.exists());
        assert!(matches!(store.poll(), Some(Polled::Unwritable(_))));
        assert_eq!(store.poll(), None, "said once");
    }
}
