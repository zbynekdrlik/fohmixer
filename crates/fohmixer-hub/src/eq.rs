//! The router's Pro-Q 4 state (#71 PR E, spec F28, D17), pure, in the
//! Stream Deck's shape (`deck.rs`): who holds which editor and in what stage,
//! the locks every client sees, the one contact on the PC's screen, when a
//! resting contact is sent again and when a silent page's contact ends, and
//! the steps of an open and a close. The router (`router/eq.rs`) carries the
//! [`Act`]s out; the platform backend (`plugwin`) does what touches a window.
//!
//! - **Locks:** an editor is held by the client that opened it until it
//!   leaves (`eq_close`), its socket closes or it opens another one; another
//!   client's open is refused `locked` (with since when). A client holds one
//!   editor: opening another closes the one it holds first (`switch`), the
//!   new one waiting for that close.
//! - **Opens run one at a time:** the hub finds an editor's window by the
//!   window list before and after `is_editor_open = true`, so two opens at
//!   once could take each other's window. The others wait in a queue.
//! - **Only listed editors open:** a path the hub's `eq_list` reads found
//!   ([`Eqs::listed`]); never an arbitrary LOM path. The open sequence reads
//!   the device first ([`open`]): still the listed Pro-Q 4 (its name), its
//!   editor closed in Live. A track's new list replaces its last one, and
//!   an instance that connects again has its lists forgotten (another set
//!   may be loaded). Each listed path names its device by its `$ref`
//!   ([`Eqs::named`]): a card's last picture is kept only while its path
//!   names the device it is of ([`Pictures`]).
//! - **The contact:** one on the screen at a time (the PC has one cursor),
//!   on an open editor of the client touching. A move goes on at once; an
//!   up at a point other than the last one moves there first; a resting
//!   contact is kept alive by the window worker on its own clock
//!   (`plugwin::KEEPALIVE_MS`); a client heard from not at all for
//!   [`SILENT_MS`] (its pings stop: the page went away) ends its contact
//!   with a cancel. A down while the client's own contact is still
//!   down ends that one first. Every contact is numbered: its phases carry
//!   the number, and the window worker's word that it ended a contact ends
//!   only that one (a late word never ends a newer contact).
//! - **The close:** never while a contact is down: the close sequence's
//!   guard ends it first (`plugwin`), then taps the inert spot, waits
//!   [`GUARD_WAIT`], sets `is_editor_open = false` and releases the window.
//!   The device it turns off is checked first ([`close`]): the held path's
//!   device if it is still the open Pro-Q 4, else the one open Pro-Q 4 of
//!   the set, else none (Live is left alone). An editor still opening
//!   closes as soon as it is open; one still queued is dropped at once.
//!
//! Times: `now` is the router's clock (ms), `wall` the hub's UTC ms (the
//! lock's "since", shown on the pages).

pub mod close;
pub mod open;
pub mod walk;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use fohmixer_proto::client::ServerMsg;
use fohmixer_proto::eq::{EqLock, EqState, Touch, reason};
use serde_json::{Value, json};

use crate::live::subs::ClientId;
use crate::plugwin::Phase;

/// A client heard from not at all for this long (ms): its contact ends.
pub const SILENT_MS: f64 = 2000.0;
/// The wait after the close guard's tap before the editor closes.
pub const GUARD_WAIT: Duration = Duration::from_millis(300);

/// An editor: its Live instance and its device's LOM path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EditorKey {
    pub instance: String,
    pub path: String,
}

impl EditorKey {
    pub fn new(instance: &str, path: &str) -> Self {
        Self {
            instance: instance.to_string(),
            path: path.to_string(),
        }
    }
}

/// Whether a client last heard `since_ms` ago is silent.
pub fn silent(since_ms: f64) -> bool {
    since_ms >= SILENT_MS
}

/// A point of the page's in a picture of `width` × `height`: rounded and
/// clamped into it (a finger past the picture's edge stays on its edge).
pub fn clamp_point(x: f64, y: f64, width: u32, height: u32) -> (i32, i32) {
    // `as` saturates and takes NaN (no JSON number is one) to 0.
    let inside = |v: f64, size: u32| v.round().clamp(0.0, f64::from(size.max(1) - 1)) as i32;
    (inside(x, width), inside(y, height))
}

