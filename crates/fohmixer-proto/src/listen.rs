//! Listen (#210): a track's sound, before its fader, in the browser.
//!
//! A silent Max for Live device, `FohMixer Listen`, sits in a parallel rack
//! chain of each listenable track (`live-device/FohMixer-Listen/`). It tells
//! the hub where it is once a second ([`wire`]'s announce, UDP), and while
//! the hub selects it ([`wire`]'s select, UDP) it streams the track's
//! samples to the hub over TCP as Jitter matrices ([`jitnet`]). The hub
//! cuts them into 20 ms frames at 48 kHz ([`frames`]), encodes Opus and
//! sends one packet per frame on `/ws/audio` ([`AudioClientMsg`],
//! [`AudioServerMsg`]). One track at a time on a hub.
//!
//! Pure: no I/O.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod frames;
pub mod jitnet;
pub mod wire;

/// The hub's default port: TCP for the devices' audio and UDP for their
/// announces (two protocols, one number), fixed in the device's patch
/// (`scripts/check_listen_device.py` keeps them equal).
pub const DEFAULT_PORT: u16 = 39111;
/// The UDP port every Listen device listens on for the hub's selects.
pub const SELECT_PORT: u16 = 39112;
/// The one sample rate the hub takes (Opus's own).
pub const SAMPLE_RATE: u32 = 48_000;
/// The samples per channel of one 20 ms frame at [`SAMPLE_RATE`].
pub const FRAME: usize = 960;
/// The channels of a frame (a mono device is doubled).
pub const CHANNELS: usize = 2;

/// A page's message on `/ws/audio` (text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioClientMsg {
    /// Listen to the track named `track` of the Live instance `instance`.
    ListenStart { instance: String, track: String },
    /// Stop listening.
    ListenStop,
}

/// What a listening socket is told (text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    /// Packets come (or will, once the device sends).
    Listening,
    /// Listening stopped (the page asked).
    Stopped,
    /// The track's sound does not come: no device, two devices, the
    /// instance offline, or nothing heard for a while.
    NoSource,
    /// Another track is heard on this hub (one at a time).
    Busy,
    /// The role reaches no such instance, or the message did not parse.
    Refused,
}

/// The hub's text message on `/ws/audio`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioServerMsg {
    /// The socket's state, with the track it is about (none for `stopped`
    /// and `refused`).
    AudioStatus {
        status: AudioState,
        #[serde(default)]
        instance: Option<String>,
        #[serde(default)]
        track: Option<String>,
    },
}

/// `/api/status` `listen`: the listener's addresses, the devices heard, the
/// track heard now and the counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListenStatus {
    /// The audio (TCP) and announce (UDP) address bound, or none.
    #[serde(default)]
    pub addr: Option<String>,
    /// Why it could not bind.
    #[serde(default)]
    pub error: Option<String>,
    /// The devices heard within the last few seconds.
    #[serde(default)]
    pub devices: Vec<ListenDeviceStatus>,
    /// The track listened to now.
    #[serde(default)]
    pub target: Option<ListenTargetStatus>,
    /// The devices' audio connections open now.
    #[serde(default)]
    pub connections: usize,
    /// The 20 ms frames cut from the selected device's audio.
    #[serde(default)]
    pub frames: u64,
    /// The Opus packets sent to the pages' sockets' channel.
    #[serde(default)]
    pub packets: u64,
    /// The selects sent to the devices.
    #[serde(default)]
    pub selects: u64,
    /// Everything refused, by reason (datagrams, chunks, connections).
    #[serde(default)]
    pub refused: BTreeMap<String, u64>,
}

/// One device heard (its last announce).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenDeviceStatus {
    pub device: i32,
    pub version: String,
    pub track: String,
    pub set: String,
    pub sample_rate: i32,
    pub sending: bool,
    /// Since its last announce.
    pub age_ms: u64,
}

/// The track listened to now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenTargetStatus {
    pub instance: String,
    pub set: String,
    pub track: String,
    /// The sockets listening to it.
    pub listeners: usize,
    /// The device selected for it; none: none or two found.
    #[serde(default)]
    pub device: Option<i32>,
    /// Why no device is selected (`no device`, `two devices`).
    #[serde(default)]
    pub problem: Option<String>,
}

#[cfg(test)]
mod tests;
