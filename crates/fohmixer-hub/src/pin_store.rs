//! The engineer PIN's argon2id hash: `<data>/secrets/pin_hashes.json`
//! (from iemmixer's `iem-server/src/pin_store.rs` @ 22372bc, trimmed to the
//! engineer role: fohmixer has no member logins).
//!
//! Never plaintext: the stored value must be an argon2id PHC string, and a
//! file that holds anything else is a load error instead of being ignored.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name inside the secrets directory.
pub const PIN_HASHES_FILE: &str = "pin_hashes.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PinFile {
    #[serde(default)]
    engineer: Option<String>,
}

/// The engineer PIN hash, persisted atomically.
#[derive(Debug)]
pub struct PinStore {
    file: PinFile,
    path: PathBuf,
}

fn check_phc(phc: &str) -> io::Result<()> {
    if phc.starts_with("$argon2id$") {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the engineer PIN entry is not an argon2id hash",
        ))
    }
}

impl PinStore {
    /// Load from `secrets_dir` (empty when the file does not exist yet).
    pub fn load(secrets_dir: &Path) -> io::Result<Self> {
        let path = secrets_dir.join(PIN_HASHES_FILE);
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<PinFile>(&text).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => PinFile::default(),
            Err(e) => return Err(e),
        };
        if let Some(phc) = &file.engineer {
            check_phc(phc)?;
        }
        Ok(Self { file, path })
    }

    pub fn engineer_hash(&self) -> Option<&str> {
        self.file.engineer.as_deref()
    }

    /// Whether a PIN hash is stored.
    pub fn has_hashes(&self) -> bool {
        self.file.engineer.is_some()
    }

    pub fn set_engineer_hash(&mut self, phc: String) -> io::Result<()> {
        check_phc(&phc)?;
        self.file.engineer = Some(phc);
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.file).map_err(io::Error::other)?;
        crate::atomic_write(&self.path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin_hash::{PEPPER_LEN, PinHasher};

    fn hasher() -> PinHasher {
        PinHasher::for_tests([1u8; PEPPER_LEN])
    }

    #[test]
    fn an_empty_directory_has_no_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = PinStore::load(dir.path()).unwrap();
        assert!(store.engineer_hash().is_none());
        assert!(!store.has_hashes());
    }

    #[test]
    fn the_hash_survives_a_reload_and_never_contains_the_pin() {
        let dir = tempfile::tempdir().unwrap();
        let h = hasher();
        {
            let mut store = PinStore::load(dir.path()).unwrap();
            store.set_engineer_hash(h.hash("2468")).unwrap();
            assert!(store.has_hashes());
        }
        let store = PinStore::load(dir.path()).unwrap();
        assert!(h.verify("2468", store.engineer_hash().unwrap()));
        let text = std::fs::read_to_string(dir.path().join(PIN_HASHES_FILE)).unwrap();
        assert!(!text.contains("\"2468\""));
    }

    #[test]
    fn a_plaintext_value_is_a_load_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PIN_HASHES_FILE), r#"{"engineer":"2468"}"#).unwrap();
        assert_eq!(
            PinStore::load(dir.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn corrupt_json_is_a_load_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PIN_HASHES_FILE), "{not json").unwrap();
        assert_eq!(
            PinStore::load(dir.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn an_unreadable_store_is_a_load_error_not_an_empty_store() {
        // Only a missing file means "no PIN yet".
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(PIN_HASHES_FILE)).unwrap();
        assert!(PinStore::load(dir.path()).is_err());
    }

    #[test]
    fn a_non_phc_value_is_refused_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PinStore::load(dir.path()).unwrap();
        assert!(store.set_engineer_hash("2468".to_string()).is_err());
        assert!(!dir.path().join(PIN_HASHES_FILE).exists());
    }

    #[test]
    fn saving_creates_the_secrets_folder() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join("secrets");
        let mut store = PinStore::load(&secrets).unwrap();
        store.set_engineer_hash(hasher().hash("1357")).unwrap();
        assert!(secrets.join(PIN_HASHES_FILE).is_file());
    }
}
