use super::*;

#[test]
fn the_constants_hold() {
    assert_eq!(DEFAULT_PORT, 39111);
    assert_eq!(SAMPLE_RATE, 48_000);
    assert_eq!(FRAME, 960);
    assert_eq!(CHANNELS, 2);
    assert_eq!(
        FRAME as u64 * 1000 / u64::from(SAMPLE_RATE),
        20,
        "20 ms frames"
    );
}

#[test]
fn a_pages_messages_parse_strictly() {
    let start: AudioClientMsg =
        serde_json::from_str(r#"{"type":"listen_start","instance":"band","track":"Vox 1"}"#)
            .unwrap();
    assert_eq!(
        start,
        AudioClientMsg::ListenStart {
            instance: "band".into(),
            track: "Vox 1".into()
        }
    );
    let stop: AudioClientMsg = serde_json::from_str(r#"{"type":"listen_stop"}"#).unwrap();
    assert_eq!(stop, AudioClientMsg::ListenStop);
    assert!(
        serde_json::from_str::<AudioClientMsg>(
            r#"{"type":"listen_start","instance":"band","track":"x","more":1}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<AudioClientMsg>(r#"{"type":"listen"}"#).is_err());
}

#[test]
fn the_hubs_status_message_reads_as_the_page_expects() {
    let msg = AudioServerMsg::AudioStatus {
        status: AudioState::NoSource,
        instance: Some("band".into()),
        track: Some("Vox 1".into()),
    };
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        serde_json::json!({"type": "audio_status", "status": "no_source", "instance": "band", "track": "Vox 1"})
    );
    for (state, word) in [
        (AudioState::Listening, "listening"),
        (AudioState::Stopped, "stopped"),
        (AudioState::Taken, "taken"),
        (AudioState::Refused, "refused"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), word);
    }
}

#[test]
fn an_older_hubs_status_reads_with_the_defaults() {
    let status: ListenStatus = serde_json::from_str("{}").unwrap();
    assert_eq!(status, ListenStatus::default());
}
