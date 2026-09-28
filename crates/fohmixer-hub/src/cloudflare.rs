//! The Cloudflare DNS API calls of the ACME DNS-01 challenge (#17): find
//! the zone of the `[tls]` name, list, add and delete the TXT records at
//! `_acme-challenge.<name>`. The token (`cf_token.rs`) needs `Zone > DNS >
//! Edit` on that zone only. The base URL is [`API`]; the tests point it at
//! a local double of the API.

use axum::http::Method;
use serde_json::{Value, json};

use crate::cf_token::CfToken;
use crate::http_client::HttpClient;

/// The Cloudflare API.
pub const API: &str = "https://api.cloudflare.com/client/v4";
/// The TTL of a challenge record (seconds; Cloudflare's minimum).
pub const TXT_TTL: u32 = 60;
/// The comment Cloudflare shows on a challenge record.
pub const TXT_COMMENT: &str = "fohmixer-hub ACME DNS-01 (removed after the check)";

/// The zones a name may belong to, longest first, down to two labels
/// (`a.b.example.org` → `a.b.example.org`, `b.example.org`, `example.org`).
pub fn zone_candidates(name: &str) -> Vec<String> {
    let labels: Vec<&str> = name.split('.').collect();
    (0..labels.len().saturating_sub(1))
        .map(|skip| labels[skip..].join("."))
        .collect()
}

/// One API client for one token.
pub struct Dns<'a> {
    http: &'a HttpClient,
    base: &'a str,
    token: &'a CfToken,
}

impl<'a> Dns<'a> {
    /// The API at `base` (normally [`API`]) with `token`.
    pub fn new(http: &'a HttpClient, base: &'a str, token: &'a CfToken) -> Self {
        Self { http, base, token }
    }

