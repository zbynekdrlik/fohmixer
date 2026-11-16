//! The LUFS history's API (#219, the design on the ticket), read only and
//! a token's: an MBC login reaches only its own Live set's points
//! (`Role::reaches`), as everywhere else.
//!
//! - `GET /api/lufs/history?instance=&target=&from=&to=` (UTC ms, at most
//!   24 h): `HistorySeries`, bucketed to at most 900 values; 400
//!   `BAD_WINDOW` for a window that ends before it starts or is too long.
//! - `GET /api/lufs/days?instance=&target=`: `HistoryDays`, newest first.
//!
//! Both read the day files on the blocking pool, never on the runtime.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use fohmixer_proto::lufs::{HistoryDays, HistorySeries};
use fohmixer_proto::roles::Role;
use serde::Deserialize;

use super::role::role_forbidden;
use crate::Hub;
use crate::auth::{Rejection, error_response};

/// The query of `GET /api/lufs/history`.
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub instance: String,
    pub target: String,
    pub from: i64,
    pub to: i64,
}

/// The query of `GET /api/lufs/days`.
#[derive(Debug, Deserialize)]
pub struct DaysQuery {
    pub instance: String,
    pub target: String,
}

/// The token's role, refused 403 `ROLE` for a point of a Live set it does
/// not reach.
fn reach(hub: &Hub, headers: &HeaderMap, instance: &str) -> Result<Role, Rejection> {
    let (_, role) = hub.auth.require(headers)?;
    if role.reaches(instance) {
        Ok(role)
    } else {
        Err(Rejection::from(role_forbidden()))
    }
}

/// The 500 of a read whose task died.
fn read_failed() -> Rejection {
    Rejection::from(error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "READ_FAILED",
        "the LUFS history could not be read",
    ))
}

/// `GET /api/lufs/history`.
pub async fn get_history(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistorySeries>, Rejection> {
    reach(&hub, &headers, &query.instance)?;
    let history = hub.lufs_history.clone();
    let series = tokio::task::spawn_blocking(move || {
        history.history(&query.instance, &query.target, query.from, query.to)
    })
    .await
    .map_err(|_| read_failed())?;
    series.map(Json).map_err(|why| {
        Rejection::from(error_response(
            StatusCode::BAD_REQUEST,
            "BAD_WINDOW",
            why.text(),
        ))
    })
}

/// `GET /api/lufs/days`.
pub async fn get_days(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Query(query): Query<DaysQuery>,
) -> Result<Json<HistoryDays>, Rejection> {
    reach(&hub, &headers, &query.instance)?;
    let history = hub.lufs_history.clone();
    let days = tokio::task::spawn_blocking(move || history.days(&query.instance, &query.target))
        .await
        .map_err(|_| read_failed())?;
    Ok(Json(HistoryDays { days }))
}
