//! The router's part of the fader sectors (#148): the document at every
//! page's attach, the pages' ticks, ARRANGE's saves and the undo, the
//! file's hand edits (polled with the layout), the first document (the
//! seed), the records and `/api/status`.
//!
//! The decisions are `fohmixer_proto::sectors`' (`tick`, `orders`, `seed`)
//! and the store's (`crate::sectors`); this is the glue. A taken change is
//! one `sectors` record (warn class) and one info line, the asker's
//! `result`, then the document to every page; a refusal is an `error` to
//! the asker alone, with no record.

use std::sync::Arc;

use fohmixer_proto::client::{SectorsStatus, ServerMsg};
use fohmixer_proto::layout::Layout;
use fohmixer_proto::sectors::{self as model, Refusal, SectorOrder, SectorsDoc, StripRef};
use serde_json::{Value, json};

use super::Router;
use crate::live::subs::ClientId;
use crate::outbox::Outbox;
use crate::sectors::{Polled, SectorStore};

/// The router's sectors: the store of `sectors.json`.
pub(super) struct SectorsIo {
    pub(super) store: SectorStore,
}

/// Whether the first document is written now: no `sectors.json`, a served
/// layout with faders, and every instance's markers read whole since the
/// hub started with no composition waiting (`every_read`).
pub(super) fn seed_due(missing: bool, pool_len: usize, every_read: bool) -> bool {
    missing && pool_len > 0 && every_read
}

/// A `sectors` record's fields: `what` (`tick`, `order`, `undo`, `seed`,
/// `file`), the page and its peer (none for the hub's own), then `extra`'s.
pub(super) fn sectors_fields(
    what: &str,
    client: Option<ClientId>,
    peer: Option<&str>,
    extra: Value,
) -> Value {
    let mut fields = json!({"what": what, "client": client, "peer": peer});
    if let (Some(map), Value::Object(more)) = (fields.as_object_mut(), extra) {
        map.extend(more);
    }
    fields
}

/// The info line of a change.
pub(super) fn sectors_line(what: &str, rev: u64) -> String {
    format!("the fader sectors changed ({what}): revision {rev}")
}

/// The warn line of a write that failed.
pub(super) fn write_problem(error: &std::io::Error) -> String {
    format!("cannot write the fader sectors (sectors.json): {error}")
}

/// The warn line of a hand edit refused.
pub(super) fn refused_line(why: &str) -> String {
    format!("sectors.json is refused, the last good document stays served: {why}")
}

impl Router {
    /// The served layout (the pool's), when a store serves one.
    fn sectors_layout(&self) -> Option<Arc<Layout>> {
        self.layout_store
            .as_ref()
            .and_then(|store| store.current().1)
    }

    /// The document as the pages get it.
    fn sectors_message(&self) -> ServerMsg {
        ServerMsg::Sectors {
            doc: self.sectors.store.doc().clone(),
            undo: self.sectors.store.has_prev(),
        }
    }

    /// A page attached: the document.
    pub(super) fn sectors_attach(&self, outbox: &Outbox) {
        outbox.reply(self.sectors_message());
    }

