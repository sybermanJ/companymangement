//! Read models for the overview screens: who works on what, and where time goes.
#![allow(clippy::type_complexity)]

use crate::{Result, TenantTx};
use chrono::NaiveDate;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct WorkloadCell {
    pub user_id: Uuid,
    pub week: NaiveDate,
    pub bucket: String,
    pub minutes: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PersonRow {
    pub user_id: Uuid,
    pub name: String,
    pub departments: Option<String>,
    pub open_items: i64,
    pub open_estimate_minutes: i64,
}

/// Logged minutes per person × ISO week × project (or "routine"/"request"
/// for work outside projects), plus each person's open load.
pub async fn workload(t: &mut TenantTx, from: NaiveDate, to: NaiveDate) -> Result<Value> {
    let cells: Vec<WorkloadCell> = sqlx::query_as::<_, WorkloadCell>(
        "SELECT l.user_id, date_trunc('week', l.started_at)::date AS week,
                COALESCE(p.key, w.kind) AS bucket, sum(l.minutes)::bigint AS minutes
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id LEFT JOIN project p ON p.id = w.project_id
         WHERE l.started_at::date BETWEEN $1 AND $2
         GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *t.tx)
    .await?;
    let people: Vec<PersonRow> = sqlx::query_as::<_, PersonRow>(
        "SELECT m.user_id, u.display_name AS name,
                (SELECT string_agg(DISTINCT d.name, ' · ') FROM user_role ur JOIN department d ON d.id = ur.department_id
                  WHERE ur.user_id = m.user_id AND (ur.valid_to IS NULL OR ur.valid_to >= current_date)) AS departments,
                (SELECT count(*) FROM work_item w WHERE w.holder_user_id = m.user_id AND w.status NOT IN ('done', 'cancelled') AND w.kind NOT IN ('phase', 'milestone')) AS open_items,
                (SELECT COALESCE(sum(GREATEST(COALESCE(w.estimate_minutes, 0) - COALESCE((SELECT sum(minutes) FROM work_log l WHERE l.work_item_id = w.id), 0), 0)), 0)
                   FROM work_item w WHERE w.holder_user_id = m.user_id AND w.status NOT IN ('done', 'cancelled'))::bigint AS open_estimate_minutes
         FROM membership m JOIN app_user u ON u.id = m.user_id
         WHERE m.tenant_id = $1 AND m.status = 'active' ORDER BY departments NULLS LAST, u.display_name",
    )
    .bind(t.tenant_id)
    .fetch_all(&mut *t.tx)
    .await?;
    let current: Vec<(Uuid, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT w.holder_user_id, w.key, w.title, p.key FROM work_item w LEFT JOIN project p ON p.id = w.project_id
         WHERE w.status = 'in_progress' AND w.holder_user_id IS NOT NULL ORDER BY w.updated_at DESC",
    )
    .fetch_all(&mut *t.tx)
    .await?;
    let now: Vec<Value> = current
        .into_iter()
        .map(|(u, k, title, p)| json!({ "user_id": u, "key": k, "title": title, "project_key": p }))
        .collect();
    Ok(json!({ "people": people, "cells": cells, "in_progress": now }))
}

/// One person's time across everything in a range: by project, by item kind, by day.
pub async fn person_breakdown(
    t: &mut TenantTx,
    user: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Value> {
    let by_project: Vec<(Option<Uuid>, Option<String>, Option<String>, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT p.id, p.key, p.name, sum(l.minutes)::bigint, COALESCE(sum(l.cost_minor), 0)::bigint, max(l.currency::text)
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id LEFT JOIN project p ON p.id = w.project_id
         WHERE l.user_id = $1 AND l.started_at::date BETWEEN $2 AND $3 GROUP BY p.id, p.key, p.name ORDER BY 4 DESC",
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *t.tx)
    .await?;
    let by_kind: Vec<(String, i64)> = sqlx::query_as(
        "SELECT CASE WHEN w.kind IN ('routine', 'request') THEN w.kind ELSE 'project' END, sum(l.minutes)::bigint
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id
         WHERE l.user_id = $1 AND l.started_at::date BETWEEN $2 AND $3 GROUP BY 1 ORDER BY 2 DESC",
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *t.tx)
    .await?;
    let by_day: Vec<(NaiveDate, i64)> = sqlx::query_as(
        "SELECT l.started_at::date, sum(l.minutes)::bigint FROM work_log l
         WHERE l.user_id = $1 AND l.started_at::date BETWEEN $2 AND $3 GROUP BY 1 ORDER BY 1",
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *t.tx)
    .await?;
    let items: Vec<(Option<String>, String, i64)> = sqlx::query_as(
        "SELECT w.key, w.title, sum(l.minutes)::bigint FROM work_log l JOIN work_item w ON w.id = l.work_item_id
         WHERE l.user_id = $1 AND l.started_at::date BETWEEN $2 AND $3 GROUP BY w.id, w.key, w.title ORDER BY 3 DESC LIMIT 30",
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *t.tx)
    .await?;
    Ok(json!({
        "by_project": by_project.into_iter().map(|(id, k, n, m, c, ccy)| json!({ "project_id": id, "key": k, "name": n, "minutes": m, "cost_minor": c, "currency": ccy })).collect::<Vec<_>>(),
        "by_kind": by_kind.into_iter().map(|(k, m)| json!({ "kind": k, "minutes": m })).collect::<Vec<_>>(),
        "by_day": by_day.into_iter().map(|(d, m)| json!({ "day": d, "minutes": m })).collect::<Vec<_>>(),
        "items": items.into_iter().map(|(k, title, m)| json!({ "key": k, "title": title, "minutes": m })).collect::<Vec<_>>(),
    }))
}
