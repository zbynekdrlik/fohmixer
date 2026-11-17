//! `GET /api/listen/select?device=<listen id>` (#210): each `FohMixer
//! Listen` device asks the hub every 250 ms whether it is the one to stream
//! (Max's `maxurl` on 127.0.0.1). One UDP select port for every device
//! failed on the PC: the devices that bind one port share one socket, and
//! only one of them heard the hub's datagram, so each device asks for
//! itself.
//!
//! No token (the device has none, as its other loopback ports): a request
//! is answered only from a loopback peer with no proxy header
//! ([`from_device`]; the tunnel connects from 127.0.0.1, so the address
//! alone never decides), else 403 `LOOPBACK_ONLY`. The answer is
//! `fohmixer_proto::listen::wire::select_body`, `{"on":true}` or
//! `{"on":false}`, from the same target state the stream takes
//! (`Listen::select`); 400 `BAD_DEVICE` without a whole-number `device`;
//! 503 `LISTEN_OFF` with `[listen] port = 0`. The device takes every answer
//! that is not `on` as off.

use std::net::SocketAddr;
use std::time::Instant;

use axum::{
    extract::{ConnectInfo, Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use fohmixer_proto::listen::wire::select_body;
use serde::Deserialize;

use crate::Hub;
use crate::access::has_proxy_header;
use crate::auth::error_response;

/// The query: the device's listen id.
#[derive(Debug, Deserialize)]
pub struct SelectQuery {
    pub device: i32,
}

/// Whether a request comes from a device on this PC: a loopback peer
/// (an IPv4-mapped one too) and no proxy header.
pub fn from_device(peer: SocketAddr, headers: &HeaderMap) -> bool {
    peer.ip().to_canonical().is_loopback() && !has_proxy_header(headers)
}

/// The answer to a device: `on` or off, compact JSON.
pub fn answer(on: bool) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        select_body(on),
    )
        .into_response()
}

pub async fn get_select(
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    query: Result<Query<SelectQuery>, QueryRejection>,
) -> Response {
    if !from_device(peer, &headers) {
        tracing::info!(peer = %peer.ip(), "listen: a select asked from off the PC refused");
        return error_response(
            StatusCode::FORBIDDEN,
            "LOOPBACK_ONLY",
            "Only a Listen device on this PC asks this",
        );
    }
    let Ok(Query(query)) = query else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_DEVICE",
            "device=<listen id> is a whole number",
        );
    };
    let Some(listen) = hub.listen.as_ref() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "LISTEN_OFF",
            "Listen is off on this hub ([listen] port = 0)",
        );
    };
    answer(listen.select(query.device, Instant::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn at(ip: IpAddr) -> SocketAddr {
        SocketAddr::new(ip, 50_000)
    }

    #[test]
    fn only_a_loopback_peer_with_no_proxy_header_is_a_device() {
        let none = HeaderMap::new();
        assert!(from_device(at(IpAddr::V4(Ipv4Addr::LOCALHOST)), &none));
        assert!(from_device(at(IpAddr::V6(Ipv6Addr::LOCALHOST)), &none));
        let mapped = Ipv4Addr::LOCALHOST.to_ipv6_mapped();
        assert!(from_device(at(IpAddr::V6(mapped)), &none));
        assert!(!from_device(at("10.0.0.5".parse().unwrap()), &none), "LAN");
        let mut proxied = HeaderMap::new();
        proxied.insert("cf-connecting-ip", HeaderValue::from_static("10.0.0.5"));
        assert!(
            !from_device(at(IpAddr::V4(Ipv4Addr::LOCALHOST)), &proxied),
            "the tunnel connects from 127.0.0.1"
        );
    }

    #[tokio::test]
    async fn the_answer_is_the_patchs_compact_json() {
        let response = answer(true);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            HeaderValue::from_static("application/json")
        );
        let body = axum::body::to_bytes(response.into_body(), 64)
            .await
            .unwrap();
        assert_eq!(&body[..], br#"{"on":true}"#);
        let off = axum::body::to_bytes(answer(false).into_body(), 64)
            .await
            .unwrap();
        assert_eq!(&off[..], br#"{"on":false}"#);
    }
}
