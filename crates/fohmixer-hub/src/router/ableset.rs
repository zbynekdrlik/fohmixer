//! The router's half of AbleSet's page (#90 B3): each client gets the
//! `ableset` message when it attaches, and every client a new one at each
//! change of the hub's link to AbleSet (`ableset/link.rs`), only with
//! `[ableset]` (a page without it shows no AbleSet tab).

use super::Router;
use crate::ableset::link::Page;
use crate::outbox::Outbox;

/// AbleSet's page as the router tells it: its page and the link.
pub(super) struct AbleSetIo {
    page: Page,
    online: bool,
}

impl Router {
    /// The router of a hub with `[ableset]`: the link is not up until the
    /// hub's first good read says so.
    #[must_use]
    pub fn with_ableset(mut self, page: Page) -> Self {
        self.ableset = Some(AbleSetIo {
            page,
            online: false,
        });
        self
    }

    /// A new client is told of AbleSet's page.
    pub(super) fn ableset_attach(&self, outbox: &Outbox) {
        if let Some(io) = &self.ableset {
            outbox.reply(io.page.msg(io.online));
        }
    }

    /// The link went up or down: every client is told.
    pub(super) fn ableset_link(&mut self, online: bool) {
        let Some(io) = self.ableset.as_mut() else {
            return;
        };
        io.online = online;
        let msg = io.page.msg(online);
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
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
            false,
            dir.to_path_buf(),
            RouterIo { tx, events },
        )
    }

    fn attach(router: &mut Router, client: u64) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
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
        let first = attach(&mut router, 1);
        assert_eq!(told(&first), [page().msg(false)], "not up before a read");
        assert!(router.handle(RouterMsg::AbleSetLink { online: true }));
        assert_eq!(told(&first), [page().msg(true)]);
        let second = attach(&mut router, 2);
        assert_eq!(told(&second), [page().msg(true)], "the link as it is");
        assert!(router.handle(RouterMsg::AbleSetLink { online: false }));
        assert_eq!(told(&first), [page().msg(false)]);
        assert_eq!(told(&second), [page().msg(false)]);
        // A hub without [ableset] tells no page of it.
        let mut without = bare(dir.path());
        let outbox = attach(&mut without, 3);
        assert!(without.handle(RouterMsg::AbleSetLink { online: true }));
        assert!(told(&outbox).is_empty());
    }
}
