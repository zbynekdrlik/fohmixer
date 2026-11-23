//! Claps from the surface (#220): the Claps strip's two messages (#260
//! removed the beat dots, so no page sends `clap_beat` any more). The
//! hub answers none of them by id: its next `claps` message shows what they
//! did, and a refusal is its own `claps` record. Each is sent at once or not
//! at all (a tap that waited in a stalled link would choose a wrong beat):
//! never through the intent store, never resent.

use fohmixer_proto::client::ClientMsg;

use super::LiveStore;

impl LiveStore {
    /// A tap of TAP at page time `t` (the finger's down, epoch ms). Whether
    /// the socket took it.
    pub fn clap_tap(self, t: f64) -> bool {
        self.send(&ClientMsg::ClapTap { t })
    }

    /// TAP held 1 s: nothing claps. Whether the socket took it.
    pub fn clap_stop(self) -> bool {
        self.send(&ClientMsg::ClapStop)
    }
}
