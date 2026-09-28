//! The certificate of the `[tls]` name from an ACME CA (#17): Let's
//! Encrypt by DNS-01 through the Cloudflare DNS API, in the hub itself (the
//! PC depends on nothing else, spec P7).
//!
//! [`keep`] runs for the hub's life: a stored certificate with 30 days or
//! more left is kept (checked every [`CHECK_EVERY`]); otherwise one attempt
//! orders a new one, writes the challenge record, lets it reach Cloudflare's
//! name servers (`[acme] propagation_s`), answers the challenge, removes the
//! record (always, also after a failure), stores the certificate and its
//! key and swaps them into the HTTPS listener. A failed attempt is logged
//! with its reason, shown in `/api/status`, and retried after
//! [`next_attempt`] (1 min doubling to 6 h; every 5 min while no Cloudflare
//! token is set, logged once); the old certificate keeps being served
//! meanwhile.
//!
//! The ACME account's key is sealed for the hub's user in
//! `secrets/acme_account.<dpapi|test>`; the Cloudflare token is
//! `cf_token.rs`'s.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use instant_acme::{
    Account, AccountBuilder, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier,
    NewAccount, NewOrder, Order, OrderStatus, RetryPolicy,
};
use serde::{Deserialize, Serialize};

use crate::cloudflare::Dns;
use crate::config::AcmeCfg;
use crate::http_client::HttpClient;
use crate::https::Https;
use crate::remote::{RemoteState, Stored};
use crate::secrets::SECRETS_DIR;
use crate::tls::{CertStore, Pem};

/// How often a kept certificate is looked at again.
pub const CHECK_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
/// The wait after a first failed attempt.
pub const FIRST_RETRY: Duration = Duration::from_secs(60);
/// The longest wait between two failed attempts.
pub const MAX_RETRY: Duration = Duration::from_secs(6 * 60 * 60);
/// The wait while no Cloudflare token is set: the check is local (nothing
/// is asked of anyone), so a token set later is used within minutes.
pub const NO_TOKEN_RETRY: Duration = Duration::from_secs(5 * 60);
/// The account file's name without its extension.
pub const ACCOUNT_STEM: &str = "acme_account";
/// Why nothing happens without a token.
pub const NO_TOKEN: &str = "no Cloudflare API token: run `fohmixer-hub cloudflare set-token` as the hub's \
     user (a token with Zone > DNS > Edit on the name's zone)";

/// The wait before the next attempt after `failures` failed ones in a row:
/// [`FIRST_RETRY`] doubling, at most [`MAX_RETRY`].
pub fn retry_delay(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_RETRY.saturating_mul(1 << doublings).min(MAX_RETRY)
}

/// The wait before the next attempt after `failures` failed ones in a row,
/// the last because of `why`.
pub fn next_attempt(why: &str, failures: u32) -> Duration {
    if why == NO_TOKEN {
        NO_TOKEN_RETRY
    } else {
        retry_delay(failures)
    }
}

/// Whether a failure is logged as a warning: always, except the missing
/// token again (every 5 min; the first one was a warning and `/api/status`
/// keeps showing it).
pub fn warns(why: &str, repeated: bool) -> bool {
    !repeated || why != NO_TOKEN
}

/// The stored account when it belongs to `directory`. A file that does not
/// parse is logged ([`parse_problem`]) and replaced by a new account.
fn stored_account(bytes: &[u8], directory: &str) -> Option<StoredAccount> {
    match serde_json::from_slice::<StoredAccount>(bytes) {
        Ok(account) => (account.directory == directory).then_some(account),
        Err(e) => {
            let problem = parse_problem(&e);
            tracing::warn!("ACME: the stored account does not parse ({problem}); making a new one");
            None
        }
    }
}

/// What is logged of a stored account that does not parse: the kind and
/// place of the error, never serde's text (it can quote a value: the key).
fn parse_problem(e: &serde_json::Error) -> String {
    format!(
        "{:?} error at line {} column {}",
        e.classify(),
        e.line(),
        e.column()
    )
}

/// What the keeper does with the stored certificate at `now`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// It has 30 days or more: keep it.
    Keep,
    /// None, unusable or due: get a new one.
    Obtain,
}

