//! Auth end to end (S3 plan, Task 6): the API and the WebSocket need a
//! token; `POST /api/auth` with the provisioned engineer PIN gives one; a
//! wrong PIN is answered 401 (the spacing after three is in `auth.rs`'s
//! tests). No Live host: this runs on Windows too.

mod support;

use fohmixer_proto::client::{AuthResponse, ServerMsg};
use serde_json::json;
use support::{Client, TestHub, http, runtime, serial};

#[test]
fn the_api_needs_a_token_and_the_engineer_pin_gives_one() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        fohmixer_hub::provision::store_pin(dir.path(), "2468").unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        for path in ["/api/status", "/api/layout"] {
            let (code, body) = http(hub.addr, "GET", path, None, None).await;
            assert_eq!(code, 401, "{path}: {body}");
        }
        let (code, _) = http(hub.addr, "GET", "/api/version", None, None).await;
        assert_eq!(code, 200, "the version stays public");
        // Two wrong PINs (within the free failures; the spacing after the
        // third is proven by the auth module's own tests, where hashing is
        // fast enough to time it).
        for _ in 0..2 {
            let (code, body) = http(
                hub.addr,
                "POST",
                "/api/auth",
                None,
                Some(&json!({"pin": "9753"})),
            )
            .await;
            assert_eq!(code, 401);
            assert_eq!(body["code"], "INVALID_PIN");
        }
        let (code, body) = http(
            hub.addr,
            "POST",
            "/api/auth",
            None,
            Some(&json!({"nope": 1})),
        )
        .await;
        assert_eq!(code, 422, "a body without a PIN: {body}");
        let (code, body) = http(
            hub.addr,
            "POST",
            "/api/auth",
            None,
            Some(&json!({"pin": "2468"})),
        )
        .await;
        assert_eq!(code, 200, "{body}");
        let login: AuthResponse = serde_json::from_value(body).unwrap();
        assert_eq!(login.expires_in, 7 * 24 * 60 * 60);
        // The token opens the WebSocket and the API.
        let client = Client::connect(&format!(
            "ws://{}/ws?token={}&proto=2",
            hub.addr, login.token
        ))
        .await;
        assert!(matches!(client.hello, ServerMsg::Hello { .. }));
        let (code, _) = http(hub.addr, "GET", "/api/status", Some(&login.token), None).await;
        assert_eq!(code, 200);
        hub.stop().await;
    });
}
