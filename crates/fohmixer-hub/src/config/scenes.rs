//! `[scenes]` (#207, the owner's ruling of 2026-11-15): the FOH hub reads
//! the service's scene from Companion's custom variable instead of
//! Companion pushing it. Companion is `[companion]`'s host (one address for
//! both of its APIs, so the table needs `[companion]`); its web server's
//! port is `companion_http_port` (Companion's default 8000).
//!
//! ```toml
//! [scenes]
//! companion_variable = "scena"   # 1..=64 of A-Z a-z 0-9 _ -
//! companion_http_port = 8000     # the default
//! ```

use anyhow::bail;
use serde::Deserialize;

use super::CompanionCfg;

/// Companion's web server (its HTTP API) when `[scenes]` names no port.
pub const DEFAULT_COMPANION_HTTP_PORT: u16 = 8000;
/// The longest variable name taken.
pub const MAX_VARIABLE_CHARS: usize = 64;

fn default_companion_http_port() -> u16 {
    DEFAULT_COMPANION_HTTP_PORT
}

/// `[scenes]`: the custom variable the hub reads and Companion's HTTP port.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenesCfg {
    pub companion_variable: String,
    #[serde(default = "default_companion_http_port")]
    pub companion_http_port: u16,
}

impl ScenesCfg {
    /// The variable's value URL on `companion`'s host:
    /// `http://<host>:<port>/api/custom-variable/<variable>/value` (an IPv6
    /// address in brackets).
    pub fn value_url(&self, companion: &CompanionCfg) -> String {
        format!(
            "http://{}/api/custom-variable/{}/value",
            self.authority(companion),
            self.companion_variable
        )
    }

    /// Companion's web server as `host:port` (the `scene` record's peer).
    pub fn authority(&self, companion: &CompanionCfg) -> String {
        let host = &companion.host;
        if host.contains(':') {
            format!("[{host}]:{}", self.companion_http_port)
        } else {
            format!("{host}:{}", self.companion_http_port)
        }
    }
}

/// Whether `name` is a variable name the URL carries as it is: 1..=64 of
/// ASCII letters, digits, `_` and `-` (Companion's custom variable names).
pub fn is_variable(name: &str) -> bool {
    (1..=MAX_VARIABLE_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `[scenes]` checked: a variable name, a port, and `[companion]` (its host).
pub fn validate(
    scenes: Option<&ScenesCfg>,
    companion: Option<&CompanionCfg>,
) -> anyhow::Result<()> {
    let Some(cfg) = scenes else {
        return Ok(());
    };
    if companion.is_none() {
        bail!("[scenes] reads Companion at [companion]'s host: add [companion]");
    }
    if !is_variable(&cfg.companion_variable) {
        bail!(
            "[scenes] companion_variable {:?}: 1..={MAX_VARIABLE_CHARS} of A-Z a-z 0-9 _ -",
            cfg.companion_variable
        );
    }
    if cfg.companion_http_port == 0 {
        bail!("[scenes] companion_http_port 0");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    const COMPANION: &str = "[companion]\nhost = \"10.0.0.7\"\n";

    fn parse(text: &str) -> anyhow::Result<Config> {
        Config::parse(text, Path::new("/data"))
    }

    #[test]
    fn the_table_is_read_with_its_default_port_and_names_the_value_url() {
        assert_eq!(DEFAULT_COMPANION_HTTP_PORT, 8000);
        let config = parse(&format!(
            "{COMPANION}[scenes]\ncompanion_variable = \"scena\"\n"
        ))
        .unwrap();
        let scenes = config.scenes.clone().unwrap();
        assert_eq!(
            scenes,
            ScenesCfg {
                companion_variable: "scena".into(),
                companion_http_port: 8000,
            }
        );
        let companion = config.companion.unwrap();
        assert_eq!(
            scenes.value_url(&companion),
            "http://10.0.0.7:8000/api/custom-variable/scena/value"
        );
        assert_eq!(scenes.authority(&companion), "10.0.0.7:8000");
        let other = parse(&format!(
            "{COMPANION}[scenes]\ncompanion_variable = \"a-b_1\"\ncompanion_http_port = 8001\n"
        ))
        .unwrap();
        assert_eq!(
            other.scenes.unwrap().value_url(&other.companion.unwrap()),
            "http://10.0.0.7:8001/api/custom-variable/a-b_1/value"
        );
        assert_eq!(parse("").unwrap().scenes, None);
    }

    #[test]
    fn an_ipv6_host_is_bracketed() {
        let companion = CompanionCfg {
            host: "fd00::7".into(),
            port: 16622,
            columns: 8,
            rows: 4,
            bitmap_px: 144,
            title: "Stream Deck".into(),
        };
        let scenes = ScenesCfg {
            companion_variable: "scena".into(),
            companion_http_port: 8000,
        };
        assert_eq!(
            scenes.value_url(&companion),
            "http://[fd00::7]:8000/api/custom-variable/scena/value"
        );
    }

    #[test]
    fn a_bad_table_is_refused() {
        for (text, why) in [
            (
                "[scenes]\ncompanion_variable = \"scena\"\n".to_string(),
                "[scenes] reads Companion at [companion]'s host: add [companion]",
            ),
            (
                format!("{COMPANION}[scenes]\ncompanion_variable = \"\"\n"),
                "[scenes] companion_variable \"\": 1..=64 of A-Z a-z 0-9 _ -",
            ),
            (
                format!("{COMPANION}[scenes]\ncompanion_variable = \"a b\"\n"),
                "[scenes] companion_variable \"a b\"",
            ),
            (
                format!("{COMPANION}[scenes]\ncompanion_variable = \"a/b\"\n"),
                "[scenes] companion_variable \"a/b\"",
            ),
            (
                format!(
                    "{COMPANION}[scenes]\ncompanion_variable = \"{}\"\n",
                    "x".repeat(65)
                ),
                "[scenes] companion_variable \"xxx",
            ),
            (
                format!(
                    "{COMPANION}[scenes]\ncompanion_variable = \"scena\"\ncompanion_http_port = 0\n"
                ),
                "[scenes] companion_http_port 0",
            ),
        ] {
            let error = format!("{:#}", parse(&text).unwrap_err());
            assert!(error.contains(why), "{text}: {error}");
        }
        let error = format!(
            "{:#}",
            parse(&format!("{COMPANION}[scenes]\nvariable = \"scena\"\n")).unwrap_err()
        );
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn a_variable_name_is_letters_digits_underscore_and_dash() {
        for good in ["scena", "a", "A-Z_09", &"x".repeat(64)] {
            assert!(is_variable(good), "{good}");
        }
        for bad in ["", "a b", "a.b", "a/b", "č", "a%20", &"x".repeat(65)] {
            assert!(!is_variable(bad), "{bad}");
        }
    }
}
