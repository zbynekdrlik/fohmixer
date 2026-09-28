//! Secrets sealed at rest for the hub's user: the PIN pepper, the Cloudflare
//! API token and the ACME account key (#17).
//!
//! Windows (the PC): DPAPI for the current user (`dpapi.rs`), so a sealed
//! file only opens for the account that sealed it — the commands that store
//! one (`pin set-engineer`, `cloudflare set-token`) run as the hub's user.
//! Other platforms are test-only: the bytes are stored as they are, in an
//! owner-only file, with a warning.

use std::io;
use std::path::Path;

#[cfg(windows)]
pub(crate) use crate::dpapi::{protect as seal, unprotect as unseal};

/// The file extension of a sealed file: `.dpapi` on Windows.
#[cfg(windows)]
pub const SEALED_EXT: &str = "dpapi";
/// The file extension of a sealed file: `.test` where nothing seals it.
#[cfg(not(windows))]
pub const SEALED_EXT: &str = "test";

/// `data` sealed: stored as it is (test platforms only).
#[cfg(not(windows))]
pub(crate) fn seal(data: &[u8]) -> io::Result<Vec<u8>> {
    tracing::warn!(
        "a secret stored unsealed: only Windows seals secrets (DPAPI); other platforms are test-only"
    );
    Ok(data.to_vec())
}

/// `data` unsealed (test platforms: as it is).
#[cfg(not(windows))]
pub(crate) fn unseal(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

/// The unsealed content of `path`, or `None` when it does not exist. A file
/// that exists but does not open is an error, never taken as missing.
pub fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(stored) => unseal(&stored).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Seals `data` into `path`, replacing it whole (owner-only on Unix).
pub fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    crate::secrets::replace_private(path, &seal(data)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_file_reads_back_and_a_missing_one_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("token.{SEALED_EXT}"));
        assert_eq!(read(&path).unwrap(), None);
        write(&path, b"first").unwrap();
        assert_eq!(read(&path).unwrap(), Some(b"first".to_vec()));
        write(&path, b"second").unwrap();
        assert_eq!(read(&path).unwrap(), Some(b"second".to_vec()));
    }

    #[test]
    fn an_unreadable_sealed_file_is_an_error_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::create_dir(&path).unwrap();
        assert!(read(&path).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn test_platforms_store_the_bytes_as_they_are() {
        assert_eq!(seal(b"abc").unwrap(), b"abc".to_vec());
        assert_eq!(unseal(b"abc").unwrap(), b"abc".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.test");
        write(&path, b"plain").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"plain");
    }

    #[cfg(windows)]
    #[test]
    fn windows_seals_with_dpapi() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.dpapi");
        write(&path, b"secret-bytes").unwrap();
        let stored = std::fs::read(&path).unwrap();
        assert_ne!(stored, b"secret-bytes".to_vec());
        assert_eq!(read(&path).unwrap(), Some(b"secret-bytes".to_vec()));
    }
}
