//! `/monitors` handlers and DTOs (DESIGN.md §4 `backend`).
//!
//! DTOs are deliberately separate from `monitra_models::Monitor` (§4: "the wire
//! format must be able to evolve independently of the internal domain
//! model").

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use monitra_models::{MonitorKind, MonitorStatus};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiError;
use crate::service;

#[derive(Serialize)]
pub struct MonitorDto {
    pub id: u64,
    pub name: String,
    pub target: String,
    pub kind: MonitorKind,
    pub interval_secs: u64,
    pub status: MonitorStatus,
    pub agent_id: Option<u64>,
}

impl From<monitra_models::Monitor> for MonitorDto {
    fn from(monitor: monitra_models::Monitor) -> Self {
        MonitorDto {
            id: monitor.id,
            name: monitor.name,
            target: monitor.target,
            kind: monitor.kind,
            interval_secs: monitor.interval_secs,
            status: monitor.status,
            agent_id: monitor.agent_id,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateMonitorRequest {
    pub name: String,
    pub target: String,
    pub kind: MonitorKind,
    pub interval_secs: u64,
    #[serde(default)]
    pub agent_id: Option<u64>,
}

#[derive(Deserialize)]
pub struct EditMonitorRequest {
    pub name: Option<String>,
    pub target: Option<String>,
    pub interval_secs: Option<u64>,
    pub agent_id: Option<u64>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateMonitorRequest>,
) -> Result<(StatusCode, Json<MonitorDto>), ApiError> {
    let monitor = service::add_monitor(
        state.store.as_ref(),
        body.name,
        body.target,
        body.kind,
        body.interval_secs,
        body.agent_id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(monitor.into())))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<MonitorDto>>, ApiError> {
    let monitors = service::list_monitors(state.store.as_ref()).await?;
    Ok(Json(monitors.into_iter().map(Into::into).collect()))
}

pub async fn show(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<MonitorDto>, ApiError> {
    let monitor = service::get_monitor(state.store.as_ref(), id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(monitor.into()))
}

pub async fn edit(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<EditMonitorRequest>,
) -> Result<StatusCode, ApiError> {
    service::edit_monitor(
        state.store.as_ref(),
        id,
        body.name,
        body.target,
        body.interval_secs,
        body.agent_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    service::remove_monitor(state.store.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    service::pause_monitor(state.store.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn resume(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, ApiError> {
    service::resume_monitor(state.store.as_ref(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct CheckResultDto {
    pub checked_at: u64,
    pub success: bool,
    pub latency_ms: u64,
    pub message: Option<String>,
}

impl From<monitra_models::CheckResult> for CheckResultDto {
    fn from(result: monitra_models::CheckResult) -> Self {
        CheckResultDto {
            checked_at: result.checked_at,
            success: result.success,
            latency_ms: result.latency_ms,
            message: result.message,
        }
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// Unix seconds — only results at or after this time. `None` returns
    /// everything retained (§5.4 pruning is what bounds this, not the API).
    pub since: Option<u64>,
}

pub async fn history(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<CheckResultDto>>, ApiError> {
    let results = service::monitor_history(state.store.as_ref(), id, query.since).await?;
    Ok(Json(results.into_iter().map(Into::into).collect()))
}
