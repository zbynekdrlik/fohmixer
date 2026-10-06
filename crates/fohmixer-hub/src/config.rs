//! The hub's configuration: `<data>/fohmixer-hub.toml` (S3 design note §2).
//!
//! ```toml
//! http_port = 8480
//! layout = "layout.json"
//! allowed_hosts = ["foh.local"]
//! [[instances]]
//! name = "band"
//! port = 39101
//! [[instances]]
//! name = "master"
//! port = 39102
//!
//! # Remote access (#17), every table optional:
//! [tls]                  # the HTTPS listener for the one public name
//! name = "foh.example.org"
//! port = 443
//! redirect_http = true   # http://<name>:<http_port> -> https (never by IP, never a tunnel request)
//! [acme]                 # its Let's Encrypt certificate, by DNS-01 on Cloudflare
//! email = "owner@example.org"
//! directory = "https://acme-v02.api.letsencrypt.org/directory"
//! propagation_s = 20
//! [access]               # the Cloudflare Access app in front of the tunnel
//! team_domain = "team.cloudflareaccess.com"
//! aud = ["<the app's AUD tag>"]
//! [tunnel]               # cloudflared's readiness, reported in /api/status
//! ready_url = "http://127.0.0.1:20241/ready"
//!
//! # The Stream Deck tab (#52), optional: Bitfocus Companion's Satellite API
//! [companion]
//! host = "companion.example.org"
//! port = 16622           # the default
//! columns = 8            # 1..=16
//! rows = 4               # 1..=8
//! bitmap_px = 144        # 32..=288, the keys' image size
//! title = "Stream Deck"  # 1..=24 characters, the tab's title
//! ```
//!
//! Every key is optional; a missing file is the defaults. `allowed_hosts`
//! names the host names the clients may use besides an IP address and
//! `localhost` (a request for any other name is refused: DNS rebinding); the
//! `[tls]` name is allowed as well. Without `[access]` every request that
//! came through a proxy (the tunnel) or from a public address is refused.
//! The data folder
//! (`FOHMIXER_DATA`, default the working folder) also holds `secrets/`, the
//! layout, its backups and `hub-state.json`. A file that does not parse or
//! does not validate stops the start, naming the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde::Deserialize;

/// The configuration file inside the data folder.
pub const CONFIG_FILE: &str = "fohmixer-hub.toml";
/// The HTTP port when the file does not set one.
pub const DEFAULT_HTTP_PORT: u16 = 8480;
/// The layout file when the file does not name one.
pub const DEFAULT_LAYOUT: &str = "layout.json";
/// How often the layout file is checked for changes.
pub const DEFAULT_LAYOUT_POLL_MS: u64 = 2000;
/// The fastest allowed layout poll.
pub const MIN_LAYOUT_POLL_MS: u64 = 50;

/// The HTTPS port when `[tls]` does not set one.
pub const DEFAULT_HTTPS_PORT: u16 = 443;
/// Let's Encrypt's production directory, the `[acme]` default.
pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// How long a new DNS-01 record gets to reach Cloudflare's name servers
/// before the CA is asked to check it (certbot's Cloudflare plugin waits 10 s).
pub const DEFAULT_PROPAGATION_S: u64 = 20;
/// The longest allowed propagation wait.
pub const MAX_PROPAGATION_S: u64 = 600;
/// cloudflared's readiness endpoint (its `--metrics 127.0.0.1:20241`).
pub const DEFAULT_TUNNEL_READY_URL: &str = "http://127.0.0.1:20241/ready";
/// Companion's Satellite API port when `[companion]` does not set one.
pub const DEFAULT_COMPANION_PORT: u16 = 16622;
/// The Stream Deck's grid when `[companion]` does not set it: Companion's
/// standard page (a Stream Deck XL).
pub const DEFAULT_DECK_COLUMNS: u32 = 8;
pub const DEFAULT_DECK_ROWS: u32 = 4;
/// The size Companion draws each key's image at (px, square).
pub const DEFAULT_DECK_BITMAP_PX: u32 = 144;
/// The tab's title.
pub const DEFAULT_DECK_TITLE: &str = "Stream Deck";

