//! The hub's side of the FohMixer script (spec §2.3; S2 design note §3.10):
//! the frames the script sends, one reconnecting client per Live instance
//! ([`client`]), the deduplicated subscription table ([`subs`]) and the
//! layout's check for unresolved names ([`names`]).

use std::time::Duration;

use serde_json::Value;

pub mod client;
pub mod names;
pub mod subs;

/// An instance is busy when Live's main-thread tick is older than this.
pub const BUSY_TICK_AGE_MS: f64 = 150.0;
/// An instance is busy when no heartbeat came for this long (the script
/// sends one every 100 ms from its own thread).
pub const HEARTBEAT_OVERDUE: Duration = Duration::from_millis(300);
/// The first reconnect delay; it doubles up to [`RECONNECT_MAX`].
pub const RECONNECT_FIRST: Duration = Duration::from_millis(250);
/// The longest reconnect delay.
pub const RECONNECT_MAX: Duration = Duration::from_secs(2);

/// Whether an instance is busy (spec §2.4): its main-thread tick is older
/// than 150 ms, or its heartbeat is overdue.
pub fn is_busy(main_tick_age_ms: f64, since_heartbeat: Duration) -> bool {
    busy_reason(main_tick_age_ms, since_heartbeat).is_some()
}

/// Why an instance is busy (for the log), or `None` when it is not.
pub fn busy_reason(main_tick_age_ms: f64, since_heartbeat: Duration) -> Option<String> {
    if main_tick_age_ms > BUSY_TICK_AGE_MS {
        Some(format!(
            "Live's main thread last ticked {main_tick_age_ms} ms before the last heartbeat"
        ))
    } else if since_heartbeat > HEARTBEAT_OVERDUE {
        Some(format!(
            "no heartbeat for {} ms",
            since_heartbeat.as_millis()
        ))
    } else {
        None
    }
}

/// What a heartbeat that came after an overdue gap says about where it was
/// held (#9), or `None` for one in time. `gap_ms` is the script's heartbeat
/// thread's time since its previous heartbeat (long: the thread did not
/// run). `sent_ms` is the script's stamp of the heartbeat and `arrived_ms`
/// the hub's clock at its arrival, both ms since the Unix epoch on the one
/// PC: a long difference means it waited on the way (the script's sender or
/// the socket).
pub fn late_heartbeat(
    since_previous: Duration,
    gap_ms: Option<f64>,
    sent_ms: Option<f64>,
    arrived_ms: Option<f64>,
) -> Option<String> {
    if since_previous <= HEARTBEAT_OVERDUE {
        return None;
    }
    let known =
        |v: Option<f64>| v.map_or_else(|| "unknown".to_string(), |ms| format!("{ms:.0} ms"));
    let transit = sent_ms
        .zip(arrived_ms)
        .map(|(sent, arrived)| arrived - sent);
    Some(format!(
        "a heartbeat {} ms after the previous one: the script's thread made it {} after its previous one, it spent {} on the way",
        since_previous.as_millis(),
        known(gap_ms),
        known(transit)
    ))
}

/// Milliseconds since the Unix epoch now (the clock of the scripts' `ts`;
/// the hub and the scripts run on one PC).
pub fn wall_ms() -> Option<f64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64() * 1000.0)
}

/// The reconnect delays: 250 ms, doubling to 2 s, back to 250 ms after a
/// connection.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            next: RECONNECT_FIRST,
        }
    }
}

impl Backoff {
    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(RECONNECT_MAX);
        delay
    }

    /// A connection succeeded: start over.
    pub fn reset(&mut self) {
        self.next = RECONNECT_FIRST;
    }
}

/// What the script says about itself on each connection (`connect`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectInfo {
    pub instance: String,
    pub set_name: String,
    pub script_version: String,
    pub live_version: String,
}

/// One item of a `values` push: a listener key and its value (and display
/// string), or why it ended (`gone`).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveValue {
    pub key: String,
    pub value: Value,
    pub display: Option<String>,
    pub error: Option<String>,
}

/// One frame from the script.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Connect(ConnectInfo),
    Disconnect,
    Heartbeat {
        main_tick_age_ms: f64,
        /// The heartbeat thread's time since its previous heartbeat.
        gap_ms: Option<f64>,
        /// When the script set it (its `ts`, ms since the Unix epoch).
        sent_ms: Option<f64>,
    },
    Result {
        uuid: String,
        data: Vec<Value>,
    },
    Values(Vec<LiveValue>),
    /// A request the script refused as a whole (malformed envelope).
    Error {
        uuid: Option<String>,
        message: String,
    },
}

fn text_of(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_string()
}

