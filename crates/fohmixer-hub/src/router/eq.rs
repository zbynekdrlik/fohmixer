//! The router's half of the Pro-Q 4 screen (#71 PR E, F28, D17): it reads
//! a strip's Pro-Q 4 instances when a page asks (`eq_list`: a walk of the
//! track's devices through the script's `get_prop`, `eq::walk`), carries out
//! the state's acts (`crate::eq`: the opens, the closes, the captures, the
//! contacts, the clients' states, the locks to every client, the `eq`
//! records), runs the open and close sequences as tasks of their own, hears
//! the window worker (`crate::plugwin`: a window gone, a contact it ended,
//! the capture's rate once a minute) and hands every window back at the stop.
//!
//! - **The open sequence** (one at a time, the state queues the others):
//!   the device read first (`eq::open`: still the listed Pro-Q 4, its editor
//!   closed in Live, else refused `moved` or `open on the PC`), the editor
//!   windows listed, `is_editor_open = true` through the script, the new
//!   window taken by the worker; when no window comes, the editor this open
//!   turned on is closed again in Live. The answer comes back as
//!   `RouterMsg::EqOpened`.
//! - **The close sequence:** the worker's guard (any contact ended, the
//!   inert spot tapped), [`GUARD_WAIT`], then `is_editor_open = false`, then
//!   the window released. A guard that could not tap leaves the editor open
//!   in Live (a value text field may be open: Pro-Q 4.02 crashes Live when
//!   its editor closes with one). The device turned off is never another
//!   one's: the close check (`eq::close`) re-reads the held path first and,
//!   when the editor moved, looks for the one open Pro-Q 4 of the set; when
//!   that is not certain, Live is left alone. An open whose window was not
//!   found turns its editor off the same way. The answer comes back as
//!   `RouterMsg::EqClosed`.

use std::sync::Arc;

use bytes::Bytes;
use fohmixer_proto::client::ServerMsg;
use fohmixer_proto::eq::{EqItem, EqState, Touch, frame, reason};
use fohmixer_proto::layout::Binding;
use serde_json::{Value, json};

use super::{Router, RouterMsg, write_failure};
use crate::eq::close::{self, Check, CloseCheck};
use crate::eq::open;
use crate::eq::walk::{Found, MAX_READS, Step, Walk};
use crate::eq::{Act, EditorKey, Eqs, GUARD_WAIT, Pictures, record};
use crate::events::EventLog;
use crate::live::client::LiveHandle;
use crate::live::subs::ClientId;
use crate::plugwin::{Plugwin, PlugwinEvent, Rate};

/// The Live property that opens and closes a plug-in's editor.
pub const EDITOR_OPEN: &str = close::EDITOR_OPEN;
/// Why a list or an open found no instance of that name.
pub const UNKNOWN_INSTANCE: &str = "unknown instance";
/// Why a walk did not end.
pub const TOO_DEEP: &str = "the devices nest too deep";

/// The Pro-Q 4 screen's part of the router.
pub(super) struct EqIo {
    state: Eqs,
    plugwin: Plugwin,
    pictures: Arc<Pictures>,
}

/// Reads the Pro-Q 4 instances of the track at `target` (a walk of its
/// devices, at most [`MAX_READS`] reads).
async fn walk(live: LiveHandle, target: String) -> Result<Vec<Found>, String> {
    let (mut walk, mut commands) = Walk::start(&target);
    for _ in 0..MAX_READS {
        let slots = live.call(commands).await.map_err(|e| e.to_string())?;
        match walk.answer(&slots) {
            Step::Read(next) => commands = next,
            Step::Done(found) => return Ok(found),
            Step::Failed(why) => return Err(why),
        }
    }
    Err(TOO_DEEP.to_string())
}

/// Opens (`open`) or closes the editor of the device at `path`.
async fn set_editor(live: &LiveHandle, path: &str, open: bool) -> Result<(), String> {
    let outcome = live
        .call(vec![json!({
            "target": path,
            "name": "set_prop",
            "args": {"prop": EDITOR_OPEN, "value": open},
        })])
        .await;
    write_failure(&outcome).map_or(Ok(()), Err)
}

/// Carries a close check (`eq::close`) out: its reads through the script
/// and its walks, one after the other (at most [`close::STEPS`] steps);
/// the close it settles on, or why Live is left alone.
async fn check_close(live: &LiveHandle, held: &str) -> Check {
    let (mut check, mut step) = CloseCheck::start(held);
    for _ in 0..close::STEPS {
        step = match step {
            Check::Read(commands) => match live.call(commands).await {
                Ok(slots) => check.answer(&slots),
                Err(e) => return Check::Leave(close::unread(&e.to_string())),
            },
            Check::Walk(targets) => {
                let mut outcomes: Vec<Result<Vec<String>, String>> = Vec::new();
                for target in targets {
                    let found = walk(live.clone(), target).await;
                    outcomes.push(found.map(|found| found.into_iter().map(|f| f.path).collect()));
                }
                check.walked(outcomes)
            }
            settled @ (Check::Close { .. } | Check::Leave(_)) => return settled,
        };
    }
    close::settled(step)
}

