//! The API routes by role (#184, spec §6.4): one table decides every route
//! ([`route_role`], [`API_PATHS`]), [`role_gate`] enforces it on every
//! matched API route (a route with no decision is refused to everyone), and
//! [`status_for`] is the status an MBC login is served.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use fohmixer_proto::client::{HubStatus, LayoutStatus};
use fohmixer_proto::roles::{MBC_INSTANCE, Role};

use crate::Hub;
use crate::auth::{bearer, error_response};

/// Who may call an API route (#184).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAccess {
    /// No token: the version, the pages' reports, the login.
    Open,
    /// Any login; the handler serves a role its own view.
    AnyRole,
    /// The engineer's login only: an MBC token gets 403 `ROLE`.
    EngineerOnly,
}

/// Every path [`api_routes`] registers, each with its decision in
/// [`route_role`] (a test holds the two together).
pub const API_PATHS: &[&str] = &[
    "/api/version",
    "/api/client-error",
    "/api/client-report",
    "/api/auth",
    "/api/layout",
    "/api/status",
    "/api/markers/migration",
    "/api/eq/picture",
    "/api/songmap",
    "/ws",
];

/// The role decision of an API route (none: a route with no decision,
/// which [`role_gate`] refuses to everyone).
pub fn route_role(path: &str) -> Option<RouteAccess> {
    match path {
        "/api/version" | "/api/client-error" | "/api/client-report" | "/api/auth" => {
            Some(RouteAccess::Open)
        }
        "/api/layout" | "/api/status" | "/ws" => Some(RouteAccess::AnyRole),
        "/api/markers/migration" | "/api/eq/picture" | "/api/songmap" => {
            Some(RouteAccess::EngineerOnly)
        }
        _ => None,
    }
}

/// Whether a request with a token of `role` (none: no valid token, which
/// the handler answers 401) is refused before its handler.
pub fn refused(access: Option<RouteAccess>, role: Option<Role>) -> bool {
    match access {
        None => true,
        Some(RouteAccess::Open | RouteAccess::AnyRole) => false,
        Some(RouteAccess::EngineerOnly) => role == Some(Role::Mbc),
    }
}

/// The 403 of a refused route.
pub fn role_forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "ROLE", "Not allowed for this login")
}

/// The role check of every matched API route ([`route_role`]).
pub async fn role_gate(
    State(hub): State<Hub>,
    path: MatchedPath,
    request: Request,
    next: Next,
) -> Response {
    let access = route_role(path.as_str());
    let role = hub.auth.role(bearer(request.headers()));
    if !refused(access, role) {
        return next.run(request).await;
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
        .unwrap_or_else(|| "-".to_string());
    match access {
        None => tracing::error!(
            path = path.as_str(),
            "an API route without a role decision refused"
        ),
        Some(_) => tracing::info!(
            role = role.map_or("-", Role::as_str),
            path = path.as_str(),
            peer = %peer,
            "role refused"
        ),
    }
    role_forbidden()
}

/// The status an MBC login is served: its Live set only, the layout's
/// revision and that set's unresolved bindings, and nothing of the FOH
/// side (every other field empty). The engineer's is the whole status.
pub fn status_for(role: Role, status: HubStatus) -> HubStatus {
    match role {
        Role::Engineer => status,
        Role::Mbc => HubStatus {
            instances: status
                .instances
                .into_iter()
                .filter(|i| i.name == MBC_INSTANCE)
                .collect(),
            layout: LayoutStatus {
                rev: status.layout.rev,
                error: None,
                unresolved: status
                    .layout
                    .unresolved
                    .into_iter()
                    .filter(|u| u.instance == MBC_INSTANCE)
                    .collect(),
                markers: Default::default(),
            },
            stage_aut: Default::default(),
            clients: 0,
            remote: Default::default(),
            client_reports: Vec::new(),
            companion: None,
            ableset: None,
            presenter: None,
            reverb: Default::default(),
            fader_links: Vec::new(),
            autotune: Default::default(),
            songmap: Vec::new(),
            lufs: Default::default(),
            sectors: Default::default(),
            colors: Default::default(),
            mute_alarms: Default::default(),
        },
    }
}

#[cfg(test)]
mod tests;
