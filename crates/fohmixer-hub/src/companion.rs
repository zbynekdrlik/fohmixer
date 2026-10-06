//! The hub's side of Bitfocus Companion's Satellite API (#52; spec
//! `docs/superpowers/specs/2026-10-06-streamdeck-tab-design.md` §4): the hub
//! registers as one Stream Deck surface over TCP and forwards the pages'
//! presses. This file is the pure protocol, tested natively: the line parser
//! ([`parse_line`], [`classify`]), the bounded line read ([`read_outcome`]),
//! the version gate ([`api_ok`]), the lines the hub writes ([`add_device`],
//! [`key_press`], [`remove_device`], [`ping`], [`pong`]), the session's
//! deadlines ([`overdue`]) and the first-in-first-out matching of the
//! `KEY-PRESS` answers ([`Fifo`]).
//!
//! Companion 5.0.7 as probed (#52 plan, "Companion 5.0.7 facts"): every
//! line it writes ends with a space before its `\n`; a quoted value is
//! `"…"` without escapes (a data URL holds `=`, `+` and `/`); `KEY-STATE`
//! carries `KEY`, `LOCATION`, `PRESSED`, `TYPE`, `BITMAP` (a
//! `data:image/webp;base64,` URL with `BITMAP_FORMAT=webp`), `COLOR` and
//! `TEXTCOLOR`; it answers `ADD-DEVICE`, `KEY-PRESS` and `REMOVE-DEVICE` with
//! `OK` / `ERROR … MESSAGE=` in order and an unknown command with a bare
//! `ERROR MESSAGE=` (no press answer); it closes a socket that sent nothing
//! for 5 s; a release of a key it does not hold is a no-op answered `OK`; a
//! key held when its surface goes away stays held until a release comes from
//! any surface.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::config::CompanionCfg;
use crate::live::subs::ClientId;

/// The surface's serial: Companion keys its settings for the surface (start
/// page, group) on it, so it never changes.
pub const SERIAL: &str = "fohmixer";
/// The name Companion shows for the surface.
pub const PRODUCT: &str = "fohmixer";
/// The longest line the hub reads (a 144 px webp key is a few KB; Companion
/// itself allows 2 MB): a longer one ends the link and bounds memory.
pub const MAX_LINE: usize = 256 * 1024;
/// How often the hub pings Companion (5.0.7 closes a socket silent for 5 s).
pub const PING_EVERY: Duration = Duration::from_secs(2);
/// No inbound line for longer than this: the link is lost.
pub const LINK_SILENCE: Duration = Duration::from_secs(5);
/// How long `BEGIN` and the `ADD-DEVICE` answer may each take.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the graceful stop's `REMOVE-DEVICE` may take.
pub const STOP_BOUND: Duration = Duration::from_millis(500);
/// The oldest Satellite API minor version of major 1 this hub speaks (1.12:
/// `BITMAP_FORMAT`).
pub const MIN_API_MINOR: u32 = 12;
/// A press the link could not carry.
pub const OFFLINE: &str = "offline";

/// One line of the Satellite API: its command and its parameters (a bare
/// word such as `OK` has no value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub cmd: String,
    pub params: BTreeMap<String, Option<String>>,
}

impl Line {
    /// The value of parameter `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(|v| v.as_deref())
    }

    /// Whether `name` is there (a bare word: `OK`, `ERROR`).
    pub fn has(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }
}

/// The parameter at the start of `rest` (no leading space): its name, its
/// value (none for a bare word), and the text after it. A quoted value runs
/// to the next `"` followed by a space, else to the line's end.
fn next_param(rest: &str) -> (String, Option<String>, &str) {
    let name_end = rest.find([' ', '=']).unwrap_or(rest.len());
    let (name, after) = rest.split_at(name_end);
    let Some(value) = after.strip_prefix('=') else {
        return (name.to_string(), None, after);
    };
    match value.strip_prefix('"') {
        Some(quoted) => match quoted.split_once("\" ") {
            Some((inner, after)) => (name.to_string(), Some(inner.to_string()), after),
            None => {
                let inner = quoted.strip_suffix('"').unwrap_or(quoted);
                (name.to_string(), Some(inner.to_string()), "")
            }
        },
        None => {
            let end = value.find(' ').unwrap_or(value.len());
            let (bare, after) = value.split_at(end);
            (name.to_string(), Some(bare.to_string()), after)
        }
    }
}

