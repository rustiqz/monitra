//! `GET /regions` — per-target regional latency/failure-rate comparison
//! (DESIGN.md §4 `backend`, ADR-011, Phase 11). Human-token authenticated,
//! same as every other read here (`auth::require_token`). The aggregation
//! itself lives in `service::region_aggregates`, shared with `monitra
//! monitor regions` (ADR-009 — "a query is never implemented twice" applies
//! just as much as the mutation version).

use axum::Json;
use axum::extract::{Query, State};
use monitra_models::MonitorKind;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::ApiError;
use crate::service;

#[derive(Deserialize)]
pub struct RegionsQuery {
    /// Unix seconds — only `CheckResult`s at or after this time feed the
    /// aggregate. Omitted means unbounded, same as `/monitors/{id}/history`.
    pub since: Option<u64>,
}

#[derive(Serialize)]
pub struct RegionAggregateDto {
    pub target: String,
    pub kind: MonitorKind,
    pub region: String,
    pub monitor_count: usize,
    pub probe_count: usize,
    pub failure_count: usize,
    pub p95_latency_ms: u64,
}

impl From<service::RegionAggregate> for RegionAggregateDto {
    fn from(aggregate: service::RegionAggregate) -> Self {
        RegionAggregateDto {
            target: aggregate.target,
            kind: aggregate.kind,
            region: aggregate.region,
            monitor_count: aggregate.monitor_count,
            probe_count: aggregate.probe_count,
            failure_count: aggregate.failure_count,
            p95_latency_ms: aggregate.p95_latency_ms,
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<RegionsQuery>,
) -> Result<Json<Vec<RegionAggregateDto>>, ApiError> {
    let aggregates = service::region_aggregates(state.store.as_ref(), query.since).await?;
    Ok(Json(aggregates.into_iter().map(Into::into).collect()))
}