    /// The document to every page.
    fn sectors_tell(&self) {
        let msg = self.sectors_message();
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// An answer to the asker alone.
    fn sectors_answer(&self, client: ClientId, msg: ServerMsg) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(msg);
        }
    }

    fn sectors_refuse(&self, client: ClientId, id: String, refusal: Refusal) {
        self.sectors_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message: refusal.text().to_string(),
            },
        );
    }

    /// One change's record (with the revision now served) and its line.
    fn sectors_record(&self, what: &str, client: Option<ClientId>, mut extra: Value) {
        let rev = self.sectors.store.doc().rev;
        if let Value::Object(map) = &mut extra {
            map.insert("rev".into(), json!(rev));
        }
        let peer = client.and_then(|c| self.peers.get(&c)).map(String::as_str);
        self.io
            .events
            .record("sectors", sectors_fields(what, client, peer, extra));
        tracing::info!(client = ?client, "{}", sectors_line(what, rev));
    }

    /// A page's change taken: written, recorded, answered, told.
    fn sectors_commit(
        &mut self,
        client: ClientId,
        id: String,
        next: SectorsDoc,
        what: &str,
        extra: Value,
    ) {
        match self.sectors.store.commit(next) {
            Ok(()) => self.sectors_done(client, id, what, extra),
            Err(e) => self.sectors_write_failed(client, id, &e),
        }
    }

    fn sectors_done(&self, client: ClientId, id: String, what: &str, extra: Value) {
        self.sectors_record(what, Some(client), extra);
        self.sectors_answer(
            client,
            ServerMsg::Result {
                id,
                data: Vec::new(),
            },
        );
        self.sectors_tell();
    }

    fn sectors_write_failed(&self, client: ClientId, id: String, error: &std::io::Error) {
        tracing::warn!("{}", write_problem(error));
        self.sectors_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message: error.to_string(),
            },
        );
    }

    /// A page's tick of `strip` in `layout`.
    pub(super) fn sectors_tick(
        &mut self,
        client: ClientId,
        id: String,
        layout: &str,
        strip: &StripRef,
        on: bool,
    ) {
        let served = self.sectors_layout();
        let pool = served.as_deref().map(model::pool).unwrap_or_default();
        match model::tick(self.sectors.store.doc(), layout, strip, on, &pool) {
            Err(refusal) => self.sectors_refuse(client, id, refusal),
            // Nothing changes: no write, no record, no news (call 13).
            Ok(None) => self.sectors_answer(
                client,
                ServerMsg::Result {
                    id,
                    data: Vec::new(),
                },
            ),
            Ok(Some(next)) => self.sectors_commit(
                client,
                id,
                next,
                "tick",
                json!({"layout": layout, "strip": strip, "on": on}),
            ),
        }
    }

    /// A page's ARRANGE save.
    pub(super) fn sectors_orders(
        &mut self,
        client: ClientId,
        id: String,
        orders: &[SectorOrder],
        rev: u64,
    ) {
        match model::orders(self.sectors.store.doc(), orders, rev) {
            Err(refusal) => self.sectors_refuse(client, id, refusal),
            Ok(next) => {
                let layouts: Vec<&str> = orders.iter().map(|o| o.layout.as_str()).collect();
                self.sectors_commit(client, id, next, "order", json!({"layouts": layouts}));
            }
        }
    }

    /// A page's undo (a second one redoes).
    pub(super) fn sectors_undo(&mut self, client: ClientId, id: String) {
        match self.sectors.store.undo() {
            Err(refusal) => self.sectors_refuse(client, id, refusal),
            Ok(Ok(())) => self.sectors_done(client, id, "undo", json!({})),
            Ok(Err(e)) => self.sectors_write_failed(client, id, &e),
        }
    }

    /// The file read again (the layout poll's tick): a hand edit is
    /// recorded and told; a bad one is logged and the last good document
    /// stays. Then the seed, when it is due (a file removed).
    pub(super) fn sectors_poll(&mut self) {
        match self.sectors.store.poll() {
            Some(Polled::Taken) => {
                self.sectors_record("file", None, json!({}));
                self.sectors_tell();
            }
            Some(Polled::Refused(why)) => tracing::warn!("{}", refused_line(&why)),
            None => {}
        }
        self.sectors_seed_if_due();
    }

    /// Writes the first document from the served layout when it is due
    /// ([`seed_due`]; after every layout, markers read and settle).
    pub(super) fn sectors_seed_if_due(&mut self) {
        let Some(served) = self.sectors_layout() else {
            return;
        };
        let every_read = self.markers_pending.is_none()
            && self.live.keys().all(|name| self.markers.has_read(name));
        let pool_len = model::pool(&served).len();
        if !seed_due(self.sectors.store.missing(), pool_len, every_read) {
            return;
        }
        match self.sectors.store.seed(model::seed(&served)) {
            Ok(()) => {
                self.sectors_record("seed", None, json!({}));
                self.sectors_tell();
            }
            Err(e) => tracing::warn!("{}", write_problem(&e)),
        }
    }

    /// `/api/status` `sectors`.
    pub(super) fn sectors_status(&self) -> SectorsStatus {
        let served = self.sectors_layout();
        let pool = served.as_deref().map(model::pool).unwrap_or_default();
        let doc = self.sectors.store.doc();
        SectorsStatus {
            rev: doc.rev,
            layouts: doc.layout_count(),
            strips: doc.strip_count(),
            unknown: model::unknown(doc, &pool),
            undo: self.sectors.store.has_prev(),
            error: self.sectors.store.error().map(str::to_string),
        }
    }
}

#[cfg(test)]
#[path = "sectors/tests.rs"]
mod tests;