/// Parses a line (its `\r`, `\n` and Companion's trailing space ignored);
/// `None` for a blank one.
pub fn parse_line(text: &str) -> Option<Line> {
    let text = text.trim();
    let (cmd, mut rest) = text.split_once(' ').unwrap_or((text, ""));
    if cmd.is_empty() {
        return None;
    }
    let mut params = BTreeMap::new();
    // Each pass takes at least one byte of `rest`, so its length bounds the
    // loop (no hand-stepped index: .claude/rules/hub-rust.md).
    for _ in 0..=rest.len() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (name, value, after) = next_param(rest);
        params.insert(name, value);
        rest = after;
    }
    Some(Line {
        cmd: cmd.to_string(),
        params,
    })
}

/// What one read of at most `MAX_LINE + 1` bytes up to a newline gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// A whole line, its line end removed.
    Line(String),
    /// More than [`MAX_LINE`] bytes without a newline.
    TooLong,
    /// The connection ended (mid-line too).
    Closed,
}

/// The outcome of a bounded `read_until` that read `read` bytes into `buf`.
pub fn read_outcome(read: usize, buf: &[u8]) -> Read {
    if read == 0 {
        return Read::Closed;
    }
    match buf.strip_suffix(b"\n") {
        Some(line) => {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            Read::Line(String::from_utf8_lossy(line).into_owned())
        }
        None if buf.len() > MAX_LINE => Read::TooLong,
        None => Read::Closed,
    }
}

/// Whether Companion's `ApiVersion` is one this hub speaks: major 1, minor
/// [`MIN_API_MINOR`] or later (`BITMAP_FORMAT`).
pub fn api_ok(version: &str) -> bool {
    let mut parts = version.split(['.', '-']).map(str::parse::<u32>);
    matches!(
        (parts.next(), parts.next()),
        (Some(Ok(1)), Some(Ok(minor))) if minor >= MIN_API_MINOR
    )
}

/// A Satellite boolean: `1` or `true` (any case).
pub fn flag(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// An answer's outcome: `OK`, else Companion's `MESSAGE` (`error` without
/// one).
pub fn reply(line: &Line) -> Result<(), String> {
    if line.has("OK") {
        Ok(())
    } else {
        Err(line.get("MESSAGE").unwrap_or("error").to_string())
    }
}

/// A key's new state as Companion sent it; a field it left out keeps the
/// cache's old value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyUpdate {
    pub key: u32,
    /// A `data:` URL (a raw rgb bitmap is none: the page cannot show it).
    pub img: Option<String>,
    pub color: Option<String>,
    pub pressed: Option<bool>,
}

fn key_update(line: &Line) -> Option<KeyUpdate> {
    let key = line.get("KEY")?.parse().ok()?;
    Some(KeyUpdate {
        key,
        img: line
            .get("BITMAP")
            .filter(|b| b.starts_with("data:"))
            .map(str::to_string),
        color: line.get("COLOR").map(str::to_string),
        pressed: line.get("PRESSED").map(flag),
    })
}

/// What a line from Companion means to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    Begin {
        companion: String,
        api: String,
    },
    /// `CAPS` and its flags, as written (logged; the hub needs none).
    Caps(String),
    /// The `ADD-DEVICE` answer.
    Added(Result<(), String>),
    /// A `KEY-PRESS` answer.
    Pressed(Result<(), String>),
    KeyState(KeyUpdate),
    KeysClear,
    /// Companion's ping: its payload, answered `PONG <payload>`.
    Ping(String),
    Pong,
    /// Anything else, by its command (counted, logged once per session).
    Other(String),
}

