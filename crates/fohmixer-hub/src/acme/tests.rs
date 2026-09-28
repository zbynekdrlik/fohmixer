//! Tests of `acme.rs`: the real ACME client (instant-acme) against the
//! local ACME double (`acme/double.rs`) and the Cloudflare DNS API double
//! (`cloudflare::double`), the keeper, and its pure decisions.

use std::sync::Arc;
use std::time::Duration;

use instant_acme::RetryPolicy;

use super::double::{self as ca_double, SharedCa};
use super::*;
use crate::cloudflare::double::{self as cf, Shared as CfApi};
use crate::config::AcmeCfg;
use crate::http_client::HttpClient;
use crate::https::Https;
use crate::remote::{RemoteState, Stored};
use crate::tls::test_certs::{TestCa, handshake};
use crate::tls::{CertInfo, CertStore};

const NAME: &str = "foh.example.org";
const DAY: i64 = 86_400;

fn cf_secret() -> String {
    "c".repeat(24)
}

/// The two doubles and an ACME client of NAME pointed at them, with the
/// Cloudflare token set in `dir`.
async fn setup(dir: &std::path::Path, set_token: bool) -> (Acme, CfApi, SharedCa) {
    let (cf_base, cf_api) = cf::start(&cf_secret(), &[("example.org", "zone-1")]).await;
    let (directory, ca) = ca_double::start(Arc::clone(&cf_api)).await;
    if set_token {
        crate::cf_token::run(dir, format!("{}\n", cf_secret()).as_bytes()).unwrap();
    }
    let cfg = AcmeCfg {
        email: Some("owner@example.org".into()),
        directory,
        propagation_s: 0,
    };
    let mut acme = Acme::new(NAME, &cfg, dir, HttpClient::new().unwrap());
    assert_eq!(acme.cloudflare_api, crate::cloudflare::API);
    acme.cloudflare_api = cf_base;
    acme.retry = RetryPolicy::new()
        .initial_delay(Duration::from_millis(10))
        .timeout(Duration::from_secs(5));
    (acme, cf_api, ca)
}

fn count(ca: &SharedCa, request: &str) -> usize {
    ca.lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| r.as_str() == request)
        .count()
}

#[tokio::test]
async fn a_certificate_comes_by_dns01_and_the_record_goes_again() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, cf_api, ca) = setup(dir.path(), true).await;
    let pem = acme.issue().await.unwrap();
    assert_eq!(pem.info.names, vec![NAME.to_string()]);
    assert!(pem.info.not_after > crate::auth::now_secs() as i64);
    assert!(pem.chain.contains("BEGIN CERTIFICATE"));
    assert!(pem.key.contains("PRIVATE KEY"));
    // The challenge record was written, checked by the CA, and removed.
    {
        let cf_state = cf_api.lock().unwrap();
        assert!(cf_state.records.is_empty(), "no record stays");
        assert!(
            cf_state
                .calls
                .iter()
                .any(|c| c == "POST /zones/zone-1/dns_records"),
            "{:?}",
            cf_state.calls
        );
        assert!(
            cf_state
                .calls
                .iter()
                .any(|c| c.starts_with("DELETE /zones/zone-1/dns_records/rec"))
        );
    }
    let accounts = ca.lock().unwrap().new_accounts.clone();
    assert_eq!(accounts.len(), 1);
    assert_eq!(
        accounts[0]["contact"],
        serde_json::json!(["mailto:owner@example.org"])
    );
    assert!(acme.account_path().is_file(), "the account is stored");
    // A second certificate reuses the stored account, and the CA reuses
    // the account's valid authorization: no new record, no challenge.
    acme.issue().await.unwrap();
    assert_eq!(count(&ca, "POST /new-account"), 1);
    assert_eq!(count(&ca, "POST /new-order"), 2);
    let posts = |cf_api: &CfApi| {
        cf_api
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| *c == "POST /zones/zone-1/dns_records")
            .count()
    };
    assert_eq!(posts(&cf_api), 1);
    let challenges = ca
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|r| r.starts_with("POST /chall/"))
        .count();
    assert_eq!(challenges, 1);
}

