//! The hub's side of the FohMixer script (spec §2.3; S2 design note §3.10):
//! the frames the script sends, one reconnecting client per Live instance
//! ([`client`]) and the deduplicated subscription table ([`subs`]).

use std::time::Duration;

use serde_json::Value;

pub mod client;
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
    main_tick_age_ms > BUSY_TICK_AGE_MS || since_heartbeat > HEARTBEAT_OVERDUE
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
                main_tick_age_ms: 412.5
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
