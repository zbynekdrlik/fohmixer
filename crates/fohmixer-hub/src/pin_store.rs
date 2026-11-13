//! The PINs' argon2id hashes: `<data>/secrets/pin_hashes.json` (from
//! iemmixer's `iem-server/src/pin_store.rs` @ 22372bc): the engineer's and,
//! since #184, the MBC operator's (`fohmixer_proto::roles::Role`). A file
//! of an older hub holds the engineer's only.
//!
//! Never plaintext: the stored value must be an argon2id PHC string, and a
//! file that holds anything else is a load error instead of being ignored.

use std::io;
use std::path::{Path, PathBuf};

use fohmixer_proto::roles::Role;
use serde::{Deserialize, Serialize};

/// File name inside the secrets directory.
pub const PIN_HASHES_FILE: &str = "pin_hashes.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PinFile {
    #[serde(default)]
    engineer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mbc: Option<String>,
}

impl PinFile {
    fn entry(&self, role: Role) -> &Option<String> {
        match role {
            Role::Engineer => &self.engineer,
            Role::Mbc => &self.mbc,
        }
    }

    fn entry_mut(&mut self, role: Role) -> &mut Option<String> {
        match role {
            Role::Engineer => &mut self.engineer,
            Role::Mbc => &mut self.mbc,
        }
    }
}

/// The PIN hashes, persisted atomically.
#[derive(Debug)]
pub struct PinStore {
    file: PinFile,
    path: PathBuf,
}

fn check_phc(role: Role, phc: &str) -> io::Result<()> {
    if phc.starts_with("$argon2id$") {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the {} PIN entry is not an argon2id hash", role.as_str()),
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
        for role in [Role::Engineer, Role::Mbc] {
            if let Some(phc) = file.entry(role) {
                check_phc(role, phc)?;
            }
        }
        Ok(Self { file, path })
    }

    /// The hash of `role`'s PIN.
    pub fn hash(&self, role: Role) -> Option<&str> {
        self.file.entry(role).as_deref()
    }

    pub fn engineer_hash(&self) -> Option<&str> {
        self.hash(Role::Engineer)
    }

    /// Whether a PIN hash is stored (either role's).
    pub fn has_hashes(&self) -> bool {
        self.file.engineer.is_some() || self.file.mbc.is_some()
    }

    /// Stores `phc` as `role`'s PIN hash.
    pub fn set_hash(&mut self, role: Role, phc: String) -> io::Result<()> {
        check_phc(role, &phc)?;
        *self.file.entry_mut(role) = Some(phc);
        self.save()
    }

    pub fn set_engineer_hash(&mut self, phc: String) -> io::Result<()> {
        self.set_hash(Role::Engineer, phc)
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
    fn the_mbc_hash_survives_a_reload_beside_the_engineers() {
        let dir = tempfile::tempdir().unwrap();
        let h = hasher();
        {
            let mut store = PinStore::load(dir.path()).unwrap();
            store.set_hash(Role::Mbc, h.hash("1357")).unwrap();
            assert!(store.has_hashes(), "the MBC PIN alone is a stored PIN");
            assert!(store.engineer_hash().is_none());
            store.set_engineer_hash(h.hash("2468")).unwrap();
        }
        let store = PinStore::load(dir.path()).unwrap();
        assert!(h.verify("1357", store.hash(Role::Mbc).unwrap()));
        assert!(h.verify("2468", store.hash(Role::Engineer).unwrap()));
        assert!(!h.verify("2468", store.hash(Role::Mbc).unwrap()));
        let text = std::fs::read_to_string(dir.path().join(PIN_HASHES_FILE)).unwrap();
        assert!(!text.contains("\"1357\""));
    }

    #[test]
    fn a_plaintext_mbc_entry_is_a_load_error() {
        let dir = tempfile::tempdir().unwrap();
        let h = hasher();
        let text = serde_json::json!({"engineer": h.hash("2468"), "mbc": "1357"}).to_string();
        std::fs::write(dir.path().join(PIN_HASHES_FILE), text).unwrap();
        let err = PinStore::load(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "the mbc PIN entry is not an argon2id hash");
        let mut store = PinStore::load(tempfile::tempdir().unwrap().path()).unwrap();
        assert!(store.set_hash(Role::Mbc, "1357".to_string()).is_err());
    }

    #[test]
    fn an_older_file_with_only_the_engineer_loads() {
        let dir = tempfile::tempdir().unwrap();
        let h = hasher();
        let text = serde_json::json!({"engineer": h.hash("2468")}).to_string();
        std::fs::write(dir.path().join(PIN_HASHES_FILE), text).unwrap();
        let store = PinStore::load(dir.path()).unwrap();
        assert_eq!(store.hash(Role::Mbc), None);
        assert!(store.has_hashes());
        assert!(h.verify("2468", store.hash(Role::Engineer).unwrap()));
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