/// One Live instance: its name (the `instance` of every command and
/// binding) and the localhost port of its FohMixer script.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceCfg {
    pub name: String,
    pub port: u16,
}

fn default_http_port() -> u16 {
    DEFAULT_HTTP_PORT
}

fn default_instances() -> Vec<InstanceCfg> {
    vec![
        InstanceCfg {
            name: "band".to_string(),
            port: 39101,
        },
        InstanceCfg {
            name: "master".to_string(),
            port: 39102,
        },
    ]
}

fn default_layout() -> PathBuf {
    PathBuf::from(DEFAULT_LAYOUT)
}

fn default_layout_poll_ms() -> u64 {
    DEFAULT_LAYOUT_POLL_MS
}

fn default_https_port() -> u16 {
    DEFAULT_HTTPS_PORT
}

fn default_true() -> bool {
    true
}

fn default_acme_directory() -> String {
    LETS_ENCRYPT.to_string()
}

fn default_propagation_s() -> u64 {
    DEFAULT_PROPAGATION_S
}

fn default_tunnel_ready_url() -> String {
    DEFAULT_TUNNEL_READY_URL.to_string()
}

fn default_companion_port() -> u16 {
    DEFAULT_COMPANION_PORT
}

fn default_deck_columns() -> u32 {
    DEFAULT_DECK_COLUMNS
}

fn default_deck_rows() -> u32 {
    DEFAULT_DECK_ROWS
}

fn default_deck_bitmap_px() -> u32 {
    DEFAULT_DECK_BITMAP_PX
}

fn default_deck_title() -> String {
    DEFAULT_DECK_TITLE.to_string()
}

/// `[tls]`: the HTTPS listener for the one public name (LAN by the router's
/// static DNS or the PC's hosts entry, the internet through the tunnel).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsCfg {
    pub name: String,
    #[serde(default = "default_https_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub redirect_http: bool,
}

/// `[acme]`: the certificate for the `[tls]` name from an ACME CA by DNS-01
/// through the Cloudflare DNS API (the token is `fohmixer-hub cloudflare
/// set-token`'s, never in this file).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeCfg {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default = "default_acme_directory")]
    pub directory: String,
    #[serde(default = "default_propagation_s")]
    pub propagation_s: u64,
}

/// `[access]`: the Cloudflare Access application whose signed assertion an
/// internet request must carry. Its team domain and AUD tag are public IDs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessCfg {
    pub team_domain: String,
    pub aud: Vec<String>,
    /// The signing keys; default `https://<team_domain>/cdn-cgi/access/certs`.
    #[serde(default)]
    pub jwks_url: Option<String>,
}

impl AccessCfg {
    /// The issuer every assertion must name.
    pub fn issuer(&self) -> String {
        format!("https://{}", self.team_domain)
    }

    /// Where the team's signing keys are fetched.
    pub fn jwks(&self) -> String {
        self.jwks_url
            .clone()
            .unwrap_or_else(|| format!("{}/cdn-cgi/access/certs", self.issuer()))
    }
}

/// `[tunnel]`: cloudflared's readiness endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelCfg {
    #[serde(default = "default_tunnel_ready_url")]
    pub ready_url: String,
}

/// `[companion]` (#52): the Stream Deck tab. The hub registers with
/// Bitfocus Companion's Satellite API at `host:port` as one Stream Deck of
/// `columns` x `rows` keys whose images Companion draws `bitmap_px` square;
/// the page's tab is titled `title`. The host is site data: the installer
/// writes it (`-CompanionHost`), never this repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionCfg {
    pub host: String,
    #[serde(default = "default_companion_port")]
    pub port: u16,
    #[serde(default = "default_deck_columns")]
    pub columns: u32,
    #[serde(default = "default_deck_rows")]
    pub rows: u32,
    #[serde(default = "default_deck_bitmap_px")]
    pub bitmap_px: u32,
    #[serde(default = "default_deck_title")]
    pub title: String,
}

impl CompanionCfg {
    /// The surface's keys: `columns` x `rows` (Companion's `KEYS_TOTAL`).
    pub fn keys(&self) -> u32 {
        self.columns * self.rows
    }
}