/// The plan for `stored` at `now`.
pub fn plan(stored: &Stored, now: i64) -> Plan {
    match stored {
        Stored::Serve(pem) if !pem.info.renewal_due(now) => Plan::Keep,
        _ => Plan::Obtain,
    }
}

/// The account on disk: the directory it belongs to and its credentials.
#[derive(Serialize, Deserialize)]
struct StoredAccount {
    directory: String,
    credentials: AccountCredentials,
}

/// The ACME client of one name.
pub struct Acme {
    pub name: String,
    pub cfg: AcmeCfg,
    pub data_dir: PathBuf,
    /// The Cloudflare API (`cloudflare::API`; a double in the tests).
    pub cloudflare_api: String,
    pub http: HttpClient,
    /// How long the CA's checks are polled.
    pub retry: RetryPolicy,
}

impl Acme {
    /// The client of `name` with `cfg`'s CA, the real Cloudflare API.
    pub fn new(name: &str, cfg: &AcmeCfg, data_dir: &std::path::Path, http: HttpClient) -> Self {
        Self {
            name: name.to_string(),
            cfg: cfg.clone(),
            data_dir: data_dir.to_path_buf(),
            cloudflare_api: crate::cloudflare::API.to_string(),
            http,
            retry: RetryPolicy::new().timeout(Duration::from_secs(120)),
        }
    }

    fn secrets(&self) -> PathBuf {
        self.data_dir.join(SECRETS_DIR)
    }

    fn account_path(&self) -> PathBuf {
        self.secrets()
            .join(format!("{ACCOUNT_STEM}.{}", crate::sealed::SEALED_EXT))
    }

    /// The HTTP client for the CA: the verified HTTPS default, plain http
    /// only for a loopback directory (`config::url_allowed`: a test double).
    fn builder(&self) -> Result<AccountBuilder, String> {
        if self.cfg.directory.starts_with("http://") {
            let plain =
                hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                    .build_http::<instant_acme::BodyWrapper<bytes::Bytes>>();
            return Ok(Account::builder_with_http(Box::new(plain)));
        }
        Account::builder().map_err(|e| format!("the ACME HTTP client: {e}"))
    }

    /// The account: the stored one of this directory, else a new one
    /// (stored at once).
    async fn account(&self) -> Result<Account, String> {
        let path = self.account_path();
        let stored = crate::sealed::read(&path)
            .map_err(|e| format!("reading the ACME account {}: {e}", path.display()))?
            .and_then(|bytes| stored_account(&bytes, &self.cfg.directory));
        if let Some(account) = stored {
            return self
                .builder()?
                .from_credentials(account.credentials)
                .await
                .map_err(|e| format!("the ACME account: {e}"));
        }
        let contact = self
            .cfg
            .email
            .as_ref()
            .map(|email| format!("mailto:{email}"));
        let contacts: Vec<&str> = contact.iter().map(String::as_str).collect();
        let (account, credentials) = self
            .builder()?
            .create(
                &NewAccount {
                    contact: &contacts,
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.cfg.directory.clone(),
                None,
            )
            .await
            .map_err(|e| format!("creating the ACME account: {e}"))?;
        let record = StoredAccount {
            directory: self.cfg.directory.clone(),
            credentials,
        };
        let json = serde_json::to_vec(&record).map_err(|e| format!("the ACME account: {e}"))?;
        std::fs::create_dir_all(self.secrets())
            .and_then(|()| crate::sealed::write(&path, &json))
            .map_err(|e| format!("storing the ACME account {}: {e}", path.display()))?;
        tracing::info!(directory = %self.cfg.directory, "ACME: created an account");
        Ok(account)
    }

    /// Writes the DNS-01 record of every pending authorization and answers
    /// its challenge; the record ids go to `records` as they are made.
    async fn authorize(
        &self,
        order: &mut Order,
        dns: &Dns<'_>,
        zone: &str,
        records: &mut Vec<String>,
    ) -> Result<(), String> {
        let mut authorizations = order.authorizations();
        while let Some(next) = authorizations.next().await {
            let mut authz = next.map_err(|e| format!("ACME authorization: {e}"))?;
            match authz.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                other => return Err(format!("ACME authorization is {other:?}")),
            }
            let mut challenge = authz
                .challenge(ChallengeType::Dns01)
                .ok_or("the CA offers no DNS-01 challenge")?;
            let fqdn = format!("_acme-challenge.{}", challenge.identifier());
            let value = challenge.key_authorization().dns_value();
            records.push(dns.add_txt(zone, &fqdn, &value).await?);
            tokio::time::sleep(Duration::from_secs(self.cfg.propagation_s)).await;
            challenge
                .set_ready()
                .await
                .map_err(|e| format!("ACME challenge: {e}"))?;
        }
        let status = order
            .poll_ready(&self.retry)
            .await
            .map_err(|e| format!("ACME check of the DNS record: {e}"))?;
        if status == OrderStatus::Ready {
            Ok(())
        } else {
            Err(format!(
                "the CA did not accept the DNS record (order {status:?})"
            ))
        }
    }

