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
//!   turned on is closed again in Live. With its page's picture area (PR G)
//!   the taken editor gets that aspect before the open answers
//!   (`Plugwin::resize`): the first frame is the new picture, and the page
//!   never sees the editor's own shape first. The answer comes back as
//!   `RouterMsg::EqOpened`, with how the resize settled (an `eq` record
//!   `resize`).
//! - **A changed area** (PR G, `eq_area`): its client's open editor is
//!   resized in a task of its own; the result comes back as
//!   `RouterMsg::EqResized` (a `resize` record), and the state's picture
//!   size, which the points are clamped to, follows. A picture the worker
//!   grabbed at another size is a `sized` record. The page reads the
//!   picture's size from the frames, so no `eq` message tells it.
//! - **The close sequence:** the worker's guard (any contact ended, the
//!   inert spot tapped), [`GUARD_WAIT`], then `is_editor_open = false`, then
//!   the window released. A guard that could not tap leaves the editor open
//!   in Live (a value text field may be open: Pro-Q 4.02 crashes Live when
//!   its editor closes with one). The device turned off is never another
//!   one's: it is turned off through the `$ref` its open read (the same
//!   object wherever it moved since). A ref whose device was deleted
//!   (`StaleRef`: Live closed its editor with it) leaves Live alone with
//!   nothing open. Only when the script does not know the ref (its registry
//!   starts over with each connection) or none is held does the close check
//!   (`eq::close`) decide: it re-reads the held path first and, when the
//!   editor moved, looks for the one open Pro-Q 4 of the set; none open is
//!   nothing to close; when it is not certain, Live is left alone. A ref
//!   turn-off that got no answer (offline, a timeout) or failed otherwise
//!   leaves Live alone too: the ref may still be good. An open whose window
//!   was not found turns its editor off the same way. The answer comes back
//!   as `RouterMsg::EqClosed` with what the close left in Live
//!   (`eq::close::Shut`): its holder hears "left open in Live" only when the
//!   editor may still be open.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use fohmixer_proto::client::ServerMsg;
use fohmixer_proto::eq::{Area, EqItem, EqState, Touch, frame, reason};
use fohmixer_proto::layout::Binding;
use serde_json::{Value, json};

use super::{Router, RouterMsg, write_failure};
use crate::eq::close::{self, Check, CloseCheck, RefOff, Shut};
use crate::eq::open;
use crate::eq::walk::{Found, MAX_READS, Step, Walk};
use crate::eq::{Act, Device, EditorKey, Eqs, GUARD_WAIT, Pictures, record};
use crate::events::EventLog;
use crate::live::client::{LiveError, LiveHandle};
use crate::live::subs::ClientId;
use crate::plugwin::{Plugwin, PlugwinEvent, Rate, Resized};

/// The Live property that opens and closes a plug-in's editor.
pub const EDITOR_OPEN: &str = close::EDITOR_OPEN;
/// Why a list or an open found no instance of that name (a reason a page
/// reads).
pub const UNKNOWN_INSTANCE: &str = reason::UNKNOWN_INSTANCE;
/// Why a walk did not end (a reason a page reads).
pub const TOO_DEEP: &str = reason::TOO_DEEP;

/// The Pro-Q 4 screen's part of the router.
pub(super) struct EqIo {
    state: Eqs,
    plugwin: Plugwin,
    pictures: Arc<Pictures>,
    /// Each open editor's device `$ref` (by session, with its instance), as
    /// its open read it: its close turns the editor off through it. A ref
    /// holds for one script connection: an instance's are forgotten when it
    /// connects again.
    refs: BTreeMap<u32, (String, Value)>,
}

