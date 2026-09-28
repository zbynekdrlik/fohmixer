//! The hub's own HTTP client (#17): the Access signing keys, the Cloudflare
//! DNS API and cloudflared's readiness. HTTPS checks the server against the
//! platform's roots (the Windows certificate store on the PC); plain http is
//! only ever used for a loopback URL (`config::url_allowed` decides what a
//! config may name; the Cloudflare API is a constant https URL). Every
//! exchange is bounded in time and in size, and an error names the URL,
//! never a token.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use axum::http::{Method, Request, header};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::de::DeserializeOwned;

/// How long one exchange (the request and the whole answer) may take.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// The largest answer read (a key set or an API answer is a few kB): 1 MiB.
pub const MAX_BODY: usize = 1_048_576;

/// The `User-Agent` of every request.
fn user_agent() -> String {
    format!("fohmixer-hub/{}", fohmixer_proto::VERSION)
}

/// An answer: the status and the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Bytes,
}

impl Reply {
    /// The body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        serde_json::from_slice(&self.body).context("the answer is not the expected JSON")
    }

    /// The first 200 characters of the body, for an error message.
    pub fn snippet(&self) -> String {
        String::from_utf8_lossy(&self.body)
            .chars()
            .take(200)
            .collect()
    }
}

/// A client, cheap to clone.
#[derive(Clone)]
pub struct HttpClient {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    timeout: Duration,
}

impl HttpClient {
    /// HTTPS verified against the platform's roots, and http.
    pub fn new() -> anyhow::Result<Self> {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .try_with_platform_verifier()
            .context("loading the platform's certificate roots")?
            .https_or_http()
            .enable_http1()
            .build();
        Ok(Self::with(https))
    }

    /// HTTPS verified against `roots` only (tests: a test CA), and http.
    pub fn with_roots(roots: rustls::RootCertStore) -> Self {
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring offers the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(config)
            .https_or_http()
            .enable_http1()
            .build();
        Self::with(https)
    }

