//! The strips' colours' request (#164): a strip's colour chosen in its
//! channel detail's COLOR, sent with an id from the pending-request counter
//! as the sectors' requests are (`sector_request`), so the hub's `result`
//! or `error` reaches its `done`. The document itself only ever comes from
//! the hub (`ServerMsg::Colors`); nothing here reaches Live.

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::colors::ColorKey;
use fohmixer_proto::sectors::StripRef;

use super::{LiveStore, ResultFn};

impl LiveStore {
    /// Sets `strip`'s colour (none: NONE). Whether the socket took it.
    pub fn set_color(
        self,
        strip: &StripRef,
        color: Option<ColorKey>,
        done: Option<ResultFn>,
    ) -> bool {
        self.sector_request(
            |id| ClientMsg::SetColor {
                id,
                strip: strip.clone(),
                color,
            },
            done,
        )
    }
}
