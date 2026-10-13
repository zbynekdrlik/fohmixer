//! AbleSet's link and its page message (#90 B3, design §3.6, §5.1).
//!
//! The hub reads AbleSet's index (`GET <upstream>/`, a few kB, never its
//! undocumented `/api`) every [`PERIOD`] with its own HTTP client. A 2xx or
//! 3xx answer is a good read; any other answer, a refused connect or no
//! answer within the period is a failed one. [`link_step`] decides: down
//! only after [`DOWN_AFTER`] failed reads in a row (one never tears an open
//! tab down), up again at the first good one, a [`Note`] only on a change.
//! A change is a log line (info up, warn down), an `ableset_link` record
//! (warn class, `events::is_warn`), `/api/status` `ableset` and, through
//! the router, the `ableset` message ([`Page::msg`]) to every page.

use std::sync::Arc;
use std::time::Duration;

use axum::http::uri::Authority;
use fohmixer_proto::client::ServerMsg;
use serde_json::{Value, json};

use super::AbleSet;
use crate::config::{AbleSetCfg, TlsCfg};
use crate::http_client::HttpClient;

/// How often AbleSet's index is read, and how long a read may take.
pub const PERIOD: Duration = Duration::from_secs(5);

/// Failed reads in a row that take the link down (about 10 s).
pub const DOWN_AFTER: u32 = 2;

/// The link as the hub knows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Link {
    /// Up or down; none before the first change (not known yet: the pages
    /// are told it is not up).
    pub online: Option<bool>,
    /// Failed reads in a row.
    pub failures: u32,
}

/// A change of the link, with the failed reads in a row: those before it
/// came up, or those that took it down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    Up { failures: u32 },
    Down { failures: u32 },
}

impl Note {
    /// Whether AbleSet is up now.
    pub fn up(self) -> bool {
        matches!(self, Self::Up { .. })
    }
}

/// The link after a read (`ok`: a good one), and the change, if any (pure).
pub fn link_step(link: Link, ok: bool) -> (Link, Option<Note>) {
    if ok {
        let note = (link.online != Some(true)).then_some(Note::Up {
            failures: link.failures,
        });
        let up = Link {
            online: Some(true),
            failures: 0,
        };
        return (up, note);
    }
    let failures = link.failures.saturating_add(1);
    let down = failures >= DOWN_AFTER && link.online != Some(false);
    let online = if down { Some(false) } else { link.online };
    let note = down.then_some(Note::Down { failures });
    (Link { online, failures }, note)
}

/// A read's answer (pure): a 2xx or 3xx is good; else why not.
pub fn answered(status: u16) -> Result<(), String> {
    if (200..400).contains(&status) {
        Ok(())
    } else {
        Err(format!("AbleSet answers {status}"))
    }
}

/// The link's client: plain http on loopback only (`config::is_upstream`),
/// so no certificate roots; a read bounded by `period`.
pub fn client(period: Duration) -> HttpClient {
    HttpClient::with_roots(rustls::RootCertStore::empty()).with_timeout(period)
}

/// One read of AbleSet's index at `url`: good, or why not.
pub async fn read(http: &HttpClient, url: &str) -> Result<(), String> {
    match http.get(url).await {
        Ok(reply) => answered(reply.status),
        Err(e) => Err(format!("AbleSet does not answer: {e:#}")),
    }
}

/// The `ableset_link` record of a change: up or not, why it went down and
/// the failed reads in a row.
pub fn link_fields(note: Note, error: Option<&str>) -> Value {
    match note {
        Note::Up { failures } => json!({"up": true, "failures": failures}),
        Note::Down { failures } => json!({"up": false, "error": error, "failures": failures}),
    }
}

/// Reads AbleSet's index at `url` every `period` into `site`'s link
/// ([`AbleSet::polled`]), handing each change to `changed` (up or not).
pub async fn watch<F>(
    site: Arc<AbleSet>,
    http: HttpClient,
    url: String,
    period: Duration,
    changed: F,
) where
    F: Fn(bool) + Send + 'static,
{
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let read = read(&http, &url).await;
        if let Some(up) = site.polled(read, crate::auth::now_secs() as i64) {
            changed(up);
        }
    }
}

/// What the pages are told of AbleSet's page: the `ableset` message but its
/// link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub name: String,
    pub parent: String,
    pub lan_port: u16,
}

impl Page {
    /// The page of `[ableset]` beside `[tls]`.
    pub fn of(cfg: &AbleSetCfg, tls: &TlsCfg) -> Self {
        Self {
            title: cfg.title.clone(),
            name: cfg.name.clone(),
            parent: tls.name.clone(),
            lan_port: lan_port(&cfg.upstream),
        }
    }