/// Turns the editor of `key` (`session`) off in Live, never another
/// device's: the close check decides which one, or none. Its outcome is
/// logged; a moved editor is an `eq` record `moved` with the path it was
/// found at (`to`).
async fn turn_off(
    live: &LiveHandle,
    key: &EditorKey,
    session: u32,
    events: &EventLog,
) -> Result<(), String> {
    match check_close(live, &key.path).await {
        Check::Close { path, moved } => {
            if moved {
                tracing::warn!(session, instance = %key.instance, from = %key.path, to = %path, "a Pro-Q 4 editor moved: the one open Pro-Q 4 of the set is turned off");
                let mut fields = record("moved", key, None, Some(session), None);
                fields["to"] = json!(path.as_str());
                events.record("eq", fields);
            } else {
                tracing::info!(session, instance = %key.instance, path = %path, "a Pro-Q 4 editor is turned off at its path");
            }
            set_editor(live, &path, false).await
        }
        Check::Leave(why) => {
            tracing::warn!(session, instance = %key.instance, path = %key.path, why = %why, "a Pro-Q 4 editor is left open in Live");
            Err(why)
        }
        Check::Read(_) | Check::Walk(_) => Err(close::UNFINISHED.to_string()),
    }
}

/// The open sequence: the device read (still the Pro-Q 4 the list named
/// `name`, its editor closed in Live), the windows listed, the editor
/// opened in Live, the new window taken; its picture's size. When no window
/// could be taken, the editor (which this open turned on: the read found it
/// closed) is turned off again, never another device's.
async fn open_editor(
    live: Option<LiveHandle>,
    plugwin: Plugwin,
    key: EditorKey,
    name: Option<String>,
    session: u32,
    events: EventLog,
) -> Result<(u32, u32), String> {
    let live = live.ok_or(UNKNOWN_INSTANCE)?;
    let name = name.ok_or(reason::UNKNOWN)?;
    let slots = live
        .call(open::read(&key.path))
        .await
        .map_err(|e| e.to_string())?;
    if let Err(why) = open::ready(&slots, &key.path, &name) {
        tracing::info!(session, instance = %key.instance, path = %key.path, why, "a Pro-Q 4 open is refused: Live is left alone");
        return Err(why.to_string());
    }
    let before = plugwin.list().await?;
    set_editor(&live, &key.path, true).await?;
    let taken = plugwin.take(session, before).await;
    if taken.is_err()
        && let Err(why) = turn_off(&live, &key, session, &events).await
    {
        tracing::warn!(session, why = %why, "an editor whose window was not found could not be closed again in Live");
    }
    taken
}

/// The close sequence: the guard, its wait, the editor turned off in Live
/// (not when the guard failed; never another device's), the window
/// released; what went wrong, if anything.
async fn close_editor(
    live: Option<LiveHandle>,
    plugwin: Plugwin,
    key: EditorKey,
    session: u32,
    events: EventLog,
) -> Option<String> {
    let guarded = plugwin.guard(session).await;
    tokio::time::sleep(GUARD_WAIT).await;
    let closed = match (guarded, live) {
        (Ok(()), Some(live)) => turn_off(&live, &key, session, &events).await,
        (Ok(()), None) => Err(UNKNOWN_INSTANCE.to_string()),
        (Err(why), _) => Err(format!("the guard failed, the editor stays open: {why}")),
    };
    plugwin.release(session, closed.is_ok()).await;
    closed.err()
}

/// The `eq` record of the capture's minute.
pub fn rate_fields(session: u32, rate: &Rate) -> Value {
    json!({
        "what": "rate",
        "session": session,
        "grabs": rate.grabs,
        "sent": rate.sent,
        "failed": rate.failed,
        "grab_ms": rate.grab_ms,
        "encode_ms": rate.encode_ms,
        "bytes": rate.bytes,
        "width": rate.width,
        "height": rate.height,
    })
}

impl Router {
    /// The router of a hub with the Pro-Q 4 screen (#71 PR E): the window
    /// worker and the editors' last pictures.
    #[must_use]
    pub fn with_eq(mut self, plugwin: Plugwin, pictures: Arc<Pictures>) -> Self {
        self.eq = Some(EqIo {
            state: Eqs::default(),
            plugwin,
            pictures,
        });
        self
    }

    /// A new client: the locks (none without the screen).
    pub(super) fn eq_attach(&self, client: ClientId, outbox: &crate::outbox::Outbox) {
        if let Some(io) = &self.eq {
            outbox.reply(ServerMsg::EqLocks {
                items: io.state.locks_for(client),
            });
        }
    }

