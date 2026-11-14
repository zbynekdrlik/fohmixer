//! The router's part of the channel detail's per-strip switches kept as
//! marks: the mute alarm (#188, `mute_alarms.json`) and the mute guard
//! (#203, `mute_guards.json`). One glue for both: the marks at every page's
//! attach, the pages' switches, the file's hand edits (polled with the
//! layout), the records and `/api/status`.
//!
//! The decision is `fohmixer_proto::strip_marks::switch` (a mark only for a
//! fader of the served layout's pool) and the store's
//! (`crate::doc_store`); this is the glue, as `router/colors.rs` is for the
//! colours. A taken change is one record (warn class, the document's
//! [`Marked::RECORD`]) and one info line, the asker's `result`, then the
//! document to every page; a refusal is an `error` to the asker alone, with
//! no record. Nothing reaches Live.

use std::sync::Arc;

use fohmixer_proto::client::{MarksStatus, ServerMsg};
use fohmixer_proto::layout::Layout;
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
use fohmixer_proto::mute_guards::MuteGuardsDoc;
use fohmixer_proto::roles;
use fohmixer_proto::sectors::{self as sectors_model, StripRef};
use fohmixer_proto::strip_marks::{self, StripMarks};
use serde_json::{Value, json};

use super::Router;
use crate::doc_store::{DocStore, Document, Polled};
use crate::live::subs::ClientId;
use crate::outbox::Outbox;

/// The router's marks of one document: its store.
pub(super) struct MarksIo<D: Document> {
    pub(super) store: DocStore<D>,
}

impl<D: Document> MarksIo<D> {
    /// The store of `D`'s file in `data_dir`.
    pub(super) fn open(data_dir: &std::path::Path) -> Self {
        Self {
            store: DocStore::open(data_dir),
        }
    }
}

/// A document of marks the router keeps: its records' name, the switch's
/// name in the log, its message to the pages and its place in the router.
pub(super) trait Marked: StripMarks + Document + Clone {
    /// The records' `ev`.
    const RECORD: &'static str;
    /// The switch's name in the log lines.
    const NAME: &'static str;
    /// The document as the pages get it.
    fn message(doc: Self) -> ServerMsg;
    /// The router's marks of this document.
    fn io(router: &Router) -> &MarksIo<Self>;
    /// The same, to change.
    fn io_mut(router: &mut Router) -> &mut MarksIo<Self>;
}

impl Marked for MuteAlarmsDoc {
    const RECORD: &'static str = "mute_alarms";
    const NAME: &'static str = "mute alarm";

    fn message(doc: Self) -> ServerMsg {
        ServerMsg::MuteAlarms { doc }
    }

    fn io(router: &Router) -> &MarksIo<Self> {
        &router.mute_alarms
    }

    fn io_mut(router: &mut Router) -> &mut MarksIo<Self> {
        &mut router.mute_alarms
    }
}

impl Marked for MuteGuardsDoc {
    const RECORD: &'static str = "mute_guards";
    const NAME: &'static str = "mute guard";

    fn message(doc: Self) -> ServerMsg {
        ServerMsg::MuteGuards { doc }
    }

    fn io(router: &Router) -> &MarksIo<Self> {
        &router.mute_guards
    }

    fn io_mut(router: &mut Router) -> &mut MarksIo<Self> {
        &mut router.mute_guards
    }
}