    fn with(https: HttpsConnector<HttpConnector>) -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build(https),
            timeout: TIMEOUT,
        }
    }

    /// The same client with another time bound (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `GET url`.
    pub async fn get(&self, url: &str) -> anyhow::Result<Reply> {
        self.send(Method::GET, url, None, None).await
    }

    /// One exchange: `method url`, a bearer token when given, a JSON body
    /// when given. Any status is an answer; a failed connection, a timeout or
    /// an answer over [`MAX_BODY`] is an error naming the URL.
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        bearer: Option<&str>,
        json: Option<&serde_json::Value>,
    ) -> anyhow::Result<Reply> {
        let mut request = Request::builder()
            .method(method)
            .uri(url)
            .header(header::USER_AGENT, user_agent())
            .header(header::ACCEPT, "application/json");
        if let Some(token) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = match json {
            Some(value) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Bytes::from(serde_json::to_vec(value)?)
            }
            None => Bytes::new(),
        };
        let request = request
            .body(Full::new(body))
            .with_context(|| format!("a request to {url}"))?;
        let exchange = async {
            let response = self.client.request(request).await?;
            let status = response.status().as_u16();
            let body = Limited::new(response.into_body(), MAX_BODY)
                .collect()
                .await
                .map_err(|e| anyhow!("reading the answer: {e}"))?
                .to_bytes();
            anyhow::Ok(Reply { status, body })
        };
        tokio::time::timeout(self.timeout, exchange)
            .await
            .map_err(|_| anyhow!("no answer within {} ms", self.timeout.as_millis()))
            .and_then(|answer| answer)
            .with_context(|| url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::HeaderMap;
    use axum::routing::{any, get};

    /// A server on 127.0.0.1 that echoes what it received, answers 418 at
    /// `/teapot`, sends 2 MiB at `/big` and never answers `/slow`.
    async fn server() -> String {
        async fn echo(
            method: Method,
            headers: HeaderMap,
            body: Bytes,
        ) -> axum::Json<serde_json::Value> {
            let h = |name: header::HeaderName| {
                headers.get(name).map(|v| v.to_str().unwrap().to_string())
            };
            axum::Json(serde_json::json!({
                "method": method.as_str(),
                "user_agent": h(header::USER_AGENT),
                "accept": h(header::ACCEPT),
                "authorization": h(header::AUTHORIZATION),
                "content_type": h(header::CONTENT_TYPE),
                "body": String::from_utf8(body.to_vec()).unwrap(),
            }))
        }
        let app = Router::new()
            .route("/echo", any(echo))
            .route(
                "/teapot",
                get(|| async { (axum::http::StatusCode::IM_A_TEAPOT, "short and stout") }),
            )
            .route("/big", get(|| async { vec![b'x'; 2 * MAX_BODY] }))
            .route("/slow", get(std::future::pending::<&'static str>));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_get_carries_the_agent_and_no_token() {
        let base = server().await;
        let reply = HttpClient::new()
            .unwrap()
            .get(&format!("{base}/echo"))
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        let seen: serde_json::Value = reply.json().unwrap();
        assert_eq!(seen["method"], "GET");
        assert_eq!(
            seen["user_agent"],
            format!("fohmixer-hub/{}", fohmixer_proto::VERSION)
        );
        assert_eq!(seen["accept"], "application/json");
        assert_eq!(seen["authorization"], serde_json::Value::Null);
        assert_eq!(seen["content_type"], serde_json::Value::Null);
        assert_eq!(seen["body"], "");
    }

    #[tokio::test]
    async fn a_send_carries_the_bearer_token_and_the_json_body() {
        let base = server().await;
        let body = serde_json::json!({"type": "TXT"});
        let reply = HttpClient::new()
            .unwrap()
            .send(
                Method::POST,
                &format!("{base}/echo"),
                Some("tok"),
                Some(&body),
            )
            .await
            .unwrap();
        let seen: serde_json::Value = reply.json().unwrap();
        assert_eq!(seen["method"], "POST");
        assert_eq!(seen["authorization"], "Bearer tok");
        assert_eq!(seen["content_type"], "application/json");
        assert_eq!(seen["body"], r#"{"type":"TXT"}"#);
    }

    #[tokio::test]
    async fn any_status_is_an_answer_with_its_body() {
        let base = server().await;
        let reply = HttpClient::new()
            .unwrap()
            .get(&format!("{base}/teapot"))
            .await
            .unwrap();
        assert_eq!(reply.status, 418);
        assert_eq!(reply.snippet(), "short and stout");
        assert!(reply.json::<serde_json::Value>().is_err());
    }

    #[tokio::test]
    async fn failures_are_errors_naming_the_url() {
        let base = server().await;
        let client = HttpClient::new()
            .unwrap()
            .with_timeout(Duration::from_millis(300));
        let slow = format!("{base}/slow");
        let error = format!("{:#}", client.get(&slow).await.unwrap_err());
        assert!(error.starts_with(&slow), "{error}");
        assert!(error.contains("no answer within 300 ms"), "{error}");
        let big = format!("{base}/big");
        let error = format!("{:#}", client.get(&big).await.unwrap_err());
        assert!(error.contains("reading the answer"), "{error}");
        let error = format!(
            "{:#}",
            client.get("http://127.0.0.1:1/x").await.unwrap_err()
        );
        assert!(error.starts_with("http://127.0.0.1:1/x"), "{error}");
        let error = format!("{:#}", client.get("not a url").await.unwrap_err());
        assert!(error.contains("a request to not a url"), "{error}");
    }

    #[tokio::test]
    async fn an_answer_of_exactly_the_limit_is_read() {
        let app = Router::new().route("/limit", get(|| async { vec![b'y'; MAX_BODY] }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let reply = HttpClient::new()
            .unwrap()
            .get(&format!("http://{addr}/limit"))
            .await
            .unwrap();
        assert_eq!(reply.body.len(), 1_048_576);
        assert_eq!(reply.snippet().len(), 200);
    }
}
