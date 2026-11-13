//! The instance link's config (#184, spec §6.1): a linked instance's
//! `link` URL and the MBC hub's `[link]` table.
//!
//! ```toml
//! # The MBC hub: serves its local instance `mbc` on the LAN.
//! [link]
//! listen = "10.0.0.5:39201"   # a given address, never 0.0.0.0 or ::
//! instance = "mbc"            # a local instance (one with a port)
//!
//! # The FOH hub: instance `mbc` is linked, reached through that link.
//! [[instances]]
//! name = "mbc"
//! link = "ws://10.0.0.5:39201"
//! ```

use std::collections::{BTreeSet, HashSet};
use std::net::SocketAddr;

use anyhow::bail;
use serde::Deserialize;

use super::InstanceCfg;

/// `[link]`: where the MBC hub serves its instance link, and which local
/// instance it relays.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkCfg {
    pub listen: SocketAddr,
    pub instance: String,
}

/// Whether `url` may be a linked instance's link: `ws://`, a host name or
/// an IPv4 address, an explicit port (not 0), at most a `/` after it, no
/// credentials, no query or fragment.
pub fn is_link_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("ws://") else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains(['@', '/', '?', '#', '[']) {
        return false;
    }
    let Some((host, port)) = rest.rsplit_once(':') else {
        return false;
    };
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    host_ok && port.parse::<u16>().is_ok_and(|p| p != 0)
}

/// One instance's way to its Live: a port of a local script, or a link.
pub(super) fn validate_instance(instance: &InstanceCfg) -> anyhow::Result<()> {
    let name = instance.name.as_str();
    match (instance.port, &instance.link) {
        (0, None) => bail!("instance {name:?}: a port or a link"),
        (p, Some(_)) if p != 0 => bail!("instance {name:?}: port or link, not both"),
        (_, Some(url)) if !is_link_url(url) => {
            bail!("instance {name:?}: link {url:?} is ws://<host>:<port> with nothing after it")
        }
        _ => Ok(()),
    }
}

/// No two instances share a link.
pub(super) fn validate_links(instances: &[InstanceCfg]) -> anyhow::Result<()> {
    let mut links = HashSet::new();
    for instance in instances {
        if let Some(url) = &instance.link
            && !links.insert(url.as_str())
        {
            bail!(
                "instance {:?}: link {url:?} is another instance's",
                instance.name
            );
        }
    }
    Ok(())
}

/// `[link]`: a given address (never every address), a port, a local
/// instance.
pub(super) fn validate_listen(link: &LinkCfg, instances: &[InstanceCfg]) -> anyhow::Result<()> {
    if link.listen.ip().is_unspecified() {
        bail!(
            "[link] listen {}: a given address, never every address",
            link.listen
        );
    }
    if link.listen.port() == 0 {
        bail!("[link] listen {}: port 0", link.listen);
    }
    let local = instances
        .iter()
        .any(|i| i.name == link.instance && i.link.is_none());
    if !local {
        bail!(
            "[link] instance {:?}: not a local instance of this hub",
            link.instance
        );
    }
    Ok(())
}

