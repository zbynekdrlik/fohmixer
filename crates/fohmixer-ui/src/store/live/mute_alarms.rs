//! The mute alarm's request (#188): a strip's MUTE ALARM switched in its
//! channel detail, sent with an id from the pending-request counter as the
//! colours' are (`set_color`), so the hub's `result` or `error` reaches its
//! `done`. The document itself only ever comes from the hub
//! (`ServerMsg::MuteAlarms`); nothing here reaches Live.

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::sectors::StripRef;

use super::{LiveStore, ResultFn};

impl LiveStore {
    /// Switches `strip`'s mute alarm to `on`. Whether the socket took it.
    pub fn set_mute_alarm(self, strip: &StripRef, on: bool, done: Option<ResultFn>) -> bool {
        self.sector_request(
            |id| ClientMsg::SetMuteAlarm {
                id,
                strip: strip.clone(),
                on,
            },
            done,
        )
    }
}
