//! The router's half of iemmixer's page (#166): each client is told of it
//! when it attaches, only with `[iem]` (a page without it shows no IEM tab).
//! The hub has no link to iemmixer: the page's frame talks to iemmixer
//! itself, so nothing here changes after the attach.

use fohmixer_proto::client::ServerMsg;

use super::Router;
use crate::config::IemCfg;
use crate::outbox::Outbox;

impl Router {
    /// The router of a hub with `[iem]`.
    #[must_use]
    pub fn with_iem(mut self, cfg: &IemCfg) -> Self {
        self.iem = Some(ServerMsg::Iem {
            url: cfg.url.clone(),
            title: cfg.title.clone(),
        });
        self
    }

    /// A new client is told of iemmixer's page.
    pub(super) fn iem_attach(&self, outbox: &Outbox) {
        if let Some(msg) = &self.iem {
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
    use crate::config::IemCfg;
    use crate::events::EventLog;
    use crate::outbox::Outbox;

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

    fn attach(router: &mut Router, client: u64) -> Arc<Outbox> {
        let outbox = Arc::new(Outbox::new());
        router.handle(RouterMsg::Attach {
            client,
            outbox: Arc::clone(&outbox),
            peer: format!("10.0.0.{client}"),
            lan: true,
        });
        outbox
    }

    /// The `iem` messages waiting in `outbox`.
    fn told(outbox: &Outbox) -> Vec<ServerMsg> {
        let waiting = outbox.take().unwrap_or_default();
        waiting
            .into_iter()
            .filter(|msg| matches!(msg, ServerMsg::Iem { .. }))
            .collect()
    }

    #[test]
    fn every_page_is_told_of_iemmixers_page_at_attach_and_only_with_iem() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = IemCfg {
            url: "https://iem.example.org/".into(),
            title: "Monitors".into(),
        };
        let mut router = bare(dir.path()).with_iem(&cfg);
        let wanted = ServerMsg::Iem {
            url: "https://iem.example.org/".into(),
            title: "Monitors".into(),
        };
        for client in [1, 2] {
            let outbox = attach(&mut router, client);
            assert_eq!(told(&outbox), [wanted.clone()], "page {client}");
        }
        // A hub without [iem] tells no page of it.
        let mut without = bare(dir.path());
        let outbox = attach(&mut without, 3);
        assert!(told(&outbox).is_empty());
    }
}
