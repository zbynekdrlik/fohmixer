//! The router's part of the strips' colours (#164): the document at every
//! page's attach, the pages' choices, the file's hand edits (polled with
//! the layout), the records and `/api/status`.
//!
//! The decision is `fohmixer_proto::colors::set_color` (a colour only for a
//! fader of the served layout's pool) and the store's (`crate::colors`);
//! this is the glue. A taken change is one `colors` record (warn class) and
//! one info line, the asker's `result`, then the document to every page; a
//! refusal is an `error` to the asker alone, with no record. Nothing
//! reaches Live.

use std::sync::Arc;

use fohmixer_proto::client::{ColorsStatus, ServerMsg};
use fohmixer_proto::colors::{self as model, ColorKey};
use fohmixer_proto::layout::Layout;
use fohmixer_proto::sectors::{self as sectors_model, StripRef};
use serde_json::{Value, json};

use super::Router;
use crate::colors::{ColorStore, Polled};
use crate::live::subs::ClientId;
use crate::outbox::Outbox;

/// The router's colours: the store of `colors.json`.
pub(super) struct ColorsIo {
    pub(super) store: ColorStore,
}

/// A `colors` record's fields: `what` (`set`, `file`), the page and its
/// peer (none for the hub's own), then `extra`'s.
pub(super) fn colors_fields(
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

/// The info line of a page's choice: the fader and its colour (`NONE`).
pub(super) fn set_line(strip: &StripRef, color: Option<ColorKey>) -> String {
    format!(
        "a strip's colour changed: {}|{} is {}",
        strip.instance,
        strip.label,
        model::shown_name(color)
    )
}

/// The info line of a hand edit taken.
pub(super) const TAKEN_LINE: &str = "colors.json was edited by hand: its colours are served";

/// The warn line of a write that failed.
pub(super) fn write_problem(error: &std::io::Error) -> String {
    format!("cannot write the strips' colours (colors.json): {error}")
}

/// The warn line of a hand edit refused.
pub(super) fn refused_line(why: &str) -> String {
    format!("colors.json is refused, the last good document stays served: {why}")
}

/// The warn line of a `colors.json` removed while the hub runs.
pub(super) const RESTORED_LINE: &str =
    "colors.json was removed; it is written back from the document served";

/// The warn line of a file the poll could not read.
pub(super) fn unreadable_line(why: &str) -> String {
    format!(
        "cannot read colors.json, the document stays served and is read again at the next poll: {why}"
    )
}

/// The warn line of a removed file the hub could not write back.
pub(super) fn unwritable_line(why: &str) -> String {
    format!(
        "colors.json was removed and cannot be written back, the document stays served and is written again at the next poll: {why}"
    )
}

impl Router {
    /// The served layout (the pool's), when a store serves one.
    fn colors_layout(&self) -> Option<Arc<Layout>> {
        self.layout_store
            .as_ref()
            .and_then(|store| store.current().1)
    }

    /// The document as the pages get it.
    fn colors_message(&self) -> ServerMsg {
        ServerMsg::Colors {
            doc: self.colors.store.doc().clone(),
        }
    }

    /// A page attached: the document.
    pub(super) fn colors_attach(&self, outbox: &Outbox) {
        outbox.reply(self.colors_message());
    }

    /// The document to every page.
    fn colors_tell(&self) {
        let msg = self.colors_message();
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// An answer to the asker alone.
    fn colors_answer(&self, client: ClientId, msg: ServerMsg) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(msg);
        }
    }

    /// A page's choice of `color` for `strip`.
    pub(super) fn colors_set(
        &mut self,
        client: ClientId,
        id: String,
        strip: &StripRef,
        color: Option<ColorKey>,
    ) {
        let served = self.colors_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        match model::set_color(self.colors.store.doc(), strip, color, &pool) {
            Err(refusal) => self.colors_refuse(client, id, refusal.text().to_string()),
            // Nothing changes: no write, no record, no news.
            Ok(None) => self.colors_taken(client, id),
            Ok(Some(next)) => match self.colors.store.commit(next) {
                Ok(()) => self.colors_done(client, id, strip, color),
                Err(e) => {
                    tracing::warn!("{}", write_problem(&e));
                    self.colors_refuse(client, id, e.to_string());
                }
            },
        }
    }

    /// The asker's `error` with `message`.
    fn colors_refuse(&self, client: ClientId, id: String, message: String) {
        self.colors_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message,
            },
        );
    }

    /// The asker's `result`.
    fn colors_taken(&self, client: ClientId, id: String) {
        self.colors_answer(
            client,
            ServerMsg::Result {
                id,
                data: Vec::new(),
            },
        );
    }

    /// A choice written: recorded, answered, told.
    fn colors_done(&self, client: ClientId, id: String, strip: &StripRef, color: Option<ColorKey>) {
        let peer = self.peers.get(&client).map(String::as_str);
        self.io.events.record(
            "colors",
            colors_fields(
                "set",
                Some(client),
                peer,
                json!({"strip": strip, "color": color}),
            ),
        );
        tracing::info!(client, "{}", set_line(strip, color));
        self.colors_taken(client, id);
        self.colors_tell();
    }

    /// The file read again (the layout poll's tick): a hand edit is
    /// recorded and told; a bad one is logged and the last good document
    /// stays; a removed one is written back and recorded (`file`, `why:
    /// deleted`).
    pub(super) fn colors_poll(&mut self) {
        match self.colors.store.poll() {
            Some(Polled::Taken) => {
                self.io
                    .events
                    .record("colors", colors_fields("file", None, None, json!({})));
                tracing::info!("{TAKEN_LINE}");
                self.colors_tell();
            }
            Some(Polled::Refused(why)) => tracing::warn!("{}", refused_line(&why)),
            Some(Polled::Unreadable(why)) => tracing::warn!("{}", unreadable_line(&why)),
            Some(Polled::Unwritable(why)) => tracing::warn!("{}", unwritable_line(&why)),
            Some(Polled::Restored) => {
                tracing::warn!("{RESTORED_LINE}");
                self.io.events.record(
                    "colors",
                    colors_fields("file", None, None, json!({"why": "deleted"})),
                );
            }
            None => {}
        }
    }

    /// `/api/status` `colors`.
    pub(super) fn colors_status(&self) -> ColorsStatus {
        let served = self.colors_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        let doc = self.colors.store.doc();
        ColorsStatus {
            strips: doc.count(),
            unknown: model::unknown(doc, &pool),
            error: self.colors.store.error().map(str::to_string),
        }
    }
}

#[cfg(test)]
#[path = "colors/tests.rs"]
mod tests;
