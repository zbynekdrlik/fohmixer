//! PIN provisioning for `fohmixer-hub pin set-engineer` and, since #184,
//! `pin set-mbc` (from iemmixer's `iem-server/src/provision.rs` @ 22372bc):
//! the PIN is read from stdin, never from argv (process listings, shell
//! history), and only its argon2id hash is stored, peppered with the
//! installation's pepper. The two roles' PINs must differ: the login tells
//! the role by the PIN alone.

use std::io::BufRead;
use std::path::Path;

use fohmixer_proto::roles::Role;

use crate::pepper;
use crate::pin_hash::{PinHasher, is_valid_pin_format};
use crate::pin_store::PinStore;
use crate::secrets::SECRETS_DIR;

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// Bad input: exit code 2.
    #[error("{0}")]
    Invalid(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Read one line from `input` and check it is a valid PIN.
pub fn read_pin(mut input: impl BufRead) -> Result<String, ProvisionError> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let pin = line.trim_end_matches(['\r', '\n']).to_string();
    if is_valid_pin_format(&pin) {
        Ok(pin)
    } else {
        Err(ProvisionError::Invalid(
            "the PIN must be exactly 4 digits".to_string(),
        ))
    }
}

/// The refusal of a PIN equal to the other role's.
pub const SAME_PINS: &str = "the engineer and MBC PINs must differ";

/// The role whose PIN must differ from `role`'s.
fn other(role: Role) -> Role {
    match role {
        Role::Engineer => Role::Mbc,
        Role::Mbc => Role::Engineer,
    }
}

/// Hash `pin` with the installation's pepper and store it as the engineer
/// PIN, in `<data_dir>/secrets/`.
pub fn store_pin(data_dir: &Path, pin: &str) -> Result<(), ProvisionError> {
    store_role_pin(data_dir, pin, Role::Engineer)
}

/// Hash `pin` and store it as `role`'s PIN; refused (nothing written) when
/// it verifies against the other role's stored hash.
pub fn store_role_pin(data_dir: &Path, pin: &str, role: Role) -> Result<(), ProvisionError> {
    let secrets_dir = data_dir.join(SECRETS_DIR);
    let hasher = PinHasher::new(pepper::load_or_create(&secrets_dir)?);
    let mut store = PinStore::load(&secrets_dir)?;
    if hasher.verify_optional(pin, store.hash(other(role))) {
        return Err(ProvisionError::Invalid(SAME_PINS.to_string()));
    }
    store.set_hash(role, hasher.hash(pin))?;
    Ok(())
}

/// The whole `pin set-engineer` command.
pub fn run(data_dir: &Path, input: impl BufRead) -> Result<(), ProvisionError> {
    run_role(data_dir, input, Role::Engineer)
}

/// The whole `pin set-<role>` command.
pub fn run_role(data_dir: &Path, input: impl BufRead, role: Role) -> Result<(), ProvisionError> {
    let pin = read_pin(input)?;
    store_role_pin(data_dir, &pin, role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_pin_accepts_four_digits_and_trims_the_newline() {
        assert_eq!(read_pin("2468\r\n".as_bytes()).unwrap(), "2468");
        assert!(matches!(
            read_pin("246\n".as_bytes()),
            Err(ProvisionError::Invalid(_))
        ));
        assert!(matches!(
            read_pin("".as_bytes()),
            Err(ProvisionError::Invalid(_))
        ));
    }

    #[test]
    fn run_stores_a_hash_the_installation_can_verify() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "2468\n".as_bytes()).unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        let hasher = PinHasher::new(pepper::load_or_create(&secrets).unwrap());
        let store = PinStore::load(&secrets).unwrap();
        assert!(hasher.verify("2468", store.engineer_hash().unwrap()));
        assert!(!hasher.verify("1357", store.engineer_hash().unwrap()));
    }

    fn hashes(dir: &Path) -> Vec<u8> {
        std::fs::read(
            dir.join(SECRETS_DIR)
                .join(crate::pin_store::PIN_HASHES_FILE),
        )
        .unwrap()
    }

    #[test]
    fn an_mbc_pin_is_stored_beside_the_engineers() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "2468\n".as_bytes()).unwrap();
        run_role(dir.path(), "1357\n".as_bytes(), Role::Mbc).unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        let hasher = PinHasher::new(pepper::load_or_create(&secrets).unwrap());
        let store = PinStore::load(&secrets).unwrap();
        assert!(hasher.verify("1357", store.hash(Role::Mbc).unwrap()));
        assert!(hasher.verify("2468", store.hash(Role::Engineer).unwrap()));
    }

    #[test]
    fn an_mbc_pin_equal_to_the_engineers_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "2468\n".as_bytes()).unwrap();
        let before = hashes(dir.path());
        let err = run_role(dir.path(), "2468\n".as_bytes(), Role::Mbc).unwrap_err();
        assert!(matches!(err, ProvisionError::Invalid(_)), "{err}");
        assert_eq!(err.to_string(), SAME_PINS);
        assert_eq!(hashes(dir.path()), before);
    }

    #[test]
    fn an_engineer_pin_equal_to_the_mbcs_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        run_role(dir.path(), "1357\n".as_bytes(), Role::Mbc).unwrap();
        run(dir.path(), "2468\n".as_bytes()).unwrap();
        let before = hashes(dir.path());
        let err = run(dir.path(), "1357\n".as_bytes()).unwrap_err();
        assert_eq!(err.to_string(), SAME_PINS);
        assert_eq!(hashes(dir.path()), before);
        // The same role's PIN set again is no conflict.
        run(dir.path(), "2468\n".as_bytes()).unwrap();
    }

    #[test]
    fn an_invalid_pin_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = run(dir.path(), "12a4\n".as_bytes()).unwrap_err();
        assert_eq!(err.to_string(), "the PIN must be exactly 4 digits");
        assert!(!dir.path().join(SECRETS_DIR).exists());
    }

    #[test]
    fn store_pin_never_re_peppers_an_existing_hash() {
        // Provisioning after the pepper was lost must not create a new
        // pepper: the stored hash would silently stop verifying.
        let dir = tempfile::tempdir().unwrap();
        store_pin(dir.path(), "2468").unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        let hashes = secrets.join(crate::pin_store::PIN_HASHES_FILE);
        std::fs::remove_file(secrets.join(pepper::PEPPER_FILE)).unwrap();
        let before = std::fs::read(&hashes).unwrap();
        let err = store_pin(dir.path(), "1357").unwrap_err();
        assert!(matches!(err, ProvisionError::Io(_)), "{err}");
        assert!(err.to_string().starts_with("I/O error: "), "{err}");
        assert!(!secrets.join(pepper::PEPPER_FILE).exists());
        assert_eq!(std::fs::read(&hashes).unwrap(), before);
    }
}
