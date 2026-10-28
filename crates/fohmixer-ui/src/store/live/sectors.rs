//! The fader sectors' requests (#148): a tick of a strip in a layout, an
//! ARRANGE save and the undo. Each carries an id from the pending-request
//! counter `cmd` uses, so the hub's `result` or `error` reaches its `done`
//! through `finish` (a socket close answers it "the hub connection closed",
//! as a command). A request sent while the hub is not connected is not
//! queued: `done` hears "not connected to the hub" at once. The document
//! itself only ever comes from the hub (`ServerMsg::Sectors`).

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::sectors::{SectorOrder, StripRef};
use leptos::prelude::*;

use super::{LiveStore, ResultFn};

impl LiveStore {
    /// Sends the message `make` builds with a fresh request id; `done` gets
    /// the hub's answer. Whether the socket took it.
    fn sector_request(
        self,
        make: impl FnOnce(String) -> ClientMsg,
        done: Option<ResultFn>,
    ) -> bool {
        let Some(id) = self.inner.try_update_value(|i| i.conn.next_id()) else {
            return false;
        };
        let sent = self.send(&make(id.clone()));
        match (sent, done) {
            (true, Some(done)) => {
                let _ = self.inner.try_update_value(|i| i.pending.insert(id, done));
            }
            (false, Some(done)) => done(Err("not connected to the hub".to_string())),
            (_, None) => {}
        }
        sent
    }

    /// Ticks `strip` on or off in `layout` (the channel detail's LAYOUTS).
    pub fn sector_tick(
        self,
        layout: &str,
        strip: &StripRef,
        on: bool,
        done: Option<ResultFn>,
    ) -> bool {
        self.sector_request(
            |id| ClientMsg::SectorTick {
                id,
                layout: layout.to_string(),
                strip: strip.clone(),
                on,
            },
            done,
        )
    }

    /// ARRANGE's save: the changed layouts' new orders, at the document's
    /// revision `rev`.
    pub fn sector_orders(self, orders: Vec<SectorOrder>, rev: u64, done: Option<ResultFn>) -> bool {
        self.sector_request(|id| ClientMsg::SectorOrders { id, orders, rev }, done)
    }

    /// ↶: the hub swaps in the version before its last change.
    pub fn sector_undo(self, done: Option<ResultFn>) -> bool {
        self.sector_request(|id| ClientMsg::SectorUndo { id }, done)
    }
}
