//! One JSON document in the data folder, read at start and polled with the
//! layout, written atomically (#164): the mechanics the fader sectors
//! (`crate::sectors`) and the strips' colours (`crate::colors`) share.
//!
//! A hand edit is taken (as [`Document::hand_edit`] makes it) and not
//! written back. A file that does not parse or check keeps the last good
//! document served (at start: the empty one) and says why
//! ([`DocStore::error`]). The poll compares the file's BYTES with the last
//! read or written, so the store's own writes are never taken as a hand
//! edit. Only `NotFound` means the file is gone: any other read error keeps
//! the bytes known and is said once while it lasts. A file removed while
//! the hub runs is written back from the document served.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A document a [`DocStore`] keeps: its file and its checks.
pub trait Document: Serialize + DeserializeOwned + Sized {
    /// The document's file in the data folder.
    const FILE: &'static str;

    /// The document served while no good file was read.
    fn empty() -> Self;

    /// Its checks' problems (none: it checks).
    fn problems(&self) -> Vec<String>;

    /// The document as served once it checked.
    fn served(self) -> Self {
        self
    }

    /// A hand edit as served after `before`, the document served until now.
    fn hand_edit(self, _before: &Self) -> Self {
        self
    }

    /// Whether a removed file counts as never there: nothing good was read
    /// (the file read was refused, `refused`, and `served` stands in), so
    /// it is missing again rather than written back.
    fn never_good(_refused: bool, _served: &Self) -> bool {
        false
    }

    /// Done in the data folder before the file is read at start.
    fn on_open(_dir: &Path) {}
}

/// What a poll found: a new document taken, a file refused (why), a file
/// that could not be read (why; said once while it lasts), a removed file
/// that could not be written back (why; said once while it lasts), or a
/// removed file written back from the document served.
#[derive(Debug, Clone, PartialEq)]
pub enum Polled {
    Taken,
    Refused(String),
    Unreadable(String),
    Unwritable(String),
    Restored,
}

/// The document as served, and what the file on disk held when the store
/// last read or wrote it.
#[derive(Debug)]
pub struct DocStore<D> {
    dir: PathBuf,
    doc: D,
    /// No file on disk since the start (or since a never-good file went);
    /// a file removed after a good one is written back instead.
    missing: bool,
    /// The file's bytes as last read or written: a poll that reads them
    /// again takes nothing (the store's own write, a rewrite in place).
    /// None while no file was read or written (a missing file is no change).
    known: Option<Vec<u8>>,
    error: Option<String>,
    /// The last poll failed (its failure was said).
    failing: bool,
}

/// The UTF-8 byte order mark a Windows editor may write first (Windows
/// PowerShell 5's `Set-Content -Encoding UTF8` does).
pub const BOM: &[u8] = b"\xEF\xBB\xBF";

/// A document's text: one leading [`BOM`] skipped, serde, then its checks
/// ([`Document::problems`]) joined with `; `, then [`Document::served`].
pub fn parse<D: Document>(text: &[u8]) -> Result<D, String> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let doc: D = serde_json::from_slice(text).map_err(|e| e.to_string())?;
    let problems = doc.problems();
    if problems.is_empty() {
        Ok(doc.served())
    } else {
        Err(problems.join("; "))
    }
}

/// A document as the file holds it.
fn text_of<D: Document>(doc: &D) -> io::Result<String> {
    serde_json::to_string_pretty(doc).map_err(io::Error::other)
}

