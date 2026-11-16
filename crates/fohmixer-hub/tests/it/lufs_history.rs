//! The LUFS history's API end to end (#219): a hub started over day files
//! a previous run left serves them, bucketed, its days newest first, and an
//! MBC login reaches only its own Live set's points. Host-free.

use std::time::Duration;

use crate::support::{TestHub, http};
use fohmixer_hub::rules::lufs::history::{MinuteLine, PointMinute, encode};
use fohmixer_proto::lufs::{HistoryDays, HistorySeries};
use fohmixer_proto::roles::Role;

const VOX: &str = "live_set tracks 1";
const DAY: i64 = 86_400_000;

/// A minute line of `instance`'s `target`, every second at `tenths`.
fn line(m: i64, instance: &str, tenths: i16) -> String {
    MinuteLine {
        m,
        points: vec![PointMinute {
            instance: instance.into(),
            target: VOX.into(),
            values: encode(&[tenths; 60]),
        }],
    }
    .text()
}

/// A token of `role` for the hub.
fn token(hub: &TestHub, role: Role) -> String {
    let secret = std::fs::read_to_string(hub.dir.join("secrets").join("jwt_secret")).unwrap();
    fohmixer_hub::auth::issue_token(secret.trim(), fohmixer_hub::auth::now_secs(), role).unwrap()
}

#[tokio::test]
async fn the_history_of_earlier_runs_is_served_and_an_mbc_login_reaches_only_its_own_points() {
    let dir = tempfile::tempdir().unwrap();
    let lufs = dir.path().join("lufs");
    std::fs::create_dir_all(&lufs).unwrap();
    // Two days ago, 10:00 UTC: a band minute and an mbc minute.
    let now = fohmixer_hub::events::now_ms();
    let m = now - now.rem_euclid(DAY) - 2 * DAY + 36_000_000;
    let date = fohmixer_hub::day_files::date_of(m);
    let name = fohmixer_hub::day_files::file_name("lufs", date);
    std::fs::write(
        lufs.join(&name),
        format!(
            "{}\n{}\n{}\n",
            line(m, "band", -230),
            line(m + 60_000, "band", -700),
            line(m, "mbc", -140)
        ),
    )
    .unwrap();
    // Past the 14 days: deleted at the start.
    let old = fohmixer_hub::day_files::file_name(
        "lufs",
        fohmixer_hub::day_files::date_of(now - 20 * DAY),
    );
    std::fs::write(lufs.join(&old), line(now - 20 * DAY, "band", -230) + "\n").unwrap();

    let hub = TestHub::start(Vec::new(), dir.path()).await;
    let path = format!(
        "/api/lufs/history?instance=band&target=live_set%20tracks%201&from={m}&to={}",
        m + 180_000
    );
    let (code, body) = hub.get(&path).await;
    assert_eq!(code, 200, "{body}");
    let series: HistorySeries = serde_json::from_value(body).unwrap();
    assert_eq!(series.from, m);
    assert_eq!(series.step_ms, 1000);
    assert_eq!(series.values.len(), 180);
    assert_eq!(series.values[0], Some(-230));
    assert_eq!(series.values[60], Some(-700));
    assert_eq!(series.values[120], None);

    let (code, body) = hub
        .get("/api/lufs/days?instance=band&target=live_set%20tracks%201")
        .await;
    assert_eq!(code, 200, "{body}");
    let days: HistoryDays = serde_json::from_value(body).unwrap();
    assert_eq!(days.days.len(), 1);
    assert_eq!(days.days[0].first_ms, m);
    assert_eq!(days.days[0].last_ms, m + 119_000);

    // A window longer than 24 h is refused.
    let (code, body) = hub
        .get(&format!(
            "/api/lufs/history?instance=band&target=x&from=0&to={}",
            DAY + 1
        ))
        .await;
    assert_eq!(code, 400, "{body}");
    assert_eq!(body["code"], "BAD_WINDOW", "{body}");

    // An MBC login: its own Live set's points only.
    let mbc = token(&hub, Role::Mbc);
    let band_path = path.clone();
    let mbc_path = path.replace("instance=band", "instance=mbc");
    let (code, body) = http(hub.addr, "GET", &band_path, Some(&mbc), None).await;
    assert_eq!(code, 403, "{body}");
    assert_eq!(body["code"], "ROLE", "{body}");
    let (code, _) = http(
        hub.addr,
        "GET",
        "/api/lufs/days?instance=band&target=live_set%20tracks%201",
        Some(&mbc),
        None,
    )
    .await;
    assert_eq!(code, 403);
    let (code, body) = http(hub.addr, "GET", &mbc_path, Some(&mbc), None).await;
    assert_eq!(code, 200, "{body}");
    let series: HistorySeries = serde_json::from_value(body).unwrap();
    assert_eq!(series.values[0], Some(-140));
    // No token: 401.
    let (code, _) = http(hub.addr, "GET", &band_path, None, None).await;
    assert_eq!(code, 401);

    // The status counts what a full queue dropped (none here).
    let status = hub.status().await;
    assert_eq!(status.lufs.history.dropped, 0);
    // The sweep runs on the writer's thread: wait for it.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while lufs.join(&old).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the old day file stays"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(lufs.join(&name).exists());
    hub.stop().await;
}
