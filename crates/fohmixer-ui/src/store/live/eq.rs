//! The Pro-Q 4 screen's glue in the store (#71 PR E): the hub's `eq_list`,
//! `eq_locks` and `eq` into the signals (`store/eq.rs`, tested), the page's
//! `eq_list`, `eq_open` (with its picture area, PR G), `eq_area`,
//! `eq_input` and `eq_close`, a card's last picture
//! fetched with the token, and the binary frames: the session in their
//! header (`fohmixer_proto::eq::frame_parts`) checked against the page's
//! open editor (`behave::eq::shows_frame`) and the JPEG handed as a blob to
//! the screen's sink. A closed socket closes the page's editor
//! (`store::eq::socket_closed`): the hub closes it with the socket.

use std::rc::Rc;

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::eq::{Area, EqItem, EqLock, frame_parts};
use fohmixer_proto::layout::Binding;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::LiveStore;
use crate::behave::eq::{Out, picture_url, shows_frame};
use crate::dom;
use crate::net;
use crate::store::eq::{EqListView, EqView, asked, socket_closed};

/// Where the open editor's frames go: the screen's painter.
pub type FrameSink = Rc<dyn Fn(web_sys::Blob)>;

impl LiveStore {
    /// The hub's `eq_list`: the latest list, numbered.
    pub(super) fn on_eq_list(self, binding: Binding, items: Vec<EqItem>, error: Option<String>) {
        let _ = self.eq_list.try_update(|list| {
            let generation = list.as_ref().map_or(0, |l| l.generation) + 1;
            *list = Some(EqListView {
                binding,
                items,
                error,
                generation,
            });
        });
    }

    /// The hub's `eq_locks`.
    pub(super) fn on_eq_locks(self, items: Vec<EqLock>) {
        let _ = self.eq_locks.try_set(items);
    }

    /// The hub's `eq`: the page's editor now.
    pub(super) fn on_eq(self, view: EqView) {
        let _ = self.eq.try_set(Some(view));
    }

    /// A binary message: a frame of the page's open editor goes to the
    /// screen's sink; another one (an editor already left) is dropped.
    pub(super) fn on_frame(self, data: &wasm_bindgen::JsValue) {
        let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() else {
            return;
        };
        let bytes = js_sys::Uint8Array::new(buffer);
        let mut head = [0_u8; fohmixer_proto::eq::FRAME_HEADER];
        if bytes.length() < head.len() as u32 {
            return;
        }
        bytes.subarray(0, head.len() as u32).copy_to(&mut head);
        let Some((session, _)) = frame_parts(&head) else {
            return;
        };
        let open = self
            .eq
            .try_with_untracked(|v| v.as_ref().and_then(EqView::open_session))
            .flatten();
        if !shows_frame(session, open) {
            return;
        }
        let sink = self.inner.try_with_value(|i| i.eq_frames.clone()).flatten();
        let Some(sink) = sink else {
            return;
        };
        let jpeg = bytes.subarray(head.len() as u32, bytes.length());
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("image/jpeg");
        match web_sys::Blob::new_with_u8_array_sequence_and_options(
            &js_sys::Array::of1(&jpeg),
            &options,
        ) {
            Ok(blob) => sink(blob),
            Err(e) => dom::log(&format!("a Pro-Q frame could not be read: {e:?}")),
        }
    }

    /// The screen's painter, or none (it left).
    pub fn eq_frames(self, sink: Option<FrameSink>) {
        let _ = self.inner.try_update_value(|i| i.eq_frames = sink);
    }

    /// Asks for the Pro-Q 4 instances of a strip's track.
    pub fn list_eq(self, binding: &Binding) {
        self.send(&ClientMsg::EqList {
            binding: binding.clone(),
        });
    }

    /// The screen of the editor at `path` on `instance` mounts: opening from
    /// now on (a closed state of it from before is not this screen's end);
    /// its `eq_open` goes once its picture area is in the page (PR G).
    pub fn eq_opening(self, instance: &str, path: &str) {
        let _ = self.eq.try_set(Some(asked(instance, path, true)));
    }

    /// Opens the editor at `path` on `instance` for a picture `area` (CSS
    /// px, PR G: the editor gets its aspect): opening from now on, or
    /// closed at once (`socket`) when the socket could not take the ask
    /// (`store::eq::asked`; the hub's answer comes in a later task either
    /// way).
    pub fn eq_open(self, instance: &str, path: &str, (w, h): (f64, f64)) {
        let sent = self.send(&ClientMsg::EqOpen {
            instance: instance.to_string(),
            path: path.to_string(),
            area: Some(Area { w, h }),
        });
        let _ = self.eq.try_set(Some(asked(instance, path, sent)));
    }

    /// The open editor's picture area changed (CSS px, PR G).
    pub fn eq_area(self, (w, h): (f64, f64)) {
        self.send(&ClientMsg::EqArea { w, h });
    }

    /// A finger on the open editor.
    pub fn eq_input(self, (touch, x, y): Out) {
        self.send(&ClientMsg::EqInput { touch, x, y });
    }

    /// The page leaves its editor.
    pub fn eq_close(self) {
        self.send(&ClientMsg::EqClose);
    }

    /// The socket closed: the page's editor with it.
    pub(super) fn eq_socket_closed(self) {
        let _ = self
            .eq
            .try_update(|view| *view = socket_closed(view.take()));
    }

    /// A card's last picture (none before a first open, or when it did not
    /// load).
    pub async fn eq_picture(self, instance: &str, path: &str) -> Option<web_sys::Blob> {
        match net::fetch_blob(&picture_url(instance, path), &self.token()).await {
            Ok((200, blob)) => blob,
            Ok((status, _)) => {
                dom::log(&format!("a Pro-Q picture answered HTTP {status}"));
                None
            }
            Err(why) => {
                dom::log(&format!("a Pro-Q picture did not load: {why}"));
                None
            }
        }
    }
}