#[tokio::test]
async fn a_record_a_crashed_attempt_left_is_removed_first() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, cf_api, _ca) = setup(dir.path(), true).await;
    let fqdn = format!("_acme-challenge.{NAME}");
    let ours = crate::cloudflare::TXT_COMMENT.to_string();
    cf_api
        .lock()
        .unwrap()
        .records
        .insert("stale".into(), (fqdn.clone(), "old-value".into(), ours));
    // A record another client (another ACME client) made is left alone.
    cf_api.lock().unwrap().records.insert(
        "foreign".into(),
        (fqdn.clone(), "theirs".into(), "certbot".into()),
    );
    acme.issue().await.unwrap();
    let records = cf_api.lock().unwrap().records.clone();
    assert_eq!(records.keys().collect::<Vec<_>>(), vec!["foreign"]);
    let calls = cf_api.lock().unwrap().calls.clone();
    assert!(calls.contains(&"DELETE /zones/zone-1/dns_records/stale".to_string()));
    assert!(!calls.contains(&"DELETE /zones/zone-1/dns_records/foreign".to_string()));
}

#[tokio::test]
async fn a_refused_challenge_is_an_error_and_the_record_still_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, cf_api, ca) = setup(dir.path(), true).await;
    ca.lock().unwrap().refuse_challenges = true;
    let error = acme.issue().await.unwrap_err();
    assert!(error.starts_with("ACME challenge: "), "{error}");
    assert!(
        error.contains("no TXT record with the key authorization"),
        "{error}"
    );
    assert!(
        cf_api.lock().unwrap().records.is_empty(),
        "removed after the failure too"
    );
    assert!(!CertStore::new(dir.path()).cert_path().exists());
}

#[tokio::test]
async fn without_a_token_nothing_is_asked_of_anyone() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, cf_api, ca) = setup(dir.path(), false).await;
    assert_eq!(acme.issue().await.unwrap_err(), NO_TOKEN);
    assert!(cf_api.lock().unwrap().calls.is_empty());
    assert!(ca.lock().unwrap().requests.is_empty());
    assert!(NO_TOKEN.contains("fohmixer-hub cloudflare set-token"));
}

#[tokio::test]
async fn cloudflare_errors_stop_the_attempt_before_the_ca() {
    let dir = tempfile::tempdir().unwrap();
    let (mut acme, cf_api, ca) = setup(dir.path(), true).await;
    acme.name = "foh.elsewhere.org".into();
    let error = acme.issue().await.unwrap_err();
    assert!(
        error.starts_with("no Cloudflare zone for foh.elsewhere.org"),
        "{error}"
    );
    acme.name = NAME.into();
    cf_api.lock().unwrap().fail = Some((500, "Internal error".into()));
    let error = acme.issue().await.unwrap_err();
    assert!(error.ends_with("HTTP 500: Internal error"), "{error}");
    assert!(ca.lock().unwrap().requests.is_empty());
    // A token file that is no token.
    crate::sealed::write(
        &crate::cf_token::token_path(&dir.path().join(crate::secrets::SECRETS_DIR)),
        b"no",
    )
    .unwrap();
    assert!(
        acme.issue()
            .await
            .unwrap_err()
            .contains("does not hold a Cloudflare API token")
    );
}

