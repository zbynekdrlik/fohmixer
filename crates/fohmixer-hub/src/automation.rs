//! The tokens of the automation routes: what an outside automation on the
//! LAN shows instead of a login.
//!
//! - [`AUTOMATION`] (#207): Companion's scene buttons' token for `POST
//!   /api/scene`; it also reads `GET /api/mbc/stream-proof` (#267).
//! - [`STREAM_PROOF`] (#274): a read-only token for `GET
//!   /api/mbc/stream-proof` only (camera-box's stream-test guard), so a
//!   reader of the proof never gets the scenes' writes.
//!
//! Each is its own secret, never the PIN or a login token: set with
//! `fohmixer-hub <its command>` (one line on stdin, never argv), sealed for
//! the hub's user in `<data>/secrets/<its stem>.<dpapi|test>` like the link
//! secret (`link.rs`), never in the config and never logged; sent as
//! `Authorization: Bearer <token>`, compared in constant time
//! ([`same_token`]). The hub reads both at its start. No automation token:
//! the hub takes no scene (`POST /api/scene` answers 503) and its pages show
//! no SCENES section.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::provision::ProvisionError;
use crate::secrets::SECRETS_DIR;

/// One kind of token: its file and its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    /// The token file's name without its extension (`sealed::SEALED_EXT`).
    pub stem: &'static str,
    /// Its name in a message.
    pub what: &'static str,
    /// The `fohmixer-hub` command that sets it.
    pub command: &'static str,
    /// What a hub without it does (the warn line of one not read).
    pub without: &'static str,
}

/// The automation token (#207).
pub const AUTOMATION: Kind = Kind {
    stem: "automation_token",
    what: "the automation token",
    command: "automation set-token",
    without: "the hub takes no scene",
};

/// The stream proof's read-only token (#274).
pub const STREAM_PROOF: Kind = Kind {
    stem: "stream_proof_token",
    what: "the stream proof token",
    command: "stream-proof set-token",
    without: "the stream proof takes the automation token only",
};

/// The shortest token taken, in characters.
pub const MIN_CHARS: usize = 32;
/// The longest token taken, in characters.
pub const MAX_CHARS: usize = 256;

/// A token of the automation routes. Its `Debug` never shows it.
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
        return Some("32 to 256 characters");
    }
    if !text.bytes().all(|b| b.is_ascii_graphic()) {
        return Some("visible ASCII, no space");
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

/// `<secrets_dir>/<stem>.<ext>`.
pub fn token_path(kind: Kind, secrets_dir: &Path) -> PathBuf {
    secrets_dir.join(format!("{}.{}", kind.stem, crate::sealed::SEALED_EXT))
}

/// The token of one line of `input` (its line end and outer spaces cut).
pub fn read_token(kind: Kind, mut input: impl BufRead) -> Result<AutomationToken, ProvisionError> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    let text = line.trim();
    match token_problem(text) {
        Some(problem) => Err(ProvisionError::Invalid(format!(
            "{} is {problem}",
            kind.what
        ))),
        None => Ok(AutomationToken(text.as_bytes().to_vec())),
    }
}

/// Seals `token` into `<data_dir>/secrets/`, replacing a stored one.
pub fn store(kind: Kind, data_dir: &Path, token: &AutomationToken) -> std::io::Result<()> {
    let secrets_dir = data_dir.join(SECRETS_DIR);
    std::fs::create_dir_all(&secrets_dir)?;
    crate::sealed::write(&token_path(kind, &secrets_dir), &token.0)
}

/// The stored token, `None` when none is set. A file that does not open or
/// does not hold a token is an error, never taken as missing.
pub fn load(kind: Kind, secrets_dir: &Path) -> std::io::Result<Option<AutomationToken>> {
    let path = token_path(kind, secrets_dir);
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
                    "{} does not hold {}: set it again with `fohmixer-hub {}`",
                    path.display(),
                    kind.what,
                    kind.command
                ),
            )
        })
}

/// The token a hub runs with, and the warn line when the stored one could
/// not be read (it runs without, never the hub's start refused).
pub fn usable(
    kind: Kind,
    loaded: std::io::Result<Option<AutomationToken>>,
) -> (Option<AutomationToken>, Option<String>) {
    match loaded {
        Ok(token) => (token, None),
        Err(e) => (
            None,
            Some(format!(
                "{} cannot be read, {}: {e}",
                kind.what, kind.without
            )),
        ),
    }
}