/// Reads the Pro-Q 4 instances of the track at `target` (a walk of its
/// devices, at most [`MAX_READS`] reads). A call that got no answer fails
/// with the protocol's reason ([`live_reason`]), as an open's does.
async fn walk(live: LiveHandle, target: String) -> Result<Vec<Found>, String> {
    let (mut walk, mut commands) = Walk::start(&target);
    for _ in 0..MAX_READS {
        let slots = live.call(commands).await.map_err(|e| live_reason(&e))?;
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
    set_editor_of(live, Value::from(path), open).await
}

/// The command that opens (`open`) or closes the editor of the device
/// `target` names (a path, or a `$ref`).
fn set_command(target: Value, open: bool) -> Vec<Value> {
    vec![json!({
        "target": target,
        "name": "set_prop",
        "args": {"prop": EDITOR_OPEN, "value": open},
    })]
}

/// Why an open's call through the script got no answer, as its page reads
/// it (the protocol's reasons): Live offline, or no answer in time (its
/// thread busy); a refused request keeps its own words.
pub fn live_reason(e: &LiveError) -> String {
    match e {
        LiveError::Offline => reason::OFFLINE.to_string(),
        LiveError::Timeout => reason::NO_ANSWER.to_string(),
        LiveError::Refused(_) => e.to_string(),
    }
}

/// Why an open's write through the script failed, as its page reads it:
/// no answer ([`live_reason`]), or the script's own answer.
fn open_failure(outcome: &Result<Vec<Value>, LiveError>) -> Option<String> {
    match outcome {
        Err(e) => Some(live_reason(e)),
        Ok(_) => write_failure(outcome),
    }
}

/// Opens (`open`) or closes the editor of the device `target` names (a path,
/// or a `$ref`).
async fn set_editor_of(live: &LiveHandle, target: Value, open: bool) -> Result<(), String> {
    let outcome = live.call(set_command(target, open)).await;
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
            settled @ (Check::Close { .. } | Check::NoneOpen | Check::Leave(_)) => return settled,
        };
    }
    close::settled(step)
}

/// Turns the editor of `key` (`session`) off in Live, never another
/// device's: through its `reference` (the device its open read, wherever it
/// moved since). A ref whose device was deleted leaves Live alone with
/// nothing open (Live closed its editor with it; the held path names
/// another device now, if any). Only without a ref, or when the script does
/// not know it (another script connection), does the close check decide
/// which one, or none; a turn-off through the ref that got no answer or
/// failed otherwise leaves Live alone (the ref may still be good). What it
/// left in Live is logged; an editor the check found moved is an `eq`
/// record `moved` with the path it was found at (`to`).
async fn turn_off(
    live: &LiveHandle,
    key: &EditorKey,
    session: u32,
    reference: Option<&Value>,
    events: &EventLog,
) -> Shut {
    if let Some(target) = reference {
        let outcome = live.call(set_command(target.clone(), false)).await;
        match close::ref_off(outcome.as_deref().map_err(ToString::to_string)) {
            RefOff::Done => {
                tracing::info!(session, instance = %key.instance, path = %key.path, "a Pro-Q 4 editor is turned off through its ref");
                return Shut::Off;
            }
            RefOff::Deleted(why) => {
                tracing::info!(session, instance = %key.instance, why = %why, "a Pro-Q 4 editor's device was deleted: Live closed its editor, nothing is turned off");
                return Shut::NoneOpen(close::deleted(&why));
            }
            RefOff::Unknown(why) => {
                tracing::info!(session, instance = %key.instance, why = %why, "a Pro-Q 4 editor's ref is unknown to the script: its held path is checked");
            }
            RefOff::Failed(why) => {
                tracing::warn!(session, instance = %key.instance, path = %key.path, why = %why, "a Pro-Q 4 editor could not be turned off through its ref: it is left open in Live");
                return Shut::LeftOpen(close::ref_failed(&why));
            }
        }
    }
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
            match set_editor(live, &path, false).await {
                Ok(()) => Shut::Off,
                Err(why) => Shut::LeftOpen(why),
            }
        }
        // The walk sees only where it looks (racks to its depth): the editor
        // may sit elsewhere, still open, so the holder hears "left open"
        // (the safe side) and the window is handed back, not taken as closed.
        Check::NoneOpen => {
            tracing::warn!(session, instance = %key.instance, path = %key.path, "a Pro-Q 4 editor moved and no open Pro-Q 4 was found where the hub looks: nothing is turned off, it may still be open");
            Shut::LeftOpen(close::NONE_OPEN.to_string())
        }
        Check::Leave(why) => {
            tracing::warn!(session, instance = %key.instance, path = %key.path, why = %why, "a Pro-Q 4 editor is left open in Live");
            Shut::LeftOpen(why)
        }
        Check::Read(_) | Check::Walk(_) => Shut::LeftOpen(close::UNFINISHED.to_string()),
    }
}

