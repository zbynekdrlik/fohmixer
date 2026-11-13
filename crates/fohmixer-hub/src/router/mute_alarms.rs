//! The router's part of the mute alarm (#188): the marks at every page's
//! attach, the pages' switches, the file's hand edits (polled with the
//! layout), the records and `/api/status`.
//!
//! The decision is `fohmixer_proto::mute_alarms::set_mute_alarm` (a mark
//! only for a fader of the served layout's pool) and the store's
//! (`crate::mute_alarms`); this is the glue, as `router/colors.rs` is for
//! the colours. A taken change is one `mute_alarms` record (warn class) and
//! one info line, the asker's `result`, then the document to every page; a
//! refusal is an `error` to the asker alone, with no record. Nothing
//! reaches Live.

use std::sync::Arc;

use fohmixer_proto::client::{MuteAlarmsStatus, ServerMsg};
use fohmixer_proto::layout::Layout;
use fohmixer_proto::mute_alarms as model;
use fohmixer_proto::sectors::{self as sectors_model, StripRef};
use serde_json::{Value, json};

use super::Router;
use crate::live::subs::ClientId;
use crate::mute_alarms::{MuteAlarmStore, Polled};
use crate::outbox::Outbox;

/// The router's marks: the store of `mute_alarms.json`.
pub(super) struct MuteAlarmsIo {
    pub(super) store: MuteAlarmStore,
}

/// A `mute_alarms` record's fields: `what` (`set`, `file`), the page and
/// its peer (none for the hub's own), then `extra`'s.
pub(super) fn alarm_fields(
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

/// The info line of a page's switch: the fader and its alarm.
pub(super) fn set_line(strip: &StripRef, on: bool) -> String {
    format!(
        "a strip's mute alarm changed: {}|{} is {}",
        strip.instance,
        strip.label,
        if on { "on" } else { "off" }
    )
}

/// The info line of a hand edit taken.
pub(super) const TAKEN_LINE: &str = "mute_alarms.json was edited by hand: its marks are served";

/// The warn line of a write that failed.
pub(super) fn write_problem(error: &std::io::Error) -> String {
    format!("cannot write the mute alarm's marks (mute_alarms.json): {error}")
}

/// The warn line of a hand edit refused.
pub(super) fn refused_line(why: &str) -> String {
    format!("mute_alarms.json is refused, the last good document stays served: {why}")
}

/// The warn line of a `mute_alarms.json` removed while the hub runs.
pub(super) const RESTORED_LINE: &str =
    "mute_alarms.json was removed; it is written back from the document served";

/// The warn line of a file the poll could not read.
pub(super) fn unreadable_line(why: &str) -> String {
    format!(
        "cannot read mute_alarms.json, the document stays served and is read again at the next poll: {why}"
    )
}

/// The warn line of a removed file the hub could not write back.
pub(super) fn unwritable_line(why: &str) -> String {
    format!(
        "mute_alarms.json was removed and cannot be written back, the document stays served and is written again at the next poll: {why}"
    )
}

impl Router {
    /// The served layout (the pool's), when a store serves one.
    fn alarms_layout(&self) -> Option<Arc<Layout>> {
        self.layout_store
            .as_ref()
            .and_then(|store| store.current().1)
    }

    /// The document as the pages get it.
    fn alarms_message(&self) -> ServerMsg {
        ServerMsg::MuteAlarms {
            doc: self.mute_alarms.store.doc().clone(),
        }
    }

    /// A page attached: the document.
    pub(super) fn alarms_attach(&self, outbox: &Outbox) {
        outbox.reply(self.alarms_message());
    }

    /// The document to every page.
    fn alarms_tell(&self) {
        let msg = self.alarms_message();
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// An answer to the asker alone.
    fn alarms_answer(&self, client: ClientId, msg: ServerMsg) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(msg);
        }
    }

    /// A page's switch of `strip`'s alarm to `on`.
    pub(super) fn alarms_set(&mut self, client: ClientId, id: String, strip: &StripRef, on: bool) {
        let served = self.alarms_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        match model::set_mute_alarm(self.mute_alarms.store.doc(), strip, on, &pool) {
            Err(refusal) => {
                tracing::info!(
                    client,
                    strip = ?format!("{}|{}", strip.instance, strip.label),
                    on,
                    "a strip's mute alarm is refused: {}",
                    refusal.text()
                );
                self.alarms_refuse(client, id, refusal.text().to_string());
            }
            // Nothing changes: no write, no record, no news.
            Ok(None) => self.alarms_taken(client, id),
            Ok(Some(next)) => match self.mute_alarms.store.commit(next) {
                Ok(()) => self.alarms_done(client, id, strip, on),
                Err(e) => {
                    tracing::warn!("{}", write_problem(&e));
                    self.alarms_refuse(client, id, e.to_string());
                }
            },
        }
    }

    /// The asker's `error` with `message`.
    fn alarms_refuse(&self, client: ClientId, id: String, message: String) {
        self.alarms_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message,
            },
        );
    }

    /// The asker's `result`.
    fn alarms_taken(&self, client: ClientId, id: String) {
        self.alarms_answer(
            client,
            ServerMsg::Result {
                id,
                data: Vec::new(),
            },
        );
    }

    /// A switch written: recorded, answered, told.
    fn alarms_done(&self, client: ClientId, id: String, strip: &StripRef, on: bool) {
        let peer = self.peers.get(&client).map(String::as_str);
        self.io.events.record(
            "mute_alarms",
            alarm_fields("set", Some(client), peer, json!({"strip": strip, "on": on})),
        );
        tracing::info!(client, "{}", set_line(strip, on));
        self.alarms_taken(client, id);
        self.alarms_tell();
    }

    /// The file read again (the layout poll's tick): a hand edit is
    /// recorded and told; a bad one is logged and the last good document
    /// stays; a removed one is written back and recorded (`file`, `why:
    /// deleted`).
    pub(super) fn alarms_poll(&mut self) {
        match self.mute_alarms.store.poll() {
            Some(Polled::Taken) => {
                self.io
                    .events
                    .record("mute_alarms", alarm_fields("file", None, None, json!({})));
                tracing::info!("{TAKEN_LINE}");
                self.alarms_tell();
            }
            Some(Polled::Refused(why)) => tracing::warn!("{}", refused_line(&why)),
            Some(Polled::Unreadable(why)) => tracing::warn!("{}", unreadable_line(&why)),
            Some(Polled::Unwritable(why)) => tracing::warn!("{}", unwritable_line(&why)),
            Some(Polled::Restored) => {
                tracing::warn!("{RESTORED_LINE}");
                self.io.events.record(
                    "mute_alarms",
                    alarm_fields("file", None, None, json!({"why": "deleted"})),
                );
            }
            None => {}
        }
    }

    /// `/api/status` `mute_alarms`.
    pub(super) fn alarms_status(&self) -> MuteAlarmsStatus {
        let served = self.alarms_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        let doc = self.mute_alarms.store.doc();
        MuteAlarmsStatus {
            strips: doc.count(),
            unknown: model::unknown(doc, &pool),
            error: self.mute_alarms.store.error().map(str::to_string),
        }
    }
}

#[cfg(test)]
#[path = "mute_alarms/tests.rs"]
mod tests;
