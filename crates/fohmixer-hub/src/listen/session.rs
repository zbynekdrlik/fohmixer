//! `/ws/audio` (#210): a page listens to one track. The upgrade takes the
//! page socket's token (`?token=`; any role, which then reaches only its
//! own Live instances); the Access check guards the internet path as for
//! every route. Text in: `AudioClientMsg` (`listen_start`, `listen_stop`);
//! text out: `AudioServerMsg::AudioStatus`; binary out: one Opus packet per
//! 20 ms frame. A socket more than [`MAX_BEHIND`] packets behind skips to
//! the newest (iemmixer's rule), so a slow link hears the track late by at
//! most that much. Nothing heard for [`NO_SOURCE_MS`] while listening is
//! told `no_source` once (and `listening` again when packets come back).
//! Another page starting another track takes the hub's one track: this page
//! sees the turn change and is told `taken` (and hears no packet of the new
//! turn, [`passes`]).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use fohmixer_proto::listen::{AudioClientMsg, AudioServerMsg, AudioState};
use fohmixer_proto::roles::Role;
use serde_json::json;
use tokio::sync::{broadcast, watch};
use tokio::time::{Instant, MissedTickBehavior};

use super::rack::Outcome;
use super::{Lease, Listen};
use crate::Hub;
use crate::ws::{Opener, SEND_TIMEOUT, WsQuery, opener};

/// A socket more than this many packets behind skips to the newest.
pub const MAX_BEHIND: usize = 4;
/// Silence that is told `no_source`.
pub const NO_SOURCE_MS: u64 = 2000;
/// The socket's ping (and the silence check's) period.
pub const PING_MS: u64 = 1000;

pub async fn ws_audio(
    ws: WebSocketUpgrade,
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<WsQuery>,
) -> Response {
    let Some(role) = hub.auth.role(query.token.as_deref()) else {
        return crate::auth::unauthorized();
    };
    let Some(listen) = hub.listen.clone() else {
        return crate::auth::error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "LISTEN_OFF",
            "Listen is off on this hub ([listen] port = 0)",
        );
    };
    let who = opener(peer, &headers);
    ws.on_upgrade(move |socket| session(socket, hub, listen, role, who))
        .into_response()
}

/// A status message.
pub fn status(state: AudioState, target: Option<(&str, &str)>) -> Message {
    let msg = AudioServerMsg::AudioStatus {
        status: state,
        instance: target.map(|(i, _)| i.to_string()),
        track: target.map(|(_, t)| t.to_string()),
    };
    Message::Text(serde_json::to_string(&msg).unwrap_or_default().into())
}

/// Whether a socket listening since `last` (its last packet, or its start)
/// is silent at `now` and not yet told so.
pub fn silent(last: Instant, now: Instant, told: bool) -> bool {
    !told && now.saturating_duration_since(last) >= Duration::from_millis(NO_SOURCE_MS)
}

/// Why a write to the page failed.
pub const WRITE_FAILED: &str = "a write to the page failed";
/// Why a page too slow to take a message is closed.
pub const TOO_SLOW: &str = "the page took over 5 s to take a message";

/// A write to the page within [`SEND_TIMEOUT`]: a page that takes nothing
/// (a locked iPad keeps its TCP window shut) must not hold the hub's one
/// track for good, so its socket ends and its lease goes.
pub async fn bounded<E>(
    write: impl std::future::Future<Output = Result<(), E>>,
) -> Result<(), &'static str> {
    match tokio::time::timeout(SEND_TIMEOUT, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(WRITE_FAILED),
        Err(_) => Err(TOO_SLOW),
    }
}

/// Whether a write went; one warn line for a page too slow to take it.
fn sent(who: &Opener, result: Result<(), &'static str>) -> bool {
    match result {
        Ok(()) => true,
        Err(why) => {
            if why == TOO_SLOW {
                tracing::warn!(peer = %who.peer, why, "listen: closing a page's audio socket");
            }
            false
        }
    }
}

/// Whether a packet sent in `sent` is for a page of `turn`, with `behind`
/// more waiting behind it: its own turn's, and at most [`MAX_BEHIND`]
/// behind (a slow socket skips to the newest).
pub fn passes(sent: i32, turn: i32, behind: usize) -> bool {
    sent == turn && behind <= MAX_BEHIND
}

/// The next packet of `turn` ([`passes`]).
async fn next_packet(
    rx: &mut broadcast::Receiver<(i32, Bytes)>,
    turn: i32,
) -> Result<Bytes, broadcast::error::RecvError> {
    loop {
        let (sent, packet) = rx.recv().await?;
        if passes(sent, turn, rx.len()) {
            return Ok(packet);
        }
    }
}