/// The contact phases a page's touch makes on an editor whose contact is
/// at `last` (none: no contact): a down starts one (ending an older one of
/// the same editor first), a move moves it, an up ends it at its point
/// (moving there first when it is not the last one), a cancel ends it where
/// it was. A move, up or cancel without a contact does nothing.
pub fn phases(touch: Touch, last: Option<(i32, i32)>, at: (i32, i32)) -> Vec<(Phase, (i32, i32))> {
    match (touch, last) {
        (Touch::Down, None) => vec![(Phase::Down, at)],
        (Touch::Down, Some(old)) => vec![(Phase::Up, old), (Phase::Down, at)],
        (Touch::Move, Some(_)) => vec![(Phase::Update, at)],
        (Touch::Up, Some(old)) if old == at => vec![(Phase::Up, at)],
        (Touch::Up, Some(_)) => vec![(Phase::Update, at), (Phase::Up, at)],
        (Touch::Cancel, Some(old)) => vec![(Phase::Cancel, old)],
        (Touch::Move | Touch::Up | Touch::Cancel, None) => Vec::new(),
    }
}

/// What the router does for the state.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// The open sequence of `key` as `session`: the window list, `is_editor_open
    /// = true`, the new window taken. `connection`: its instance's connection
    /// it starts on ([`Eqs::connection`]); the device's `$ref` the open reads
    /// holds for that connection only.
    Open {
        key: EditorKey,
        session: u32,
        connection: u32,
    },
    /// The close sequence: the guard (any contact ended, the inert spot
    /// tapped), [`GUARD_WAIT`], `is_editor_open = false`, the window released.
    Close {
        key: EditorKey,
        session: u32,
        why: &'static str,
    },
    /// Its frames start going to its holder.
    Capture {
        key: EditorKey,
        session: u32,
        holder: ClientId,
    },
    /// A contact phase (of contact number `contact`) into the window of
    /// `session`.
    Touch {
        session: u32,
        contact: u32,
        phase: Phase,
        at: (i32, i32),
    },
    /// A client's editor state (`eq`).
    Tell { client: ClientId, msg: ServerMsg },
    /// The locks changed: every client gets them.
    Locks,
    /// An `eq` record of the event log, and the hub log's line.
    Record(Value),
}

/// A device a list found at a path: its name and its `$ref` (the script's
/// registry id: the same device for as long as the script's connection
/// lasts, wherever it moves; another device at the path has another).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub id: String,
}

/// What one list found: each editor with its device.
type Listing = BTreeMap<EditorKey, Device>;

/// An editor's stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Its client's previous editor is still closing.
    Waiting,
    /// Behind another open.
    Queued,
    /// Its open sequence runs.
    Opening,
    /// Its window is open, its picture this size.
    Open { width: u32, height: u32 },
    /// Its close sequence runs.
    Closing,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Queued => "queued",
            Self::Opening => "opening",
            Self::Open { .. } => "open",
            Self::Closing => "closing",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Editor {
    holder: ClientId,
    since: f64,
    session: u32,
    stage: Stage,
    /// Why it closes (set once a close is asked for).
    why: Option<&'static str>,
}

/// The contact on the screen.
#[derive(Debug, Clone, PartialEq)]
struct Contact {
    key: EditorKey,
    session: u32,
    /// Its number (one counter for the hub's contacts).
    number: u32,
    at: (i32, i32),
}

/// The `eq` message of an editor being opened.
pub fn opening_msg(key: &EditorKey) -> ServerMsg {
    state_msg(key, EqState::Opening, None, None, None)
}

/// The `eq` message of an open editor.
pub fn open_msg(key: &EditorKey, session: u32, width: u32, height: u32) -> ServerMsg {
    state_msg(
        key,
        EqState::Open,
        Some((session, width, height)),
        None,
        None,
    )
}