impl Inbound {
    /// The command of the line it came from.
    pub fn name(&self) -> String {
        match self {
            Self::Begin { .. } => "BEGIN".into(),
            Self::Caps(_) => "CAPS".into(),
            Self::Added(_) => "ADD-DEVICE".into(),
            Self::Pressed(_) => "KEY-PRESS".into(),
            Self::KeyState(_) => "KEY-STATE".into(),
            Self::KeysClear => "KEYS-CLEAR".into(),
            Self::Ping(_) => "PING".into(),
            Self::Pong => "PONG".into(),
            Self::Other(cmd) => cmd.clone(),
        }
    }
}

/// Classifies one line from Companion.
pub fn classify(text: &str) -> Inbound {
    if let Some(payload) = text.strip_prefix("PING ") {
        return Inbound::Ping(payload.trim().to_string());
    }
    let Some(line) = parse_line(text) else {
        return Inbound::Other(String::new());
    };
    match line.cmd.as_str() {
        "BEGIN" => Inbound::Begin {
            companion: line.get("CompanionVersion").unwrap_or_default().to_string(),
            api: line.get("ApiVersion").unwrap_or_default().to_string(),
        },
        "CAPS" => Inbound::Caps(
            text.trim()
                .strip_prefix("CAPS")
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        "ADD-DEVICE" => Inbound::Added(reply(&line)),
        "KEY-PRESS" => Inbound::Pressed(reply(&line)),
        "KEY-STATE" => {
            key_update(&line).map_or_else(|| Inbound::Other(line.cmd.clone()), Inbound::KeyState)
        }
        "KEYS-CLEAR" => Inbound::KeysClear,
        "PONG" => Inbound::Pong,
        _ => Inbound::Other(line.cmd),
    }
}

/// The device id of connection attempt `n` (a fresh one per connection:
/// Companion refuses an id another, possibly dead, socket still holds).
pub fn device_id(n: u64) -> String {
    format!("fohmixer-{n}")
}

/// The `ADD-DEVICE` line of the surface: `columns` × `rows` keys, webp
/// images of `bitmap_px`, colours as hex, no text, no brightness.
pub fn add_device(device: &str, deck: &CompanionCfg) -> String {
    format!(
        "ADD-DEVICE DEVICEID=\"{device}\" SERIAL=\"{SERIAL}\" PRODUCT_NAME=\"{PRODUCT}\" \
         KEYS_TOTAL={} KEYS_PER_ROW={} BITMAPS={} BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n",
        deck.keys(),
        deck.columns,
        deck.bitmap_px
    )
}

/// A press (`down`) or release of `key`.
pub fn key_press(device: &str, key: u32, down: bool) -> String {
    format!(
        "KEY-PRESS DEVICEID=\"{device}\" KEY={key} PRESSED={}\n",
        u8::from(down)
    )
}

/// The graceful stop's line.
pub fn remove_device(device: &str) -> String {
    format!("REMOVE-DEVICE DEVICEID=\"{device}\"\n")
}

/// The hub's ping number `n` (its own counter).
pub fn ping(n: u64) -> String {
    format!("PING {n}\n")
}

/// The answer to Companion's `PING <payload>`.
pub fn pong(payload: &str) -> String {
    format!("PONG {payload}\n")
}

/// Whether the link is lost: nothing came in for longer than
/// [`LINK_SILENCE`].
pub fn link_silent(since_heard: Duration) -> bool {
    since_heard > LINK_SILENCE
}

/// Where a session stands: waiting for `BEGIN`, for the `ADD-DEVICE`
/// answer, or registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Begin,
    Adding,
    Up,
}