/// Whether `name` is a DNS name with at least two labels (a public name,
/// never an IP address): labels of letters, digits and inner hyphens, 1–63
/// characters, the last one with a letter.
pub fn is_dns_name(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    let label_ok = |label: &&str| {
        (1..=63).contains(&label.len())
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    labels.len() >= 2
        && name.len() <= 253
        && labels.iter().all(label_ok)
        && labels
            .last()
            .is_some_and(|tld| tld.chars().any(|c| c.is_ascii_alphabetic()))
}

/// Whether `value` is an e-mail address for the ACME account's contact:
/// `local@domain`, the domain a DNS name, no space, quote, angle bracket or
/// separator in the local part (the CA refuses the account otherwise).
pub fn is_email(value: &str) -> bool {
    value.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && is_dns_name(domain)
            && !local
                .chars()
                .any(|c| c.is_whitespace() || "\"<>@\\,;".contains(c))
    })
}

/// Whether `value` is an Access application's AUD tag: letters, digits, `-`
/// and `_` (Cloudflare's are 64 hex digits), never empty.
pub fn is_aud(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a URL may be fetched by the hub: `https://…`, or `http://` to a
/// loopback address (test doubles, cloudflared's readiness), never plain
/// http across a network.
pub fn url_allowed(url: &str) -> bool {
    let Ok(uri) = url.parse::<axum::http::Uri>() else {
        return false;
    };
    let host = uri.host().unwrap_or("");
    match uri.scheme_str() {
        Some("https") => true,
        Some("http") => {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            host.eq_ignore_ascii_case("localhost")
                || bare
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        _ => false,
    }
}

/// The hub's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_instances")]
    pub instances: Vec<InstanceCfg>,
    /// The layout file, relative to the data folder.
    #[serde(default = "default_layout")]
    pub layout: PathBuf,
    #[serde(default = "default_layout_poll_ms")]
    pub layout_poll_ms: u64,
    /// Host names the clients may use, besides IP addresses and `localhost`.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub tls: Option<TlsCfg>,
    #[serde(default)]
    pub acme: Option<AcmeCfg>,
    #[serde(default)]
    pub access: Option<AccessCfg>,
    #[serde(default)]
    pub tunnel: Option<TunnelCfg>,
    /// The Stream Deck tab (#52); none: no tab.
    #[serde(default)]
    pub companion: Option<CompanionCfg>,
    /// The data folder (where the file was read from).
    #[serde(skip)]
    pub data_dir: PathBuf,
}

impl Config {
    /// The defaults, in `data_dir`.
    pub fn defaults(data_dir: &Path) -> Self {
        Self {
            http_port: DEFAULT_HTTP_PORT,
            instances: default_instances(),
            layout: default_layout(),
            layout_poll_ms: DEFAULT_LAYOUT_POLL_MS,
            allowed_hosts: Vec::new(),
            tls: None,
            acme: None,
            access: None,
            tunnel: None,
            companion: None,
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// `<data_dir>/fohmixer-hub.toml`, or the defaults when it does not
    /// exist.
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                Self::parse(&text, data_dir).with_context(|| format!("config {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file: the defaults");
                Ok(Self::defaults(data_dir))
            }
            Err(e) => Err(e).with_context(|| format!("reading config {}", path.display())),
        }
    }

    /// `fohmixer-hub config check <file>` (the installer runs it before it
    /// stops the running hub): `file` read and validated as the config, its
    /// folder as the data folder. The error names the file and the problem.
    pub fn check_file(file: &Path) -> anyhow::Result<()> {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("reading config {}", file.display()))?;
        let dir = file.parent().unwrap_or(Path::new("."));
        Self::parse(&text, dir).with_context(|| format!("config {}", file.display()))?;
        Ok(())
    }

    /// Parses and validates a config text for `data_dir`.
    pub fn parse(text: &str, data_dir: &Path) -> anyhow::Result<Self> {
        let mut config: Self = toml::from_str(text)?;
        config.data_dir = data_dir.to_path_buf();
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut names = HashSet::new();
        let mut ports = HashSet::new();
        for instance in &self.instances {
            let name = instance.name.as_str();
            if name.is_empty() || name.contains('|') || name.trim() != name {
                bail!("instance name {name:?}: not empty, no `|`, no outer spaces");
            }
            if !names.insert(name) {
                bail!("instance {name:?} is listed twice");
            }
            if instance.port == 0 {
                bail!("instance {name:?}: port 0");
            }
            if !ports.insert(instance.port) {
                bail!(
                    "instance {name:?}: port {} is another instance's",
                    instance.port
                );
            }
        }
        for host in &self.allowed_hosts {
            if host.is_empty() || host.contains(|c: char| c == ':' || c.is_whitespace()) {
                bail!("allowed host {host:?}: a name without a port or spaces");
            }
        }
        if self.layout_poll_ms < MIN_LAYOUT_POLL_MS {
            bail!(
                "layout_poll_ms {} is below {MIN_LAYOUT_POLL_MS}",
                self.layout_poll_ms
            );
        }
        self.validate_companion()?;
        self.validate_remote()
    }

    /// `[companion]` (#52): a host without spaces or quotes, a port, the
    /// grid's and the image's bounds, a title of 1..=24 characters.
    fn validate_companion(&self) -> anyhow::Result<()> {
        let Some(deck) = &self.companion else {
            return Ok(());
        };
        if deck.host.is_empty() || deck.host.contains(|c: char| c.is_whitespace() || c == '"') {
            bail!(
                "[companion] host {:?}: a name or address without spaces",
                deck.host
            );
        }
        if deck.port == 0 {
            bail!("[companion] port 0");
        }
        if !(1..=16).contains(&deck.columns) {
            bail!("[companion] columns {}: 1..=16", deck.columns);
        }
        if !(1..=8).contains(&deck.rows) {
            bail!("[companion] rows {}: 1..=8", deck.rows);
        }
        if !(32..=288).contains(&deck.bitmap_px) {
            bail!("[companion] bitmap_px {}: 32..=288", deck.bitmap_px);
        }
        if !(1..=24).contains(&deck.title.chars().count()) {
            bail!("[companion] title {:?}: 1..=24 characters", deck.title);
        }
        Ok(())
    }

    /// The remote-access tables (#17).
    fn validate_remote(&self) -> anyhow::Result<()> {
        if let Some(tls) = &self.tls {
            if !is_dns_name(&tls.name) {
                bail!("[tls] name {:?}: a DNS name like foh.example.org", tls.name);
            }
            if tls.port == self.http_port {
                bail!("[tls] port {} is the HTTP port", tls.port);
            }
        }
        if let Some(acme) = &self.acme {
            if self.tls.is_none() {
                bail!("[acme] needs [tls]: the certificate is for its name");
            }
            if !url_allowed(&acme.directory) {
                bail!("[acme] directory {:?}: an https URL", acme.directory);
            }
            if let Some(email) = &acme.email
                && !is_email(email)
            {
                bail!("[acme] email {email:?}: an address like owner@example.org");
            }
            if acme.propagation_s > MAX_PROPAGATION_S {
                bail!(
                    "[acme] propagation_s {} is above {MAX_PROPAGATION_S}",
                    acme.propagation_s
                );
            }
        }
        if let Some(access) = &self.access {
            if !is_dns_name(&access.team_domain) {
                bail!(
                    "[access] team_domain {:?}: a name like team.cloudflareaccess.com",
                    access.team_domain
                );
            }
            if access.aud.is_empty() || access.aud.iter().any(|a| !is_aud(a)) {
                bail!(
                    "[access] aud {:?}: the Access application's AUD tag(s), letters, digits, - and _",
                    access.aud
                );
            }
            if !url_allowed(&access.jwks()) {
                bail!("[access] jwks_url {:?}: an https URL", access.jwks());
            }
        }
        if let Some(tunnel) = &self.tunnel
            && !url_allowed(&tunnel.ready_url)
        {
            bail!(
                "[tunnel] ready_url {:?}: cloudflared's local readiness URL",
                tunnel.ready_url
            );
        }
        Ok(())
    }

    /// The host names a request may name besides IP addresses and
    /// `localhost`: `allowed_hosts` and the `[tls]` name.
    pub fn trusted_hosts(&self) -> Vec<String> {
        let mut hosts = self.allowed_hosts.clone();
        hosts.extend(self.tls.iter().map(|tls| tls.name.clone()));
        hosts
    }

    /// The layout file's path.
    pub fn layout_path(&self) -> PathBuf {
        self.data_dir.join(&self.layout)
    }

    /// Whether `name` is a configured instance.
    pub fn has_instance(&self, name: &str) -> bool {
        self.instances.iter().any(|i| i.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> PathBuf {
        PathBuf::from("/data")
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let config = Config::parse("", &data()).unwrap();
        assert_eq!(config, Config::defaults(&data()));
        assert_eq!(config.http_port, 8480);
        assert_eq!(
            config.instances,
            vec![
                InstanceCfg {
                    name: "band".into(),
                    port: 39101
                },
                InstanceCfg {
                    name: "master".into(),
                    port: 39102
                }
            ]
        );
        assert_eq!(config.layout_path(), PathBuf::from("/data/layout.json"));
        assert_eq!(config.layout_poll_ms, 2000);
    }

    #[test]
    fn every_key_is_read() {
        let config = Config::parse(
            "http_port = 9000\nlayout = \"surface.json\"\nlayout_poll_ms = 100\n\
             allowed_hosts = [\"foh.local\"]\n\
             [[instances]]\nname = \"band\"\nport = 40001\n",
            &data(),
        )
        .unwrap();
        assert_eq!(config.http_port, 9000);
        assert_eq!(config.layout_path(), PathBuf::from("/data/surface.json"));
        assert_eq!(config.layout_poll_ms, 100);
        assert_eq!(config.allowed_hosts, vec!["foh.local".to_string()]);
        assert_eq!(config.instances.len(), 1);
        assert!(config.has_instance("band"));
        assert!(!config.has_instance("master"));
        let none = Config::parse("instances = []\n", &data()).unwrap();
        assert!(none.instances.is_empty());
    }

    #[test]
    fn bad_configs_are_refused() {
        for (text, message) in [
            (
                "[[instances]]\nname = \"a\"\nport = 1\n[[instances]]\nname = \"a\"\nport = 2\n",
                "listed twice",
            ),
            ("[[instances]]\nname = \"\"\nport = 1\n", "not empty"),
            ("[[instances]]\nname = \"a|b\"\nport = 1\n", "not empty"),
            ("[[instances]]\nname = \" a\"\nport = 1\n", "not empty"),
            ("[[instances]]\nname = \"a\"\nport = 0\n", "port 0"),
            (
                "[[instances]]\nname = \"a\"\nport = 1\n[[instances]]\nname = \"b\"\nport = 1\n",
                "port 1 is another instance's",
            ),
            ("allowed_hosts = [\"\"]\n", "without a port"),
            ("allowed_hosts = [\"foh:8480\"]\n", "without a port"),
            ("allowed_hosts = [\"foh local\"]\n", "without a port"),
            ("layout_poll_ms = 49\n", "below 50"),
            ("http_port = \"x\"\n", "invalid type"),
            ("colour = 1\n", "unknown field"),
        ] {
            let error = format!("{:#}", Config::parse(text, &data()).unwrap_err());
            assert!(error.contains(message), "{text}: {error}");
        }
        assert!(Config::parse("layout_poll_ms = 50\n", &data()).is_ok());
        assert!(
            Config::parse(
                "[[instances]]\nname = \"a\"\nport = 1\n[[instances]]\nname = \"b\"\nport = 2\n",
                &data()
            )
            .is_ok()
        );
    }

    const REMOTE: &str = "[tls]\nname = \"foh.example.org\"\n\
        [acme]\nemail = \"owner@example.org\"\n\
        [access]\nteam_domain = \"team.cloudflareaccess.com\"\naud = [\"aud-1\", \"aud-2\"]\n\
        [tunnel]\n";

    #[test]
    fn the_remote_tables_are_read_with_their_defaults() {
        let config = Config::parse(REMOTE, &data()).unwrap();
        assert_eq!(
            config.tls,
            Some(TlsCfg {
                name: "foh.example.org".into(),
                port: 443,
                redirect_http: true
            })
        );
        assert_eq!(
            config.acme,
            Some(AcmeCfg {
                email: Some("owner@example.org".into()),
                directory: "https://acme-v02.api.letsencrypt.org/directory".into(),
                propagation_s: 20
            })
        );
        let access = config.access.clone().unwrap();
        assert_eq!(access.aud, vec!["aud-1".to_string(), "aud-2".to_string()]);
        assert_eq!(access.issuer(), "https://team.cloudflareaccess.com");
        assert_eq!(
            access.jwks(),
            "https://team.cloudflareaccess.com/cdn-cgi/access/certs"
        );
        assert_eq!(
            config.tunnel.as_ref().unwrap().ready_url,
            "http://127.0.0.1:20241/ready"
        );
        assert_eq!(
            config.trusted_hosts(),
            vec!["foh.example.org".to_string()],
            "the public name is a trusted host"
        );
        let explicit = Config::parse(
            "allowed_hosts = [\"foh.local\"]\n[tls]\nname = \"foh.example.org\"\nport = 8443\n\
             redirect_http = false\n[acme]\ndirectory = \"http://127.0.0.1:9/dir\"\npropagation_s = 0\n\
             [access]\nteam_domain = \"t.example.com\"\naud = [\"a\"]\n\
             jwks_url = \"http://127.0.0.1:9/certs\"\n[tunnel]\nready_url = \"http://127.0.0.1:1/ready\"\n",
            &data(),
        )
        .unwrap();
        let tls = explicit.tls.clone().unwrap();
        assert_eq!((tls.port, tls.redirect_http), (8443, false));
        assert_eq!(explicit.acme.clone().unwrap().propagation_s, 0);
        assert_eq!(explicit.acme.as_ref().unwrap().email, None);
        assert_eq!(
            explicit.access.as_ref().unwrap().jwks(),
            "http://127.0.0.1:9/certs"
        );
        assert_eq!(
            explicit.trusted_hosts(),
            vec!["foh.local".to_string(), "foh.example.org".to_string()]
        );
        let none = Config::parse("", &data()).unwrap();
        assert!(none.trusted_hosts().is_empty());
        assert_eq!(
            (none.tls, none.acme, none.access, none.tunnel),
            (None, None, None, None)
        );
    }

    #[test]
    fn bad_remote_tables_are_refused() {
        for (text, message) in [
            ("[tls]\nname = \"foh\"\n", "a DNS name"),
            ("[tls]\nname = \"10.0.0.5\"\n", "a DNS name"),
            (
                "[tls]\nname = \"foh.example.org\"\nport = 8480\n",
                "is the HTTP port",
            ),
            (
                "[tls]\nname = \"foh.example.org\"\ncolour = 1\n",
                "unknown field",
            ),
            ("[acme]\n", "[acme] needs [tls]"),
            (
                "[tls]\nname = \"foh.example.org\"\n[acme]\ndirectory = \"http://ca.example.org/dir\"\n",
                "an https URL",
            ),
            (
                "[tls]\nname = \"foh.example.org\"\n[acme]\npropagation_s = 601\n",
                "above 600",
            ),
            (
                "[tls]\nname = \"foh.example.org\"\n[acme]\nemail = \"owner example.org\"\n",
                "an address like owner@example.org",
            ),
            (
                "[access]\nteam_domain = \"team\"\naud = [\"a\"]\n",
                "team.cloudflareaccess.com",
            ),
            (
                "[access]\nteam_domain = \"t.example.com\"\naud = []\n",
                "AUD tag",
            ),
            (
                "[access]\nteam_domain = \"t.example.com\"\naud = [\" \"]\n",
                "AUD tag",
            ),
            (
                "[access]\nteam_domain = \"t.example.com\"\naud = [\"a\", \"b.c\"]\n",
                "AUD tag",
            ),
            (
                "[access]\nteam_domain = \"t.example.com\"\naud = [\"a\"]\njwks_url = \"http://keys.example.com/\"\n",
                "jwks_url",
            ),
            (
                "[tunnel]\nready_url = \"http://10.0.0.5:20241/ready\"\n",
                "readiness URL",
            ),
        ] {
            let error = format!("{:#}", Config::parse(text, &data()).unwrap_err());
            assert!(error.contains(message), "{text}: {error}");
        }
        assert!(
            Config::parse(
                "[tls]\nname = \"foh.example.org\"\n[acme]\npropagation_s = 600\n",
                &data()
            )
            .is_ok()
        );
    }

    #[test]
    fn a_dns_name_has_two_labels_and_no_address() {
        for name in [
            "foh.example.org",
            "FOH.Example.org",
            "a-b.example.org",
            "x.co",
            "e2e.cloudflareaccess.test",
            &format!("{}.org", "a".repeat(63)),
        ] {
            assert!(is_dns_name(name), "{name}");
        }
        for name in [
            "",
            "foh",
            "foh.",
            ".foh.org",
            "foh..org",
            "-foh.org",
            "foh-.org",
            "foh.org-",
            "f_h.org",
            "foh.example.org:443",
            "10.0.0.5",
            "foh.123",
            &format!("{}.org", "a".repeat(64)),
        ] {
            assert!(!is_dns_name(name), "{name}");
        }
        // 253 characters is the longest name.
        let long = |last: usize| {
            format!(
                "{}.{}.{}.{}.org",
                "a".repeat(63),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(last)
            )
        };
        assert_eq!(long(57).len(), 253);
        assert!(is_dns_name(&long(57)));
        assert!(!is_dns_name(&long(58)));
    }

    #[test]
    fn a_fetched_url_is_https_or_http_on_loopback() {
        for url in [
            "https://acme-v02.api.letsencrypt.org/directory",
            "https://team.cloudflareaccess.com/cdn-cgi/access/certs",
            "http://127.0.0.1:20241/ready",
            "http://localhost:9/x",
            "http://[::1]:9/x",
            "http://127.1.2.3/",
        ] {
            assert!(url_allowed(url), "{url}");
        }
        for url in [
            "http://10.0.0.5:20241/ready",
            "http://example.org/",
            "ftp://127.0.0.1/",
            "/relative",
            "not a url",
            "",
        ] {
            assert!(!url_allowed(url), "{url}");
        }
    }

    #[test]
    fn load_reads_the_file_or_takes_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load(dir.path()).unwrap(),
            Config::defaults(dir.path())
        );
        std::fs::write(dir.path().join(CONFIG_FILE), "http_port = 8500\n").unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.http_port, 8500);
        assert_eq!(config.data_dir, dir.path());
        std::fs::write(dir.path().join(CONFIG_FILE), "http_port = 70000\n").unwrap();
        let error = format!("{:#}", Config::load(dir.path()).unwrap_err());
        assert!(error.starts_with("config "), "{error}");
        assert!(error.contains(CONFIG_FILE), "{error}");
    }

    #[test]
    fn an_acme_contact_is_an_address() {
        for good in ["owner@example.org", "a.b-c+d@foh.example.org"] {
            assert!(is_email(good), "{good}");
        }
        for bad in [
            "",
            "owner",
            "owner.example.org",
            "@example.org",
            "owner@",
            "owner@localhost",
            "owner@10.0.0.5",
            "own er@example.org",
            "own\ter@example.org",
            "\"owner\"@example.org",
            "<owner>@example.org",
            "a@b@example.org",
            "a,b@example.org",
            "a;b@example.org",
            "a\\b@example.org",
        ] {
            assert!(!is_email(bad), "{bad}");
        }
    }

    #[test]
    fn an_aud_tag_is_letters_digits_hyphens_and_underscores() {
        for good in ["a", "aud-1", "A_b-9", &"f".repeat(64)] {
            assert!(is_aud(good), "{good}");
        }
        for bad in [
            "",
            " ",
            "aud 1",
            "aud.1",
            "aud\"1",
            "aud\n",
            "aud/1",
            "aud\u{e9}",
        ] {
            assert!(!is_aud(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_config_file_is_checked_as_the_hub_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.toml");
        assert!(
            format!("{:#}", Config::check_file(&file).unwrap_err()).starts_with("reading config ")
        );
        std::fs::write(&file, "http_port = 8500\n").unwrap();
        Config::check_file(&file).unwrap();
        std::fs::write(
            &file,
            "http_port = 8480\n[tls]\nname = \"foh.example.org\"\nport = 8480\n",
        )
        .unwrap();
        let error = format!("{:#}", Config::check_file(&file).unwrap_err());
        assert!(error.starts_with("config "), "{error}");
        assert!(error.contains("new.toml"), "{error}");
        assert!(
            error.ends_with("[tls] port 8480 is the HTTP port"),
            "{error}"
        );
    }

    #[test]
    fn an_unreadable_config_is_an_error_not_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        let error = format!("{:#}", Config::load(dir.path()).unwrap_err());
        assert!(error.starts_with("reading config "), "{error}");
    }

    #[test]
    fn the_companion_table_is_read_with_its_defaults() {
        let config = Config::parse("[companion]\nhost = \"10.0.0.7\"\n", &data()).unwrap();
        let deck = config.companion.unwrap();
        assert_eq!(
            deck,
            CompanionCfg {
                host: "10.0.0.7".into(),
                port: 16622,
                columns: 8,
                rows: 4,
                bitmap_px: 144,
                title: "Stream Deck".into(),
            }
        );
        assert_eq!(deck.keys(), 32);
        let full = Config::parse(
            "[companion]\nhost = \"companion.example.org\"\nport = 16700\ncolumns = 5\nrows = 3\n\
             bitmap_px = 72\ntitle = \"Deck\"\n",
            &data(),
        )
        .unwrap()
        .companion
        .unwrap();
        assert_eq!(
            (
                full.host.as_str(),
                full.port,
                full.columns,
                full.rows,
                full.bitmap_px,
                full.title.as_str()
            ),
            ("companion.example.org", 16700, 5, 3, 72, "Deck")
        );
        assert_eq!(full.keys(), 15);
        assert_eq!(Config::parse("", &data()).unwrap().companion, None);
    }

    #[test]
    fn a_bad_companion_table_is_refused_at_its_bounds() {
        let table = |extra: &str| format!("[companion]\nhost = \"10.0.0.7\"\n{extra}");
        for (text, message) in [
            (
                "[companion]\nport = 1\n".to_string(),
                "missing field `host`",
            ),
            (
                "[companion]\nhost = \"\"\n".to_string(),
                "[companion] host \"\"",
            ),
            (
                "[companion]\nhost = \"a b\"\n".to_string(),
                "[companion] host \"a b\"",
            ),
            (
                "[companion]\nhost = \"a\\\"b\"\n".to_string(),
                "[companion] host \"a\\\"b\"",
            ),
            (table("port = 0\n"), "[companion] port 0"),
            (table("columns = 0\n"), "[companion] columns 0: 1..=16"),
            (table("columns = 17\n"), "[companion] columns 17: 1..=16"),
            (table("rows = 0\n"), "[companion] rows 0: 1..=8"),
            (table("rows = 9\n"), "[companion] rows 9: 1..=8"),
            (
                table("bitmap_px = 31\n"),
                "[companion] bitmap_px 31: 32..=288",
            ),
            (
                table("bitmap_px = 289\n"),
                "[companion] bitmap_px 289: 32..=288",
            ),
            (
                table("title = \"\"\n"),
                "[companion] title \"\": 1..=24 characters",
            ),
            (
                table("title = \"Stream Deck of the FOH 12\"\n"),
                "1..=24 characters",
            ),
            (table("colour = 1\n"), "unknown field"),
        ] {
            let error = format!("{:#}", Config::parse(&text, &data()).unwrap_err());
            assert!(error.contains(message), "{text}: {error}");
        }
        for ok in [
            "columns = 1\nrows = 1\nbitmap_px = 32\n",
            "columns = 16\nrows = 8\nbitmap_px = 288\n",
            // 24 characters, one of them not ASCII: characters, not bytes.
            "title = \"Stream Deck of the FOH Ž\"\n",
        ] {
            assert!(Config::parse(&table(ok), &data()).is_ok(), "{ok}");
        }
    }
}