    /// One attempt: a new certificate for the name, checked.
    pub async fn issue(&self) -> Result<Pem, String> {
        let token = crate::cf_token::load(&self.secrets())
            .map_err(|e| e.to_string())?
            .ok_or(NO_TOKEN)?;
        let dns = Dns::new(&self.http, &self.cloudflare_api, &token);
        let zone = dns.zone_id(&self.name).await?;
        let fqdn = format!("_acme-challenge.{}", self.name);
        // A record an interrupted attempt left behind.
        for id in dns.txt_records(&zone, &fqdn).await? {
            dns.delete(&zone, &id).await?;
        }
        let account = self.account().await?;
        let mut order = account
            .new_order(&NewOrder::new(&[Identifier::Dns(self.name.clone())]))
            .await
            .map_err(|e| format!("ACME order: {e}"))?;
        let mut records = Vec::new();
        let authorized = self.authorize(&mut order, &dns, &zone, &mut records).await;
        for id in &records {
            if let Err(why) = dns.delete(&zone, id).await {
                tracing::warn!(record = %id, "ACME: the challenge record stays: {why}");
            }
        }
        authorized?;
        let key = order
            .finalize()
            .await
            .map_err(|e| format!("ACME finalize: {e}"))?;
        let chain = order
            .poll_certificate(&self.retry)
            .await
            .map_err(|e| format!("ACME certificate: {e}"))?;
        crate::tls::checked(chain, key, &self.name)
    }

    /// A new certificate, stored and served.
    async fn obtain(
        &self,
        store: &CertStore,
        https: &Https,
        state: &RemoteState,
    ) -> Result<Pem, String> {
        let pem = self.issue().await?;
        store
            .save(&pem)
            .map_err(|e| format!("storing the certificate: {e}"))?;
        crate::remote::serve(https, state, &pem)?;
        Ok(pem)
    }

    /// One attempt that stores and serves what it gets: `Err(the wait
    /// before the next one)` when it fails.
    async fn renew(
        &self,
        store: &CertStore,
        https: &Https,
        state: &RemoteState,
    ) -> Result<(), Duration> {
        tracing::info!(name = %self.name, directory = %self.cfg.directory, "ACME: getting a certificate");
        match self.obtain(store, https, state).await {
            Ok(pem) => {
                state.acme_ok(crate::auth::now_secs() as i64);
                tracing::info!(name = %self.name, not_after = pem.info.not_after, "ACME: new certificate served");
                Ok(())
            }
            Err(why) => {
                let (failures, repeated) = state.acme_failed(why.clone());
                let wait = next_attempt(&why, failures);
                if warns(&why, repeated) {
                    tracing::warn!(
                        name = %self.name,
                        failures,
                        retry_s = wait.as_secs(),
                        "ACME: no certificate: {why}"
                    );
                } else {
                    tracing::debug!(failures, "ACME: still no certificate: {why}");
                }
                Err(wait)
            }
        }
    }
}

/// Keeps the name's certificate for the hub's life (see the module doc).
pub async fn keep(acme: Acme, https: Arc<Https>, state: Arc<RemoteState>) {
    let store = CertStore::new(&acme.data_dir);
    loop {
        let now = crate::auth::now_secs() as i64;
        let wait = match plan(&crate::remote::stored(&store, &acme.name), now) {
            Plan::Keep => CHECK_EVERY,
            Plan::Obtain => match acme.renew(&store, &https, &state).await {
                Ok(()) => CHECK_EVERY,
                Err(wait) => wait,
            },
        };
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
#[path = "acme/double.rs"]
pub(crate) mod double;

#[cfg(test)]
#[path = "acme/tests.rs"]
mod tests;