#[tokio::test]
async fn another_directory_or_a_corrupt_file_gets_a_new_account() {
    let dir = tempfile::tempdir().unwrap();
    let (mut acme, cf_api, ca) = setup(dir.path(), true).await;
    acme.cfg.email = None;
    acme.issue().await.unwrap();
    let accounts = ca.lock().unwrap().new_accounts.clone();
    assert_eq!(accounts[0]["contact"], serde_json::json!([]));
    // Another CA (staging → production): a new account there.
    let (other, other_ca) = ca_double::start(Arc::clone(&cf_api)).await;
    acme.cfg.directory = other;
    acme.issue().await.unwrap();
    assert_eq!(count(&other_ca, "POST /new-account"), 1);
    // Back to the first CA: the stored account is the other CA's now.
    let first = ca.lock().unwrap().base.clone();
    acme.cfg.directory = format!("{first}/directory");
    std::fs::write(acme.account_path(), b"{not json").unwrap();
    acme.issue().await.unwrap();
    assert_eq!(count(&ca, "POST /new-account"), 2);
    // An account file that cannot be read is an error, never replaced.
    std::fs::remove_file(acme.account_path()).unwrap();
    std::fs::create_dir(acme.account_path()).unwrap();
    assert!(
        acme.issue()
            .await
            .unwrap_err()
            .starts_with("reading the ACME account")
    );
}

#[test]
fn an_https_directory_uses_the_verified_client() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = AcmeCfg {
        email: None,
        directory: "https://acme-staging-v02.api.letsencrypt.org/directory".into(),
        propagation_s: 20,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _inside = runtime.enter();
    let acme = Acme::new(NAME, &cfg, dir.path(), HttpClient::new().unwrap());
    assert!(acme.builder().is_ok());
}

#[tokio::test]
async fn the_keeper_gets_stores_and_serves_a_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, _cf, _ca) = setup(dir.path(), true).await;
    let https = Arc::new(Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap());
    let state = Arc::new(RemoteState::default());
    let task = tokio::spawn(keep(acme, Arc::clone(&https), Arc::clone(&state)));
    for _ in 0..500 {
        if https.serving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    task.abort();
    assert!(https.serving());
    let store = CertStore::new(dir.path());
    assert!(matches!(
        crate::remote::stored(&store, NAME),
        Stored::Serve(_)
    ));
    let config =
        crate::config::Config::parse("[tls]\nname = \"foh.example.org\"\n[acme]\n", dir.path())
            .unwrap();
    let status = state
        .snapshot(&config, Some(&https), crate::auth::now_secs() as i64)
        .https
        .unwrap();
    assert_eq!(status.cert_names, vec![NAME.to_string()]);
    assert!(status.last_issued.is_some());
    assert_eq!((status.acme_error, status.acme_failures), (None, 0));
    https.stop(Duration::from_millis(10));
    assert!(https.stopped(Duration::from_secs(5)).await);
}

#[tokio::test]
async fn a_failed_attempt_is_counted_and_keeps_the_served_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let (acme, _cf, ca) = setup(dir.path(), false).await;
    let https = Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap();
    let state = RemoteState::default();
    let store = CertStore::new(dir.path());
    // The old certificate, 10 days left, is served.
    let old_ca = TestCa::new();
    let now = crate::auth::now_secs() as i64;
    let (chain, key) = old_ca.leaf(&[NAME], now - 80 * DAY, now + 10 * DAY);
    https
        .serve(crate::tls::server_config(&chain, &key).unwrap())
        .unwrap();
    // No token: retried every 5 minutes, the old certificate stays.
    assert_eq!(
        acme.renew(&store, &https, &state).await,
        Err(NO_TOKEN_RETRY)
    );
    assert_eq!(
        acme.renew(&store, &https, &state).await,
        Err(NO_TOKEN_RETRY)
    );
    let config =
        crate::config::Config::parse("[tls]\nname = \"foh.example.org\"\n", dir.path()).unwrap();
    let status = state.snapshot(&config, Some(&https), 0).https.unwrap();
    assert_eq!(status.acme_error.as_deref(), Some(NO_TOKEN));
    assert_eq!(status.acme_failures, 2);
    let served = handshake(https.addr(), NAME, &old_ca.pem).await.unwrap();
    assert_eq!(served.not_after, now + 10 * DAY);
    // With the token set, the next attempt succeeds, resets the count and
    // serves the CA's certificate.
    crate::cf_token::run(dir.path(), format!("{}\n", cf_secret()).as_bytes()).unwrap();
    assert_eq!(acme.renew(&store, &https, &state).await, Ok(()));
    let status = state.snapshot(&config, Some(&https), 0).https.unwrap();
    assert_eq!(status.acme_failures, 0);
    let ca_pem = ca.lock().unwrap().test_ca.pem.clone();
    let served = handshake(https.addr(), NAME, &ca_pem).await.unwrap();
    assert!(served.not_after > now + 30 * DAY, "{served:?}");
    // A refused challenge (a new authorization: another account) is an
    // error retried after a minute; the new certificate stays served.
    std::fs::remove_file(acme.account_path()).unwrap();
    ca.lock().unwrap().refuse_challenges = true;
    assert_eq!(acme.renew(&store, &https, &state).await, Err(FIRST_RETRY));
    assert_eq!(
        handshake(https.addr(), NAME, &ca_pem).await.unwrap(),
        served
    );
    https.stop(Duration::from_millis(10));
    assert!(https.stopped(Duration::from_secs(5)).await);
}