/// The `eq` message of an editor closed (or refused) for `why`, held by
/// another client since `since` when locked.
pub fn closed_msg(key: &EditorKey, why: &str, since: Option<f64>) -> ServerMsg {
    state_msg(key, EqState::Closed, None, Some(why), since)
}

fn state_msg(
    key: &EditorKey,
    state: EqState,
    open: Option<(u32, u32, u32)>,
    why: Option<&str>,
    since: Option<f64>,
) -> ServerMsg {
    ServerMsg::Eq {
        instance: key.instance.clone(),
        path: key.path.clone(),
        state,
        session: open.map(|(session, _, _)| session),
        width: open.map(|(_, width, _)| width),
        height: open.map(|(_, _, height)| height),
        reason: why.map(str::to_string),
        since,
    }
}

/// The fields of an `eq` record: what happened (`what`) to which editor,
/// its client and session, and why.
pub fn record(
    what: &str,
    key: &EditorKey,
    client: Option<ClientId>,
    session: Option<u32>,
    why: Option<&str>,
) -> Value {
    json!({
        "what": what,
        "instance": key.instance,
        "path": key.path,
        "client": client,
        "session": session,
        "why": why,
    })
}

/// The Pro-Q 4 editors as the router knows them.
#[derive(Debug, Default)]
pub struct Eqs {
    editors: BTreeMap<EditorKey, Editor>,
    /// The editor each client holds (or is closing).
    held: BTreeMap<ClientId, EditorKey>,
    /// The editor a client opens next, once its held one closed.
    next: BTreeMap<ClientId, EditorKey>,
    queue: VecDeque<EditorKey>,
    /// An open sequence runs.
    opening: bool,
    contact: Option<Contact>,
    heard: HashMap<ClientId, f64>,
    /// What each list found (by its instance and its track's LOM target):
    /// the editors that may be opened, with their device.
    lists: BTreeMap<(String, String), Listing>,
    /// The last session number given out.
    sessions: u32,
    /// The last contact number given out.
    contacts: u32,
    /// How often each instance connected again since the hub started (its
    /// connection's number: a `$ref` holds for one connection only).
    connections: BTreeMap<String, u32>,
}

impl Eqs {
    /// The list of the track at `target` on `instance` found these editors
    /// (each with its device): they may be opened. It replaces that track's
    /// last list, and an editor it names is its own from now on: another
    /// track's older list no longer names it (a track or a device moved, so
    /// a strip not listed since may still hold the path of the device
    /// another strip has now).
    pub fn listed(
        &mut self,
        instance: &str,
        target: &str,
        found: impl IntoIterator<Item = (EditorKey, Device)>,
    ) {
        let found: Listing = found.into_iter().collect();
        for listing in self.lists.values_mut() {
            listing.retain(|key, _| !found.contains_key(key));
        }
        self.lists
            .insert((instance.to_string(), target.to_string()), found);
    }

    /// The device each listed path names now, by its `$ref` ([`Pictures`]
    /// keeps a card's picture only while its path names its device).
    pub fn named(&self) -> BTreeMap<EditorKey, String> {
        self.lists
            .values()
            .flatten()
            .map(|(key, device)| (key.clone(), device.id.clone()))
            .collect()
    }

    /// Whether a list names `key`.
    fn is_listed(&self, key: &EditorKey) -> bool {
        self.lists.values().any(|listing| listing.contains_key(key))
    }

    /// The device name a list found at `key` (the open checks it): the
    /// newest list's, the only one that names it ([`Eqs::listed`]).
    pub fn name_of(&self, key: &EditorKey) -> Option<&str> {
        self.lists
            .values()
            .find_map(|listing| listing.get(key))
            .map(|device| device.name.as_str())
    }

    /// An instance connected again: its lists may name other devices now
    /// (another set), so they are forgotten until listed again, and its
    /// connection's number goes up.
    pub fn forget(&mut self, instance: &str) {
        self.lists.retain(|(of, _), _| of.as_str() != instance);
        *self.connections.entry(instance.to_string()).or_default() += 1;
    }