/// One item of a `values` push.
fn value_item(item: &Value) -> Result<LiveValue, String> {
    let key = item
        .get("key")
        .and_then(Value::as_str)
        .ok_or("value item without key")?;
    Ok(LiveValue {
        key: key.to_string(),
        value: item.get("value").cloned().unwrap_or(Value::Null),
        display: item
            .get("display")
            .and_then(Value::as_str)
            .map(str::to_string),
        error: item
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Parses one text frame of the script.
pub fn parse_frame(text: &str) -> Result<Frame, String> {
    let message: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let event = message
        .get("event")
        .and_then(Value::as_str)
        .ok_or("no event")?;
    let data = message.get("data");
    match event {
        "connect" => Ok(Frame::Connect(ConnectInfo {
            instance: text_of(data.and_then(|d| d.get("instance"))),
            set_name: text_of(data.and_then(|d| d.get("set_name"))),
            script_version: text_of(data.and_then(|d| d.get("script_version"))),
            live_version: text_of(data.and_then(|d| d.get("live_version"))),
        })),
        "disconnect" => Ok(Frame::Disconnect),
        "heartbeat" => {
            let age = data
                .and_then(|d| d.get("main_tick_age_ms"))
                .and_then(Value::as_f64)
                .ok_or("heartbeat without main_tick_age_ms")?;
            Ok(Frame::Heartbeat {
                main_tick_age_ms: age,
                gap_ms: data.and_then(|d| d.get("gap_ms")).and_then(Value::as_f64),
                sent_ms: message.get("ts").and_then(Value::as_f64),
            })
        }
        "result" => {
            let uuid = message
                .get("uuid")
                .and_then(Value::as_str)
                .ok_or("result without uuid")?;
            let data = data
                .and_then(Value::as_array)
                .ok_or("result without data")?;
            Ok(Frame::Result {
                uuid: uuid.to_string(),
                data: data.clone(),
            })
        }
        "values" => {
            let items = data
                .and_then(Value::as_array)
                .ok_or("values without data")?;
            let items: Result<Vec<LiveValue>, String> = items.iter().map(value_item).collect();
            items.map(Frame::Values)
        }
        "error" => Ok(Frame::Error {
            uuid: message
                .get("uuid")
                .and_then(Value::as_str)
                .map(str::to_string),
            message: data.and_then(Value::as_str).unwrap_or("error").to_string(),
        }),
        other => Err(format!("unknown event {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn busy_is_a_late_tick_or_an_overdue_heartbeat() {
        let fresh = Duration::from_millis(100);
        assert!(!is_busy(10.0, fresh));
        assert!(!is_busy(150.0, fresh));
        assert!(is_busy(150.1, fresh));
        assert!(!is_busy(0.0, Duration::from_millis(300)));
        assert!(is_busy(0.0, Duration::from_millis(301)));
    }

    #[test]
    fn busy_says_why() {
        let fresh = Duration::from_millis(100);
        assert_eq!(busy_reason(150.0, Duration::from_millis(300)), None);
        assert_eq!(
            busy_reason(150.5, fresh).as_deref(),
            Some("Live's main thread last ticked 150.5 ms before the last heartbeat")
        );
        assert_eq!(
            busy_reason(12.0, Duration::from_millis(1250)).as_deref(),
            Some("no heartbeat for 1250 ms")
        );
        // A late tick is the reason even when the heartbeat is also late.
        assert!(
            busy_reason(400.0, Duration::from_millis(900))
                .unwrap()
                .starts_with("Live's main thread")
        );
    }

    #[test]
    fn a_late_heartbeat_says_where_it_was_held() {
        assert_eq!(
            late_heartbeat(
                Duration::from_millis(300),
                Some(100.0),
                Some(1000.0),
                Some(1001.0)
            ),
            None
        );
        assert_eq!(
            late_heartbeat(
                Duration::from_millis(301),
                Some(1290.4),
                Some(5000.0),
                Some(5002.0)
            )
            .as_deref(),
            Some(
                "a heartbeat 301 ms after the previous one: the script's thread made it 1290 ms after its previous one, it spent 2 ms on the way"
            )
        );
        assert_eq!(
            late_heartbeat(Duration::from_millis(1500), None, None, Some(1.0)).as_deref(),
            Some(
                "a heartbeat 1500 ms after the previous one: the script's thread made it unknown after its previous one, it spent unknown on the way"
            )
        );
        assert!(
            late_heartbeat(
                Duration::from_secs(1),
                Some(100.0),
                Some(9000.0),
                Some(10_250.0)
            )
            .unwrap()
            .ends_with("it spent 1250 ms on the way")
        );
    }

    #[test]
    fn wall_ms_is_the_unix_clock_in_milliseconds() {
        let now = wall_ms().unwrap();
        // Between late August 2026 and 2100: milliseconds, not seconds.
        assert!(
            now > 1_788_000_000_000.0 && now < 4_102_444_800_000.0,
            "{now}"
        );
        let later = wall_ms().unwrap();
        assert!(later >= now && later - now < 1000.0, "{now} {later}");
    }

    #[test]
    fn backoff_doubles_to_two_seconds_and_resets() {
        let mut b = Backoff::default();
        let delays: Vec<u64> = (0..6).map(|_| b.next_delay().as_millis() as u64).collect();
        assert_eq!(delays, vec![250, 500, 1000, 2000, 2000, 2000]);
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(250));
    }

    #[test]
    fn frames_parse() {
        assert_eq!(
            parse_frame(
                r#"{"event":"connect","data":{"instance":"band","set_name":"Test Site","script_version":"0.1.0","live_version":"12.2.5","proto":1},"ts":1}"#
            ),
            Ok(Frame::Connect(ConnectInfo {
                instance: "band".into(),
                set_name: "Test Site".into(),
                script_version: "0.1.0".into(),
                live_version: "12.2.5".into(),
            }))
        );
        assert_eq!(
            parse_frame(r#"{"event":"connect","data":null}"#),
            Ok(Frame::Connect(ConnectInfo::default()))
        );
        assert_eq!(
            parse_frame(r#"{"event":"disconnect","data":null,"ts":1}"#),
            Ok(Frame::Disconnect)
        );
        assert_eq!(
            parse_frame(
                r#"{"event":"heartbeat","data":{"main_tick_age_ms":412.5,"max_cmd_ms":0.2}}"#
            ),
            Ok(Frame::Heartbeat {
                main_tick_age_ms: 412.5,
                gap_ms: None,
                sent_ms: None,
            })
        );
        assert_eq!(
            parse_frame(
                r#"{"event":"heartbeat","data":{"main_tick_age_ms":31.0,"max_cmd_ms":0.2,"gap_ms":107.5},"ts":1790000000123}"#
            ),
            Ok(Frame::Heartbeat {
                main_tick_age_ms: 31.0,
                gap_ms: Some(107.5),
                sent_ms: Some(1_790_000_000_123.0),
            })
        );
        assert_eq!(
            parse_frame(r#"{"event":"result","uuid":"s1","data":[{"ok":true,"data":1}],"ts":2}"#),
            Ok(Frame::Result {
                uuid: "s1".into(),
                data: vec![json!({"ok": true, "data": 1})]
            })
        );
        assert_eq!(
            parse_frame(
                r#"{"event":"values","data":[{"key":"live_1.value","value":0.5,"display":"-6.0 dB"},{"key":"live_2.mute","value":null},{"key":"live_3.name","error":"gone"}]}"#
            ),
            Ok(Frame::Values(vec![
                LiveValue {
                    key: "live_1.value".into(),
                    value: json!(0.5),
                    display: Some("-6.0 dB".into()),
                    error: None
                },
                LiveValue {
                    key: "live_2.mute".into(),
                    value: Value::Null,
                    display: None,
                    error: None
                },
                LiveValue {
                    key: "live_3.name".into(),
                    value: Value::Null,
                    display: None,
                    error: Some("gone".into())
                },
            ]))
        );
        assert_eq!(
            parse_frame(
                r#"{"event":"error","uuid":"s9","data":"missing or invalid commands array"}"#
            ),
            Ok(Frame::Error {
                uuid: Some("s9".into()),
                message: "missing or invalid commands array".into()
            })
        );
        assert_eq!(
            parse_frame(r#"{"event":"error","data":null}"#),
            Ok(Frame::Error {
                uuid: None,
                message: "error".into()
            })
        );
    }

    #[test]
    fn broken_frames_are_errors() {
        for (text, error) in [
            ("nope", "not JSON"),
            (r#"{"data":1}"#, "no event"),
            (
                r#"{"event":"heartbeat","data":{}}"#,
                "heartbeat without main_tick_age_ms",
            ),
            (r#"{"event":"result","data":[]}"#, "result without uuid"),
            (
                r#"{"event":"result","uuid":"x","data":{}}"#,
                "result without data",
            ),
            (r#"{"event":"values","data":{}}"#, "values without data"),
            (
                r#"{"event":"values","data":[{"value":1}]}"#,
                "value item without key",
            ),
            (r#"{"event":"surprise"}"#, "unknown event \"surprise\""),
        ] {
            let got = parse_frame(text).unwrap_err();
            assert!(got.starts_with(error), "{text}: {got}");
        }
    }
}