/// What an open sequence gave: the picture's size, the device's `$ref`,
/// and how the editor's resize to its page's area settled (PR G; none:
/// nothing was asked).
pub type Opened = ((u32, u32), Value, Option<Resized>);

/// The open sequence: the device read (still the Pro-Q 4 the list named
/// `name`, its editor closed in Live), the windows listed, the editor
/// opened in Live, the new window taken, and with a page's `area` the
/// editor given its aspect (PR G); its picture's size (the resized one),
/// the device's `$ref` and the resize. When no window could be taken, the
/// editor (which this open turned on: the read found it closed) is turned
/// off again through its ref, never another device's.
async fn open_editor(
    live: Option<LiveHandle>,
    plugwin: Plugwin,
    key: EditorKey,
    listed: Option<Device>,
    session: u32,
    area: Option<Area>,
    events: EventLog,
) -> Result<Opened, String> {
    let live = live.ok_or(UNKNOWN_INSTANCE)?;
    let listed = listed.ok_or(reason::UNKNOWN)?;
    let slots = live
        .call(open::read(&key.path))
        .await
        .map_err(|e| live_reason(&e))?;
    let reference = match open::ready(&slots, &key.path, &listed) {
        Ok(reference) => reference,
        Err(why) => {
            tracing::info!(session, instance = %key.instance, path = %key.path, why, "a Pro-Q 4 open is refused: Live is left alone");
            return Err(why.to_string());
        }
    };
    let before = plugwin.list().await?;
    let set = live.call(set_command(reference.clone(), true)).await;
    if let Some(why) = open_failure(&set) {
        return Err(why);
    }
    let taken = plugwin.take(session, before).await;
    if taken.is_err()
        && let Shut::LeftOpen(why) = turn_off(&live, &key, session, Some(&reference), &events).await
    {
        tracing::warn!(session, why = %why, "an editor whose window was not found could not be closed again in Live");
    }
    let size = taken?;
    let resized = match area {
        Some(area) => plugwin.resize(session, area).await,
        None => None,
    };
    Ok((resized.map_or(size, |r| r.client), reference, resized))
}

/// The close sequence: the guard, its wait, the editor turned off in Live
/// (not when the guard failed; through its `reference` when there is one;
/// never another device's), the window released (Live closed the editor
/// unless it may still be open); what it left in Live.
async fn close_editor(
    live: Option<LiveHandle>,
    plugwin: Plugwin,
    key: EditorKey,
    session: u32,
    reference: Option<Value>,
    events: EventLog,
) -> Shut {
    let guarded = plugwin.guard(session).await;
    tokio::time::sleep(GUARD_WAIT).await;
    let shut = match (guarded, live) {
        (Ok(()), Some(live)) => turn_off(&live, &key, session, reference.as_ref(), &events).await,
        (Ok(()), None) => Shut::LeftOpen(UNKNOWN_INSTANCE.to_string()),
        (Err(why), _) => Shut::LeftOpen(format!("the guard failed, the editor stays open: {why}")),
    };
    plugwin.release(session, !shut.left_open()).await;
    shut
}

/// The `eq` record of a resize as it settled (PR G): the size asked (`w`,
/// `h`), the picture's size then, whether it landed, how long it took.
pub fn resize_fields(key: &EditorKey, session: u32, resized: &Resized) -> Value {
    let mut fields = record("resize", key, None, Some(session), None);
    fields["w"] = json!(resized.asked.0);
    fields["h"] = json!(resized.asked.1);
    fields["client_w"] = json!(resized.client.0);
    fields["client_h"] = json!(resized.client.1);
    fields["ok"] = json!(resized.ok);
    fields["ms"] = json!(resized.ms);
    fields
}

