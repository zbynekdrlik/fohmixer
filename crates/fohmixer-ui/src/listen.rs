//! Listen's glue (#210 phase 2): the audio socket to the hub's `/ws/audio`,
//! the player (`listen_player.js`: WebCodecs' Opus decoder, Web Audio, the
//! -1 dBFS limiter worklet) and the signals the 🎧 key and the column foot
//! read. It decides nothing: [`crate::behave::listen::Listen`] does, and
//! this carries its acts out. One per surface (a context of `Surface`), so
//! listening goes on while the detail closes and the foot says so.

use std::time::Duration;

use fohmixer_proto::client::LiveCommand;
use fohmixer_proto::layout::Strip;
use fohmixer_proto::listen::AudioServerMsg;
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::behave::label::shown_label;
use crate::behave::listen::{
    Act, Listen, Phase, RECONNECT_MS, Target, TrackName, VOLUME_DEFAULT, gain, name_of, track_name,
};
use crate::components::buttons::anchor_name;
use crate::store::LiveStore;
use crate::{dom, net};

#[wasm_bindgen(module = "/listen_player.js")]
extern "C" {
    #[wasm_bindgen(js_name = listenDecoder)]
    fn listen_decoder() -> bool;
    #[wasm_bindgen(js_name = startListenPlayer)]
    fn start_player();
    #[wasm_bindgen(js_name = stopListenPlayer)]
    fn stop_player();
    #[wasm_bindgen(js_name = setListenGain)]
    fn set_gain(value: f64);
    #[wasm_bindgen(js_name = resumeListenPlayer)]
    fn resume_player();
    #[wasm_bindgen(js_name = feedListenPacket)]
    fn feed_packet(bytes: &js_sys::Uint8Array);
}

/// The label a strip's channel shows.
fn label_of(strip: &Strip) -> String {
    shown_label(strip.label.as_deref(), &anchor_name(&strip.binding))
}

/// What the key and the foot show.
#[derive(Debug, Clone, PartialEq)]
pub struct ListenView {
    pub phase: Phase,
    pub lit: bool,
    pub label: Option<String>,
}

struct AudioSocket {
    ws: web_sys::WebSocket,
    _on_open: Closure<dyn FnMut()>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl AudioSocket {
    /// Closes it without its close handler (a close the page made is no
    /// lost socket).
    fn close(self) {
        self.ws.set_onclose(None);
        self.ws.set_onmessage(None);
        self.ws.set_onopen(None);
        let _ = self.ws.close();
    }
}

struct Inner {
    token: String,
    machine: Listen,
    socket: Option<AudioSocket>,
    /// Each socket's number: a late event of an older one does nothing.
    number: u64,
    stopped: bool,
    /// The window's listeners that resume a suspended player (removed at
    /// `stop`).
    resumers: Vec<WindowListenerHandle>,
}

/// The page's listening (a context of `Surface`).
#[derive(Clone, Copy)]
pub struct Listener {
    inner: StoredValue<Inner, LocalStorage>,
    pub view: RwSignal<ListenView>,
    /// The volume's place, 0 to 1 (`behave::listen::gain`).
    pub volume: RwSignal<f64>,
}

impl Listener {
    pub fn new(token: String) -> Self {
        set_gain(gain(VOLUME_DEFAULT));
        let me = Self {
            inner: StoredValue::new_local(Inner {
                token,
                machine: Listen::new(),
                socket: None,
                number: 0,
                stopped: false,
                resumers: Vec::new(),
            }),
            view: RwSignal::new(ListenView {
                phase: Phase::Off,
                lit: false,
                label: None,
            }),
            volume: RwSignal::new(VOLUME_DEFAULT),
        };
        // iOS suspends a page's audio when the page is hidden or the screen
        // locks, and only a user's gesture resumes it: the page shown again
        // and every lift of a finger ask the player to resume (it does
        // nothing while it plays or is stopped).
        let resumers = vec![
            window_event_listener_untyped("visibilitychange", |_| resume_player()),
            window_event_listener_untyped("pointerup", |_| resume_player()),
        ];
        let _ = me.inner.try_update_value(|i| i.resumers = resumers);
        me
    }

    /// The surface goes: the socket closes, the player stops.
    pub fn stop(self) {
        let Some((socket, resumers)) = self.inner.try_update_value(|i| {
            i.stopped = true;
            (i.socket.take(), std::mem::take(&mut i.resumers))
        }) else {
            return;
        };
        for handle in resumers {
            handle.remove();
        }
        if let Some(socket) = socket {
            socket.close();
        }
        stop_player();
    }

    /// The volume moved.
    pub fn set_volume(self, pos: f64) {
        let _ = self.volume.try_set(pos);
        set_gain(gain(pos));
    }

    /// The 🎧 key's tap on the detail of `strip`: the machine decides
    /// (stop, or start: the player starts here, inside the gesture, and the
    /// strip's track is asked for).
    pub fn tap(self, store: LiveStore, strip: &Strip) {
        let decoder = listen_decoder();
        let label = label_of(strip);
        if let Some(Some(n)) = self.run("tap", move |m| m.tap(label, decoder)) {
            self.resolve(store, strip, n);
        }
    }

    /// The detail shows `strip` now: while listening, its track is asked
    /// for and followed.
    pub fn follow(self, store: LiveStore, strip: &Strip) {
        if self.inner.try_with_value(|i| i.machine.lit()) != Some(true) {
            return;
        }
        let label = label_of(strip);
        if let Some(Some(n)) = self.run("follow", move |m| (Vec::new(), m.follow(label))) {
            self.resolve(store, strip, n);
        }
    }