/// A record's fields: `what` (`set`, `file`), the page and its peer (none
/// for the hub's own), then `extra`'s.
pub(super) fn mark_fields(
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

/// The info line of a page's switch: the fader and its mark.
pub(super) fn set_line<D: Marked>(strip: &StripRef, on: bool) -> String {
    format!(
        "a strip's {} changed: {}|{} is {}",
        D::NAME,
        strip.instance,
        strip.label,
        if on { "on" } else { "off" }
    )
}

/// The info line of a hand edit taken.
pub(super) fn taken_line<D: Marked>() -> String {
    format!("{} was edited by hand: its marks are served", D::FILE)
}

/// The warn line of a write that failed.
pub(super) fn write_problem<D: Marked>(error: &std::io::Error) -> String {
    format!(
        "cannot write the {}'s marks ({}): {error}",
        D::NAME,
        D::FILE
    )
}

/// The warn line of a hand edit refused.
pub(super) fn refused_line<D: Marked>(why: &str) -> String {
    format!(
        "{} is refused, the last good document stays served: {why}",
        D::FILE
    )
}

/// The warn line of a file removed while the hub runs.
pub(super) fn restored_line<D: Marked>() -> String {
    format!(
        "{} was removed; it is written back from the document served",
        D::FILE
    )
}

/// The warn line of a file the poll could not read.
pub(super) fn unreadable_line<D: Marked>(why: &str) -> String {
    format!(
        "cannot read {}, the document stays served and is read again at the next poll: {why}",
        D::FILE
    )
}

/// The warn line of a removed file the hub could not write back.
pub(super) fn unwritable_line<D: Marked>(why: &str) -> String {
    format!(
        "{} was removed and cannot be written back, the document stays served and is written again at the next poll: {why}",
        D::FILE
    )
}

impl Router {
    /// The served layout (the pool's), when a store serves one.
    fn marks_layout(&self) -> Option<Arc<Layout>> {
        self.layout_store
            .as_ref()
            .and_then(|store| store.current().1)
    }

    /// `D`'s document as the pages get it.
    fn marks_message<D: Marked>(&self) -> ServerMsg {
        D::message(D::io(self).store.doc().clone())
    }

    /// A page attached: `D`'s document.
    pub(super) fn marks_attach<D: Marked>(&self, outbox: &Outbox) {
        outbox.reply(self.marks_message::<D>());
    }

    /// `D`'s document to every page.
    fn marks_tell<D: Marked>(&self) {
        let msg = self.marks_message::<D>();
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// An answer to the asker alone.
    fn marks_answer(&self, client: ClientId, msg: ServerMsg) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(msg);
        }
    }

    /// A page's switch of `strip`'s mark in `D` to `on`.
    pub(super) fn marks_set<D: Marked>(
        &mut self,
        client: ClientId,
        id: String,
        strip: &StripRef,
        on: bool,
    ) {
        if !roles::reaches_strip(self.role_of(client), strip) {
            tracing::info!(client, "a strip's {} is refused: role", D::NAME);
            self.marks_refuse(client, id, crate::ws::role::REFUSED.to_string());
            return;
        }
        let served = self.marks_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        match strip_marks::switch(D::io(self).store.doc(), strip, on, &pool) {
            Err(refusal) => {
                tracing::info!(
                    client,
                    strip = ?format!("{}|{}", strip.instance, strip.label),
                    on,
                    "a strip's {} is refused: {}",
                    D::NAME,
                    refusal.text()
                );
                self.marks_refuse(client, id, refusal.text().to_string());
            }
            // Nothing changes: no write, no record, no news.
            Ok(None) => self.marks_taken(client, id),
            Ok(Some(next)) => match D::io_mut(self).store.commit(next) {
                Ok(()) => self.marks_done::<D>(client, id, strip, on),
                Err(e) => {
                    tracing::warn!("{}", write_problem::<D>(&e));
                    self.marks_refuse(client, id, e.to_string());
                }
            },
        }
    }

    /// The asker's `error` with `message`.
    fn marks_refuse(&self, client: ClientId, id: String, message: String) {
        self.marks_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message,
            },
        );
    }

    /// The asker's `result`.
    fn marks_taken(&self, client: ClientId, id: String) {
        self.marks_answer(
            client,
            ServerMsg::Result {
                id,
                data: Vec::new(),
            },
        );
    }

    /// A switch written: recorded, answered, told.
    fn marks_done<D: Marked>(&self, client: ClientId, id: String, strip: &StripRef, on: bool) {
        let peer = self.peers.get(&client).map(String::as_str);
        self.io.events.record(
            D::RECORD,
            mark_fields("set", Some(client), peer, json!({"strip": strip, "on": on})),
        );
        tracing::info!(client, "{}", set_line::<D>(strip, on));
        self.marks_taken(client, id);
        self.marks_tell::<D>();
    }

    /// `D`'s file read again (the layout poll's tick): a hand edit is
    /// recorded and told; a bad one is logged and the last good document
    /// stays; a removed one is written back and recorded (`file`, `why:
    /// deleted`).
    pub(super) fn marks_poll<D: Marked>(&mut self) {
        match D::io_mut(self).store.poll() {
            Some(Polled::Taken) => {
                self.io
                    .events
                    .record(D::RECORD, mark_fields("file", None, None, json!({})));
                tracing::info!("{}", taken_line::<D>());
                self.marks_tell::<D>();
            }
            Some(Polled::Refused(why)) => tracing::warn!("{}", refused_line::<D>(&why)),
            Some(Polled::Unreadable(why)) => tracing::warn!("{}", unreadable_line::<D>(&why)),
            Some(Polled::Unwritable(why)) => tracing::warn!("{}", unwritable_line::<D>(&why)),
            Some(Polled::Restored) => {
                tracing::warn!("{}", restored_line::<D>());
                self.io.events.record(
                    D::RECORD,
                    mark_fields("file", None, None, json!({"why": "deleted"})),
                );
            }
            None => {}
        }
    }

    /// `/api/status`'s part of `D` (`mute_alarms`, `mute_guards`).
    pub(super) fn marks_status<D: Marked>(&self) -> MarksStatus {
        let served = self.marks_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        let store = &D::io(self).store;
        MarksStatus {
            strips: store.doc().count(),
            unknown: strip_marks::unknown(store.doc(), &pool),
            error: store.error().map(str::to_string),
        }
    }
}

#[cfg(test)]
#[path = "marks/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "marks/guard_tests.rs"]
mod guard_tests;
