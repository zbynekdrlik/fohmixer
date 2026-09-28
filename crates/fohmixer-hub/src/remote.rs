//! Remote access's live state (#17) for `/api/status`: the served
//! certificate, the ACME client's last result, the tunnel's readiness; and
//! the start of the HTTPS listener on the stored certificate.

use std::sync::{Mutex, MutexGuard, PoisonError};

use fohmixer_proto::client::{HttpsStatus, RemoteStatus, TunnelStatus};

use crate::config::Config;
use crate::https::Https;
use crate::tls::{CertInfo, CertStore};

#[derive(Debug, Default)]
struct Inner {
    cert: Option<CertInfo>,
    cert_error: Option<String>,
    acme_error: Option<String>,
    acme_failures: u32,
    last_issued: Option<i64>,
    tunnel: Option<TunnelStatus>,
}

/// The state the listener, the ACME client and the tunnel check write.
#[derive(Debug, Default)]
pub struct RemoteState(Mutex<Inner>);

impl RemoteState {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The listener serves `cert`.
    pub fn set_cert(&self, cert: CertInfo) {
        let mut inner = self.inner();
        inner.cert = Some(cert);
        inner.cert_error = None;
    }

    /// The stored certificate is not served, because of `why`.
    pub fn set_cert_error(&self, why: String) {
        self.inner().cert_error = Some(why);
    }

    /// The ACME client got a certificate at `now`.
    pub fn acme_ok(&self, now: i64) {
        let mut inner = self.inner();
        inner.acme_error = None;
        inner.acme_failures = 0;
        inner.last_issued = Some(now);
    }

    /// An ACME attempt failed: how many in a row now.
    pub fn acme_failed(&self, why: String) -> u32 {
        let mut inner = self.inner();
        inner.acme_error = Some(why);
        inner.acme_failures += 1;
        inner.acme_failures
    }

    /// The tunnel's latest check.
    pub fn set_tunnel(&self, status: TunnelStatus) {
        self.inner().tunnel = Some(status);
    }

    /// The tunnel's latest check, if any.
    pub fn tunnel(&self) -> Option<TunnelStatus> {
        self.inner().tunnel.clone()
    }

    /// The `remote` part of `/api/status` at `now`.
    pub fn snapshot(&self, config: &Config, https: Option<&Https>, now: i64) -> RemoteStatus {
        let inner = self.inner();
        RemoteStatus {
            name: config.tls.as_ref().map(|tls| tls.name.clone()),
            https: https.map(|https| HttpsStatus {
                port: https.addr().port(),
                serving: https.serving(),
                cert_names: inner
                    .cert
                    .as_ref()
                    .map(|c| c.names.clone())
                    .unwrap_or_default(),
                not_after: inner.cert.as_ref().map(|c| c.not_after),
                days_left: inner.cert.as_ref().map(|c| c.days_left(now)),
                cert_error: inner.cert_error.clone(),
                acme: config.acme.is_some(),
                acme_error: inner.acme_error.clone(),
                acme_failures: inner.acme_failures,
                last_issued: inner.last_issued,
            }),
            access: config.access.is_some(),
            tunnel: config.tunnel.as_ref().and(inner.tunnel.clone()),
        }
    }
}

/// What the start does with the stored certificate of `name`.
#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    /// Serve it.
    Serve(Box<crate::tls::Pem>),
    /// None stored yet.
    Missing,
    /// It cannot be served (the reason).
    Unusable(String),
}

/// The stored certificate of `name` in `store`.
pub fn stored(store: &CertStore, name: &str) -> Stored {
    match store.load(name) {
        Ok(Some(Ok(pem))) => Stored::Serve(Box::new(pem)),
        Ok(None) => Stored::Missing,
        Ok(Some(Err(why))) => Stored::Unusable(why),
        Err(e) => Stored::Unusable(format!("reading {}: {e}", store.cert_path().display())),
    }
}

/// Serves `pem` on `https` and records it.
pub fn serve(https: &Https, state: &RemoteState, pem: &crate::tls::Pem) -> Result<(), String> {
    let config = crate::tls::server_config(&pem.chain, &pem.key)?;
    https
        .serve(config)
        .map_err(|e| format!("starting the HTTPS listener: {e}"))?;
    state.set_cert(pem.info.clone());
    Ok(())
}

