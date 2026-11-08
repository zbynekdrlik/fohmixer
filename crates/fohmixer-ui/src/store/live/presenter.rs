//! Presenter's stage tab's glue in the store (#90): the hub's `presenter`
//! into its signal (kept through a reconnect until the hub's next one, as
//! the deck's and AbleSet's), a held key's `presenter_switch` (sent at once
//! or not at all: never queued, never sent again after a reconnect; a stage
//! display must not switch late) and its `presenter_ack` (a failure flashes
//! the key). A socket close forgets the switches waiting for their answer.

use fohmixer_proto::client::{ClientMsg, StageLayout};
use leptos::prelude::*;

use super::{FailFn, LiveStore};
use crate::behave::presenter::PresenterInfo;
use crate::dom;

impl LiveStore {
    /// The hub's `presenter`.
    pub(super) fn on_presenter(
        self,
        online: Option<bool>,
        layouts: Vec<StageLayout>,
        current: Option<String>,
    ) {
        let _ = self.presenter.try_set(Some(PresenterInfo {
            online,
            layouts,
            current,
        }));
    }

    /// A switch's answer: a failure flashes its key.
    pub(super) fn on_presenter_ack(self, seq: u64, code: &str, ok: bool, error: Option<String>) {
        if let Some(Some(flash)) = self.inner.try_update_value(|i| i.presenter.ack(seq, ok)) {
            let why = error.unwrap_or_default();
            dom::log(&format!("the stage layout {code} did not switch: {why}"));
            flash(why);
        }
    }

    /// The socket closed: no answer will come.
    pub(super) fn presenter_closed(self) {
        let _ = self.inner.try_update_value(|i| i.presenter.clear());
    }

    /// A held key's switch to `code`: onto the socket at once with the
    /// page's own number, or not at all (`Err` hands `on_fail` back to flash
    /// the key).
    pub fn presenter_switch(self, code: &str, on_fail: FailFn) -> Result<u64, FailFn> {
        let Some(seq) = self.inner.try_update_value(|i| i.presenter.next_seq()) else {
            return Err(on_fail);
        };
        let msg = ClientMsg::PresenterSwitch {
            code: code.to_string(),
            seq,
        };
        if !self.send(&msg) {
            return Err(on_fail);
        }
        let _ = self
            .inner
            .try_update_value(|i| i.presenter.sent(seq, true, on_fail));
        Ok(seq)
    }
}