    /// Answers ask `n` with `strip`'s target: at once for a track by name,
    /// after Live's answer for one by place (none: nothing to listen to).
    fn resolve(self, store: LiveStore, strip: &Strip, n: u64) {
        let instance = strip.binding.instance.clone();
        let label = label_of(strip);
        let make = move |track: Option<String>| {
            track.map(|track| Target {
                instance: instance.clone(),
                track,
                label,
            })
        };
        match track_name(&strip.binding) {
            TrackName::Known(name) => self.answer(n, make(Some(name))),
            TrackName::None => self.answer(n, None),
            TrackName::Read(path) => {
                let read = LiveCommand {
                    target: Value::String(path),
                    name: "get_prop".to_string(),
                    args: json!({ "prop": "name" }),
                };
                store.cmd(
                    &strip.binding.instance,
                    vec![read],
                    Some(Box::new(move |outcome: Result<Vec<Value>, String>| {
                        self.answer(n, make(name_of(&outcome)));
                    })),
                );
            }
        }
    }

    fn answer(self, n: u64, target: Option<Target>) {
        self.run("answer", move |m| (m.answer(n, target), ()));
    }

    /// Runs one step of the machine (its acts and whatever else it
    /// returns), records it, shows it and carries out its acts.
    fn run<R>(self, what: &str, step: impl FnOnce(&mut Listen) -> (Vec<Act>, R)) -> Option<R> {
        let (acts, out, phase, track, view) = self.inner.try_update_value(|i| {
            let (acts, out) = step(&mut i.machine);
            let m = &i.machine;
            let track = m.want().map(|t| (t.instance.clone(), t.track.clone()));
            let view = ListenView {
                phase: m.phase(),
                lit: m.lit(),
                label: m.label().map(str::to_string),
            };
            (acts, out, m.phase(), track, view)
        })?;
        crate::diag::record(&crate::diag::trace::listen(
            dom::epoch_now(),
            what,
            phase.name(),
            track.as_ref().map(|(i, t)| (i.as_str(), t.as_str())),
        ));
        if self.view.try_with_untracked(|v| *v != view) == Some(true) {
            let _ = self.view.try_set(view);
        }
        for act in acts {
            self.act(act);
        }
        Some(out)
    }

    fn act(self, act: Act) {
        match act {
            Act::Play => start_player(),
            Act::Open => self.open(),
            Act::Reopen(turn) => {
                set_timeout(
                    move || {
                        let due = self
                            .inner
                            .try_with_value(|i| !i.stopped && i.machine.reopen_due(turn))
                            .unwrap_or(false);
                        if due {
                            self.open();
                        }
                    },
                    Duration::from_millis(RECONNECT_MS),
                );
            }
            Act::Close => {
                let socket = self.inner.try_update_value(|i| i.socket.take()).flatten();
                if let Some(socket) = socket {
                    socket.close();
                }
            }
            Act::Silence => stop_player(),
            Act::Send(msg) => {
                let text = serde_json::to_string(&msg).unwrap_or_default();
                let sent = self
                    .inner
                    .try_with_value(|i| i.socket.as_ref().map(|s| s.ws.send_with_str(&text)))
                    .flatten();
                if let Some(Err(e)) = sent {
                    dom::log(&format!(
                        "listen: the audio socket refused a message: {e:?}"
                    ));
                }
            }
        }
    }

    fn open(self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let Some((token, stopped)) = self.inner.try_with_value(|i| (i.token.clone(), i.stopped))
        else {
            return;
        };
        if stopped {
            return;
        }
        let url = net::audio_url(
            &location.protocol().unwrap_or_default(),
            &location.host().unwrap_or_default(),
            &token,
        );
        let ws = match web_sys::WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(e) => {
                dom::log(&format!("listen: cannot open the audio socket: {e:?}"));
                self.run("close", |m| (m.closed(), ()));
                return;
            }
        };
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let Some(number) = self.inner.try_update_value(|i| {
            i.number += 1;
            i.number
        }) else {
            return;
        };
        let mine = move |me: Self| {
            me.inner
                .try_with_value(|i| i.number == number)
                .unwrap_or(false)
        };
        let on_open = Closure::wrap(Box::new(move || {
            if mine(self) {
                dom::log("listen: the audio socket opened");
                self.run("open", |m| (m.opened(), ()));
            }
        }) as Box<dyn FnMut()>);
        let on_message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            if !mine(self) {
                return;
            }
            let data = event.data();
            if let Some(text) = data.as_string() {
                self.on_text(&text);
            } else if let Ok(buffer) = data.dyn_into::<js_sys::ArrayBuffer>() {
                feed_packet(&js_sys::Uint8Array::new(&buffer));
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        let on_close = Closure::wrap(Box::new(move |event: web_sys::CloseEvent| {
            if !mine(self) {
                return;
            }
            dom::log(&format!(
                "listen: the audio socket closed ({})",
                event.code()
            ));
            let _ = self.inner.try_update_value(|i| i.socket = None);
            self.run("close", |m| (m.closed(), ()));
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        let socket = AudioSocket {
            ws,
            _on_open: on_open,
            _on_message: on_message,
            _on_close: on_close,
        };
        let old = self
            .inner
            .try_update_value(|i| i.socket.replace(socket))
            .flatten();
        if let Some(old) = old {
            old.close();
        }
    }

    fn on_text(self, text: &str) {
        match serde_json::from_str::<AudioServerMsg>(text) {
            Ok(AudioServerMsg::AudioStatus {
                status,
                instance,
                track,
            }) => {
                dom::log(&format!("listen: the hub says {status:?}"));
                self.run("status", move |m| {
                    (m.status(status, instance.as_deref(), track.as_deref()), ())
                });
            }
            Err(e) => dom::log(&format!("listen: an audio message did not parse: {e}")),
        }
    }
}