/// The names of the linked instances.
pub(super) fn linked(instances: &[InstanceCfg]) -> BTreeSet<String> {
    instances
        .iter()
        .filter(|i| i.link.is_some())
        .map(|i| i.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::Config;
    use super::*;

    fn parse(text: &str) -> anyhow::Result<Config> {
        Config::parse(text, &PathBuf::from("/data"))
    }

    fn refused(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn a_linked_instance_has_a_link_and_no_port() {
        let config = parse(
            "[[instances]]\nname = \"band\"\nport = 39101\n\
             [[instances]]\nname = \"mbc\"\nlink = \"ws://10.0.0.5:39201\"\n",
        )
        .unwrap();
        let mbc = &config.instances[1];
        assert_eq!(mbc.port, 0);
        assert_eq!(mbc.link.as_deref(), Some("ws://10.0.0.5:39201"));
        assert_eq!(config.instances[0].link, None);
        assert_eq!(
            config.linked_instances(),
            BTreeSet::from(["mbc".to_string()])
        );
        assert!(config.link.is_none());
    }

    #[test]
    fn port_and_link_together_are_refused() {
        assert!(
            refused(
                "[[instances]]\nname = \"mbc\"\nport = 39103\nlink = \"ws://10.0.0.5:39201\"\n"
            )
            .contains("instance \"mbc\": port or link, not both")
        );
        assert!(
            refused("[[instances]]\nname = \"mbc\"\n")
                .contains("instance \"mbc\": a port or a link")
        );
        assert!(
            refused(
                "[[instances]]\nname = \"a\"\nlink = \"ws://10.0.0.5:39201\"\n\
                 [[instances]]\nname = \"b\"\nlink = \"ws://10.0.0.5:39201\"\n"
            )
            .contains("is another instance's")
        );
        assert!(
            refused("[[instances]]\nname = \"mbc\"\nlink = \"wss://10.0.0.5:39201\"\n")
                .contains("ws://<host>:<port>")
        );
    }

    #[test]
    fn a_link_url_is_ws_with_a_port_and_nothing_else() {
        for (url, ok) in [
            ("ws://10.0.0.5:39201", true),
            ("ws://10.0.0.5:39201/", true),
            ("ws://mbc-hub.lan:39201", true),
            ("ws://127.0.0.1:1", true),
            ("wss://10.0.0.5:39201", false),
            ("http://10.0.0.5:39201", false),
            ("ws://10.0.0.5", false),
            ("ws://10.0.0.5:", false),
            ("ws://10.0.0.5:0", false),
            ("ws://10.0.0.5:65536", false),
            ("ws://10.0.0.5:39201/x", false),
            ("ws://user@10.0.0.5:39201", false),
            ("ws://10.0.0.5:39201?q", false),
            ("ws://10.0.0.5:39201#f", false),
            ("ws://[::1]:39201", false),
            ("ws://:39201", false),
            ("ws://bad host:39201", false),
            ("ws://a_b:39201", false),
            ("", false),
        ] {
            assert_eq!(is_link_url(url), ok, "{url}");
        }
    }

    #[test]
    fn the_link_listens_on_a_given_address_never_on_all() {
        let ok = parse(
            "[[instances]]\nname = \"mbc\"\nport = 39103\n\
             [link]\nlisten = \"10.0.0.5:39201\"\ninstance = \"mbc\"\n",
        )
        .unwrap();
        assert_eq!(
            ok.link,
            Some(LinkCfg {
                listen: "10.0.0.5:39201".parse().unwrap(),
                instance: "mbc".to_string(),
            })
        );
        for listen in ["0.0.0.0:39201", "[::]:39201"] {
            let text = format!(
                "[[instances]]\nname = \"mbc\"\nport = 39103\n\
                 [link]\nlisten = \"{listen}\"\ninstance = \"mbc\"\n"
            );
            assert!(
                refused(&text).contains("a given address, never every address"),
                "{listen}"
            );
        }
        assert!(
            refused(
                "[[instances]]\nname = \"mbc\"\nport = 39103\n\
                 [link]\nlisten = \"10.0.0.5:0\"\ninstance = \"mbc\"\n"
            )
            .contains("port 0")
        );
        assert!(
            refused(
                "[[instances]]\nname = \"mbc\"\nport = 39103\n\
                 [link]\nlisten = \"10.0.0.5:39201\"\ninstance = \"mbc\"\nextra = 1\n"
            )
            .contains("unknown field")
        );
    }

    #[test]
    fn the_link_serves_a_local_instance_only() {
        assert!(
            refused(
                "[[instances]]\nname = \"mbc\"\nlink = \"ws://10.0.0.6:39201\"\n\
                 [link]\nlisten = \"10.0.0.5:39201\"\ninstance = \"mbc\"\n"
            )
            .contains("[link] instance \"mbc\": not a local instance")
        );
        assert!(
            refused(
                "[[instances]]\nname = \"band\"\nport = 39101\n\
                 [link]\nlisten = \"10.0.0.5:39201\"\ninstance = \"mbc\"\n"
            )
            .contains("not a local instance")
        );
    }
}