/// The `eq` record of a picture grabbed at another size than the worker
/// knew (PR G).
pub fn sized_fields(key: &EditorKey, session: u32, size: (u32, u32)) -> Value {
    let mut fields = record("sized", key, None, Some(session), None);
    fields["w"] = json!(size.0);
    fields["h"] = json!(size.1);
    fields
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
        "gap_ms": rate.gap_ms,
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
            refs: BTreeMap::new(),
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
        // The walk ran on this target (`eq_list` checked it).
        let target = binding.target().unwrap_or_default();
        let devices = found.iter().map(|f| Device {
            name: f.name.clone(),
            id: f.id.clone(),
        });
        io.state.listed(
            &binding.instance,
            &target,
            keys.iter().cloned().zip(devices),
        );
        // A card's picture whose path names another device now goes.
        io.pictures.named(io.state.named());
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

    /// `instance` connected (again): its lists, its cards' pictures and its
    /// editors' refs are forgotten (another set may be loaded, and the
    /// script's object registry starts over). On `Connected`, not on
    /// `Disconnected`: both changes show only from the new connection on,
    /// and a walk answered from the old connection after its
    /// `Disconnected` (its reads already done) would otherwise list paths
    /// of the old one again.
    pub(super) fn eq_connected(&mut self, instance: &str) {
        if let Some(io) = self.eq.as_mut() {
            io.state.forget(instance);
            io.pictures.named(io.state.named());
            io.refs.retain(|_, (of, _)| of.as_str() != instance);
        }
    }

    /// `client` opens the editor of the device at `path` on `instance`,
    /// its page's picture area with it (PR G).
    pub(super) fn eq_open(
        &mut self,
        client: ClientId,
        instance: &str,
        path: &str,
        area: Option<Area>,
    ) {
        let wall = crate::live::wall_ms().unwrap_or(0.0);
        let Some(io) = self.eq.as_mut() else {
            if let Some(outbox) = self.clients.get(&client) {
                let key = EditorKey::new(instance, path);
                outbox.reply(crate::eq::closed_msg(&key, reason::OFF, None));
            }
            return;
        };
        io.state.keep_area(client, area);
        let acts = io.state.open(client, &EditorKey::new(instance, path), wall);
        self.eq_acts(acts);
    }

    /// `client`'s picture area changed (PR G): its open editor follows.
    pub(super) fn eq_area(&mut self, client: ClientId, area: Area) {
        if let Some(io) = self.eq.as_mut() {
            let acts = io.state.area(client, area);
            self.eq_acts(acts);
        }
    }

    /// A resize of `key` (`session`) settled (PR G: the open's, or a
    /// changed area's): recorded, and the state's picture size follows (an
    /// editor still opening takes it from its open's answer).
    pub(super) fn eq_resized(&mut self, key: &EditorKey, session: u32, resized: Option<&Resized>) {
        let (Some(io), Some(resized)) = (self.eq.as_mut(), resized) else {
            return;
        };
        tracing::info!(session, instance = %key.instance, w = resized.asked.0, h = resized.asked.1, client_w = resized.client.0, client_h = resized.client.1, ok = resized.ok, ms = resized.ms, "a Pro-Q 4 editor's resize settled");
        io.state.sized(key, session, resized.client);
        self.io
            .events
            .record("eq", resize_fields(key, session, resized));
    }

    /// An open sequence ended: the device's `$ref` is kept for its close,
    /// only for the editor of that session and only when the instance did
    /// not connect again since the open began (`connection`: the script's
    /// registry starts over with each connection, and a pointer it
    /// registers anew can name another object). A stale answer keeps none.
    pub(super) fn eq_opened(
        &mut self,
        key: &EditorKey,
        session: u32,
        outcome: Result<(u32, u32), String>,
        reference: Option<Value>,
        connection: u32,
    ) {
        if let Some(io) = self.eq.as_mut() {
            if let Some(reference) = reference
                && io.state.key_of(session).as_ref() == Some(key)
                && io.state.connection(&key.instance) == connection
            {
                io.refs.insert(session, (key.instance.clone(), reference));
            }
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

    /// A close sequence ended: what it left in Live. A close that did not
    /// turn its editor off is recorded (`problem`: it may still be open,
    /// which its holder hears; `none_open`: nothing is open there any more,
    /// and its holder hears its own close).
    pub(super) fn eq_closed(&mut self, key: &EditorKey, session: u32, outcome: Shut) {
        let Some(io) = self.eq.as_mut() else {
            return;
        };
        io.refs.remove(&session);
        if let Some((what, why)) = outcome.note() {
            tracing::warn!(session, instance = %key.instance, path = %key.path, what, why = %why, "a Pro-Q 4 editor's close did not turn it off");
            self.io
                .events
                .record("eq", record(what, key, None, Some(session), Some(why)));
        }
        let acts = io.state.closed(key, session, outcome.left_open());
        self.eq_acts(acts);
    }

    /// The window worker's event.
    pub(super) fn eq_worker(&mut self, event: PlugwinEvent) {
        let Some(io) = self.eq.as_mut() else {
            return;
        };
        match event {
            PlugwinEvent::Lost { session } => {
                io.refs.remove(&session);
                let acts = io
                    .state
                    .key_of(session)
                    .map(|key| io.state.lost(&key, session))
                    .unwrap_or_default();
                self.eq_acts(acts);
            }
            PlugwinEvent::ContactEnded {
                session,
                contact,
                why,
            } => {
                io.state.contact_ended(session, contact);
                tracing::info!(session, contact, why = %why, "the window worker ended a Pro-Q 4 contact");
                self.io.events.record(
                    "eq",
                    json!({"what": "contact_ended", "session": session, "contact": contact, "why": why}),
                );
            }
            PlugwinEvent::Sized {
                session,
                width,
                height,
            } => {
                let Some(key) = io.state.key_of(session) else {
                    return;
                };
                io.state.sized(&key, session, (width, height));
                tracing::info!(
                    session,
                    width,
                    height,
                    "a Pro-Q 4 editor's picture was grabbed at another size"
                );
                self.io
                    .events
                    .record("eq", sized_fields(&key, session, (width, height)));
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
                    gap_ms = rate.gap_ms,
                    "a Pro-Q 4 editor's capture and touches over the last minute"
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
            Act::Open {
                key,
                session,
                connection,
                area,
            } => {
                let (live, plugwin, tx, events) = (
                    self.live.get(&key.instance).cloned(),
                    io.plugwin.clone(),
                    self.io.tx.clone(),
                    self.io.events.clone(),
                );
                let listed = io.state.device_of(&key).cloned();
                tokio::spawn(async move {
                    let opened =
                        open_editor(live, plugwin, key.clone(), listed, session, area, events)
                            .await;
                    let (outcome, reference, resized) = match opened {
                        Ok((size, reference, resized)) => (Ok(size), Some(reference), resized),
                        Err(why) => (Err(why), None, None),
                    };
                    let _ = tx.send(RouterMsg::EqOpened {
                        key,
                        session,
                        outcome,
                        reference,
                        connection,
                        resized,
                    });
                });
            }
            Act::Resize { key, session, area } => {
                let (plugwin, tx) = (io.plugwin.clone(), self.io.tx.clone());
                tokio::spawn(async move {
                    let resized = plugwin.resize(session, area).await;
                    let _ = tx.send(RouterMsg::EqResized {
                        key,
                        session,
                        resized,
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
                let reference = io
                    .refs
                    .get(&session)
                    .map(|(_, reference)| reference.clone());
                tokio::spawn(async move {
                    let outcome =
                        close_editor(live, plugwin, key.clone(), session, reference, events).await;
                    let _ = tx.send(RouterMsg::EqClosed {
                        key,
                        session,
                        outcome,
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
                // The card's picture is the device the open read (its
                // `$ref`): kept only while its path names that device.
                let device = io
                    .refs
                    .get(&session)
                    .and_then(|(_, reference)| open::ref_id(reference))
                    .map(str::to_string);
                io.plugwin.capture(
                    session,
                    Arc::new(move |session: u32, jpeg: Bytes| {
                        outbox.eq_frame(Bytes::from(frame(session, &jpeg)));
                        if let Some(device) = &device {
                            pictures.put(&key, device, jpeg);
                        }
                    }),
                );
            }
            Act::Touch {
                session,
                contact,
                phase,
                at,
            } => io.plugwin.touch(session, contact, phase, at),
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