    /// The number of `instance`'s connection now (0 until it connected
    /// again): an open's answer from an older one brings a `$ref` of an
    /// older script registry.
    pub fn connection(&self, instance: &str) -> u32 {
        self.connections.get(instance).copied().unwrap_or(0)
    }

    /// Something came from `client` at `now`.
    pub fn heard(&mut self, client: ClientId, now: f64) {
        self.heard.insert(client, now);
    }

    /// The locks as `client` sees them: every taken editor, its own marked.
    pub fn locks_for(&self, client: ClientId) -> Vec<EqLock> {
        self.editors
            .iter()
            .map(|(key, editor)| EqLock {
                instance: key.instance.clone(),
                path: key.path.clone(),
                mine: editor.holder == client,
                since: editor.since,
            })
            .collect()
    }

    /// The editor `client` holds, its session and stage name (the tests
    /// read the state through it; `/api/status` shows nothing of the Pro-Q
    /// 4 screen).
    pub fn held_by(&self, client: ClientId) -> Option<(EditorKey, u32, &'static str)> {
        let key = self.held.get(&client)?;
        let editor = self.editors.get(key)?;
        Some((key.clone(), editor.session, editor.stage.name()))
    }

    /// The editor of `session` (0, a session not given out, is none).
    pub fn key_of(&self, session: u32) -> Option<EditorKey> {
        self.editors
            .iter()
            .find(|(_, e)| e.session == session && session > 0)
            .map(|(key, _)| key.clone())
    }

    /// `client` opens `key` at `wall`.
    pub fn open(&mut self, client: ClientId, key: &EditorKey, wall: f64) -> Vec<Act> {
        if !self.is_listed(key) {
            return vec![
                Act::Tell {
                    client,
                    msg: closed_msg(key, reason::UNKNOWN, None),
                },
                Act::Record(record(
                    "refused",
                    key,
                    Some(client),
                    None,
                    Some(reason::UNKNOWN),
                )),
            ];
        }
        if let Some(editor) = self.editors.get(key) {
            return self.open_taken(client, key, editor.clone());
        }
        let mut acts = Vec::new();
        // A newer open replaces the one waiting for this client's close.
        if let Some(waiting) = self.next.remove(&client) {
            self.editors.remove(&waiting);
            acts.push(Act::Locks);
        }
        let stage = if self.held.contains_key(&client) {
            Stage::Waiting
        } else {
            Stage::Queued
        };
        self.editors.insert(
            key.clone(),
            Editor {
                holder: client,
                since: wall,
                session: 0,
                stage,
                why: None,
            },
        );
        acts.push(Act::Tell {
            client,
            msg: opening_msg(key),
        });
        acts.push(Act::Locks);
        acts.push(Act::Record(record("take", key, Some(client), None, None)));
        if stage == Stage::Waiting {
            self.next.insert(client, key.clone());
            acts.extend(self.close(client, reason::SWITCH, false));
        } else {
            self.held.insert(client, key.clone());
            self.queue.push_back(key.clone());
            acts.extend(self.start_next());
        }
        acts
    }

    /// An open of an editor someone holds: another client is refused, its
    /// own holder hears its state again (or, closing, to try again later).
    fn open_taken(&self, client: ClientId, key: &EditorKey, editor: Editor) -> Vec<Act> {
        if editor.holder != client {
            return vec![
                Act::Tell {
                    client,
                    msg: closed_msg(key, reason::LOCKED, Some(editor.since)),
                },
                Act::Record(record(
                    "refused",
                    key,
                    Some(client),
                    None,
                    Some(reason::LOCKED),
                )),
            ];
        }
        let msg = match editor.stage {
            Stage::Open { width, height } => open_msg(key, editor.session, width, height),
            Stage::Closing => closed_msg(key, reason::CLOSING, None),
            Stage::Waiting | Stage::Queued | Stage::Opening => opening_msg(key),
        };
        vec![Act::Tell { client, msg }]
    }

