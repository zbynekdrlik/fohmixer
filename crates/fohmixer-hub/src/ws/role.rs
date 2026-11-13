//! One socket's role (#184, spec §6.4): what an MBC login may send and what
//! it is sent. Both decisions are pure and match every message type with
//! no `_` arm, so a new message type is a compile error until it is
//! decided here. The engineer is never refused or filtered.
//!
//! The hub calls [`inbound`] on every message a socket sends, before it
//! acts on it, and its writer maps every message it is about to write
//! through [`outbound`]. The router decides the documents' edits itself
//! (`roles::may_edit_sectors`, `roles::reaches_strip`): those messages pass
//! here.

use fohmixer_proto::client::{AckItem, ClientMsg, ServerMsg, ValueItem, set_key};
use fohmixer_proto::colors::ColorsDoc;
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
use fohmixer_proto::roles::{self, MBC_INSTANCE, Role};

/// The word a refused message is answered with.
pub const REFUSED: &str = "role";

/// The most `role refused` lines per socket per `ration::GAP`.
pub const REFUSED_LINES: u32 = 8;

/// The Live set a key (a hub key or a write key) belongs to: its first
/// field.
fn key_instance(key: &str) -> &str {
    key.split('|').next().unwrap_or("")
}

/// Whether `role` may send `msg`: Ok, or the word it is refused with.
pub fn inbound(role: Role, msg: &ClientMsg) -> Result<(), &'static str> {
    let ok = match role {
        Role::Engineer => true,
        Role::Mbc => match msg {
            ClientMsg::Cmd { instance, .. }
            | ClientMsg::Sub { instance, .. }
            | ClientMsg::Set { instance, .. } => role.reaches(instance),
            ClientMsg::Sets { sets } => sets.iter().all(|s| role.reaches(&s.instance)),
            ClientMsg::Unsub { .. }
            | ClientMsg::Ping { .. }
            | ClientMsg::Trace { .. }
            | ClientMsg::Idle { .. }
            | ClientMsg::EqFrameAck { .. } => true,
            // The router checks the document (`may_edit_sectors`,
            // `reaches_strip`).
            ClientMsg::SectorTick { .. }
            | ClientMsg::SectorEdit { .. }
            | ClientMsg::SetColor { .. }
            | ClientMsg::SetMuteAlarm { .. } => true,
            ClientMsg::SetHub { .. }
            | ClientMsg::DeckView { .. }
            | ClientMsg::DeckPress { .. }
            | ClientMsg::PresenterSwitch { .. }
            | ClientMsg::EqList { .. }
            | ClientMsg::EqOpen { .. }
            | ClientMsg::EqArea { .. }
            | ClientMsg::EqInput { .. }
            | ClientMsg::EqView { .. }
            | ClientMsg::EqClose
            | ClientMsg::EqUndo { .. } => false,
        },
    };
    if ok { Ok(()) } else { Err(REFUSED) }
}

