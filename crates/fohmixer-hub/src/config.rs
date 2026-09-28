//! The hub's configuration: `<data>/fohmixer-hub.toml` (S3 design note §2).
//!
//! ```toml
//! http_port = 8480
//! layout = "layout.json"
//! [[instances]]
//! name = "band"
//! port = 39101
//! [[instances]]
//! name = "master"
//! port = 39102
//! ```
//!
//! Every key is optional; a missing file is the defaults. The data folder
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

    /// Parses and validates a config text for `data_dir`.
    pub fn parse(text: &str, data_dir: &Path) -> anyhow::Result<Self> {
        let mut config: Self = toml::from_str(text)?;
        config.data_dir = data_dir.to_path_buf();
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        let mut names = HashSet::new();
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
        }
        if self.layout_poll_ms < MIN_LAYOUT_POLL_MS {
            bail!(
                "layout_poll_ms {} is below {MIN_LAYOUT_POLL_MS}",
                self.layout_poll_ms
            );
        }
        Ok(())
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
             [[instances]]\nname = \"band\"\nport = 40001\n",
            &data(),
        )
        .unwrap();
        assert_eq!(config.http_port, 9000);
        assert_eq!(config.layout_path(), PathBuf::from("/data/surface.json"));
        assert_eq!(config.layout_poll_ms, 100);
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
            ("layout_poll_ms = 49\n", "below 50"),
            ("http_port = \"x\"\n", "invalid type"),
            ("colour = 1\n", "unknown field"),
        ] {
            let error = format!("{:#}", Config::parse(text, &data()).unwrap_err());
            assert!(error.contains(message), "{text}: {error}");
        }
        assert!(Config::parse("layout_poll_ms = 50\n", &data()).is_ok());
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
    fn an_unreadable_config_is_an_error_not_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        let error = format!("{:#}", Config::load(dir.path()).unwrap_err());
        assert!(error.starts_with("reading config "), "{error}");
    }
}
