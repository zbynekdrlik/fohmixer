//! The automation token (#207): what an outside automation on the LAN
//! (Companion's trigger on the service's scene) shows to `POST /api/scene`.
//!
//! - Its own secret, never the PIN or a login token: set with `fohmixer-hub
//!   automation set-token` (one line on stdin, never argv), sealed for the
//!   hub's user in `<data>/secrets/automation_token.<dpapi|test>` like the
//!   link secret (`link.rs`), never in the config and never logged.
//! - Sent as `Authorization: Bearer <token>`, compared in constant time
//!   ([`same_token`]).
//! - No token set: the hub takes no scene (`POST /api/scene` answers 503)
//!   and its pages show no SCENES section. The hub reads it at its start.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::provision::ProvisionError;
use crate::secrets::SECRETS_DIR;

/// The token file's name without its extension (`sealed::SEALED_EXT`).
pub const TOKEN_STEM: &str = "automation_token";
/// The shortest token taken, in characters.
pub const MIN_CHARS: usize = 32;
/// The longest token taken, in characters.
pub const MAX_CHARS: usize = 256;

/// The automation token. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct AutomationToken(Vec<u8>);

impl std::fmt::Debug for AutomationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AutomationToken(..)")
    }
}

impl AutomationToken {
    /// The token of `text` when it is one ([`token_problem`] says why not).
    pub fn of(text: &str) -> Option<Self> {
        token_problem(text)
            .is_none()
            .then(|| Self(text.as_bytes().to_vec()))
    }

    /// Whether `presented` (a request's bearer) is this token.
    pub fn accepts(&self, presented: Option<&str>) -> bool {
        presented.is_some_and(|p| same_token(&self.0, p.as_bytes()))
    }
}

/// Why `text` is no token: its length, or a character that is not visible
/// ASCII (a space, a control character, anything past `~`).
pub fn token_problem(text: &str) -> Option<&'static str> {
    let n = text.chars().count();
    if !(MIN_CHARS..=MAX_CHARS).contains(&n) {
        return Some("the automation token is 32 to 256 characters");
    }
    if !text.bytes().all(|b| b.is_ascii_graphic()) {
        return Some("the automation token is visible ASCII, no space");
    }
    None
}

/// Whether two tokens are equal, in time that does not depend on where
/// they differ (a length differs at once: the length is no secret).
pub fn same_token(known: &[u8], presented: &[u8]) -> bool {
    known.len() == presented.len()
        && known
            .iter()
            .zip(presented)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// `<secrets_dir>/automation_token.<ext>`.
pub fn token_path(secrets_dir: &Path) -> PathBuf {
    secrets_dir.join(format!("{TOKEN_STEM}.{}", crate::sealed::SEALED_EXT))
}

/// The token of one line of `input` (its line end and outer spaces cut).
pub fn read_token(mut input: impl BufRead) -> Result<AutomationToken, ProvisionError> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let text = line.trim();
    match token_problem(text) {
        Some(problem) => Err(ProvisionError::Invalid(problem.into())),
        None => Ok(AutomationToken(text.as_bytes().to_vec())),
    }
}

/// Seals `token` into `<data_dir>/secrets/`, replacing a stored one.
pub fn store(data_dir: &Path, token: &AutomationToken) -> std::io::Result<()> {
    let secrets_dir = data_dir.join(SECRETS_DIR);
    std::fs::create_dir_all(&secrets_dir)?;
    crate::sealed::write(&token_path(&secrets_dir), &token.0)
}

/// The stored token, `None` when none is set. A file that does not open or
/// does not hold a token is an error, never taken as missing.
pub fn load(secrets_dir: &Path) -> std::io::Result<Option<AutomationToken>> {
    let path = token_path(secrets_dir);
    let Some(bytes) = crate::sealed::read(&path)? else {
        return Ok(None);
    };
    std::str::from_utf8(&bytes)
        .ok()
        .and_then(AutomationToken::of)
        .map(Some)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} does not hold an automation token: set it again with \
                     `fohmixer-hub automation set-token`",
                    path.display()
                ),
            )
        })
}