/// A socket's listening: its lease, its packets, the turn and the track.
struct Listening {
    lease: Lease,
    rx: broadcast::Receiver<(i32, Bytes)>,
    instance: String,
    track: String,
    last: Instant,
    told_silent: bool,
}

/// The longest instance or track name a page may ask for (a Live track's
/// name is far shorter; the records and lines carry it).
pub const MAX_NAME: usize = 256;

/// Whether a page's instance and track names are within [`MAX_NAME`]
/// characters and not empty.
pub fn names_ok(instance: &str, track: &str) -> bool {
    [instance, track]
        .iter()
        .all(|name| !name.is_empty() && name.chars().count() <= MAX_NAME)
}

/// What a page whose start switched its track's rack (`racked`: the
/// outcome, none when nothing was switched or its answer was lost) is told:
/// `no_source` at once when the track has no `Listen` rack to switch on
/// (#238; the page keeps the track, and a device that comes is heard), else
/// `listening` (a write Live failed is the rack task's warn record only).
pub fn told_of(racked: Option<&Outcome>) -> AudioState {
    match racked {
        Some(Outcome::Missing(_)) => AudioState::NoSource,
        _ => AudioState::Listening,
    }
}

/// What a page's start gets: listening, or the state it is told.
fn start(
    hub: &Hub,
    listen: &Arc<Listen>,
    role: Role,
    who: (u64, &str),
    instance: &str,
    track: &str,
) -> Result<Listening, AudioState> {
    if !names_ok(instance, track) || !role.reaches(instance) {
        return Err(AudioState::Refused);
    }
    let set = hub.set_name(instance).ok_or(AudioState::NoSource)?;
    let rx = listen.subscribe();
    let lease = listen.start(instance, &set, track, who.0, who.1);
    Ok(Listening {
        lease,
        rx,
        instance: instance.to_string(),
        track: track.to_string(),
        last: Instant::now(),
        told_silent: false,
    })
}