    /// The next queued editor's open, when none runs.
    fn start_next(&mut self) -> Vec<Act> {
        if self.opening {
            return Vec::new();
        }
        let Some(key) = self.queue.pop_front() else {
            return Vec::new();
        };
        let Some(editor) = self.editors.get_mut(&key) else {
            return Vec::new();
        };
        self.sessions += 1;
        editor.session = self.sessions;
        editor.stage = Stage::Opening;
        self.opening = true;
        let connection = self.connections.get(&key.instance).copied().unwrap_or(0);
        vec![
            Act::Record(record(
                "open",
                &key,
                Some(editor.holder),
                Some(self.sessions),
                None,
            )),
            Act::Open {
                key,
                session: self.sessions,
                connection,
            },
        ]
    }

    /// The open sequence of `key` (`session`) ended: its picture's size, or
    /// why it failed (its window is not open then).
    pub fn opened(
        &mut self,
        key: &EditorKey,
        session: u32,
        outcome: Result<(u32, u32), String>,
    ) -> Vec<Act> {
        self.opening = false;
        let mut acts = Vec::new();
        let ours = self
            .editors
            .get(key)
            .filter(|e| e.session == session && e.stage == Stage::Opening)
            .cloned();
        if let Some(editor) = ours {
            match outcome {
                Ok((width, height)) => {
                    acts.push(Act::Record(record(
                        "opened",
                        key,
                        Some(editor.holder),
                        Some(session),
                        None,
                    )));
                    if let Some(e) = self.editors.get_mut(key) {
                        e.stage = Stage::Open { width, height };
                    }
                    match editor.why {
                        // Left while it opened: closed now.
                        Some(why) => acts.extend(self.close_key(key, why)),
                        None => {
                            acts.push(Act::Tell {
                                client: editor.holder,
                                msg: open_msg(key, session, width, height),
                            });
                            acts.push(Act::Capture {
                                key: key.clone(),
                                session,
                                holder: editor.holder,
                            });
                        }
                    }
                }
                Err(why) => {
                    acts.push(Act::Record(record(
                        "failed",
                        key,
                        Some(editor.holder),
                        Some(session),
                        Some(&why),
                    )));
                    acts.extend(self.release(key, &why));
                }
            }
        }
        acts.extend(self.start_next());
        acts
    }

    /// `client` leaves its editor (`why`: exit, detach, switch); with
    /// `drop_next` the editor waiting for that close is dropped too (the
    /// page left the screen).
    pub fn close(&mut self, client: ClientId, why: &'static str, drop_next: bool) -> Vec<Act> {
        let mut acts = Vec::new();
        if drop_next && let Some(waiting) = self.next.remove(&client) {
            self.editors.remove(&waiting);
            acts.push(Act::Locks);
        }
        if let Some(key) = self.held.get(&client).cloned() {
            acts.extend(self.close_key(&key, why));
        }
        acts
    }

    /// `key` closes for `why`, whatever its stage.
    fn close_key(&mut self, key: &EditorKey, why: &'static str) -> Vec<Act> {
        let Some(editor) = self.editors.get_mut(key) else {
            return Vec::new();
        };
        match editor.stage {
            Stage::Queued => {
                self.queue.retain(|k| k != key);
                self.release(key, why)
            }
            Stage::Opening => {
                editor.why = Some(why);
                Vec::new()
            }
            Stage::Open { .. } => {
                editor.stage = Stage::Closing;
                editor.why = Some(why);
                let (holder, session) = (editor.holder, editor.session);
                // The guard ends the contact on the screen; the state
                // forgets it now (nothing more goes to a closing editor).
                if self.contact.as_ref().is_some_and(|c| &c.key == key) {
                    self.contact = None;
                }
                vec![
                    Act::Record(record("close", key, Some(holder), Some(session), Some(why))),
                    Act::Close {
                        key: key.clone(),
                        session,
                        why,
                    },
                ]
            }
            Stage::Waiting | Stage::Closing => Vec::new(),
        }
    }

