//! The strips' colours on disk (#164): `<data>/colors.json`, read at start
//! and polled with the layout, written atomically (the sectors' file
//! pattern, `crate::sectors`, without its seed and revision: a missing
//! file is the empty document, every strip NONE).
//!
//! A hand edit is taken as it is and not written back. A file that does
//! not parse or check keeps the last good document served (at start: the
//! empty one) and says why ([`ColorStore::error`]). A file removed while
//! the hub runs is written back from the document served, once one was
//! taken from disk or written.

use std::io;
use std::path::{Path, PathBuf};

use fohmixer_proto::colors::ColorsDoc;

/// The document's file in the data folder.
pub const COLORS_FILE: &str = "colors.json";

/// What a poll found: a new document taken, a file refused (why), a file
/// that could not be read or written back (why; said once while it lasts),
/// or a removed file written back.
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
pub struct ColorStore {
    dir: PathBuf,
    doc: ColorsDoc,
    /// The file's bytes as last read or written: a poll that reads them
    /// again takes nothing (the store's own write, a rewrite in place).
    /// None while no file was read or written (a missing file is no change).
    known: Option<Vec<u8>>,
    error: Option<String>,
    /// The last poll failed (its failure was said).
    failing: bool,
}

/// The UTF-8 byte order mark a Windows editor may write first.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// A document's text: one leading [`BOM`] skipped, serde, then its checks
/// (`ColorsDoc::validate`) joined with `; `.
pub fn parse(text: &[u8]) -> Result<ColorsDoc, String> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let doc: ColorsDoc = serde_json::from_slice(text).map_err(|e| e.to_string())?;
    let errors = doc.validate();
    if errors.is_empty() {
        Ok(doc)
    } else {
        Err(errors.join("; "))
    }
}

/// A document as the file holds it.
fn text_of(doc: &ColorsDoc) -> io::Result<String> {
    serde_json::to_string_pretty(doc).map_err(io::Error::other)
}

impl ColorStore {
    /// The store of `data_dir`: its file as read now. A missing file is the
    /// empty document; one that does not parse or check is the empty
    /// document with its error.
    pub fn open(data_dir: &Path) -> Self {
        let mut store = Self {
            dir: data_dir.to_path_buf(),
            doc: ColorsDoc::empty(),
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
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => store.error = Some(e.to_string()),
        }
        store
    }

    fn path(&self) -> PathBuf {
        self.dir.join(COLORS_FILE)
    }

    /// The document served.
    pub fn doc(&self) -> &ColorsDoc {
        &self.doc
    }

