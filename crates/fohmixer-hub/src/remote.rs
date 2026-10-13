//! Remote access's live state (#17) for `/api/status`: the served
//! certificates (the `[tls]` name's and, #90, AbleSet's name's), the ACME
//! client's last result for each, the tunnel's readiness; and the start of
//! the HTTPS listener on the stored certificates.

use std::sync::{Mutex, MutexGuard, PoisonError};

use fohmixer_proto::client::{AbleSetStatus, CertStatus, HttpsStatus, RemoteStatus, TunnelStatus};

use crate::config::Config;
use crate::https::Https;
use crate::tls::{CertInfo, CertStore, Slot};

/// One name's certificate: the served one, why the stored one is not
/// served, the ACME client's last result.
#[derive(Debug, Default)]
struct NameState {
    cert: Option<CertInfo>,
    cert_error: Option<String>,
    acme_error: Option<String>,
    acme_failures: u32,
    last_issued: Option<i64>,
}

impl NameState {
    /// Its part of `/api/status` at `now` (`serving`: whether the listener
    /// serves it).
    fn status(&self, serving: bool, now: i64) -> CertStatus {
        CertStatus {
            serving,
            cert_names: self
                .cert
                .as_ref()
                .map(|c| c.names.clone())
                .unwrap_or_default(),
            not_after: self.cert.as_ref().map(|c| c.not_after),
            days_left: self.cert.as_ref().map(|c| c.days_left(now)),
            cert_error: self.cert_error.clone(),
            acme_error: self.acme_error.clone(),
            acme_failures: self.acme_failures,
            last_issued: self.last_issued,
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    bind_error: Option<String>,
    primary: NameState,
    ableset: NameState,
    tunnel: Option<TunnelStatus>,
}

impl Inner {
    fn name(&mut self, slot: Slot) -> &mut NameState {
        match slot {
            Slot::Primary => &mut self.primary,
            Slot::AbleSet => &mut self.ableset,
        }
    }
}

/// The state the listener, the ACME clients and the tunnel check write.
#[derive(Debug, Default)]
pub struct RemoteState(Mutex<Inner>);

impl RemoteState {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Why the HTTPS port could not be bound (`None`: it is bound).
    pub fn set_bind_error(&self, why: Option<String>) {
        self.inner().bind_error = why;
    }

    /// The listener serves `cert` for `slot`.
    pub fn set_cert(&self, slot: Slot, cert: CertInfo) {
        let mut inner = self.inner();
        let name = inner.name(slot);
        name.cert = Some(cert);
        name.cert_error = None;
    }

    /// The stored certificate of `slot` is not served, because of `why`.
    pub fn set_cert_error(&self, slot: Slot, why: String) {
        self.inner().name(slot).cert_error = Some(why);
    }

    /// The ACME client of `slot` got a certificate at `now`.
    pub fn acme_ok(&self, slot: Slot, now: i64) {
        let mut inner = self.inner();
        let name = inner.name(slot);
        name.acme_error = None;
        name.acme_failures = 0;
        name.last_issued = Some(now);
    }

    /// An ACME attempt of `slot` failed because of `why`: how many in a row
    /// now, and whether the one before failed for the same reason.
    pub fn acme_failed(&self, slot: Slot, why: String) -> (u32, bool) {
        let mut inner = self.inner();
        let name = inner.name(slot);
        let repeated = name.acme_error.as_ref() == Some(&why);
        name.acme_error = Some(why);
        name.acme_failures += 1;
        (name.acme_failures, repeated)
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
            https: config.tls.as_ref().map(|tls| {
                let cert = inner.primary.status(https.is_some_and(Https::serving), now);
                HttpsStatus {
                    port: https.map_or(tls.port, |https| https.addr().port()),
                    bound: https.is_some(),
                    bind_error: inner.bind_error.clone(),
                    serving: cert.serving,
                    cert_names: cert.cert_names,
                    not_after: cert.not_after,
                    days_left: cert.days_left,
                    cert_error: cert.cert_error,
                    acme: config.acme.is_some(),
                    acme_error: cert.acme_error,
                    acme_failures: cert.acme_failures,
                    last_issued: cert.last_issued,
                }
            }),
            access: config.access.is_some(),
            tunnel: config.tunnel.as_ref().and(inner.tunnel.clone()),
        }
    }

    /// The `ableset` part of `/api/status` at `now` (#90): none without
    /// `[ableset]`.
    pub fn ableset(
        &self,
        config: &Config,
        https: Option<&Https>,
        now: i64,
    ) -> Option<AbleSetStatus> {
        let serving = https.is_some_and(|https| https.serves(Slot::AbleSet));
        config.ableset.as_ref().map(|ableset| AbleSetStatus {
            name: ableset.name.clone(),
            cert: self.inner().ableset.status(serving, now),
            // The hub counts its tunnels (`ableset::AbleSet::open_sockets`).
            sockets: 0,
        })
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

/// Serves `pem` as the certificate of `slot` for `name` on `https` and
/// records it.
pub fn serve(
    https: &Https,
    state: &RemoteState,
    slot: Slot,
    name: &str,
    pem: &crate::tls::Pem,
) -> Result<(), String> {
    https.set_cert(slot, name, pem)?;
    state.set_cert(slot, pem.info.clone());
    Ok(())
}

/// At the start: the HTTPS listener serves the stored certificate of `slot`
/// for `name`, when there is a usable one (else it waits for the ACME
/// client).
pub fn serve_stored(
    https: &Https,
    state: &RemoteState,
    data_dir: &std::path::Path,
    slot: Slot,
    name: &str,
) {
    let store = CertStore::slot(data_dir, slot);
    let outcome = match stored(&store, name) {
        Stored::Serve(pem) => serve(https, state, slot, name, &pem).err(),
        Stored::Missing => Some(match slot {
            Slot::Primary => format!(
                "no certificate for {name} yet: HTTPS starts with the first one (plain HTTP serves meanwhile)"
            ),
            Slot::AbleSet => format!(
                "no certificate for {name} yet: it is served beside the [tls] name's once it is there"
            ),
        }),
        Stored::Unusable(why) => Some(why),
    };
    if let Some(why) = outcome {
        tracing::warn!(name, "HTTPS: {why}");
        state.set_cert_error(slot, why);
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
        let unbound = empty.https.clone().unwrap();
        assert_eq!(
            (unbound.port, unbound.bound, unbound.serving),
            (443, false, false)
        );
        state.set_bind_error(Some("binding HTTPS 0.0.0.0:443: in use".into()));
        assert_eq!(
            state
                .snapshot(&full, None, 0)
                .https
                .unwrap()
                .bind_error
                .as_deref(),
            Some("binding HTTPS 0.0.0.0:443: in use")
        );
        state.set_bind_error(None);
        assert_eq!(empty.tunnel.unwrap().ready_connections, 3);
        // A tunnel check without [tunnel] is not reported.
        assert_eq!(state.snapshot(&none, None, 0).tunnel, None);

        let https = Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap();
        let status = state.snapshot(&full, Some(&https), 0).https.unwrap();
        assert_eq!(status.port, https.addr().port());
        assert!(status.bound);
        assert_eq!(status.bind_error, None);
        assert!(!status.serving);
        assert!(status.acme);
        assert_eq!((status.not_after, status.days_left), (None, None));
        state.set_cert_error(Slot::Primary, "no certificate yet".into());
        assert_eq!(state.acme_failed(Slot::Primary, "first".into()), (1, false));
        assert_eq!(
            state.acme_failed(Slot::Primary, "second".into()),
            (2, false)
        );
        assert_eq!(state.acme_failed(Slot::Primary, "second".into()), (3, true));
        let status = state.snapshot(&full, Some(&https), 0).https.unwrap();
        assert_eq!(status.cert_error.as_deref(), Some("no certificate yet"));
        assert_eq!(status.acme_error.as_deref(), Some("second"));
        assert_eq!(status.acme_failures, 3);
        state.set_cert(
            Slot::Primary,
            CertInfo {
                names: vec!["foh.example.org".into()],
                not_after: 100 * DAY,
            },
        );
        state.acme_ok(Slot::Primary, 7);
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
        // After a success the same reason again is a first failure.
        assert_eq!(
            state.acme_failed(Slot::Primary, "second".into()),
            (1, false)
        );
        let manual = config("[tls]\nname = \"foh.example.org\"\n");
        assert!(!state.snapshot(&manual, Some(&https), 0).https.unwrap().acme);
        assert!(!state.snapshot(&manual, Some(&https), 0).access);
    }

    #[test]
    fn each_names_acme_results_are_its_own() {
        let state = RemoteState::default();
        let both = config(
            "[tls]\nname = \"foh.example.org\"\n[acme]\n[ableset]\nname = \"ableset.example.org\"\n",
        );
        let acme = |slot: Slot| match slot {
            Slot::Primary => {
                let tls = state.snapshot(&both, None, 0).https.unwrap();
                (tls.acme_error, tls.acme_failures, tls.last_issued)
            }
            Slot::AbleSet => {
                let cert = state.ableset(&both, None, 0).unwrap().cert;
                (cert.acme_error, cert.acme_failures, cert.last_issued)
            }
        };
        let failed = |why: &str, failures: u32| -> (Option<String>, u32, Option<i64>) {
            (Some(why.to_string()), failures, None)
        };
        // AbleSet's failures leave the [tls] name's status as it was.
        assert_eq!(state.acme_failed(Slot::AbleSet, "limit".into()), (1, false));
        assert_eq!(state.acme_failed(Slot::AbleSet, "limit".into()), (2, true));
        assert_eq!(acme(Slot::AbleSet), failed("limit", 2));
        assert_eq!(acme(Slot::Primary), (None, 0, None));
        // The [tls] name's: the same reason is its own first failure, and
        // AbleSet's count goes on from its own.
        assert_eq!(state.acme_failed(Slot::Primary, "limit".into()), (1, false));
        assert_eq!(state.acme_failed(Slot::AbleSet, "limit".into()), (3, true));
        assert_eq!(acme(Slot::Primary), failed("limit", 1));
        // A success clears only its own name's failures.
        state.acme_ok(Slot::Primary, 7);
        assert_eq!(acme(Slot::Primary), (None, 0, Some(7)));
        assert_eq!(acme(Slot::AbleSet), failed("limit", 3));
        assert_eq!(state.acme_failed(Slot::Primary, "zone".into()), (1, false));
        state.acme_ok(Slot::AbleSet, 9);
        assert_eq!(acme(Slot::AbleSet), (None, 0, Some(9)));
        assert_eq!(acme(Slot::Primary), (Some("zone".to_string()), 1, Some(7)));
    }

    #[tokio::test]
    async fn the_start_serves_a_usable_stored_certificate_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        assert_eq!(stored(&store, "foh.example.org"), Stored::Missing);
        let state = RemoteState::default();
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap();
        serve_stored(&https, &state, dir.path(), Slot::Primary, "foh.example.org");
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
        // AbleSet's name (#90): its own store and status.
        let both = config(
            "[tls]\nname = \"foh.example.org\"\n[ableset]\nname = \"ableset.example.org\"\n",
        );
        let ableset = |at: i64| state.ableset(&both, Some(&https), at).unwrap().cert;
        serve_stored(
            &https,
            &state,
            dir.path(),
            Slot::AbleSet,
            "ableset.example.org",
        );
        let error = ableset(0).cert_error.unwrap();
        assert_eq!(
            error,
            "no certificate for ableset.example.org yet: it is served beside the [tls] name's once it is there"
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
        serve_stored(&https, &state, dir.path(), Slot::Primary, "foh.example.org");
        assert!(
            !https.serving(),
            "a certificate for another name is not served"
        );
        let (chain, key) = ca.leaf(&["ableset.example.org"], now() - DAY, now() + 60 * DAY);
        let pem = crate::tls::checked(chain, key, "ableset.example.org").unwrap();
        CertStore::slot(dir.path(), Slot::AbleSet)
            .save(&pem)
            .unwrap();
        serve_stored(
            &https,
            &state,
            dir.path(),
            Slot::AbleSet,
            "ableset.example.org",
        );
        assert!(!https.serving(), "only the [tls] name's starts the server");
        let waiting = ableset(0);
        assert_eq!(waiting.cert_names, vec!["ableset.example.org".to_string()]);
        assert_eq!((waiting.serving, waiting.cert_error), (false, None));

        let t = now();
        let (chain, key) = ca.leaf(&["foh.example.org"], t - DAY, t + 90 * DAY);
        std::fs::write(store.cert_path(), chain).unwrap();
        std::fs::write(store.key_path(), key).unwrap();
        serve_stored(&https, &state, dir.path(), Slot::Primary, "foh.example.org");
        assert!(https.serving());
        let status = state.snapshot(&full, Some(&https), t).https.unwrap();
        assert_eq!(status.cert_error, None);
        assert_eq!(status.days_left, Some(90));
        assert!(ableset(t).serving, "served beside it now");
        assert_eq!(state.ableset(&full, Some(&https), t), None);
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