    /// An `eq_list` answer to `client`.
    fn eq_list_reply(
        &self,
        client: ClientId,
        binding: Binding,
        items: Vec<EqItem>,
        error: Option<String>,
    ) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(ServerMsg::EqList {
                binding,
                items,
                error,
            });
        }
    }

    /// `client` asks for the Pro-Q 4 instances of a strip's track: a walk,
    /// whose answer comes back as [`RouterMsg::EqListed`].
    pub(super) fn eq_list(&self, client: ClientId, binding: Binding) {
        if self.eq.is_none() {
            self.eq_list_reply(client, binding, Vec::new(), Some(reason::OFF.to_string()));
            return;
        }
        let target = match binding.target() {
            Ok(target) => target,
            Err(e) => {
                self.eq_list_reply(client, binding, Vec::new(), Some(e.to_string()));
                return;
            }
        };
        let Some(live) = self.live.get(&binding.instance).cloned() else {
            self.eq_list_reply(
                client,
                binding,
                Vec::new(),
                Some(UNKNOWN_INSTANCE.to_string()),
            );
            return;
        };
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = walk(live, target).await;
            let _ = tx.send(RouterMsg::EqListed {
                client,
                binding,
                outcome,
            });
        });
    }

    /// A walk ended: what it found may be opened, and goes to `client`.
    pub(super) fn eq_listed(
        &mut self,
        client: ClientId,
        binding: Binding,
        outcome: Result<Vec<Found>, String>,
    ) {
        let Some(io) = self.eq.as_mut() else {
            return;
        };
        let found = match outcome {
            Ok(found) => found,
            Err(why) => {
                tracing::info!(client, instance = %binding.instance, why = %why, "a strip's Pro-Q 4 instances could not be read");
                self.eq_list_reply(client, binding, Vec::new(), Some(why));
                return;
            }
        };
        let keys: Vec<EditorKey> = found
            .iter()
            .map(|f| EditorKey::new(&binding.instance, &f.path))
            .collect();
        io.state.listed(
            keys.iter()
                .cloned()
                .zip(found.iter().map(|f| f.name.clone())),
        );
        let items = found
            .into_iter()
            .zip(&keys)
            .map(|(f, key)| EqItem {
                path: f.path,
                place: f.place,
                name: f.name,
                picture: io.pictures.has(key),
            })
            .collect();
        self.eq_list_reply(client, binding, items, None);
    }

    /// `client` opens the editor of the device at `path` on `instance`.
    pub(super) fn eq_open(&mut self, client: ClientId, instance: &str, path: &str) {
        let wall = crate::live::wall_ms().unwrap_or(0.0);
        let Some(io) = self.eq.as_mut() else {
            if let Some(outbox) = self.clients.get(&client) {
                let key = EditorKey::new(instance, path);
                outbox.reply(crate::eq::closed_msg(&key, reason::OFF, None));
            }
            return;
        };
        let acts = io.state.open(client, &EditorKey::new(instance, path), wall);
        self.eq_acts(acts);
    }

    /// An open sequence ended.
    pub(super) fn eq_opened(
        &mut self,
        key: &EditorKey,
        session: u32,
        outcome: Result<(u32, u32), String>,
    ) {
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.opened(key, session, outcome);
            self.eq_acts(acts);
        }
    }

    /// A finger of `client` on its editor.
    pub(super) fn eq_input(&mut self, client: ClientId, touch: Touch, x: f64, y: f64) {
        let now = self.now_ms();
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.input(client, touch, x, y, now);
            self.eq_acts(acts);
        }
    }

    /// `client` left its editor.
    pub(super) fn eq_close(&mut self, client: ClientId) {
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.close(client, reason::EXIT, true);
            self.eq_acts(acts);
        }
    }

    /// A close sequence ended (`problem`: what went wrong).
    pub(super) fn eq_closed(&mut self, key: &EditorKey, session: u32, problem: Option<String>) {
        let Some(io) = self.eq.as_mut() else {
            return;
        };
        if let Some(problem) = &problem {
            tracing::warn!(session, instance = %key.instance, path = %key.path, problem = %problem, "a Pro-Q 4 editor's close went wrong");
            self.io.events.record(
                "eq",
                record("problem", key, None, Some(session), Some(problem)),
            );
        }
        let acts = io.state.closed(key, session);
        self.eq_acts(acts);
    }

    /// The window worker's event.
    pub(super) fn eq_worker(&mut self, event: PlugwinEvent) {
        let Some(io) = self.eq.as_mut() else {
            return;
        };
        match event {
            PlugwinEvent::Lost { session } => {
                let acts = io
                    .state
                    .key_of(session)
                    .map(|key| io.state.lost(&key, session))
                    .unwrap_or_default();
                self.eq_acts(acts);
            }
            PlugwinEvent::ContactEnded { session, why } => {
                io.state.contact_ended(session);
                tracing::info!(session, why = %why, "the window worker ended a Pro-Q 4 contact");
                self.io.events.record(
                    "eq",
                    json!({"what": "contact_ended", "session": session, "why": why}),
                );
            }
            PlugwinEvent::Rate { session, rate } => {
                tracing::info!(
                    session,
                    grabs = rate.grabs,
                    sent = rate.sent,
                    failed = rate.failed,
                    grab_ms = rate.grab_ms,
                    encode_ms = rate.encode_ms,
                    bytes = rate.bytes,
                    width = rate.width,
                    height = rate.height,
                    "a Pro-Q 4 editor's capture over the last minute"
                );
                self.io.events.record("eq", rate_fields(session, &rate));
            }
        }
    }

    /// Something came from `client`.
    pub(super) fn eq_heard(&mut self, client: ClientId) {
        let now = self.now_ms();
        if let Some(io) = self.eq.as_mut() {
            io.state.heard(client, now);
        }
    }

    /// The contacts' clock.
    pub(super) fn eq_tick(&mut self) {
        let now = self.now_ms();
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.tick(now);
            self.eq_acts(acts);
        }
    }

    /// `client`'s socket closed: its editor closes.
    pub(super) fn eq_detach(&mut self, client: ClientId) {
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.detach(client);
            self.eq_acts(acts);
        }
    }

    /// The hub stops: the worker ends any contact and hands every window
    /// back (its z-order as it was); the editors stay open in Live.
    pub(super) fn eq_stop(&mut self) {
        let Some(io) = &self.eq else {
            return;
        };
        for (key, session) in io.state.taken() {
            tracing::info!(session, instance = %key.instance, path = %key.path, "the hub stops: a Pro-Q 4 editor's window is handed back, the editor stays open in Live");
        }
        io.plugwin.stop();
    }

    /// Carries out the state's acts.
    fn eq_acts(&self, acts: Vec<Act>) {
        for act in acts {
            self.eq_act(act);
        }
    }

    fn eq_act(&self, act: Act) {
        let Some(io) = &self.eq else {
            return;
        };
        match act {
            Act::Open { key, session } => {
                let (live, plugwin, tx, events) = (
                    self.live.get(&key.instance).cloned(),
                    io.plugwin.clone(),
                    self.io.tx.clone(),
                    self.io.events.clone(),
                );
                let name = io.state.name_of(&key).map(str::to_string);
                tokio::spawn(async move {
                    let outcome =
                        open_editor(live, plugwin, key.clone(), name, session, events).await;
                    let _ = tx.send(RouterMsg::EqOpened {
                        key,
                        session,
                        outcome,
                    });
                });
            }
            Act::Close { key, session, .. } => {
                let (live, plugwin, tx, events) = (
                    self.live.get(&key.instance).cloned(),
                    io.plugwin.clone(),
                    self.io.tx.clone(),
                    self.io.events.clone(),
                );
                tokio::spawn(async move {
                    let problem = close_editor(live, plugwin, key.clone(), session, events).await;
                    let _ = tx.send(RouterMsg::EqClosed {
                        key,
                        session,
                        problem,
                    });
                });
            }
            Act::Capture {
                key,
                session,
                holder,
            } => {
                let Some(outbox) = self.clients.get(&holder).cloned() else {
                    return;
                };
                let pictures = Arc::clone(&io.pictures);
                io.plugwin.capture(
                    session,
                    Arc::new(move |session: u32, jpeg: Bytes| {
                        outbox.eq_frame(Bytes::from(frame(session, &jpeg)));
                        pictures.put(&key, jpeg);
                    }),
                );
            }
            Act::Touch { session, phase, at } => io.plugwin.touch(session, phase, at),
            Act::Tell { client, msg } => {
                if let Some(outbox) = self.clients.get(&client) {
                    if matches!(
                        msg,
                        ServerMsg::Eq {
                            state: EqState::Closed,
                            ..
                        }
                    ) {
                        outbox.forget_eq();
                    }
                    outbox.reply(msg);
                }
            }
            Act::Locks => {
                for (client, outbox) in &self.clients {
                    outbox.reply(ServerMsg::EqLocks {
                        items: io.state.locks_for(*client),
                    });
                }
            }
            Act::Record(fields) => {
                tracing::info!(
                    what = %fields["what"],
                    instance = %fields["instance"],
                    path = %fields["path"],
                    client = %fields["client"],
                    session = %fields["session"],
                    why = %fields["why"],
                    "Pro-Q 4 editor"
                );
                self.io.events.record("eq", fields);
            }
        }
    }
}

#[cfg(test)]
mod tests;
