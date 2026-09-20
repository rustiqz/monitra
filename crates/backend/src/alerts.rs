//! `GET /alerts` — the global alert-history feed (DESIGN.md §5.1, ADR-009's
//! `AlertEvent`; endpoint added Phase 9 so the TUI/web Alerts screen has
//! something to read).

use axum::Json;
use axum::extract::State;
use monitra_models::MonitorStatus;
use serde::Serialize;

use crate::AppState;
use crate::error::ApiError;
use crate::service;

#[derive(Serialize)]
pub struct AlertEventDto {
    pub id: u64,
    pub monitor_id: u64,
    pub transitioned_to: MonitorStatus,
    pub occurred_at: u64,
    pub sinks_attempted: String,
    pub delivery_outcome: String,
}

impl From<monitra_models::AlertEvent> for AlertEventDto {
    fn from(event: monitra_models::AlertEvent) -> Self {
        AlertEventDto {
            id: event.id,
            monitor_id: event.monitor_id,
            transitioned_to: event.transitioned_to,
            occurred_at: event.occurred_at,
            sinks_attempted: event.sinks_attempted,
            delivery_outcome: event.delivery_outcome,
        }
    }
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<AlertEventDto>>, ApiError> {
    let events = service::list_all_alert_events(state.store.as_ref()).await?;
    Ok(Json(events.into_iter().map(Into::into).collect()))
}