    /// The `ableset` message with the link `online`.
    pub fn msg(&self, online: bool) -> ServerMsg {
        ServerMsg::Ableset {
            title: self.title.clone(),
            name: self.name.clone(),
            parent: self.parent.clone(),
            online,
            lan_port: self.lan_port,
        }
    }
}

/// AbleSet's own port on the PC (pure): the upstream's, 80 when it names
/// none.
pub fn lan_port(upstream: &str) -> u16 {
    upstream
        .strip_prefix("http://")
        .and_then(|a| a.parse::<Authority>().ok())
        .and_then(|a| a.port_u16())
        .unwrap_or(80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_is_down_only_after_two_failed_reads_in_a_row_and_up_at_the_first_good_one() {
        assert_eq!((PERIOD, DOWN_AFTER), (Duration::from_secs(5), 2));
        let start = Link::default();
        assert_eq!(start.online, None, "not known yet");
        let link = |online, failures| Link { online, failures };
        let up = link(Some(true), 0);
        // The first good read brings it up; another is no news.
        let first = Some(Note::Up { failures: 0 });
        assert_eq!(link_step(start, true), (up, first));
        assert_eq!(link_step(up, true), (up, None));
        // One failed read does not take it down, and a good one starts the
        // count over.
        let once = link(Some(true), 1);
        assert_eq!(link_step(up, false), (once, None));
        assert_eq!(link_step(once, true), (up, None));
        // The second takes it down; more are counted, no news.
        let down = link(Some(false), 2);
        assert_eq!(
            link_step(once, false),
            (down, Some(Note::Down { failures: 2 }))
        );
        assert_eq!(link_step(down, false), (link(Some(false), 3), None));
        // Up again at the first good read, with the failed ones before it.
        let back = Some(Note::Up { failures: 3 });
        assert_eq!(link_step(link(Some(false), 3), true), (up, back));
        // AbleSet down when the hub starts: down at the second failed read.
        let one = link(None, 1);
        assert_eq!(link_step(start, false), (one, None));
        assert_eq!(
            link_step(one, false),
            (down, Some(Note::Down { failures: 2 }))
        );
        assert!(Note::Up { failures: 1 }.up());
        assert!(!Note::Down { failures: 2 }.up());
    }

    #[test]
    fn a_read_is_good_for_a_2xx_or_3xx_answer_only() {
        for good in [200, 204, 302, 399] {
            assert_eq!(answered(good), Ok(()), "{good}");
        }
        for bad in [101, 199, 400, 404, 503] {
            assert_eq!(answered(bad), Err(format!("AbleSet answers {bad}")));
        }
    }

    #[tokio::test]
    async fn a_read_ableset_does_not_answer_says_so_with_the_url() {
        let read = read(&client(PERIOD), "http://127.0.0.1:1/").await;
        let why = read.unwrap_err();
        assert!(
            why.starts_with("AbleSet does not answer: http://127.0.0.1:1/"),
            "{why}"
        );
    }

    #[test]
    fn a_change_is_recorded_with_its_reads_and_why_it_went_down() {
        let up = link_fields(Note::Up { failures: 3 }, None);
        assert_eq!(up, json!({"up": true, "failures": 3}));
        let down = link_fields(Note::Down { failures: 2 }, Some("AbleSet answers 503"));
        assert_eq!(
            down,
            json!({"up": false, "error": "AbleSet answers 503", "failures": 2})
        );
    }

    #[test]
    fn the_pages_are_told_ablesets_title_name_parent_link_and_port() {
        let cfg = AbleSetCfg {
            name: "ableset.example.org".into(),
            upstream: "http://[::1]:3000".into(),
            access_aud: Vec::new(),
            title: "Setlist".into(),
        };
        let tls = TlsCfg {
            name: "foh.example.org".into(),
            port: 8443,
            redirect_http: true,
        };
        let page = Page::of(&cfg, &tls);
        assert_eq!(
            page.msg(true),
            ServerMsg::Ableset {
                title: "Setlist".into(),
                name: "ableset.example.org".into(),
                parent: "foh.example.org".into(),
                online: true,
                lan_port: 3000,
            }
        );
        assert!(matches!(
            page.msg(false),
            ServerMsg::Ableset { online: false, .. }
        ));
        for (upstream, port) in [
            ("http://127.0.0.1:80", 80),
            ("http://127.0.0.1:8080", 8080),
            ("http://localhost", 80),
            ("http://[::1]", 80),
        ] {
            assert_eq!(lan_port(upstream), port, "{upstream}");
        }
    }
}
