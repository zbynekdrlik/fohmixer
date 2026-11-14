//! The mute guard's request (#203): a strip's MUTE GUARD switched in its
//! channel detail, sent with an id from the pending-request counter as the
//! mute alarm's is (`set_mute_alarm`), so the hub's `result` or `error`
//! reaches its `done`. The document itself only ever comes from the hub
//! (`ServerMsg::MuteGuards`); nothing here reaches Live.

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::sectors::StripRef;

use super::{LiveStore, ResultFn};

impl LiveStore {
    /// Switches `strip`'s mute guard to `on`. Whether the socket took it.
    pub fn set_mute_guard(self, strip: &StripRef, on: bool, done: Option<ResultFn>) -> bool {
        self.sector_request(
            |id| ClientMsg::SetMuteGuard {
                id,
                strip: strip.clone(),
                on,
            },
            done,
        )
    }
}
