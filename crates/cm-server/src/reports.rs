//! Overview screens: portfolio, team workload, and one person's breakdown.

use crate::work::{begin, project_json};
use crate::{ApiError, AppState, Session};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Range {
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
}

fn range(r: &Range, default_days: i64) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let to = r.to.unwrap_or_else(|| Utc::now().date_naive());
    let from = r.from.unwrap_or(to - Duration::days(default_days));
    if to < from || (to - from).num_days() > 400 {
        return Err(ApiError::bad_request("range must be 0–400 days"));
    }
    Ok((from, to))
}

pub async fn portfolio(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let projects = cm_store::work::list_projects(&mut t).await?;
    let mut rows = Vec::new();
    for p in &projects {
        rows.push(project_json(&mut t, &c, p).await?);
    }
    let stats = cm_store::flow::handoff_stats(&mut t).await?;
    t.commit().await?;
    Ok(Json(json!({ "projects": rows, "handoffs": stats })))
}

pub async fn workload(
    State(s): State<AppState>,
    session: Session,
    Query(q): Query<Range>,
) -> Result<impl IntoResponse, ApiError> {
    let (from, to) = range(&q, 27)?;
    let (mut t, _) = begin(&s, &session).await?;
    let v = cm_store::reports::workload(&mut t, from, to).await?;
    t.commit().await?;
    Ok(Json(json!({ "from": from, "to": to, "data": v })))
}

/// Anyone can see hours. Costs only reach Finance and Admins.
pub async fn person(
    State(s): State<AppState>,
    session: Session,
    Path(user): Path<Uuid>,
    Query(q): Query<Range>,
) -> Result<impl IntoResponse, ApiError> {
    let (from, to) = range(&q, 29)?;
    let (mut t, c) = begin(&s, &session).await?;
    cm_store::work::ensure_active_member(&mut t, user).await?;
    let mut v = cm_store::reports::person_breakdown(&mut t, user, from, to).await?;
    t.commit().await?;
    if !c.sees_money() {
        if let Some(rows) = v["by_project"].as_array_mut() {
            for r in rows {
                if let Some(o) = r.as_object_mut() {
                    o.remove("cost_minor");
                    o.remove("currency");
                }
            }
        }
    }
    Ok(Json(json!({ "from": from, "to": to, "data": v })))
}
