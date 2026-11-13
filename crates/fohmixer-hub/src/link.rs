//! The instance link (#184, spec §6.1): how a hub reaches a Live instance
//! another hub holds (the FOH hub's `mbc`, the MBC hub's local one).
//!
//! - The link secret: 32 random bytes, set with `fohmixer-hub link
//!   set-secret` (one line of standard base64 on stdin, never argv), sealed
//!   for the hub's user in `<data>/secrets/link_secret.<dpapi|test>` like
//!   the other secrets, never in the config and never logged. Both hubs
//!   hold the same one.
//! - The link token: an HS256 JWT signed with it, `sub: "hub"`, valid
//!   [`LINK_TOKEN_SECS`]; the dialling hub makes a new one per connection
//!   and sends it as `Authorization: Bearer …` ([`link_token`]); the
//!   serving hub checks it at the upgrade ([`link_claims_ok`]).
//! - The serving side is [`server`]; the dialling side is the Live client
//!   (`live::client::instance_request`).

use std::io::BufRead;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::provision::ProvisionError;
use crate::secrets::SECRETS_DIR;

pub mod server;

/// The secret file's name without its extension (`sealed::SEALED_EXT`).
pub const SECRET_STEM: &str = "link_secret";
/// The secret's length in bytes.
pub const SECRET_BYTES: usize = 32;
/// How long a link token is valid, in seconds (one connection's handshake).
pub const LINK_TOKEN_SECS: u64 = 60;
/// The subject of a link token: another hub, never a person's login.
pub const LINK_SUB: &str = "hub";

/// The link secret. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkSecret([u8; SECRET_BYTES]);

impl std::fmt::Debug for LinkSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkSecret(..)")
    }
}

impl LinkSecret {
    /// The secret of these bytes (tests, the harness's generated one).
    pub fn from_bytes(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// The secret of `bytes`, when they are exactly [`SECRET_BYTES`].
    fn of(bytes: &[u8]) -> Option<Self> {
        <[u8; SECRET_BYTES]>::try_from(bytes).ok().map(Self)
    }
}

/// `<secrets_dir>/link_secret.<ext>`.
pub fn secret_path(secrets_dir: &Path) -> PathBuf {
    secrets_dir.join(format!("{SECRET_STEM}.{}", crate::sealed::SEALED_EXT))
}

/// The secret of one line of `input`: standard base64 of exactly 32 bytes
/// (its line end and outer spaces cut).
pub fn read_secret(mut input: impl BufRead) -> Result<LinkSecret, ProvisionError> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    STANDARD
        .decode(line.trim())
        .ok()
        .and_then(|bytes| LinkSecret::of(&bytes))
        .ok_or_else(|| ProvisionError::Invalid("the link secret is 32 bytes in base64".into()))
}

/// Seals `secret` into `<data_dir>/secrets/`, replacing a stored one.
pub fn store(data_dir: &Path, secret: &LinkSecret) -> std::io::Result<()> {
    let secrets_dir = data_dir.join(SECRETS_DIR);
    std::fs::create_dir_all(&secrets_dir)?;
    crate::sealed::write(&secret_path(&secrets_dir), &secret.0)
}

/// The stored secret, `None` when none is set. A file that does not open or
/// does not hold 32 bytes is an error, never taken as missing.
pub fn load(secrets_dir: &Path) -> std::io::Result<Option<LinkSecret>> {
    let path = secret_path(secrets_dir);
    let Some(bytes) = crate::sealed::read(&path)? else {
        return Ok(None);
    };
    LinkSecret::of(&bytes).map(Some).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} does not hold a link secret: set it again with \
                 `fohmixer-hub link set-secret`",
                path.display()
            ),
        )
    })
}

/// The secret a hub runs with, and the warn line when the stored one could
/// not be read (it runs without: links are refused, a linked instance's
/// attempts fail, never the hub's start).
pub fn usable(loaded: std::io::Result<Option<LinkSecret>>) -> (Option<LinkSecret>, Option<String>) {
    match loaded {
        Ok(secret) => (secret, None),
        Err(e) => (
            None,
            Some(format!(
                "the link secret cannot be read, the instance link runs without it: {e}"
            )),
        ),
    }
}

/// The whole `link set-secret` command.
pub fn run(data_dir: &Path, input: impl BufRead) -> Result<(), ProvisionError> {
    let secret = read_secret(input)?;
    store(data_dir, &secret)?;
    Ok(())
}

/// A link token's claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkClaims {
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
}

/// A new link token at the Unix time `now`.
pub fn link_token(secret: &LinkSecret, now: u64) -> String {
    let claims = LinkClaims {
        sub: LINK_SUB.to_string(),
        iat: now,
        exp: now + LINK_TOKEN_SECS,
    };
    // HS256 over a fixed struct cannot fail to encode.
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&secret.0),
    )
    .unwrap_or_default()
}

/// Why a link connection was refused at its upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRefusal {
    /// No `Authorization: Bearer …` header.
    NoHeader,
    /// Not a token of this secret (another secret, not a JWT, another
    /// algorithm).
    BadToken,
    /// Its `exp` passed.
    Expired,
    /// Signed by the secret, but for another subject.
    NotHub,
    /// This hub holds no link secret: it refuses every link.
    NoSecret,
}

impl LinkRefusal {
    /// The refusal in words (logged and recorded; never the token).
    pub fn text(self) -> &'static str {
        match self {
            Self::NoHeader => "no bearer token",
            Self::BadToken => "not a token of the link secret",
            Self::Expired => "the token expired",
            Self::NotHub => "the token is not a hub's",
            Self::NoSecret => "no link secret",
        }
    }
}

/// Whether the `Authorization` header value `header` carries a valid link
/// token of `secret` at the Unix time `now` (valid through the second of
/// its `exp`, no leeway).
pub fn link_claims_ok(
    header: Option<&str>,
    secret: &LinkSecret,
    now: u64,
) -> Result<(), LinkRefusal> {
    let token = header
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(LinkRefusal::NoHeader)?;
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    validation.leeway = 0;
    validation.required_spec_claims.clear();
    let data = decode::<LinkClaims>(token, &DecodingKey::from_secret(&secret.0), &validation)
        .map_err(|_| LinkRefusal::BadToken)?;
    if data.claims.exp < now {
        return Err(LinkRefusal::Expired);
    }
    if data.claims.sub != LINK_SUB {
        return Err(LinkRefusal::NotHub);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
