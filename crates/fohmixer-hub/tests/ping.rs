//! The client's liveness probe (the UI's watchdog, #8): `ping` is answered
//! `pong` through the client's outbox, as often as it is sent. No Live
//! host: this runs on Windows too.

mod support;

use std::time::Duration;

use fohmixer_proto::client::{ClientMsg, ServerMsg};
use support::{TestHub, runtime, serial};

#[test]
fn every_ping_is_answered_pong() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let mut client = hub.client().await;
        for _ in 0..3 {
            client.send(&ClientMsg::Ping).await;
            client
                .wait(Duration::from_secs(3), |m| {
                    matches!(m, ServerMsg::Pong).then_some(())
                })
                .await;
        }
        hub.stop().await;
    });
}