impl<D: Document> DocStore<D> {
    /// The store of `data_dir`: its file as read now. A missing file is the
    /// empty document; one that does not parse or check is the empty
    /// document with its error.
    pub fn open(data_dir: &Path) -> Self {
        D::on_open(data_dir);
        let mut store = Self {
            dir: data_dir.to_path_buf(),
            doc: D::empty(),
            missing: false,
            known: None,
            error: None,
            failing: false,
        };
        match std::fs::read(store.path()) {
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

    fn path(&self) -> PathBuf {
        self.dir.join(D::FILE)
    }

    /// The document served.
    pub fn doc(&self) -> &D {
        &self.doc
    }

    /// Whether no file is on disk (missing since the start, or a never-good
    /// one removed).
    pub fn missing(&self) -> bool {
        self.missing
    }

    /// Why the file on disk is not the document served.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the file again: a new document (a hand edit) is taken as
    /// [`Document::hand_edit`] makes it; a bad one is refused and the last
    /// good one stays. Bytes the store already knows take nothing. A file
    /// missing since the start stays missing; one removed later is written
    /// back from the document served ([`Polled::Restored`]). A file that
    /// cannot be read (on the PC another program holding it) keeps the
    /// bytes known, so the same bytes read later are no change:
    /// [`Polled::Unreadable`] once, then nothing until a read succeeds.
    pub fn poll(&mut self) -> Option<Polled> {
        let bytes = match std::fs::read(self.path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return self.gone(),
            Err(e) => return self.once(Polled::Unreadable(e.to_string())),
        };
        self.failing = false;
        self.missing = false;
        if self.known.as_deref() == Some(bytes.as_slice()) {
            return None;
        }
        let parsed = parse::<D>(&bytes);
        self.known = Some(bytes);
        match parsed {
            Ok(doc) => {
                self.doc = doc.hand_edit(&self.doc);
                self.error = None;
                Some(Polled::Taken)
            }
            Err(why) => {
                self.error = Some(why.clone());
                Some(Polled::Refused(why))
            }
        }
    }

    /// No file: missing since the start (nothing read or written yet),
    /// nothing to do; removed since, the document served is written back,
    /// unless nothing good was ever read ([`Document::never_good`]): then
    /// it is missing again.
    fn gone(&mut self) -> Option<Polled> {
        self.known.as_ref()?;
        if D::never_good(self.error.is_some(), &self.doc) {
            self.known = None;
            self.missing = true;
            return None;
        }
        let written = text_of(&self.doc).and_then(|text| {
            crate::atomic_write(&self.path(), &text)?;
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

    /// A failure (`polled`), said only when the last poll did not fail too.
    fn once(&mut self, polled: Polled) -> Option<Polled> {
        let first = !self.failing;
        self.failing = true;
        first.then_some(polled)
    }

    /// A change: `next` is written and served. A failed write changes
    /// nothing.
    pub fn commit(&mut self, next: D) -> io::Result<()> {
        let text = text_of(&next)?;
        crate::atomic_write(&self.path(), &text)?;
        self.doc = next;
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.failing = false;
        self.error = None;
        Ok(())
    }

    /// The first document: written and served. A failed write changes
    /// nothing.
    pub fn seed(&mut self, seeded: D) -> io::Result<()> {
        let text = text_of(&seeded)?;
        crate::atomic_write(&self.path(), &text)?;
        self.doc = seeded;
        self.known = Some(text.into_bytes());
        self.missing = false;
        self.error = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// A document of the defaults' hooks: a list of words, none empty.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Words {
        words: Vec<String>,
    }

    impl Document for Words {
        const FILE: &'static str = "words.json";

        fn empty() -> Self {
            Words { words: Vec::new() }
        }

        fn problems(&self) -> Vec<String> {
            self.words
                .iter()
                .filter(|w| w.is_empty())
                .map(|_| "an empty word".to_string())
                .collect()
        }
    }

    type Store = DocStore<Words>;

    fn doc(words: &[&str]) -> Words {
        Words {
            words: words.iter().map(|w| (*w).to_string()).collect(),
        }
    }

    fn write(dir: &Path, doc: &Words) {
        std::fs::write(dir.join(Words::FILE), serde_json::to_string(doc).unwrap()).unwrap();
    }

    fn read(dir: &Path) -> Words {
        serde_json::from_slice(&std::fs::read(dir.join(Words::FILE)).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_file_is_the_empty_document_and_stays_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        assert_eq!(store.doc(), &Words::empty());
        assert!(store.missing());
        assert_eq!(store.error(), None);
        assert_eq!(store.poll(), None);
        assert!(store.missing());
        assert!(!dir.path().join(Words::FILE).exists(), "nothing written");
        // A file that comes is a hand edit, and the store is no longer
        // missing.
        write(dir.path(), &doc(&["Vox"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert!(!store.missing());
        assert_eq!(store.doc(), &doc(&["Vox"]));
    }

    #[test]
    fn open_reads_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        assert_eq!(store.doc(), &doc(&["Vox"]));
        assert!(!store.missing());
        assert_eq!(store.error(), None);
        assert_eq!(store.poll(), None, "the file as opened");
    }

    #[test]
    fn a_bad_file_at_start_serves_the_empty_document_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(Words::FILE), "{").unwrap();
        let store = Store::open(dir.path());
        assert_eq!(store.doc(), &Words::empty());
        assert_eq!(
            store.error(),
            Some(parse::<Words>(b"{").unwrap_err().as_str())
        );
        assert!(!store.missing(), "a file is there");
        // A document that parses but does not check is refused too.
        write(dir.path(), &doc(&["Vox", ""]));
        let store = Store::open(dir.path());
        assert_eq!(store.doc(), &Words::empty());
        assert_eq!(store.error(), Some("an empty word"));
    }

    /// A file unreadable at start (another reason than a missing one) is
    /// served as the empty document with its error, and is not missing.
    #[cfg(unix)]
    #[test]
    fn a_file_unreadable_at_start_is_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(Words::FILE)).unwrap();
        let store = Store::open(dir.path());
        assert_eq!(store.doc(), &Words::empty());
        assert!(store.error().is_some());
        assert!(!store.missing());
    }

    #[test]
    fn parse_joins_every_problem_and_reads_past_one_bom() {
        assert_eq!(
            parse::<Words>(br#"{"words": ["", "Vox", ""]}"#),
            Err("an empty word; an empty word".to_string())
        );
        assert_eq!(parse::<Words>(br#"{"words": []}"#), Ok(Words::empty()));
        let text = serde_json::to_string(&doc(&["Vox"])).unwrap();
        let mut bommed = BOM.to_vec();
        bommed.extend_from_slice(text.as_bytes());
        assert_eq!(parse::<Words>(&bommed), Ok(doc(&["Vox"])));
        // Only one: a second is no JSON.
        let mut twice = BOM.to_vec();
        twice.extend_from_slice(&bommed);
        assert!(parse::<Words>(&twice).is_err());
        // A file with a BOM opens as the document it holds.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(Words::FILE), &bommed).unwrap();
        let store = Store::open(dir.path());
        assert_eq!(store.doc(), &doc(&["Vox"]));
        assert_eq!(store.error(), None);
    }

    #[test]
    fn commit_writes_and_serves_the_change_and_its_own_write_is_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        assert!(store.missing());
        store.commit(doc(&["Vox", "Bass"])).unwrap();
        assert!(!store.missing());
        assert_eq!(read(dir.path()), doc(&["Vox", "Bass"]));
        assert_eq!(store.doc(), &doc(&["Vox", "Bass"]));
        assert_eq!(store.poll(), None);
        assert!(!dir.path().join("words.tmp").exists());
    }

    #[test]
    fn a_commit_clears_the_error_of_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(Words::FILE), "{").unwrap();
        let mut store = Store::open(dir.path());
        assert!(store.error().is_some());
        store.commit(doc(&["Vox"])).unwrap();
        assert_eq!(store.error(), None);
    }

    #[test]
    fn a_hand_edit_is_taken_and_not_written_back() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        write(dir.path(), &doc(&["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(&["Bass"]));
        assert_eq!(read(dir.path()), doc(&["Bass"]), "not written back");
        assert_eq!(store.poll(), None, "the same bytes again");
    }

    #[test]
    fn a_bad_hand_edit_keeps_the_last_good_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        std::fs::write(dir.path().join(Words::FILE), "{").unwrap();
        let why = parse::<Words>(b"{").unwrap_err();
        assert_eq!(store.poll(), Some(Polled::Refused(why.clone())));
        assert_eq!(store.doc(), &doc(&["Vox"]));
        assert_eq!(store.error(), Some(why.as_str()));
        assert_eq!(store.poll(), None, "refused once");
        write(dir.path(), &doc(&["Keys"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.error(), None);
        assert_eq!(store.doc(), &doc(&["Keys"]));
    }

    #[test]
    fn a_file_removed_is_written_back_from_the_served_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        std::fs::remove_file(dir.path().join(Words::FILE)).unwrap();
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert!(!store.missing());
        assert_eq!(read(dir.path()), doc(&["Vox"]));
        assert_eq!(store.doc(), &doc(&["Vox"]), "still served");
        assert_eq!(store.poll(), None, "its own write");
        // A new file after it is a hand edit as any.
        write(dir.path(), &doc(&["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(&["Bass"]));
    }

    /// The default [`Document::never_good`]: a bad file at start, then
    /// removed, is written back as the empty document served.
    #[test]
    fn a_bad_file_at_start_then_removed_is_written_back_by_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(Words::FILE), "{").unwrap();
        let mut store = Store::open(dir.path());
        std::fs::remove_file(dir.path().join(Words::FILE)).unwrap();
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert!(!store.missing());
        assert_eq!(read(dir.path()), Words::empty());
    }

    #[test]
    fn a_file_that_cannot_be_read_keeps_its_bytes_known_and_says_so_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        let path = dir.path().join(Words::FILE);
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
        // The same bytes back are no change.
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(store.poll(), None);
        assert_eq!(store.doc(), &doc(&["Vox"]));
        // A failure after a good read is said again.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(store.poll(), Some(Polled::Unreadable(_))));
    }

    #[test]
    fn a_removed_file_that_cannot_be_written_back_is_unwritable_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        // The write back needs the folder: gone, every write fails.
        std::fs::remove_file(dir.path().join(Words::FILE)).unwrap();
        let folder = dir.path().to_path_buf();
        drop(dir);
        assert!(!folder.exists());
        assert!(matches!(store.poll(), Some(Polled::Unwritable(_))));
        assert_eq!(store.poll(), None, "said once");
    }

    #[test]
    fn a_write_into_a_missing_folder_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("gone"));
        assert!(store.commit(doc(&["Vox"])).is_err());
        assert_eq!(store.doc(), &Words::empty());
        assert!(store.seed(doc(&["Vox"])).is_err());
        assert_eq!(store.doc(), &Words::empty());
        assert!(store.missing());
    }

    #[test]
    fn seed_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        assert!(store.missing());
        store.seed(doc(&["Vox"])).unwrap();
        assert_eq!(read(dir.path()), doc(&["Vox"]));
        assert!(!store.missing());
        assert_eq!(store.doc(), &doc(&["Vox"]));
        assert_eq!(store.poll(), None, "its own write");
        // A seed over a refused file clears its error.
        std::fs::write(dir.path().join(Words::FILE), "{").unwrap();
        let mut store = Store::open(dir.path());
        assert!(store.error().is_some());
        store.seed(doc(&["Vox"])).unwrap();
        assert_eq!(store.error(), None);
    }

    /// A file that is there but cannot be read is never taken as removed:
    /// nothing is written over it (`poll`'s `NotFound` guard). Unix only:
    /// the read is refused by the file's mode, which the test's own
    /// (non-root) user honours.
    #[cfg(unix)]
    #[test]
    fn a_file_refused_by_its_mode_is_unreadable_and_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox"]));
        let mut store = Store::open(dir.path());
        let path = dir.path().join(Words::FILE);
        let hand = serde_json::to_string(&doc(&["Vox 2"])).unwrap();
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
        assert_eq!(store.doc(), &doc(&["Vox"]));
    }
}