    /// Why the file on disk is not the document served.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the file again: a new document (a hand edit) is taken; a bad
    /// one is refused and the last good one stays. Bytes the store already
    /// knows take nothing. A file that cannot be read keeps the bytes
    /// known: [`Polled::Unreadable`] once, then nothing until a read
    /// succeeds.
    pub fn poll(&mut self) -> Option<Polled> {
        let bytes = match std::fs::read(self.path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return self.gone(),
            Err(e) => return self.once(Polled::Unreadable(e.to_string())),
        };
        self.failing = false;
        if self.known.as_deref() == Some(bytes.as_slice()) {
            return None;
        }
        let parsed = parse(&bytes);
        self.known = Some(bytes);
        match parsed {
            Ok(doc) => {
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

    /// No `colors.json`: missing since the start (nothing read or written
    /// yet), nothing to do; removed since, the document served is written
    /// back.
    fn gone(&mut self) -> Option<Polled> {
        self.known.as_ref()?;
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
    pub fn commit(&mut self, next: ColorsDoc) -> io::Result<()> {
        let text = text_of(&next)?;
        crate::atomic_write(&self.path(), &text)?;
        self.doc = next;
        self.known = Some(text.into_bytes());
        self.failing = false;
        self.error = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::colors::ColorKey;
    use fohmixer_proto::sectors::StripRef;

    fn band(label: &str) -> StripRef {
        StripRef {
            instance: "band".into(),
            label: label.into(),
        }
    }

    /// A document with these band faders pink.
    fn doc(labels: &[&str]) -> ColorsDoc {
        labels.iter().fold(ColorsDoc::empty(), |doc, l| {
            doc.with(&band(l), Some(ColorKey::Pink))
        })
    }

    fn write(dir: &Path, doc: &ColorsDoc) {
        std::fs::write(dir.join(COLORS_FILE), serde_json::to_string(doc).unwrap()).unwrap();
    }

    fn read(dir: &Path) -> ColorsDoc {
        serde_json::from_slice(&std::fs::read(dir.join(COLORS_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_file_is_the_empty_document_and_stays_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColorStore::open(dir.path());
        assert_eq!(store.doc(), &ColorsDoc::empty());
        assert_eq!(store.error(), None);
        assert_eq!(store.poll(), None);
        assert!(!dir.path().join(COLORS_FILE).exists(), "nothing written");
    }

    #[test]
    fn open_reads_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        assert_eq!(store.doc(), &doc(&["Vox 1"]));
        assert_eq!(store.error(), None);
        assert_eq!(store.poll(), None, "the file as opened");
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
    fn parse_joins_every_problem_and_reads_past_one_bom() {
        assert_eq!(
            parse(br#"{"colors": {"band": {"": "red", " ": "blue"}}}"#),
            Err(
                r#"a fader of "band" has an empty label; a fader of "band" has an empty label"#
                    .to_string()
            )
        );
        assert_eq!(parse(br#"{"colors": {}}"#), Ok(ColorsDoc::empty()));
        let text = serde_json::to_string(&doc(&["Vox 1"])).unwrap();
        let mut bommed = BOM.to_vec();
        bommed.extend_from_slice(text.as_bytes());
        assert_eq!(parse(&bommed), Ok(doc(&["Vox 1"])));
        let mut twice = BOM.to_vec();
        twice.extend_from_slice(&bommed);
        assert!(parse(&twice).is_err());
    }

    #[test]
    fn commit_writes_and_serves_the_change_and_its_own_write_is_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColorStore::open(dir.path());
        store.commit(doc(&["Vox 1", "Bass"])).unwrap();
        assert_eq!(read(dir.path()), doc(&["Vox 1", "Bass"]));
        assert_eq!(store.doc(), &doc(&["Vox 1", "Bass"]));
        assert_eq!(store.poll(), None);
        assert!(!dir.path().join("colors.tmp").exists());
    }

    #[test]
    fn a_hand_edit_is_taken_and_a_bad_one_keeps_the_last_good_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        write(dir.path(), &doc(&["Bass"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.doc(), &doc(&["Bass"]));
        assert_eq!(read(dir.path()), doc(&["Bass"]), "not written back");
        assert_eq!(store.poll(), None, "the same bytes again");
        std::fs::write(dir.path().join(COLORS_FILE), "{").unwrap();
        let why = parse(b"{").unwrap_err();
        assert_eq!(store.poll(), Some(Polled::Refused(why.clone())));
        assert_eq!(store.doc(), &doc(&["Bass"]));
        assert_eq!(store.error(), Some(why.as_str()));
        assert_eq!(store.poll(), None, "refused once");
        write(dir.path(), &doc(&["Keys"]));
        assert_eq!(store.poll(), Some(Polled::Taken));
        assert_eq!(store.error(), None);
    }

    #[test]
    fn a_file_removed_is_written_back_from_the_served_document() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(COLORS_FILE)).unwrap();
        assert_eq!(store.poll(), Some(Polled::Restored));
        assert_eq!(read(dir.path()), doc(&["Vox 1"]));
        assert_eq!(store.poll(), None, "its own write");
    }

    #[test]
    fn a_file_that_cannot_be_read_says_so_once_and_keeps_its_bytes_known() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        let path = dir.path().join(COLORS_FILE);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(store.poll(), Some(Polled::Unreadable(_))));
        assert_eq!(store.poll(), None, "not again while it lasts");
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(store.poll(), None, "the same bytes: no change");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            matches!(store.poll(), Some(Polled::Unreadable(_))),
            "said again after a good read"
        );
    }

    #[test]
    fn a_removed_file_that_cannot_be_written_back_is_unwritable_once() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        std::fs::remove_file(dir.path().join(COLORS_FILE)).unwrap();
        let folder = dir.path().to_path_buf();
        drop(dir);
        assert!(!folder.exists());
        assert!(matches!(store.poll(), Some(Polled::Unwritable(_))));
        assert_eq!(store.poll(), None, "said once");
    }

    #[test]
    fn a_write_into_a_missing_folder_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColorStore::open(&dir.path().join("gone"));
        assert!(store.commit(doc(&["Vox 1"])).is_err());
        assert_eq!(store.doc(), &ColorsDoc::empty());
    }

    /// A file that is there but cannot be read is never taken as removed:
    /// nothing is written over it (`poll`'s `NotFound` guard).
    #[cfg(unix)]
    #[test]
    fn a_file_refused_by_its_mode_is_unreadable_and_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &doc(&["Vox 1"]));
        let mut store = ColorStore::open(dir.path());
        let path = dir.path().join(COLORS_FILE);
        let hand = serde_json::to_string(&doc(&["Vox 2"])).unwrap();
        std::fs::write(&path, &hand).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let polled = store.poll();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(polled, Some(Polled::Unreadable(_))), "{polled:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), hand);
        assert_eq!(store.doc(), &doc(&["Vox 1"]));
    }

    /// A file unreadable at start (another reason than a missing one) is
    /// served as the empty document with its error.
    #[cfg(unix)]
    #[test]
    fn a_file_unreadable_at_start_is_its_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(COLORS_FILE)).unwrap();
        let store = ColorStore::open(dir.path());
        assert_eq!(store.doc(), &ColorsDoc::empty());
        assert!(store.error().is_some());
    }
}
