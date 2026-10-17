//! The router's half of AbleSet's page (#90 B3): each client gets the
//! `ableset` message when it attaches, and every client a new one at each
//! change of the hub's link to AbleSet (`ableset/link.rs`), only with
//! `[ableset]` (a page without it shows no AbleSet tab). Each message names
//! whether that client's own socket came over the LAN (#90 PR C review M2:
//! a failed probe is AbleSet's sign-in only through the tunnel).

use super::Router;
use crate::ableset::link::Page;
use crate::outbox::Outbox;

/// AbleSet's page as the router tells it: its page and the link (none:
/// not known yet).
pub(super) struct AbleSetIo {
    page: Page,
    online: Option<bool>,
}

impl Router {
    /// The router of a hub with `[ableset]`: the link is not known until
    /// the hub's first read decides (#90 B3 review 4: a page tells "not
    /// known yet" apart from "down").
    #[must_use]
    pub fn with_ableset(mut self, page: Page) -> Self {
        self.ableset = Some(AbleSetIo { page, online: None });
        self
    }

    /// A new client, whose socket came over the LAN (`lan`) or not, is told
    /// of AbleSet's page.
    pub(super) fn ableset_attach(&self, outbox: &Outbox, lan: bool) {
        if let Some(io) = &self.ableset {
            outbox.reply(io.page.msg(io.online, lan));
        }
    }

    /// The link went up or down: every client is told, each its own `lan`.
    pub(super) fn ableset_link(&mut self, online: bool) {
        let Some(io) = self.ableset.as_mut() else {
            return;
        };
        io.online = Some(online);
        for (client, outbox) in &self.clients {
            let lan = self.lan.get(client).copied().unwrap_or(false);
            outbox.reply(io.page.msg(io.online, lan));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fohmixer_proto::client::ServerMsg;
    use tokio::sync::mpsc;

    use super::super::{Router, RouterIo, RouterMsg};
    use crate::ableset::link::Page;
    use crate::events::EventLog;
    use crate::outbox::Outbox;

    fn page() -> Page {
        Page {
            title: "AbleSet".into(),
            name: "ableset.example.org".into(),
            parent: "foh.example.org".into(),
            lan_port: 80,
        }
    }

    fn bare(dir: &std::path::Path) -> Router {
        let (events, _) = EventLog::channel(64);
        let (tx, _) = mpsc::unbounded_channel();
        Router::new(
            BTreeMap::new(),
            crate::rules::HubState::default(),
            dir.to_path_buf(),
            RouterIo { tx, events },
        )
    }

    /// A client whose socket came over the LAN (`lan`) or through the
    /// tunnel.
    fn attach(router: &mut Router, client: u64, lan: bool) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
            lan,
        });
        outbox
    }

    /// The `ableset` messages waiting in `outbox`.
    fn told(outbox: &Outbox) -> Vec<ServerMsg> {
        let waiting = outbox.take().unwrap_or_default();
        waiting
            .into_iter()
            .filter(|msg| matches!(msg, ServerMsg::Ableset { .. }))
            .collect()
    }

    #[test]
    fn every_page_is_told_of_ablesets_page_at_attach_and_at_each_link_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut router = bare(dir.path()).with_ableset(page());
        let first = attach(&mut router, 1, true);
        assert_eq!(
            told(&first),
            [page().msg(None, true)],
            "not known before a read"
        );
        assert!(router.handle(RouterMsg::AbleSetLink { online: true }));
        assert_eq!(told(&first), [page().msg(Some(true), true)]);
        let second = attach(&mut router, 2, true);
        assert_eq!(
            told(&second),
            [page().msg(Some(true), true)],
            "the link as it is"
        );
        assert!(router.handle(RouterMsg::AbleSetLink { online: false }));
        assert_eq!(told(&first), [page().msg(Some(false), true)]);
        assert_eq!(told(&second), [page().msg(Some(false), true)]);
        // A hub without [ableset] tells no page of it.
        let mut without = bare(dir.path());
        let outbox = attach(&mut without, 3, true);
        assert!(without.handle(RouterMsg::AbleSetLink { online: true }));
        assert!(told(&outbox).is_empty());
    }

    #[test]
    fn each_page_is_told_whether_its_own_socket_came_over_the_lan() {
        // #90 PR C review M2: a failed probe is AbleSet's sign-in only for a
        // page that came through the tunnel; the page cannot tell itself.
        let dir = tempfile::tempdir().unwrap();
        let mut router = bare(dir.path()).with_ableset(page());
        let lan = attach(&mut router, 1, true);
        let tunnel = attach(&mut router, 2, false);
        assert_eq!(told(&lan), [page().msg(None, true)]);
        assert_eq!(told(&tunnel), [page().msg(None, false)]);
        // Every link change tells each page its own again.
        assert!(router.handle(RouterMsg::AbleSetLink { online: true }));
        assert_eq!(told(&lan), [page().msg(Some(true), true)]);
        assert_eq!(told(&tunnel), [page().msg(Some(true), false)]);
        // A page gone takes its own with it; the next one with its number
        // is told its own.
        router.handle(RouterMsg::Detach { client: 2 });
        assert!(!router.lan.contains_key(&2));
        let again = attach(&mut router, 2, true);
        assert_eq!(told(&again), [page().msg(Some(true), true)]);
        assert!(router.handle(RouterMsg::AbleSetLink { online: false }));
        assert_eq!(told(&again), [page().msg(Some(false), true)]);
        assert_eq!(told(&lan), [page().msg(Some(false), true)]);
    }
}