/// Why a session ends now, if it does: no `BEGIN` or no `ADD-DEVICE` answer
/// within [`ADD_TIMEOUT`] of its phase's start; once registered, nothing
/// from Companion for longer than [`LINK_SILENCE`].
pub fn overdue(phase: Phase, in_phase: Duration, since_heard: Duration) -> Option<&'static str> {
    match phase {
        Phase::Begin => (in_phase > ADD_TIMEOUT).then_some("no BEGIN from Companion within 3 s"),
        Phase::Adding => (in_phase > ADD_TIMEOUT).then_some("no ADD-DEVICE answer within 3 s"),
        Phase::Up => link_silent(since_heard).then_some("nothing from Companion for 5 s"),
    }
}

/// Counts a line of `cmd` the hub ignores; whether it is the session's first
/// of that command (the one logged).
pub fn first_seen(counts: &mut BTreeMap<String, u64>, cmd: &str) -> bool {
    let n = counts.entry(cmd.to_string()).or_insert(0);
    *n += 1;
    *n == 1
}

/// Milliseconds from `earlier` to `later` (0 when it is not later).
pub fn ms_between(earlier: Instant, later: Instant) -> f64 {
    later.saturating_duration_since(earlier).as_secs_f64() * 1000.0
}

/// A press for Companion: a page's (its client and `seq`) or one the hub
/// makes itself (`from` none: a release on a detach, a silence, a reconnect,
/// the stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub key: u32,
    pub down: bool,
    pub from: Option<(ClientId, u64)>,
}

/// Companion's answer to a press, or `offline` for one no link carried.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub press: Press,
    pub ok: bool,
    pub error: Option<String>,
    /// From the hub's write to Companion's answer (none: not answered).
    pub rtt_ms: Option<f64>,
}

impl Answer {
    /// `press` was not carried: no session, or the link went first.
    pub fn offline(press: Press) -> Self {
        Self {
            press,
            ok: false,
            error: Some(OFFLINE.to_string()),
            rtt_ms: None,
        }
    }
}

/// The presses written to Companion, waiting for their answers (Companion
/// answers in order).
#[derive(Debug, Default)]
pub struct Fifo {
    waiting: VecDeque<(Press, Instant)>,
}

impl Fifo {
    /// `press` went out at `at`.
    pub fn sent(&mut self, press: Press, at: Instant) {
        self.waiting.push_back((press, at));
    }

    /// Companion answered the oldest press at `at`; none when nothing waits
    /// (an answer to nothing the hub sent).
    pub fn answer(&mut self, result: Result<(), String>, at: Instant) -> Option<Answer> {
        let (press, sent) = self.waiting.pop_front()?;
        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(message) => (false, Some(message)),
        };
        Some(Answer {
            press,
            ok,
            error,
            rtt_ms: Some(ms_between(sent, at)),
        })
    }

    /// The link went: every waiting press, oldest first, answered offline.
    pub fn offline(&mut self) -> Vec<Answer> {
        self.waiting
            .drain(..)
            .map(|(press, _)| Answer::offline(press))
            .collect()
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// What the Companion task tells the router, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum CompanionEvent {
    /// `ADD-DEVICE OK`: the surface is registered. `attempts`: the attempts
    /// of the outage that ended (1 when the first one worked); `down_ms`: how
    /// long the link was down (none the first time).
    Up {
        companion: String,
        api: String,
        attempts: u64,
        down_ms: Option<f64>,
    },
    /// A registered session ended.
    Down { error: String },
    /// The first failed attempt of an outage (`refused`: Companion refused
    /// it, by its API version or `ADD-DEVICE ERROR`).
    Failed {
        error: String,
        refused: bool,
        companion: Option<String>,
        api: Option<String>,
        attempts: u64,
    },
    /// A key's new state.
    Key(KeyUpdate),
    /// `KEYS-CLEAR`.
    Clear,
    /// A press's answer (offline too).
    Answered(Answer),
}

#[cfg(test)]
mod tests;