#[test]
fn failed_attempts_are_retried_after_a_minute_doubling_to_six_hours() {
    let min = |m: u64| Duration::from_secs(60 * m);
    assert_eq!(retry_delay(0), min(1));
    assert_eq!(retry_delay(1), min(1));
    assert_eq!(retry_delay(2), min(2));
    assert_eq!(retry_delay(3), min(4));
    assert_eq!(retry_delay(9), min(256));
    assert_eq!(retry_delay(10), min(360));
    assert_eq!(retry_delay(u32::MAX), min(360));
    assert_eq!(FIRST_RETRY, min(1));
    assert_eq!(MAX_RETRY, min(360));
    assert_eq!(CHECK_EVERY, min(720));
}

#[test]
fn a_missing_token_is_looked_for_every_five_minutes_and_warned_of_once() {
    assert_eq!(NO_TOKEN_RETRY, Duration::from_secs(300));
    assert_eq!(next_attempt(NO_TOKEN, 1), NO_TOKEN_RETRY);
    assert_eq!(next_attempt(NO_TOKEN, 12), NO_TOKEN_RETRY);
    assert_eq!(next_attempt("HTTP 500", 1), FIRST_RETRY);
    assert_eq!(next_attempt("HTTP 500", 12), MAX_RETRY);
    assert!(warns(NO_TOKEN, false));
    assert!(!warns(NO_TOKEN, true));
    assert!(warns("HTTP 500", true));
    assert!(warns("HTTP 500", false));
}

#[test]
fn a_stored_account_counts_for_its_own_directory_only() {
    let directory = "https://ca.example/directory";
    let bytes = serde_json::to_vec(&serde_json::json!({
        "directory": directory,
        "credentials": {
            "id": "https://ca.example/acct/1",
            "key_pkcs8": "AAAA",
            "directory": directory,
        },
    }))
    .unwrap();
    assert!(stored_account(&bytes, directory).is_some());
    assert!(stored_account(&bytes, "https://other.example/directory").is_none());
    assert!(stored_account(b"{not json", directory).is_none());
}

#[test]
fn a_certificate_is_kept_until_thirty_days_are_left() {
    let now = 1_000_000_000;
    let pem = |days: i64| {
        Stored::Serve(Box::new(crate::tls::Pem {
            chain: String::new(),
            key: String::new(),
            info: CertInfo {
                names: vec![NAME.into()],
                not_after: now + days * DAY,
            },
        }))
    };
    assert_eq!(plan(&pem(60), now), Plan::Keep);
    assert_eq!(plan(&pem(30), now), Plan::Keep);
    assert_eq!(plan(&pem(29), now), Plan::Obtain);
    assert_eq!(plan(&Stored::Missing, now), Plan::Obtain);
    assert_eq!(plan(&Stored::Unusable("x".into()), now), Plan::Obtain);
}