/// The token a hub runs with, and the warn line when the stored one could
/// not be read (it runs without: no scene is taken, never the hub's start
/// refused).
pub fn usable(
    loaded: std::io::Result<Option<AutomationToken>>,
) -> (Option<AutomationToken>, Option<String>) {
    match loaded {
        Ok(token) => (token, None),
        Err(e) => (
            None,
            Some(format!(
                "the automation token cannot be read, the hub takes no scene: {e}"
            )),
        ),
    }
}

/// The whole `automation set-token` command.
pub fn run(data_dir: &Path, input: impl BufRead) -> Result<(), ProvisionError> {
    let token = read_token(input)?;
    store(data_dir, &token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789ABCDEF";

    #[test]
    fn the_numbers_are_its_bounds() {
        assert_eq!(TOKEN_STEM, "automation_token");
        assert_eq!(MIN_CHARS, 32);
        assert_eq!(MAX_CHARS, 256);
    }

    #[test]
    fn a_token_is_32_to_256_visible_ascii_characters() {
        assert_eq!(token_problem(GOOD), None);
        assert_eq!(token_problem(&"~".repeat(256)), None);
        assert_eq!(token_problem(&"!".repeat(33)), None);
        let length = Some("the automation token is 32 to 256 characters");
        assert_eq!(token_problem(&GOOD[1..]), length);
        assert_eq!(token_problem(&"a".repeat(257)), length);
        assert_eq!(token_problem(""), length);
        let chars = Some("the automation token is visible ASCII, no space");
        assert_eq!(token_problem(&format!("{} b", &GOOD[2..])), chars);
        assert_eq!(token_problem(&format!("{}\t", &GOOD[1..])), chars);
        assert_eq!(token_problem(&format!("{}\u{7f}", &GOOD[1..])), chars);
        assert_eq!(token_problem(&format!("{}é", &GOOD[1..])), chars);
    }

    #[test]
    fn same_token_needs_every_byte_and_the_length() {
        assert!(same_token(b"abc", b"abc"));
        assert!(!same_token(b"abc", b"abd"));
        assert!(!same_token(b"abc", b"xbc"));
        assert!(!same_token(b"abc", b"ab"));
        assert!(!same_token(b"ab", b"abc"));
        assert!(same_token(b"", b""));
    }

    #[test]
    fn a_token_accepts_only_itself() {
        let token = AutomationToken::of(GOOD).unwrap();
        assert!(token.accepts(Some(GOOD)));
        assert!(!token.accepts(Some(&GOOD[1..])));
        assert!(!token.accepts(None));
        assert_eq!(AutomationToken::of("short"), None);
        assert_eq!(format!("{token:?}"), "AutomationToken(..)");
    }

    #[test]
    fn set_token_reads_one_line_and_stores_it_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        assert!(load(&secrets).unwrap().is_none(), "none set yet");
        run(dir.path(), format!("  {GOOD}\r\nmore\n").as_bytes()).unwrap();
        assert_eq!(load(&secrets).unwrap(), AutomationToken::of(GOOD));
        assert!(token_path(&secrets).exists());
        // A bad line stores nothing new.
        let refused = run(dir.path(), "too short\n".as_bytes());
        assert!(
            matches!(refused, Err(ProvisionError::Invalid(ref m)) if m.contains("32 to 256")),
            "{refused:?}"
        );
        assert_eq!(load(&secrets).unwrap(), AutomationToken::of(GOOD));
    }

    #[test]
    fn a_stored_file_that_holds_no_token_is_an_error_and_the_hub_runs_without() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        std::fs::create_dir_all(&secrets).unwrap();
        crate::sealed::write(&token_path(&secrets), b"short").unwrap();
        let err = load(&secrets).unwrap_err();
        assert!(err.to_string().contains("automation set-token"), "{err}");
        let (token, warning) = usable(Err(err));
        assert!(token.is_none());
        assert!(
            warning
                .unwrap()
                .starts_with("the automation token cannot be read, the hub takes no scene: ")
        );
        let (token, warning) = usable(Ok(AutomationToken::of(GOOD)));
        assert!(token.is_some() && warning.is_none());
    }
}