    /// The close sequence of `key` (`session`) ended; `left_open`: it left
    /// the editor open in Live (its guard could not tap, or the hub could
    /// not be sure which device to turn off), and its holder hears that
    /// ([`reason::LEFT_OPEN`]) instead of why it closed.
    pub fn closed(&mut self, key: &EditorKey, session: u32, left_open: bool) -> Vec<Act> {
        let Some(editor) = self
            .editors
            .get(key)
            .filter(|e| e.session == session && e.stage == Stage::Closing)
            .cloned()
        else {
            return Vec::new();
        };
        let why = editor.why.unwrap_or(reason::EXIT);
        let mut acts = vec![Act::Record(record(
            "closed",
            key,
            Some(editor.holder),
            Some(session),
            Some(why),
        ))];
        let told = if left_open { reason::LEFT_OPEN } else { why };
        acts.extend(self.release(key, told));
        acts
    }

    /// The window of `key` (`session`) went away while open (closed in
    /// Live or on the PC): released at once. A closing one finishes its
    /// close.
    pub fn lost(&mut self, key: &EditorKey, session: u32) -> Vec<Act> {
        let open = self
            .editors
            .get(key)
            .is_some_and(|e| e.session == session && matches!(e.stage, Stage::Open { .. }));
        if !open {
            return Vec::new();
        }
        if self.contact.as_ref().is_some_and(|c| &c.key == key) {
            self.contact = None;
        }
        let holder = self.editors.get(key).map(|e| e.holder);
        let mut acts = vec![Act::Record(record(
            "lost",
            key,
            holder,
            Some(session),
            Some(reason::GONE),
        ))];
        acts.extend(self.release(key, reason::GONE));
        acts
    }

    /// `key` is free again: its holder hears why, every client the locks;
    /// the editor its holder opens next starts.
    fn release(&mut self, key: &EditorKey, why: &str) -> Vec<Act> {
        let Some(editor) = self.editors.remove(key) else {
            return Vec::new();
        };
        let holder = editor.holder;
        if self.held.get(&holder) == Some(key) {
            self.held.remove(&holder);
        }
        let mut acts = vec![
            Act::Tell {
                client: holder,
                msg: closed_msg(key, why, None),
            },
            Act::Locks,
        ];
        if let Some(next) = self.next.remove(&holder) {
            if let Some(waiting) = self.editors.get_mut(&next) {
                waiting.stage = Stage::Queued;
            }
            self.held.insert(holder, next.clone());
            self.queue.push_back(next);
            acts.extend(self.start_next());
        }
        acts
    }

    /// `client`'s socket closed: its editor closes, the one it waited for
    /// is dropped, and it is forgotten.
    pub fn detach(&mut self, client: ClientId) -> Vec<Act> {
        self.heard.remove(&client);
        self.close(client, reason::DETACH, true)
    }

    /// A finger of `client` on its editor's picture at `now`.
    pub fn input(&mut self, client: ClientId, touch: Touch, x: f64, y: f64, now: f64) -> Vec<Act> {
        self.heard(client, now);
        let Some(key) = self.held.get(&client).cloned() else {
            return Vec::new();
        };
        let Some(editor) = self.editors.get(&key) else {
            return Vec::new();
        };
        let Stage::Open { width, height } = editor.stage else {
            return Vec::new();
        };
        let session = editor.session;
        // One contact on the screen: another editor's keeps it.
        if self.contact.as_ref().is_some_and(|c| c.key != key) {
            return vec![Act::Record(record(
                "busy",
                &key,
                Some(client),
                Some(session),
                Some(touch.name()),
            ))];
        }
        let at = clamp_point(x, y, width, height);
        let last = self.contact.as_ref().map(|c| c.at);
        let steps = phases(touch, last, at);
        let ends = matches!(touch, Touch::Up | Touch::Cancel);
        // A down starts a new contact; the other phases are the old one's
        // (a down's first phase, ending the old contact, included).
        let old = self.contact.as_ref().map_or(0, |c| c.number);
        let number = if touch == Touch::Down {
            self.contacts += 1;
            self.contacts
        } else {
            old
        };
        let mut acts = Vec::new();
        if let Some((_, at)) = steps.last() {
            self.contact = (!ends).then(|| Contact {
                key: key.clone(),
                session,
                number,
                at: *at,
            });
            // A contact's start and end are records; its moves are not.
            if touch != Touch::Move {
                acts.push(Act::Record(record(
                    "touch",
                    &key,
                    Some(client),
                    Some(session),
                    Some(touch.name()),
                )));
            }
        }
        acts.extend(steps.into_iter().map(|(phase, at)| Act::Touch {
            session,
            contact: if phase == Phase::Down { number } else { old },
            phase,
            at,
        }));
        acts
    }