/// The id a refused message is answered with (a command's or a document
/// edit's; none for the rest).
pub fn refused_id(msg: &ClientMsg) -> Option<String> {
    match msg {
        ClientMsg::Cmd { id, .. }
        | ClientMsg::SectorTick { id, .. }
        | ClientMsg::SectorEdit { id, .. }
        | ClientMsg::SetColor { id, .. }
        | ClientMsg::SetMuteAlarm { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// The answer to a refused message: a refused write's failed ack per key
/// (a page settles its open write by its ack, never by an `error`), else
/// the `error` with the message's id ([`refused_id`]).
pub fn refusal(msg: &ClientMsg, word: &str) -> ServerMsg {
    let failed = |instance: &str, target: &str, prop: &str, seq: u64| AckItem {
        key: set_key(instance, target, prop),
        seq,
        value: None,
        error: Some(word.to_string()),
        superseded: false,
    };
    match msg {
        ClientMsg::Set {
            instance,
            target,
            prop,
            seq,
            ..
        } => ServerMsg::Ack {
            items: vec![failed(instance, target, prop, *seq)],
        },
        ClientMsg::Sets { sets } => ServerMsg::Ack {
            items: sets
                .iter()
                .map(|s| failed(&s.instance, &s.target, &s.prop, s.seq))
                .collect(),
        },
        _ => ServerMsg::Error {
            id: refused_id(msg),
            message: word.to_string(),
        },
    }
}

/// The colours of the Live set `mbc` only.
fn mbc_colors(doc: ColorsDoc) -> ColorsDoc {
    ColorsDoc {
        colors: doc
            .colors
            .into_iter()
            .filter(|(instance, _)| instance == MBC_INSTANCE)
            .collect(),
    }
}

/// The mute alarm's marks of the Live set `mbc` only.
fn mbc_alarms(doc: MuteAlarmsDoc) -> MuteAlarmsDoc {
    MuteAlarmsDoc {
        mute_alarms: doc
            .mute_alarms
            .into_iter()
            .filter(|(instance, _)| instance == MBC_INSTANCE)
            .collect(),
    }
}

/// The values of the Live set `mbc` only.
fn mbc_values(items: Vec<ValueItem>) -> Vec<ValueItem> {
    items
        .into_iter()
        .filter(|i| key_instance(&i.sub) == MBC_INSTANCE)
        .collect()
}

/// What of `msg` `role` is sent: the engineer everything; an MBC login only
/// what belongs to the Live set `mbc` and the socket's own answers (a
/// `result`, `subbed`, `ack` or `error` answers what this socket sent, and
/// its band or master requests were refused before they went anywhere), a
/// message left with nothing dropped.
pub fn outbound(role: Role, msg: ServerMsg) -> Option<ServerMsg> {
    let Role::Mbc = role else {
        return Some(msg);
    };
    let mbc = |instance: &str| instance == MBC_INSTANCE;
    match msg {
        ServerMsg::Hello { .. }
        | ServerMsg::Result { .. }
        | ServerMsg::Subbed { .. }
        | ServerMsg::Ack { .. }
        | ServerMsg::Layout { .. }
        | ServerMsg::Pong { .. }
        | ServerMsg::Error { .. }
        | ServerMsg::Unknown => Some(msg),
        ServerMsg::Values { items } => {
            let items = mbc_values(items);
            (!items.is_empty()).then_some(ServerMsg::Values { items })
        }
        ServerMsg::Lufs { items } => {
            let items: Vec<_> = items.into_iter().filter(|i| mbc(&i.instance)).collect();
            (!items.is_empty()).then_some(ServerMsg::Lufs { items })
        }
        ServerMsg::Instance { ref name, .. } => mbc(name).then_some(msg),
        ServerMsg::Link { ref instance, .. } => mbc(instance).then_some(msg),
        ServerMsg::Sectors { doc } => Some(ServerMsg::Sectors {
            doc: roles::sectors_for(role, &doc),
        }),
        ServerMsg::Colors { doc } => Some(ServerMsg::Colors {
            doc: mbc_colors(doc),
        }),
        ServerMsg::MuteAlarms { doc } => Some(ServerMsg::MuteAlarms {
            doc: mbc_alarms(doc),
        }),
        ServerMsg::Hub { .. }
        | ServerMsg::Deck { .. }
        | ServerMsg::DeckKeys { .. }
        | ServerMsg::DeckAck { .. }
        | ServerMsg::EqList { .. }
        | ServerMsg::EqLocks { .. }
        | ServerMsg::Eq { .. }
        | ServerMsg::EqUndo { .. }
        | ServerMsg::Ableset { .. }
        | ServerMsg::Iem { .. }
        | ServerMsg::Presenter { .. }
        | ServerMsg::PresenterAck { .. }
        | ServerMsg::Reverb(_)
        | ServerMsg::Autotune(_)
        | ServerMsg::Rec(_) => None,
    }
}

#[cfg(test)]
mod tests;