/// At the start: the HTTPS listener serves the stored certificate, when
/// there is a usable one (else it waits for the ACME client).
pub fn serve_stored(https: &Https, state: &RemoteState, data_dir: &std::path::Path, name: &str) {
    let store = CertStore::new(data_dir);
    let outcome = match stored(&store, name) {
        Stored::Serve(pem) => serve(https, state, &pem).err(),
        Stored::Missing => Some(format!(
            "no certificate for {name} yet: HTTPS starts with the first one (plain HTTP serves meanwhile)"
        )),
        Stored::Unusable(why) => Some(why),
    };
    if let Some(why) = outcome {
        tracing::warn!(name, "HTTPS: {why}");
        state.set_cert_error(why);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::test_certs::TestCa;

    const DAY: i64 = 86_400;

    fn now() -> i64 {
        crate::auth::now_secs() as i64
    }

    fn config(remote: &str) -> Config {
        Config::parse(remote, std::path::Path::new("/data")).unwrap()
    }

    #[test]
    fn the_snapshot_reports_what_is_configured_and_what_happened() {
        let state = RemoteState::default();
        let none = config("");
        assert_eq!(state.snapshot(&none, None, 0), RemoteStatus::default());
        let full = config(
            "[tls]\nname = \"foh.example.org\"\n[acme]\n[access]\nteam_domain = \"t.example.com\"\naud = [\"a\"]\n[tunnel]\n",
        );
        state.set_tunnel(TunnelStatus {
            ready_connections: 3,
            error: None,
            checked: Some(5),
        });
        let empty = state.snapshot(&full, None, 0);
        assert_eq!(empty.name.as_deref(), Some("foh.example.org"));
        assert!(empty.access);
        assert_eq!(empty.https, None, "no listener");
        assert_eq!(empty.tunnel.unwrap().ready_connections, 3);
        // A tunnel check without [tunnel] is not reported.
        assert_eq!(state.snapshot(&none, None, 0).tunnel, None);

        let https = Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap();
        let status = state.snapshot(&full, Some(&https), 0).https.unwrap();
        assert_eq!(status.port, https.addr().port());
        assert!(!status.serving);
        assert!(status.acme);
        assert_eq!((status.not_after, status.days_left), (None, None));
        state.set_cert_error("no certificate yet".into());
        assert_eq!(state.acme_failed("first".into()), 1);
        assert_eq!(state.acme_failed("second".into()), 2);
        let status = state.snapshot(&full, Some(&https), 0).https.unwrap();
        assert_eq!(status.cert_error.as_deref(), Some("no certificate yet"));
        assert_eq!(status.acme_error.as_deref(), Some("second"));
        assert_eq!(status.acme_failures, 2);
        state.set_cert(CertInfo {
            names: vec!["foh.example.org".into()],
            not_after: 100 * DAY,
        });
        state.acme_ok(7);
        let status = state.snapshot(&full, Some(&https), 40 * DAY).https.unwrap();
        assert_eq!(status.cert_names, vec!["foh.example.org".to_string()]);
        assert_eq!(status.not_after, Some(100 * DAY));
        assert_eq!(status.days_left, Some(60));
        assert_eq!(
            status.cert_error, None,
            "a served certificate clears the error"
        );
        assert_eq!((status.acme_error, status.acme_failures), (None, 0));
        assert_eq!(status.last_issued, Some(7));
        let manual = config("[tls]\nname = \"foh.example.org\"\n");
        assert!(!state.snapshot(&manual, Some(&https), 0).https.unwrap().acme);
        assert!(!state.snapshot(&manual, Some(&https), 0).access);
    }

    #[tokio::test]
    async fn the_start_serves_a_usable_stored_certificate_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        assert_eq!(stored(&store, "foh.example.org"), Stored::Missing);
        let state = RemoteState::default();
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap();
        serve_stored(&https, &state, dir.path(), "foh.example.org");
        assert!(!https.serving());
        let full = config("[tls]\nname = \"foh.example.org\"\n");
        let error = state
            .snapshot(&full, Some(&https), 0)
            .https
            .unwrap()
            .cert_error
            .unwrap();
        assert!(
            error.starts_with("no certificate for foh.example.org yet"),
            "{error}"
        );

        let ca = TestCa::new();
        let (chain, key) = ca.leaf(&["other.example.org"], now() - DAY, now() + 90 * DAY);
        std::fs::create_dir_all(dir.path().join("tls")).unwrap();
        std::fs::write(store.cert_path(), chain).unwrap();
        std::fs::write(store.key_path(), key).unwrap();
        assert!(matches!(
            stored(&store, "foh.example.org"),
            Stored::Unusable(_)
        ));
        serve_stored(&https, &state, dir.path(), "foh.example.org");
        assert!(
            !https.serving(),
            "a certificate for another name is not served"
        );

        let t = now();
        let (chain, key) = ca.leaf(&["foh.example.org"], t - DAY, t + 90 * DAY);
        std::fs::write(store.cert_path(), chain).unwrap();
        std::fs::write(store.key_path(), key).unwrap();
        serve_stored(&https, &state, dir.path(), "foh.example.org");
        assert!(https.serving());
        let status = state.snapshot(&full, Some(&https), t).https.unwrap();
        assert_eq!(status.cert_error, None);
        assert_eq!(status.days_left, Some(90));
        https.stop(std::time::Duration::from_millis(10));
        assert!(https.stopped(std::time::Duration::from_secs(5)).await);

        std::fs::remove_file(store.key_path()).unwrap();
        std::fs::create_dir(store.key_path()).unwrap();
        match stored(&store, "foh.example.org") {
            Stored::Unusable(why) => assert!(why.starts_with("reading "), "{why}"),
            other => panic!("{other:?}"),
        }
    }
}