    /// The router's tick at `now`: a silent client's contact ends with a
    /// cancel (a resting one is the window worker's to keep alive).
    pub fn tick(&mut self, now: f64) -> Vec<Act> {
        let Some(contact) = self.contact.as_ref() else {
            return Vec::new();
        };
        let holder = self.editors.get(&contact.key).map(|e| e.holder);
        let quiet = holder
            .and_then(|c| self.heard.get(&c))
            .is_none_or(|at| silent(now - at));
        if !quiet {
            return Vec::new();
        }
        let (session, number, at) = (contact.session, contact.number, contact.at);
        let key = contact.key.clone();
        self.contact = None;
        vec![
            Act::Record(record("silent", &key, holder, Some(session), None)),
            Act::Touch {
                session,
                contact: number,
                phase: Phase::Cancel,
                at,
            },
        ]
    }

    /// The backend ended contact `number` of `session` itself (its point was
    /// not the editor's any more, or the close guard needed the screen): it
    /// is forgotten. A later contact (a newer number) is not that one.
    pub fn contact_ended(&mut self, session: u32, number: u32) {
        if self
            .contact
            .as_ref()
            .is_some_and(|c| c.session == session && c.number == number)
        {
            self.contact = None;
        }
    }

    /// The hub stops: the editors whose windows are taken (open or
    /// closing), each with its session, to hand back. The state is left as
    /// it is (the router ends).
    pub fn taken(&self) -> Vec<(EditorKey, u32)> {
        self.editors
            .iter()
            .filter(|(_, e)| matches!(e.stage, Stage::Open { .. } | Stage::Closing))
            .map(|(key, e)| (key.clone(), e.session))
            .collect()
    }
}

/// What [`Pictures`] holds: the device each listed path names (its `$ref`)
/// and each path's last picture, with the device it is of.
#[derive(Debug, Default)]
struct Kept {
    names: BTreeMap<EditorKey, String>,
    jpegs: BTreeMap<EditorKey, (String, Bytes)>,
}

/// The last picture of each editor (`GET /api/eq/picture`), kept for the
/// detail's card: the frames' JPEG as the capture sent it, only while the
/// lists name its path for the device it is of. Two Pro-Q 4s that keep
/// their default name change places when a track moves above them: the
/// path then names the other device ([`Eqs::named`], by `$ref`), so the
/// picture goes, and a frame of the editor opened there before is no
/// longer kept.
#[derive(Debug, Default)]
pub struct Pictures {
    inner: Mutex<Kept>,
}

impl Pictures {
    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The newest picture of `key`, of the device `id`: kept while the
    /// lists name `key` for that device.
    pub fn put(&self, key: &EditorKey, id: &str, jpeg: Bytes) {
        let mut kept = self.lock();
        if kept.names.get(key).map(String::as_str) == Some(id) {
            kept.jpegs.insert(key.clone(), (id.to_string(), jpeg));
        }
    }

    /// The last picture of `key`, if any.
    pub fn get(&self, key: &EditorKey) -> Option<Bytes> {
        self.lock().jpegs.get(key).map(|(_, jpeg)| jpeg.clone())
    }

    /// Whether a picture of `key` is kept.
    pub fn has(&self, key: &EditorKey) -> bool {
        self.lock().jpegs.contains_key(key)
    }

    /// The device each listed path names now ([`Eqs::named`], after every
    /// list and connect): a picture whose path names another device, or
    /// none, goes.
    pub fn named(&self, names: BTreeMap<EditorKey, String>) {
        let mut kept = self.lock();
        kept.jpegs
            .retain(|key, entry| names.get(key) == Some(&entry.0));
        kept.names = names;
    }
}

#[cfg(test)]
mod tests;