async fn session(mut socket: WebSocket, hub: Hub, listen: Arc<Listen>, role: Role, who: Opener) {
    let client = hub.next_client();
    tracing::info!(client, peer = %who.peer, source = who.source, "listen: a page's audio socket opened");
    let mut listening: Option<Listening> = None;
    // The turn now: a turn not this page's own means its track was taken.
    let mut turns: watch::Receiver<i32> = listen.turns();
    let mut ping = tokio::time::interval(Duration::from_millis(PING_MS));
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let has = listening.is_some();
        tokio::select! {
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                // A new start or a stop first lets go of the old track.
                let out = match serde_json::from_str::<AudioClientMsg>(&text) {
                    Ok(AudioClientMsg::ListenStart { instance, track }) => {
                        listening = None;
                        match start(&hub, &listen, role, (client, &who.peer), &instance, &track) {
                            Ok(mut l) => {
                                // The track's rack is switched on before the
                                // page is told (#238); each LOM call is
                                // bounded by the Live client's own timeout.
                                let racked = match l.lease.take_rack() {
                                    Some(outcome) => outcome.await.ok(),
                                    None => None,
                                };
                                let state = told_of(racked.as_ref());
                                // No rack: told now, the silence's word said.
                                l.told_silent = state == AudioState::NoSource;
                                // The silence counts from the switch-on's
                                // end, not from before its wait: the device
                                // only comes up then.
                                l.last = Instant::now();
                                listen.events.record("listen", json!({"what": "page", "on": true,
                                    "instance": instance, "track": track, "peer": who.peer}));
                                listening = Some(l);
                                status(state, Some((&instance, &track)))
                            }
                            Err(state) => status(state, Some((&instance, &track))),
                        }
                    }
                    Ok(AudioClientMsg::ListenStop) => {
                        if listening.take().is_some() {
                            listen.events.record("listen", json!({"what": "page", "on": false, "peer": who.peer}));
                        }
                        status(AudioState::Stopped, None)
                    }
                    Err(_) => status(AudioState::Refused, None),
                };
                if !sent(&who, bounded(socket.send(out)).await) {
                    break;
                }
            }
            taken = turns.changed(), if has => {
                let turn = taken.map(|()| *turns.borrow_and_update());
                // The newest start took the track: this page stops.
                let Some(l) = listening.as_ref() else { continue };
                if turn.is_ok_and(|turn| turn == l.lease.turn()) {
                    continue;
                }
                let told = status(AudioState::Taken, Some((&l.instance, &l.track)));
                listening = None;
                if !sent(&who, bounded(socket.send(told)).await) {
                    break;
                }
            }
            got = async {
                match listening.as_mut() {
                    Some(l) => next_packet(&mut l.rx, l.lease.turn()).await,
                    None => std::future::pending().await,
                }
            }, if has => {
                let Some(l) = listening.as_mut() else { continue };
                match got {
                    Ok(packet) => {
                        l.last = Instant::now();
                        if std::mem::take(&mut l.told_silent) {
                            let again = status(AudioState::Listening, Some((&l.instance, &l.track)));
                            if !sent(&who, bounded(socket.send(again)).await) {
                                break;
                            }
                        }
                        if !sent(&who, bounded(socket.send(Message::Binary(packet))).await) {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = ping.tick() => {
                if let Some(l) = listening.as_mut()
                    && silent(l.last, Instant::now(), l.told_silent)
                {
                    l.told_silent = true;
                    let told = status(AudioState::NoSource, Some((&l.instance, &l.track)));
                    if !sent(&who, bounded(socket.send(told)).await) {
                        break;
                    }
                }
                if !sent(&who, bounded(socket.send(Message::Ping(Bytes::new()))).await) {
                    break;
                }
            }
        }
    }
    tracing::info!(peer = %who.peer, "listen: a page's audio socket closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_told_once_after_two_seconds() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        assert!(!silent(t0, t0 + ms(1999), false));
        assert!(silent(t0, t0 + ms(2000), false));
        assert!(!silent(t0, t0 + ms(5000), true));
        assert!(!silent(t0 + ms(10), t0, false), "a clock that went back");
        assert_eq!((MAX_BEHIND, NO_SOURCE_MS, PING_MS), (4, 2000, 1000));
    }

    #[test]
    fn a_status_names_its_track() {
        let Message::Text(text) = status(AudioState::Taken, Some(("band", "Vox 1"))) else {
            panic!("text");
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            json!({"type": "audio_status", "status": "taken", "instance": "band", "track": "Vox 1"})
        );
        let Message::Text(text) = status(AudioState::Stopped, None) else {
            panic!("text");
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["instance"], serde_json::Value::Null);
    }

    #[tokio::test(start_paused = true)]
    async fn a_write_the_page_never_takes_ends_after_five_seconds() {
        let t0 = tokio::time::Instant::now();
        let stuck = bounded(std::future::pending::<Result<(), ()>>()).await;
        assert_eq!(stuck, Err(TOO_SLOW));
        assert_eq!(t0.elapsed(), SEND_TIMEOUT);
        assert_eq!(SEND_TIMEOUT, Duration::from_secs(5));
        assert_eq!(bounded(async { Err::<(), _>(()) }).await, Err(WRITE_FAILED));
        assert_eq!(bounded(async { Ok::<(), ()>(()) }).await, Ok(()));
    }

    #[test]
    fn a_start_with_no_rack_is_told_no_source_at_once() {
        let missing = Outcome::Missing("no Listen rack".into());
        assert_eq!(told_of(Some(&missing)), AudioState::NoSource);
        assert_eq!(told_of(Some(&Outcome::Done)), AudioState::Listening);
        let failed = Outcome::Failed("offline".into());
        assert_eq!(told_of(Some(&failed)), AudioState::Listening);
        assert_eq!(told_of(None), AudioState::Listening);
    }

    #[test]
    fn a_name_is_not_empty_and_at_most_256_characters() {
        let at = "ž".repeat(MAX_NAME);
        assert!(names_ok("band", &at));
        assert!(!names_ok("band", &format!("{at}x")));
        assert!(!names_ok("", "Vox 1"));
        assert!(!names_ok("band", ""));
        assert!(!names_ok(&format!("{at}x"), "Vox 1"));
        assert_eq!(MAX_NAME, 256);
    }

    #[tokio::test]
    async fn a_slow_socket_skips_to_the_newest_packet() {
        let (tx, mut rx) = broadcast::channel(64);
        for k in 0..10u8 {
            tx.send((3, Bytes::from(vec![k]))).unwrap();
        }
        // Ten waiting: the first that leaves at most four behind it.
        assert_eq!(next_packet(&mut rx, 3).await.unwrap()[0], 5);
        assert_eq!(next_packet(&mut rx, 3).await.unwrap()[0], 6);
        for k in 7..10u8 {
            assert_eq!(next_packet(&mut rx, 3).await.unwrap()[0], k);
        }
    }

    #[tokio::test]
    async fn a_page_hears_only_its_own_turns_packets() {
        let (tx, mut rx) = broadcast::channel(64);
        tx.send((2, Bytes::from_static(b"old"))).unwrap();
        tx.send((3, Bytes::from_static(b"new"))).unwrap();
        assert_eq!(
            next_packet(&mut rx, 3).await.unwrap(),
            Bytes::from_static(b"new")
        );
        assert!(passes(3, 3, MAX_BEHIND));
        assert!(!passes(3, 3, MAX_BEHIND + 1));
        assert!(!passes(2, 3, 0));
        assert!(!passes(4, 3, 0));
    }
}