/// The two kinds, so each is checked against the other.
pub const KINDS: [Kind; 2] = [AUTOMATION, STREAM_PROOF];

/// Why `token`, about to be stored as `kind`, is refused: it equals the
/// token stored as `other` (the read-only token would then write scenes).
/// An `other` that cannot be read is no match (the hub says so at its start).
pub fn same_as_other(
    kind: Kind,
    other: Kind,
    stored: Option<&AutomationToken>,
    token: &AutomationToken,
) -> Option<String> {
    (kind != other && stored == Some(token))
        .then(|| format!("{} equals {}: set another", kind.what, other.what))
}

/// The read-only token a hub runs with beside `automation`, and the warn
/// line when the two are equal (it is dropped: a reader of the proof never
/// gets the scenes' writes).
pub fn read_only_apart(
    automation: Option<&AutomationToken>,
    read_only: Option<AutomationToken>,
) -> (Option<AutomationToken>, Option<String>) {
    match read_only {
        Some(token) if automation == Some(&token) => (
            None,
            Some(format!(
                "{} equals {}, the hub takes it for nothing: set another with `fohmixer-hub {}`",
                STREAM_PROOF.what, AUTOMATION.what, STREAM_PROOF.command
            )),
        ),
        other => (other, None),
    }
}

/// The whole `fohmixer-hub <kind's command>`: a token equal to the other
/// kind's stored one is refused (exit 2), nothing stored.
pub fn run(kind: Kind, data_dir: &Path, input: impl BufRead) -> Result<(), ProvisionError> {
    let token = read_token(kind, input)?;
    let secrets_dir = data_dir.join(SECRETS_DIR);
    for other in KINDS {
        let stored = load(other, &secrets_dir).ok().flatten();
        if let Some(problem) = same_as_other(kind, other, stored.as_ref(), &token) {
            return Err(ProvisionError::Invalid(problem));
        }
    }
    store(kind, data_dir, &token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789ABCDEF";

    #[test]
    fn the_numbers_are_its_bounds() {
        assert_eq!(AUTOMATION.stem, "automation_token");
        assert_eq!(STREAM_PROOF.stem, "stream_proof_token");
        assert_eq!(MIN_CHARS, 32);
        assert_eq!(MAX_CHARS, 256);
    }

    #[test]
    fn a_token_is_32_to_256_visible_ascii_characters() {
        assert_eq!(token_problem(GOOD), None);
        assert_eq!(token_problem(&"~".repeat(256)), None);
        assert_eq!(token_problem(&"!".repeat(33)), None);
        let length = Some("32 to 256 characters");
        assert_eq!(token_problem(&GOOD[1..]), length);
        assert_eq!(token_problem(&"a".repeat(257)), length);
        assert_eq!(token_problem(""), length);
        let chars = Some("visible ASCII, no space");
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
        // Two differences that cancel under XOR (the fold must OR them).
        assert!(!same_token(b"ab", b"ba"));
        assert!(!same_token(b"aa", b"cc"));
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
        assert!(
            load(AUTOMATION, &secrets).unwrap().is_none(),
            "none set yet"
        );
        run(
            AUTOMATION,
            dir.path(),
            format!("  {GOOD}\r\nmore\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(
            load(AUTOMATION, &secrets).unwrap(),
            AutomationToken::of(GOOD)
        );
        assert!(token_path(AUTOMATION, &secrets).exists());
        // A bad line stores nothing new.
        let refused = run(AUTOMATION, dir.path(), "too short\n".as_bytes());
        assert!(
            matches!(refused, Err(ProvisionError::Invalid(ref m)) if m == "the automation token is 32 to 256 characters"),
            "{refused:?}"
        );
        assert_eq!(
            load(AUTOMATION, &secrets).unwrap(),
            AutomationToken::of(GOOD)
        );
    }

    #[test]
    fn a_stored_file_that_holds_no_token_is_an_error_and_the_hub_runs_without() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        std::fs::create_dir_all(&secrets).unwrap();
        crate::sealed::write(&token_path(AUTOMATION, &secrets), b"short").unwrap();
        let err = load(AUTOMATION, &secrets).unwrap_err();
        assert!(err.to_string().contains("automation set-token"), "{err}");
        let (token, warning) = usable(AUTOMATION, Err(err));
        assert!(token.is_none());
        assert!(
            warning
                .unwrap()
                .starts_with("the automation token cannot be read, the hub takes no scene: ")
        );
        let (token, warning) = usable(AUTOMATION, Ok(AutomationToken::of(GOOD)));
        assert!(token.is_some() && warning.is_none());
    }

    #[test]
    fn the_read_only_tokens_words_name_its_own_command() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        std::fs::create_dir_all(&secrets).unwrap();
        crate::sealed::write(&token_path(STREAM_PROOF, &secrets), b"short").unwrap();
        assert!(
            load(AUTOMATION, &secrets).unwrap().is_none(),
            "its own file"
        );
        let err = load(STREAM_PROOF, &secrets).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not hold the stream proof token: set it again with `fohmixer-hub stream-proof set-token`"),
            "{err}"
        );
        let (token, warning) = usable(STREAM_PROOF, Err(err));
        assert!(token.is_none());
        assert!(warning.unwrap().starts_with(
            "the stream proof token cannot be read, the stream proof takes the automation token only: "
        ));
        let refused = read_token(STREAM_PROOF, "short\n".as_bytes());
        assert!(
            matches!(refused, Err(ProvisionError::Invalid(ref m)) if m == "the stream proof token is 32 to 256 characters"),
            "{refused:?}"
        );
    }

    #[test]
    fn a_token_equal_to_the_other_kinds_is_refused_and_nothing_stored() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = dir.path().join(SECRETS_DIR);
        run(AUTOMATION, dir.path(), format!("{GOOD}\n").as_bytes()).unwrap();
        let refused = run(STREAM_PROOF, dir.path(), format!("{GOOD}\n").as_bytes());
        assert!(
            matches!(refused, Err(ProvisionError::Invalid(ref m))
                if m == "the stream proof token equals the automation token: set another"),
            "{refused:?}"
        );
        assert_eq!(load(STREAM_PROOF, &secrets).unwrap(), None);
        // Another value is taken; then the automation token cannot be set to it.
        let other = "x".repeat(32);
        run(STREAM_PROOF, dir.path(), format!("{other}\n").as_bytes()).unwrap();
        assert_eq!(
            load(STREAM_PROOF, &secrets).unwrap(),
            AutomationToken::of(&other)
        );
        let refused = run(AUTOMATION, dir.path(), format!("{other}\n").as_bytes());
        assert!(
            matches!(refused, Err(ProvisionError::Invalid(ref m))
                if m == "the automation token equals the stream proof token: set another"),
            "{refused:?}"
        );
        assert_eq!(
            load(AUTOMATION, &secrets).unwrap(),
            AutomationToken::of(GOOD)
        );
        // A kind set again to its own value is no clash.
        run(AUTOMATION, dir.path(), format!("{GOOD}\n").as_bytes()).unwrap();
    }

    #[test]
    fn same_as_other_needs_another_kind_and_an_equal_stored_token() {
        let a = AutomationToken::of(GOOD).unwrap();
        let b = AutomationToken::of(&"y".repeat(32)).unwrap();
        assert_eq!(same_as_other(AUTOMATION, AUTOMATION, Some(&a), &a), None);
        assert_eq!(same_as_other(AUTOMATION, STREAM_PROOF, Some(&b), &a), None);
        assert_eq!(same_as_other(AUTOMATION, STREAM_PROOF, None, &a), None);
        assert_eq!(
            same_as_other(AUTOMATION, STREAM_PROOF, Some(&a), &a).as_deref(),
            Some("the automation token equals the stream proof token: set another")
        );
        assert_eq!(KINDS, [AUTOMATION, STREAM_PROOF]);
    }

    #[test]
    fn a_read_only_token_equal_to_the_automation_token_is_dropped_at_the_start() {
        let a = AutomationToken::of(GOOD).unwrap();
        let b = AutomationToken::of(&"y".repeat(32)).unwrap();
        assert_eq!(
            read_only_apart(Some(&a), Some(b.clone())),
            (Some(b.clone()), None)
        );
        assert_eq!(read_only_apart(None, Some(b.clone())), (Some(b), None));
        assert_eq!(read_only_apart(Some(&a), None), (None, None));
        let (token, warning) = read_only_apart(Some(&a), Some(a.clone()));
        assert_eq!(token, None);
        assert_eq!(
            warning.as_deref(),
            Some(
                "the stream proof token equals the automation token, the hub takes it for nothing: \
                 set another with `fohmixer-hub stream-proof set-token`"
            )
        );
    }
}
