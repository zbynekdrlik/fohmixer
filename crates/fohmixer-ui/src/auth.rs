//! The engineer's login (S4 design note §6, spec X8): a four-digit PIN
//! gives a token (`POST /api/auth`) that the device keeps in local storage,
//! so there is no re-login during a service. A refused token sends the app
//! back to the login (the store's `logout`).

use fohmixer_proto::client::{AuthRequest, AuthResponse};

use crate::store::TOKEN_KEY;

/// The PIN length (the hub accepts exactly four digits).
pub const PIN_LEN: usize = 4;

/// The stored token, if any.
pub fn stored_token() -> Option<String> {
    crate::dom::storage_get(TOKEN_KEY).filter(|t| !t.is_empty())
}

/// Why a login failed, as the login page says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    WrongPin,
    TooManyAttempts,
    Unreachable(String),
    Other(String),
}

impl LoginError {
    /// The message the login page shows.
    pub fn message(&self) -> String {
        match self {
            Self::WrongPin => "Wrong PIN".to_string(),
            Self::TooManyAttempts => "Too many attempts, wait a moment".to_string(),
            Self::Unreachable(why) => format!("The hub is not reachable ({why})"),
            Self::Other(why) => format!("Login failed: {why}"),
        }
    }
}

/// The outcome of a `POST /api/auth` answer.
pub fn login_outcome(status: u16, body: &str) -> Result<String, LoginError> {
    match status {
        200 => serde_json::from_str::<AuthResponse>(body)
            .map(|auth| auth.token)
            .map_err(|e| LoginError::Other(format!("unreadable answer: {e}"))),
        401 => Err(LoginError::WrongPin),
        429 => Err(LoginError::TooManyAttempts),
        other => Err(LoginError::Other(format!(
            "HTTP {other}: {}",
            crate::net::api_message(body)
        ))),
    }
}

/// The body of a login request.
pub fn login_body(pin: &str) -> String {
    serde_json::to_string(&AuthRequest {
        pin: pin.to_string(),
    })
    .unwrap_or_default()
}

/// Logs in with `pin`: the token, stored, or why not.
pub async fn login(pin: &str) -> Result<String, LoginError> {
    let body = login_body(pin);
    let (status, text) = crate::net::fetch_text("POST", "/api/auth", None, Some(&body))
        .await
        .map_err(LoginError::Unreachable)?;
    let token = login_outcome(status, &text)?;
    crate::dom::storage_set(TOKEN_KEY, &token);
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_answer_is_a_token_or_a_reason() {
        assert_eq!(
            login_outcome(200, r#"{"token":"t.o.k","expires_in":604800}"#),
            Ok("t.o.k".to_string())
        );
        assert!(
            matches!(login_outcome(200, "{}"), Err(LoginError::Other(e)) if e.starts_with("unreadable answer: "))
        );
        assert_eq!(login_outcome(401, ""), Err(LoginError::WrongPin));
        assert_eq!(login_outcome(429, ""), Err(LoginError::TooManyAttempts));
        assert_eq!(
            login_outcome(503, r#"{"code":"X","message":"busy"}"#),
            Err(LoginError::Other("HTTP 503: busy".into()))
        );
    }

    #[test]
    fn the_login_page_words_each_failure() {
        assert_eq!(LoginError::WrongPin.message(), "Wrong PIN");
        assert_eq!(
            LoginError::TooManyAttempts.message(),
            "Too many attempts, wait a moment"
        );
        assert_eq!(
            LoginError::Unreachable("offline".into()).message(),
            "The hub is not reachable (offline)"
        );
        assert_eq!(LoginError::Other("x".into()).message(), "Login failed: x");
    }

    #[test]
    fn the_request_body_carries_the_pin() {
        assert_eq!(login_body("0421"), r#"{"pin":"0421"}"#);
    }
}