    /// One call: its `result`, or an error with the HTTP status and
    /// Cloudflare's first error message (never the token).
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        let what = format!("Cloudflare {method} {path}");
        let reply = self
            .http
            .send(
                method,
                &format!("{}{path}", self.base),
                Some(self.token.expose()),
                body,
            )
            .await
            .map_err(|e| format!("{what}: {e:#}"))?;
        let answer: Value = reply.json().unwrap_or(Value::Null);
        if (200..300).contains(&reply.status) && answer["success"] == json!(true) {
            return Ok(answer["result"].clone());
        }
        let message = answer["errors"][0]["message"]
            .as_str()
            .map_or_else(|| reply.snippet(), str::to_string);
        Err(format!("{what}: HTTP {}: {message}", reply.status))
    }

    /// The id of the zone `name` belongs to (the longest zone this token sees).
    pub async fn zone_id(&self, name: &str) -> Result<String, String> {
        for candidate in zone_candidates(name) {
            let zones = self
                .call(Method::GET, &format!("/zones?name={candidate}"), None)
                .await?;
            if let Some(id) = zones[0]["id"].as_str() {
                return Ok(id.to_string());
            }
        }
        Err(format!(
            "no Cloudflare zone for {name}: the token must see its zone (Zone > DNS > Edit)"
        ))
    }

    /// The ids of the TXT records at `fqdn`.
    pub async fn txt_records(&self, zone: &str, fqdn: &str) -> Result<Vec<String>, String> {
        let records = self
            .call(
                Method::GET,
                &format!("/zones/{zone}/dns_records?type=TXT&name={fqdn}"),
                None,
            )
            .await?;
        Ok(records
            .as_array()
            .map(|all| {
                all.iter()
                    .filter_map(|r| r["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Adds the TXT record `fqdn` = `content`: its id.
    pub async fn add_txt(&self, zone: &str, fqdn: &str, content: &str) -> Result<String, String> {
        let body = json!({
            "type": "TXT",
            "name": fqdn,
            "content": content,
            "ttl": TXT_TTL,
            "comment": TXT_COMMENT,
        });
        let record = self
            .call(
                Method::POST,
                &format!("/zones/{zone}/dns_records"),
                Some(&body),
            )
            .await?;
        record["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("Cloudflare created a TXT record without an id: {record}"))
    }

    /// Deletes the record `id`.
    pub async fn delete(&self, zone: &str, id: &str) -> Result<(), String> {
        self.call(
            Method::DELETE,
            &format!("/zones/{zone}/dns_records/{id}"),
            None,
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
pub(crate) mod double {
    //! A local double of the Cloudflare DNS API: zones by name, TXT records
    //! kept in memory, bearer token checked, calls counted. The ACME tests
    //! read its records the way the CA's check reads DNS.

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{delete, get};
    use axum::{Json, Router};
    use serde_json::{Value, json};

    /// What the double holds.
    #[derive(Default)]
    pub struct Api {
        /// Zone name → id.
        pub zones: BTreeMap<String, String>,
        /// Record id → (name, content).
        pub records: BTreeMap<String, (String, String)>,
        /// Every call: `METHOD path`.
        pub calls: Vec<String>,
        /// The token it accepts.
        pub token: String,
        /// Answer every call with this status and error (a failing API).
        pub fail: Option<(u16, String)>,
        next_id: u64,
    }

    pub type Shared = Arc<Mutex<Api>>;

    fn answer(
        ok: bool,
        result: Value,
        status: StatusCode,
        message: &str,
    ) -> (StatusCode, Json<Value>) {
        let errors = if ok {
            json!([])
        } else {
            json!([{"code": 1000, "message": message}])
        };
        (
            status,
            Json(json!({"success": ok, "errors": errors, "result": result})),
        )
    }

    /// The check every call passes first: the token, then a forced failure.
    fn gate(api: &mut Api, headers: &HeaderMap, call: String) -> Option<(StatusCode, Json<Value>)> {
        api.calls.push(call);
        let bearer = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if bearer != format!("Bearer {}", api.token) {
            return Some(answer(
                false,
                Value::Null,
                StatusCode::FORBIDDEN,
                "Authentication error",
            ));
        }
        api.fail.clone().map(|(status, message)| {
            answer(
                false,
                Value::Null,
                StatusCode::from_u16(status).unwrap(),
                &message,
            )
        })
    }

    async fn zones(
        State(api): State<Shared>,
        headers: HeaderMap,
        Query(q): Query<BTreeMap<String, String>>,
    ) -> (StatusCode, Json<Value>) {
        let mut api = api.lock().unwrap();
        let name = q.get("name").cloned().unwrap_or_default();
        if let Some(refused) = gate(&mut api, &headers, format!("GET /zones?name={name}")) {
            return refused;
        }
        let found: Vec<Value> = api
            .zones
            .get(&name)
            .map(|id| json!({"id": id, "name": name}))
            .into_iter()
            .collect();
        answer(true, json!(found), StatusCode::OK, "")
    }

    async fn list_or_add(
        State(api): State<Shared>,
        Path(zone): Path<String>,
        headers: HeaderMap,
        method: axum::http::Method,
        Query(q): Query<BTreeMap<String, String>>,
        body: Option<Json<Value>>,
    ) -> (StatusCode, Json<Value>) {
        let mut api = api.lock().unwrap();
        if let Some(refused) = gate(
            &mut api,
            &headers,
            format!("{method} /zones/{zone}/dns_records"),
        ) {
            return refused;
        }
        if !api.zones.values().any(|id| *id == zone) {
            return answer(false, Value::Null, StatusCode::NOT_FOUND, "no such zone");
        }
        if method == axum::http::Method::GET {
            let name = q.get("name").cloned().unwrap_or_default();
            let found: Vec<Value> = api
                .records
                .iter()
                .filter(|(_, (n, _))| *n == name)
                .map(|(id, (n, c))| json!({"id": id, "type": "TXT", "name": n, "content": c}))
                .collect();
            return answer(true, json!(found), StatusCode::OK, "");
        }
        let body = body.map(|b| b.0).unwrap_or(Value::Null);
        assert_eq!(body["type"], "TXT", "only TXT records: {body}");
        assert_eq!(body["ttl"], 60);
        api.next_id += 1;
        let id = format!("rec{}", api.next_id);
        let name = body["name"].as_str().unwrap().to_string();
        let content = body["content"].as_str().unwrap().to_string();
        api.records.insert(id.clone(), (name, content));
        answer(true, json!({"id": id}), StatusCode::OK, "")
    }

    async fn remove(
        State(api): State<Shared>,
        Path((zone, id)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        let mut api = api.lock().unwrap();
        if let Some(refused) = gate(
            &mut api,
            &headers,
            format!("DELETE /zones/{zone}/dns_records/{id}"),
        ) {
            return refused;
        }
        match api.records.remove(&id) {
            Some(_) => answer(true, json!({"id": id}), StatusCode::OK, ""),
            None => answer(
                false,
                Value::Null,
                StatusCode::NOT_FOUND,
                "Record not found",
            ),
        }
    }

    /// Starts the double: its base URL (`/client/v4` style) and its state.
    pub async fn start(token: &str, zones: &[(&str, &str)]) -> (String, Shared) {
        let api: Shared = Arc::new(Mutex::new(Api {
            zones: zones
                .iter()
                .map(|(n, id)| (n.to_string(), id.to_string()))
                .collect(),
            records: BTreeMap::new(),
            calls: Vec::new(),
            token: token.to_string(),
            fail: None,
            next_id: 0,
        }));
        let app = Router::new()
            .route("/client/v4/zones", get(zones))
            .route(
                "/client/v4/zones/{zone}/dns_records",
                get(list_or_add).post(list_or_add),
            )
            .route("/client/v4/zones/{zone}/dns_records/{id}", delete(remove))
            .with_state(Arc::clone(&api));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/client/v4"), api)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str) -> CfToken {
        crate::cf_token::read_token(format!("{text}\n").as_bytes()).unwrap()
    }

    #[test]
    fn a_name_belongs_to_one_of_its_parent_zones() {
        assert_eq!(
            zone_candidates("_acme-challenge.foh.example.org"),
            vec![
                "_acme-challenge.foh.example.org",
                "foh.example.org",
                "example.org"
            ]
        );
        assert_eq!(zone_candidates("example.org"), vec!["example.org"]);
        assert!(zone_candidates("org").is_empty());
        assert!(zone_candidates("").is_empty());
    }

    #[tokio::test]
    async fn the_zone_the_records_and_their_removal() {
        let secret = "k".repeat(24);
        let (base, api) = double::start(&secret, &[("example.org", "zone-1")]).await;
        let http = HttpClient::new().unwrap();
        let tok = token(&secret);
        let dns = Dns::new(&http, &base, &tok);
        assert_eq!(dns.zone_id("foh.example.org").await.unwrap(), "zone-1");
        let fqdn = "_acme-challenge.foh.example.org";
        assert!(dns.txt_records("zone-1", fqdn).await.unwrap().is_empty());
        let first = dns.add_txt("zone-1", fqdn, "value-1").await.unwrap();
        let second = dns.add_txt("zone-1", fqdn, "value-2").await.unwrap();
        assert_ne!(first, second);
        assert_eq!(
            dns.txt_records("zone-1", fqdn).await.unwrap(),
            vec![first.clone(), second.clone()]
        );
        assert!(
            dns.txt_records("zone-1", "other.example.org")
                .await
                .unwrap()
                .is_empty()
        );
        dns.delete("zone-1", &first).await.unwrap();
        assert_eq!(dns.txt_records("zone-1", fqdn).await.unwrap(), vec![second]);
        let error = dns.delete("zone-1", &first).await.unwrap_err();
        assert_eq!(
            error,
            format!(
                "Cloudflare DELETE /zones/zone-1/dns_records/{first}: HTTP 404: Record not found"
            )
        );
        let calls = api.lock().unwrap().calls.clone();
        assert_eq!(calls[0], "GET /zones?name=foh.example.org");
        assert_eq!(calls[1], "GET /zones?name=example.org");
    }

    #[tokio::test]
    async fn api_errors_name_the_call_and_cloudflares_message() {
        let secret = "k".repeat(24);
        let (base, api) = double::start(&secret, &[("example.org", "zone-1")]).await;
        let http = HttpClient::new().unwrap();
        let wrong = token(&"w".repeat(24));
        let error = Dns::new(&http, &base, &wrong)
            .zone_id("foh.example.org")
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "Cloudflare GET /zones?name=foh.example.org: HTTP 403: Authentication error"
        );
        assert!(!error.contains(&"w".repeat(24)), "never the token");
        let tok = token(&secret);
        let dns = Dns::new(&http, &base, &tok);
        let error = dns.zone_id("foh.other.org").await.unwrap_err();
        assert!(
            error.starts_with("no Cloudflare zone for foh.other.org"),
            "{error}"
        );
        api.lock().unwrap().fail = Some((500, "Internal error".into()));
        assert_eq!(
            dns.add_txt("zone-1", "x.example.org", "v")
                .await
                .unwrap_err(),
            "Cloudflare POST /zones/zone-1/dns_records: HTTP 500: Internal error"
        );
        // Nothing answers: the call and the transport error.
        let gone = Dns::new(&http, "http://127.0.0.1:1/client/v4", &tok);
        let error = gone.zone_id("foh.example.org").await.unwrap_err();
        assert!(
            error.starts_with("Cloudflare GET /zones?name=foh.example.org: http://127.0.0.1:1"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_success_needs_a_2xx_status_and_success_true() {
        use axum::routing::get;
        let app = axum::Router::new()
            .route(
                "/ok-false",
                get(|| async { axum::Json(json!({"success": false, "errors": [], "result": 1})) }),
            )
            .route(
                "/created",
                get(|| async {
                    (
                        axum::http::StatusCode::CREATED,
                        axum::Json(json!({"success": true, "result": {"id": "r9"}})),
                    )
                }),
            )
            .route(
                "/redirect",
                get(|| async {
                    (
                        axum::http::StatusCode::MULTIPLE_CHOICES,
                        axum::Json(json!({"success": true, "result": 1})),
                    )
                }),
            )
            .route(
                "/plain",
                get(|| async { (axum::http::StatusCode::BAD_GATEWAY, "gateway down") }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://{addr}");
        let http = HttpClient::new().unwrap();
        let tok = token(&"k".repeat(24));
        let dns = Dns::new(&http, &base, &tok);
        let error = dns.call(Method::GET, "/ok-false", None).await.unwrap_err();
        assert!(
            error.starts_with("Cloudflare GET /ok-false: HTTP 200: {"),
            "{error}"
        );
        assert!(error.contains("\"success\":false"), "{error}");
        assert_eq!(
            dns.call(Method::GET, "/created", None).await.unwrap(),
            json!({"id": "r9"})
        );
        assert!(dns.call(Method::GET, "/redirect", None).await.is_err());
        assert_eq!(
            dns.call(Method::GET, "/plain", None).await.unwrap_err(),
            "Cloudflare GET /plain: HTTP 502: gateway down"
        );
    }

    #[tokio::test]
    async fn a_created_record_without_an_id_is_an_error() {
        use axum::routing::post;
        let app = axum::Router::new().route(
            "/zones/z/dns_records",
            post(|| async { axum::Json(json!({"success": true, "result": {}})) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = HttpClient::new().unwrap();
        let tok = token(&"k".repeat(24));
        let base = format!("http://{addr}");
        let error = Dns::new(&http, &base, &tok)
            .add_txt("z", "x.example.org", "v")
            .await
            .unwrap_err();
        assert_eq!(error, "Cloudflare created a TXT record without an id: {}");
    }
}
