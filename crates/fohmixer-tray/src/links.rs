//! The hub's links, from its config `<data>\fohmixer-hub.toml` (read only;
//! the hub's `config.rs` owns the file and validates it): Open fohmixer
//! (`http_port`), the version poll, and Copy URL (the `[tls]` name, the one
//! public name of remote access, #17). Only those keys are read; any other
//! key is the hub's business and is ignored here.

use std::path::Path;

use serde::Deserialize;

/// The config file in the data folder (the hub's `config::CONFIG_FILE`).
pub const CONFIG_FILE: &str = "fohmixer-hub.toml";
/// The hub's HTTP port when the file does not set one.
pub const DEFAULT_HTTP_PORT: u16 = 8480;
/// The `[tls]` port when the table does not set one.
pub const DEFAULT_HTTPS_PORT: u16 = 443;

/// The keys of the hub's config the tray reads.
#[derive(Debug, Deserialize)]
struct HubToml {
    #[serde(default = "default_http_port")]
    http_port: u16,
    #[serde(default)]
    tls: Option<TlsToml>,
}

#[derive(Debug, Deserialize)]
struct TlsToml {
    name: String,
    #[serde(default = "default_https_port")]
    port: u16,
}

fn default_http_port() -> u16 {
    DEFAULT_HTTP_PORT
}

fn default_https_port() -> u16 {
    DEFAULT_HTTPS_PORT
}

/// Where the tray's menu and poll go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Links {
    /// Open fohmixer: the hub on this PC, `http://localhost:<http_port>/`.
    pub open: String,
    /// The tag manual (#68): `/znacky.html` of the same hub.
    pub manual: String,
    /// The version poll: `http://127.0.0.1:<http_port>/api/version` (an
    /// address: `localhost` may resolve to `::1` first).
    pub version: String,
    /// Copy URL: the public URL, `None` without remote access (the item is
    /// then disabled).
    pub public: Option<String>,
}

impl Links {
    /// The links of a hub on `http_port`, with remote access on
    /// `tls` = (name, HTTPS port) when set up.
    pub fn new(http_port: u16, tls: Option<(&str, u16)>) -> Self {
        Self {
            open: format!("http://localhost:{http_port}/"),
            manual: format!("http://localhost:{http_port}/znacky.html"),
            version: format!("http://127.0.0.1:{http_port}/api/version"),
            public: tls.map(|(name, port)| public_url(name, port)),
        }
    }

    /// The links of a hub without a config file (its defaults).
    pub fn defaults() -> Self {
        Self::new(DEFAULT_HTTP_PORT, None)
    }

    /// The links a config text gives.
    pub fn parse(text: &str) -> Result<Self, String> {
        let toml: HubToml = toml::from_str(text).map_err(|e| e.to_string())?;
        let tls = toml.tls.as_ref().map(|t| (t.name.as_str(), t.port));
        Ok(Self::new(toml.http_port, tls))
    }

    /// The links of the hub whose data folder is `data_dir`: its config file,
    /// or the defaults when there is none (the hub then runs on its defaults
    /// too). A file that cannot be read or parsed is an error naming it; the
    /// caller falls back to the defaults and logs it.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join(CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::defaults()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

/// The public URL of remote access: `https://<name>/`, with `:<port>` when it
/// is not 443 (the installer's desktop shortcut, `Get-FohPublicUrl`, says the
/// same).
pub fn public_url(name: &str, port: u16) -> String {
    if port == DEFAULT_HTTPS_PORT {
        format!("https://{name}/")
    } else {
        format!("https://{name}:{port}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_port_8480_without_a_public_url() {
        assert_eq!(
            Links::defaults(),
            Links {
                open: "http://localhost:8480/".to_string(),
                manual: "http://localhost:8480/znacky.html".to_string(),
                version: "http://127.0.0.1:8480/api/version".to_string(),
                public: None,
            }
        );
    }

    #[test]
    fn the_http_port_comes_from_the_config() {
        let links = Links::parse("http_port = 18481\n").unwrap();
        assert_eq!(links.open, "http://localhost:18481/");
        assert_eq!(links.version, "http://127.0.0.1:18481/api/version");
        assert_eq!(links.public, None);
    }

    #[test]
    fn an_empty_config_is_the_defaults() {
        assert_eq!(Links::parse("").unwrap(), Links::defaults());
    }

    #[test]
    fn the_installers_config_with_remote_access_gives_the_public_url() {
        // The shape Install-Fohmixer.ps1 writes (New-FohHubToml +
        // Get-FohRemoteToml), every other key ignored here.
        let text = "# fohmixer-hub configuration, written by Install-Fohmixer.ps1\r\n\
            http_port = 8480\r\nlayout = \"layout.json\"\r\n\r\n\
            [[instances]]\r\nname = \"band\"\r\nport = 39101\r\n\r\n\
            [[instances]]\r\nname = \"master\"\r\nport = 39102\r\n\r\n\
            [tls]\r\nname = \"foh.example.org\"\r\nport = 443\r\n\r\n\
            [acme]\r\nemail = \"owner@example.org\"\r\n\r\n\
            [access]\r\nteam_domain = \"team.cloudflareaccess.com\"\r\naud = [\"aud-1\"]\r\n\r\n\
            [tunnel]\r\nready_url = \"http://127.0.0.1:20241/ready\"\r\n";
        let links = Links::parse(text).unwrap();
        assert_eq!(links.open, "http://localhost:8480/");
        assert_eq!(links.public.as_deref(), Some("https://foh.example.org/"));
    }

    #[test]
    fn a_tls_table_without_a_port_is_on_443() {
        let links = Links::parse("[tls]\nname = \"foh.example.org\"\n").unwrap();
        assert_eq!(links.public.as_deref(), Some("https://foh.example.org/"));
    }

    #[test]
    fn another_https_port_is_in_the_public_url() {
        let links = Links::parse("[tls]\nname = \"foh.example.org\"\nport = 18443\n").unwrap();
        assert_eq!(
            links.public.as_deref(),
            Some("https://foh.example.org:18443/")
        );
    }

    #[test]
    fn public_url_names_the_port_only_when_it_is_not_443() {
        assert_eq!(public_url("a.example.org", 443), "https://a.example.org/");
        assert_eq!(
            public_url("a.example.org", 444),
            "https://a.example.org:444/"
        );
        assert_eq!(
            public_url("a.example.org", 442),
            "https://a.example.org:442/"
        );
    }

    #[test]
    fn a_config_that_does_not_parse_is_an_error() {
        let err = Links::parse("http_port = \"x\"\n").unwrap_err();
        assert!(err.contains("http_port"), "{err}");
        assert!(
            Links::parse("[tls]\nport = 443\n").is_err(),
            "a [tls] without a name"
        );
    }

    #[test]
    fn load_reads_the_config_in_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            "http_port = 9000\n[tls]\nname = \"n.example.org\"\n",
        )
        .unwrap();
        assert_eq!(
            Links::load(dir.path()).unwrap(),
            Links::new(9000, Some(("n.example.org", 443)))
        );
    }

    #[test]
    fn load_without_a_config_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Links::load(dir.path()).unwrap(), Links::defaults());
    }

    #[test]
    fn load_of_a_bad_config_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), "http_port = [\n").unwrap();
        let err = Links::load(dir.path()).unwrap_err();
        assert!(err.contains(CONFIG_FILE), "{err}");
    }

    #[test]
    fn load_of_an_unreadable_config_names_the_file() {
        // A folder where the file should be: reading it fails, not NotFound.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        let err = Links::load(dir.path()).unwrap_err();
        assert!(err.contains(CONFIG_FILE), "{err}");
    }
}
