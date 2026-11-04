//! Hourly rates with history, exchange rates, cost frozen on each work log,
//! timesheet periods, cost thresholds and alerts.
//!
//! Money is only ever read by Finance and Admins; the server enforces that.
//! Costs are frozen when a log is saved (rate card effective on the work date,
//! FX rate effective on the work date). Later rate changes never silently
//! rewrite history: missing values are filled in, and changing existing
//! values needs an explicit, audited recalculation of open periods.

use crate::flow::{admin_people, finance_people, notify};
use crate::work::get_project;
use crate::{Result, StoreError, TenantTx};
use chrono::{DateTime, NaiveDate, Utc};
use cm_domain::threshold::{self, Evaluation, ThresholdRule};
use cm_domain::{fx, money, Currency, Money};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub async fn audit(t: &mut TenantTx, actor: Uuid, action: &str, payload: Value) -> Result<()> {
    sqlx::query("INSERT INTO audit_log (id, tenant_id, actor_id, action, payload) VALUES ($1, $2, $3, $4, $5)")
        .bind(Uuid::now_v7())
        .bind(t.tenant_id)
        .bind(actor)
        .bind(action)
        .bind(payload)
        .execute(&mut *t.tx)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AuditRow {
    pub id: Uuid,
    pub actor_name: String,
    pub action: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub async fn list_audit(t: &mut TenantTx) -> Result<Vec<AuditRow>> {
    Ok(sqlx::query_as::<_, AuditRow>(
        "SELECT a.id, u.display_name AS actor_name, a.action, a.payload, a.created_at
         FROM audit_log a JOIN app_user u ON u.id = a.actor_id ORDER BY a.created_at DESC LIMIT 200",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Format minor units for messages, e.g. `USD 41,200.00`.
pub fn fmt_money(minor: i64, ccy: &str) -> String {
    let exp = Currency::parse(ccy)
        .map(|c| c.minor_exponent())
        .unwrap_or(2);
    let neg = minor < 0;
    let m = minor.unsigned_abs();
    let div = 10_u64.pow(exp);
    let major = (m / div).to_string();
    let mut grouped = String::new();
    for (i, ch) in major.chars().enumerate() {
        if i > 0 && (major.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let frac = if exp > 0 {
        format!(".{:0width$}", m % div, width = exp as usize)
    } else {
        String::new()
    };
    format!("{ccy} {}{grouped}{frac}", if neg { "-" } else { "" })
}

// ---------- rate cards ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RateRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_name: String,
    pub rate_minor: i64,
    pub currency: String,
    pub effective_from: NaiveDate,
    pub note: String,
    pub set_by_name: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_rates(t: &mut TenantTx) -> Result<Vec<RateRow>> {
    Ok(sqlx::query_as::<_, RateRow>(
        "SELECT r.id, r.user_id, u.display_name AS user_name, r.rate_minor, r.currency, r.effective_from, r.note,
                s.display_name AS set_by_name, r.created_at
         FROM rate_card r JOIN app_user u ON u.id = r.user_id JOIN app_user s ON s.id = r.set_by
         ORDER BY u.display_name, r.effective_from DESC",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn add_rate(
    t: &mut TenantTx,
    actor: Uuid,
    user: Uuid,
    rate_minor: i64,
    currency: &str,
    effective_from: NaiveDate,
    note: &str,
) -> Result<(Uuid, u64)> {
    crate::work::ensure_active_member(t, user).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO rate_card (id, tenant_id, user_id, rate_minor, currency, effective_from, note, set_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(user)
    .bind(rate_minor)
    .bind(currency)
    .bind(effective_from)
    .bind(note)
    .bind(actor)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| crate::map_unique(e, "a rate for that person and date"))?;
    audit(
        t,
        actor,
        "rate.set",
        json!({ "user": user, "rate_minor": rate_minor, "currency": currency, "effective_from": effective_from }),
    )
    .await?;
    // Fill entries that had no rate yet (never overwrite frozen costs).
    let ids: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM work_log WHERE user_id = $1 AND rate_minor IS NULL AND NOT locked AND started_at::date >= $2",
    )
    .bind(user)
    .bind(effective_from)
    .fetch_all(&mut *t.tx)
    .await?;
    let n = ids.len() as u64;
    for (l,) in ids {
        freeze_cost(t, l).await?;
    }
    if n > 0 {
        evaluate_all(t).await?;
    }
    Ok((id, n))
}

/// Affected entries if a rate effective from `from` were changed now: open
/// entries that already have a frozen cost from that date on.
pub async fn affected_by_backdate(
    t: &mut TenantTx,
    user: Option<Uuid>,
    from: NaiveDate,
) -> Result<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM work_log WHERE NOT locked AND started_at::date >= $1 AND ($2::uuid IS NULL OR user_id = $2)
           AND cost_minor IS NOT NULL",
    )
    .bind(from)
    .bind(user)
    .fetch_one(&mut *t.tx)
    .await?;
    Ok(n)
}

// ---------- exchange rates ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct FxRow {
    pub id: Uuid,
    pub from_ccy: String,
    pub to_ccy: String,
    pub rate: Decimal,
    pub effective_date: NaiveDate,
    pub set_by_name: String,
}

pub async fn list_fx(t: &mut TenantTx) -> Result<Vec<FxRow>> {
    Ok(sqlx::query_as::<_, FxRow>(
        "SELECT f.id, f.from_ccy, f.to_ccy, f.rate, f.effective_date, u.display_name AS set_by_name
         FROM fx_rate f JOIN app_user u ON u.id = f.set_by ORDER BY f.effective_date DESC, f.from_ccy, f.to_ccy",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn add_fx(
    t: &mut TenantTx,
    actor: Uuid,
    from: &str,
    to: &str,
    rate: Decimal,
    on: NaiveDate,
) -> Result<(Uuid, u64)> {
    if rate <= Decimal::ZERO {
        return Err(StoreError::Invalid("rate must be positive"));
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO fx_rate (id, tenant_id, from_ccy, to_ccy, rate, effective_date, set_by) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(from)
    .bind(to)
    .bind(rate)
    .bind(on)
    .bind(actor)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| crate::map_unique(e, "a rate for that pair and date"))?;
    audit(
        t,
        actor,
        "fx.set",
        json!({ "from": from, "to": to, "rate": rate.to_string(), "date": on }),
    )
    .await?;
    // Convert entries that were waiting for this pair.
    let ids: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM work_log WHERE NOT locked AND cost_minor IS NOT NULL AND project_cost_minor IS NULL
           AND ((currency = $1 AND project_currency = $2) OR (currency = $2 AND project_currency = $1))
           AND started_at::date >= $3",
    )
    .bind(from)
    .bind(to)
    .bind(on)
    .fetch_all(&mut *t.tx)
    .await?;
    let n = ids.len() as u64;
    for (l,) in ids {
        freeze_cost(t, l).await?;
    }
    if n > 0 {
        evaluate_all(t).await?;
    }
    Ok((id, n))
}

/// Pairs and dates that block cost conversion.
pub async fn pending_fx(t: &mut TenantTx) -> Result<Vec<Value>> {
    let rows: Vec<(String, String, NaiveDate, i64)> = sqlx::query_as(
        "SELECT currency, project_currency, min(started_at::date), count(*) FROM work_log
         WHERE cost_minor IS NOT NULL AND project_cost_minor IS NULL AND project_currency IS NOT NULL
         GROUP BY currency, project_currency ORDER BY 3",
    )
    .fetch_all(&mut *t.tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(f, to, d, n)| json!({ "from": f, "to": to, "earliest": d, "entries": n }))
        .collect())
}

/// Entries with no hourly rate for the person on that date.
pub async fn missing_rates(t: &mut TenantTx) -> Result<Vec<Value>> {
    let rows: Vec<(Uuid, String, NaiveDate, i64)> = sqlx::query_as(
        "SELECT l.user_id, u.display_name, min(l.started_at::date), count(*) FROM work_log l JOIN app_user u ON u.id = l.user_id
         WHERE l.rate_minor IS NULL GROUP BY l.user_id, u.display_name ORDER BY 2",
    )
    .fetch_all(&mut *t.tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, d, n)| json!({ "user_id": id, "user_name": name, "earliest": d, "entries": n }))
        .collect())
}

async fn fx_on(
    t: &mut TenantTx,
    from: &str,
    to: &str,
    on: NaiveDate,
) -> Result<Option<(Uuid, Decimal)>> {
    let direct: Option<(Uuid, Decimal)> = sqlx::query_as(
        "SELECT id, rate FROM fx_rate WHERE from_ccy = $1 AND to_ccy = $2 AND effective_date <= $3
         ORDER BY effective_date DESC LIMIT 1",
    )
    .bind(from)
    .bind(to)
    .bind(on)
    .fetch_optional(&mut *t.tx)
    .await?;
    if direct.is_some() {
        return Ok(direct);
    }
    let inverse: Option<(Uuid, Decimal)> = sqlx::query_as(
        "SELECT id, rate FROM fx_rate WHERE from_ccy = $2 AND to_ccy = $1 AND effective_date <= $3
         ORDER BY effective_date DESC LIMIT 1",
    )
    .bind(from)
    .bind(to)
    .bind(on)
    .fetch_optional(&mut *t.tx)
    .await?;
    Ok(inverse.and_then(|(id, r)| Decimal::ONE.checked_div(r).map(|inv| (id, inv))))
}

/// Freeze (or refreeze, for open entries) the cost of one work log.
pub async fn freeze_cost(t: &mut TenantTx, log: Uuid) -> Result<()> {
    let row: Option<(Uuid, NaiveDate, i32, bool, Option<String>)> = sqlx::query_as(
        "SELECT l.user_id, l.started_at::date, l.minutes, l.locked, p.currency::text
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id LEFT JOIN project p ON p.id = w.project_id
         WHERE l.id = $1",
    )
    .bind(log)
    .fetch_optional(&mut *t.tx)
    .await?;
    let (user, day, minutes, locked, project_ccy) = row.ok_or(StoreError::NotFound)?;
    if locked {
        return Ok(());
    }
    let rate: Option<(Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, rate_minor, currency::text FROM rate_card WHERE user_id = $1 AND effective_from <= $2
         ORDER BY effective_from DESC LIMIT 1",
    )
    .bind(user)
    .bind(day)
    .fetch_optional(&mut *t.tx)
    .await?;
    let (rate_id, rate_minor, cost, ccy, fx_id, project_cost) = match rate {
        None => (None, None, None, None, None, None),
        Some((rid, rminor, rccy)) => {
            let c = Currency::parse(&rccy).map_err(|_| StoreError::Invalid("bad currency"))?;
            let cost =
                money::labour_cost(u32::try_from(minutes).unwrap_or(0), Money::new(rminor, c))
                    .map_err(|_| StoreError::Invalid("cost overflow"))?;
            let (fx_id, pc) = match &project_ccy {
                None => (None, None),
                Some(pccy) if *pccy == rccy => (None, Some(cost.minor)),
                Some(pccy) => {
                    let to =
                        Currency::parse(pccy).map_err(|_| StoreError::Invalid("bad currency"))?;
                    match fx_on(t, &rccy, pccy, day).await? {
                        Some((fid, r)) => (
                            Some(fid),
                            Some(
                                fx::convert(cost, to, r)
                                    .map_err(|_| StoreError::Invalid("conversion failed"))?
                                    .minor,
                            ),
                        ),
                        None => (None, None),
                    }
                }
            };
            (
                Some(rid),
                Some(rminor),
                Some(cost.minor),
                Some(rccy),
                fx_id,
                pc,
            )
        }
    };
    sqlx::query(
        "UPDATE work_log SET rate_card_id = $2, rate_minor = $3, cost_minor = $4, currency = $5,
                project_currency = $6, fx_rate_id = $7, project_cost_minor = $8
         WHERE id = $1",
    )
    .bind(log)
    .bind(rate_id)
    .bind(rate_minor)
    .bind(cost)
    .bind(ccy)
    .bind(project_ccy)
    .bind(fx_id)
    .bind(project_cost)
    .execute(&mut *t.tx)
    .await?;
    Ok(())
}

/// Explicit, audited recalculation of every open entry from a date on (after
/// a backdated rate change). Closed periods never change.
pub async fn recalculate(t: &mut TenantTx, actor: Uuid, from: NaiveDate) -> Result<u64> {
    let ids: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM work_log WHERE NOT locked AND started_at::date >= $1")
            .bind(from)
            .fetch_all(&mut *t.tx)
            .await?;
    for (l,) in &ids {
        freeze_cost(t, *l).await?;
    }
    audit(
        t,
        actor,
        "cost.recalculate",
        json!({ "from": from, "entries": ids.len() }),
    )
    .await?;
    evaluate_all(t).await?;
    Ok(ids.len() as u64)
}

// ---------- timesheet periods ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PeriodRow {
    pub id: Uuid,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
    pub status: String,
    pub closed_by_name: Option<String>,
    pub closed_at: Option<DateTime<Utc>>,
    pub entries: i64,
}

pub async fn list_periods(t: &mut TenantTx) -> Result<Vec<PeriodRow>> {
    Ok(sqlx::query_as::<_, PeriodRow>(
        "SELECT p.id, p.starts_on, p.ends_on, p.status, u.display_name AS closed_by_name, p.closed_at,
                (SELECT count(*) FROM work_log l WHERE l.started_at::date BETWEEN p.starts_on AND p.ends_on) AS entries
         FROM timesheet_period p LEFT JOIN app_user u ON u.id = p.closed_by ORDER BY p.starts_on DESC",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn in_closed_period(t: &mut TenantTx, day: NaiveDate) -> Result<bool> {
    let r: Option<(bool,)> = sqlx::query_as(
        "SELECT true FROM timesheet_period WHERE status = 'closed' AND $1 BETWEEN starts_on AND ends_on LIMIT 1",
    )
    .bind(day)
    .fetch_optional(&mut *t.tx)
    .await?;
    Ok(r.is_some())
}

/// Close a period: entries in it are locked and their costs never change again.
pub async fn close_period(
    t: &mut TenantTx,
    actor: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<u64> {
    if to < from {
        return Err(StoreError::Invalid("end date is before start date"));
    }
    let overlap: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM timesheet_period WHERE status = 'closed' AND starts_on <= $2 AND ends_on >= $1 LIMIT 1",
    )
    .bind(from)
    .bind(to)
    .fetch_optional(&mut *t.tx)
    .await?;
    if overlap.is_some() {
        return Err(StoreError::Conflict(
            "a closed period overlapping these dates",
        ));
    }
    sqlx::query(
        "INSERT INTO timesheet_period (id, tenant_id, starts_on, ends_on, status, closed_by, closed_at)
         VALUES ($1, $2, $3, $4, 'closed', $5, now())",
    )
    .bind(Uuid::now_v7())
    .bind(t.tenant_id)
    .bind(from)
    .bind(to)
    .bind(actor)
    .execute(&mut *t.tx)
    .await?;
    let r = sqlx::query(
        "UPDATE work_log SET locked = true WHERE NOT locked AND started_at::date BETWEEN $1 AND $2",
    )
    .bind(from)
    .bind(to)
    .execute(&mut *t.tx)
    .await?;
    audit(
        t,
        actor,
        "period.close",
        json!({ "from": from, "to": to, "entries": r.rows_affected() }),
    )
    .await?;
    Ok(r.rows_affected())
}

// ---------- project cost ----------

#[derive(Debug, Clone, Serialize)]
pub struct ProjectCost {
    pub currency: String,
    pub budget_minor: Option<i64>,
    pub spent_minor: i64,
    pub forecast_minor: i64,
    pub used_pct: Option<u32>,
    pub costed_minutes: i64,
    pub uncosted_entries: i64,
    pub remaining_minutes: i64,
}

pub async fn project_cost(t: &mut TenantTx, project: Uuid) -> Result<ProjectCost> {
    let p = get_project(t, project).await?;
    let (spent, costed_minutes, uncosted): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(sum(l.project_cost_minor), 0)::bigint,
                COALESCE(sum(l.minutes) FILTER (WHERE l.project_cost_minor IS NOT NULL), 0)::bigint,
                count(*) FILTER (WHERE l.project_cost_minor IS NULL)
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id WHERE w.project_id = $1",
    )
    .bind(project)
    .fetch_one(&mut *t.tx)
    .await?;
    let (remaining,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(sum(GREATEST(COALESCE(w.estimate_minutes, 0) -
                    COALESCE((SELECT sum(minutes) FROM work_log l WHERE l.work_item_id = w.id), 0), 0)), 0)::bigint
         FROM work_item w WHERE w.project_id = $1 AND w.kind NOT IN ('phase', 'milestone') AND w.status NOT IN ('done', 'cancelled')",
    )
    .bind(project)
    .fetch_one(&mut *t.tx)
    .await?;
    // Forecast at completion = actual + remaining estimate × blended rate so far.
    let forecast = if costed_minutes > 0 {
        let extra = i128::from(spent) * i128::from(remaining) / i128::from(costed_minutes);
        spent.saturating_add(i64::try_from(extra).unwrap_or(i64::MAX))
    } else {
        spent
    };
    Ok(ProjectCost {
        currency: p.currency.clone(),
        budget_minor: p.labour_budget_minor,
        spent_minor: spent,
        forecast_minor: forecast,
        used_pct: threshold::budget_used_percent(spent, p.labour_budget_minor),
        costed_minutes,
        uncosted_entries: uncosted,
        remaining_minutes: remaining,
    })
}

pub async fn set_budget(
    t: &mut TenantTx,
    actor: Uuid,
    project: Uuid,
    budget: Option<i64>,
) -> Result<()> {
    get_project(t, project).await?;
    sqlx::query("UPDATE project SET labour_budget_minor = $2 WHERE id = $1")
        .bind(project)
        .bind(budget)
        .execute(&mut *t.tx)
        .await?;
    audit(
        t,
        actor,
        "budget.set",
        json!({ "project": project, "budget_minor": budget }),
    )
    .await?;
    evaluate_project(t, project).await
}

/// Cost by person for a project (Finance view).
#[allow(clippy::type_complexity)]
pub async fn project_people(t: &mut TenantTx, project: Uuid) -> Result<Vec<Value>> {
    let rows: Vec<(Uuid, String, i64, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT l.user_id, u.display_name, sum(l.minutes)::bigint, COALESCE(sum(l.project_cost_minor), 0)::bigint,
                (SELECT r.rate_minor FROM rate_card r WHERE r.user_id = l.user_id AND r.effective_from <= current_date ORDER BY r.effective_from DESC LIMIT 1),
                (SELECT r.currency::text FROM rate_card r WHERE r.user_id = l.user_id AND r.effective_from <= current_date ORDER BY r.effective_from DESC LIMIT 1)
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id JOIN app_user u ON u.id = l.user_id
         WHERE w.project_id = $1 GROUP BY l.user_id, u.display_name ORDER BY 4 DESC",
    )
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, min, cost, rate, ccy)| {
            json!({ "user_id": id, "user_name": name, "minutes": min, "cost_minor": cost, "rate_minor": rate, "rate_currency": ccy })
        })
        .collect())
}

// ---------- thresholds and alerts ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ThresholdRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub metric: String,
    pub mode: String,
    pub amount_minor: Option<i64>,
    pub percent: Option<i32>,
    pub level: String,
    pub notify_pm: bool,
    pub notify_finance: bool,
    pub notify_admins: bool,
    pub alert_state: Option<String>,
    pub alert_fired_at: Option<DateTime<Utc>>,
}

const THRESHOLD_SELECT: &str = "
    SELECT c.id, c.project_id, c.metric, c.mode, c.amount_minor, c.percent, c.level, c.notify_pm, c.notify_finance, c.notify_admins,
           a.state AS alert_state, a.fired_at AS alert_fired_at
    FROM cost_threshold c LEFT JOIN alert a ON a.threshold_id = c.id AND a.state <> 'resolved'";

pub struct NewThreshold {
    pub project_id: Uuid,
    pub metric: String,
    pub mode: String,
    pub amount_minor: Option<i64>,
    pub percent: Option<i32>,
    pub level: String,
    pub notify_pm: bool,
    pub notify_finance: bool,
    pub notify_admins: bool,
}

pub async fn list_thresholds(t: &mut TenantTx, project: Uuid) -> Result<Vec<ThresholdRow>> {
    Ok(sqlx::query_as::<_, ThresholdRow>(&format!(
        "{THRESHOLD_SELECT} WHERE c.project_id = $1 ORDER BY c.level, c.created_at"
    ))
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn create_threshold(t: &mut TenantTx, actor: Uuid, n: NewThreshold) -> Result<Uuid> {
    get_project(t, n.project_id).await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO cost_threshold (id, tenant_id, project_id, metric, mode, amount_minor, percent, level,
                                     notify_pm, notify_finance, notify_admins, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(n.project_id)
    .bind(&n.metric)
    .bind(&n.mode)
    .bind(n.amount_minor)
    .bind(n.percent)
    .bind(&n.level)
    .bind(n.notify_pm)
    .bind(n.notify_finance)
    .bind(n.notify_admins)
    .bind(actor)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.is_check_violation() => StoreError::Invalid("threshold: amount or percent is required"),
        _ => StoreError::Db(e),
    })?;
    audit(t, actor, "threshold.create", json!({ "project": n.project_id, "mode": n.mode, "amount_minor": n.amount_minor, "percent": n.percent, "level": n.level })).await?;
    evaluate_project(t, n.project_id).await?;
    Ok(id)
}

pub async fn delete_threshold(t: &mut TenantTx, actor: Uuid, id: Uuid) -> Result<()> {
    let r = sqlx::query("DELETE FROM cost_threshold WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    audit(t, actor, "threshold.delete", json!({ "id": id })).await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AlertRow {
    pub id: Uuid,
    pub threshold_id: Uuid,
    pub project_id: Uuid,
    pub project_key: String,
    pub project_name: String,
    pub currency: String,
    pub metric: String,
    pub mode: String,
    pub percent: Option<i32>,
    pub level: String,
    pub observed_minor: i64,
    pub limit_minor: i64,
    pub state: String,
    pub fired_at: DateTime<Utc>,
    pub ack_by_name: Option<String>,
    pub ack_note: String,
    pub resolved_at: Option<DateTime<Utc>>,
}

const ALERT_SELECT: &str = "
    SELECT a.id, a.threshold_id, a.project_id, p.key AS project_key, p.name AS project_name, p.currency::text AS currency,
           c.metric, c.mode, c.percent, a.level, a.observed_minor, a.limit_minor, a.state, a.fired_at,
           u.display_name AS ack_by_name, a.ack_note, a.resolved_at
    FROM alert a JOIN project p ON p.id = a.project_id JOIN cost_threshold c ON c.id = a.threshold_id
    LEFT JOIN app_user u ON u.id = a.ack_by";

pub async fn list_alerts(
    t: &mut TenantTx,
    project: Option<Uuid>,
    open_only: bool,
) -> Result<Vec<AlertRow>> {
    Ok(sqlx::query_as::<_, AlertRow>(&format!(
        "{ALERT_SELECT} WHERE ($1::uuid IS NULL OR a.project_id = $1) AND (NOT $2 OR a.state <> 'resolved')
         ORDER BY a.fired_at DESC LIMIT 200"
    ))
    .bind(project)
    .bind(open_only)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn ack_alert(t: &mut TenantTx, actor: Uuid, id: Uuid, note: &str) -> Result<()> {
    let r = sqlx::query("UPDATE alert SET state = 'acknowledged', ack_by = $2, ack_note = $3 WHERE id = $1 AND state = 'open'")
        .bind(id)
        .bind(actor)
        .bind(note.trim())
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    audit(
        t,
        actor,
        "alert.ack",
        json!({ "id": id, "note": note.trim() }),
    )
    .await
}

async fn evaluate_all(t: &mut TenantTx) -> Result<()> {
    let ids: Vec<(Uuid,)> = sqlx::query_as("SELECT DISTINCT project_id FROM cost_threshold")
        .fetch_all(&mut *t.tx)
        .await?;
    for (p,) in ids {
        evaluate_project(t, p).await?;
    }
    Ok(())
}

/// Compare the project's frozen cost (and forecast) with each threshold:
/// open an alert on crossing, resolve it when the value falls back below.
pub async fn evaluate_project(t: &mut TenantTx, project: Uuid) -> Result<()> {
    let ths = list_thresholds(t, project).await?;
    if ths.is_empty() {
        return Ok(());
    }
    let cost = project_cost(t, project).await?;
    let p = get_project(t, project).await?;
    for th in ths {
        let rule = match (th.mode.as_str(), th.amount_minor, th.percent) {
            ("absolute", Some(a), _) => ThresholdRule::Absolute { amount_minor: a },
            ("percent", _, Some(pct)) => ThresholdRule::PercentOfBudget {
                percent: Decimal::from(pct),
            },
            _ => continue,
        };
        let observed = if th.metric == "forecast" {
            cost.forecast_minor
        } else {
            cost.spent_minor
        };
        match threshold::evaluate(rule, cost.budget_minor, observed, th.alert_state.is_some()) {
            Evaluation::Fire { limit_minor } => {
                sqlx::query(
                    "INSERT INTO alert (id, tenant_id, threshold_id, project_id, level, observed_minor, limit_minor)
                     VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
                )
                .bind(Uuid::now_v7())
                .bind(t.tenant_id)
                .bind(th.id)
                .bind(project)
                .bind(&th.level)
                .bind(observed)
                .bind(limit_minor)
                .execute(&mut *t.tx)
                .await?;
                let metric = if th.metric == "forecast" {
                    "forecast"
                } else {
                    "labour cost"
                };
                let money_text = format!(
                    "{} {metric} {} ≥ {}",
                    p.key,
                    fmt_money(observed, &p.currency),
                    fmt_money(limit_minor, &p.currency)
                );
                let pct = cost
                    .used_pct
                    .map(|x| format!("{x}%"))
                    .unwrap_or_else(|| "—".into());
                let hours_text = format!("{} {metric}: budget used {pct}", p.key);
                let subject = format!("[{}] {} {}", th.level, p.key, p.name);
                let link = format!("/finance?project={project}");
                let mut money_people: Vec<Uuid> = Vec::new();
                if th.notify_finance {
                    money_people.extend(finance_people(t).await?);
                }
                if th.notify_admins {
                    money_people.extend(admin_people(t).await?);
                }
                money_people.sort();
                money_people.dedup();
                for u in &money_people {
                    notify(t, *u, "alert", &subject, &money_text, &link).await?;
                }
                // PMs see hours only: their message carries the percentage, not amounts.
                if th.notify_pm && !money_people.contains(&p.owner_id) {
                    notify(
                        t,
                        p.owner_id,
                        "alert",
                        &subject,
                        &hours_text,
                        &format!("/projects/{project}"),
                    )
                    .await?;
                }
            }
            Evaluation::Resolve => {
                sqlx::query("UPDATE alert SET state = 'resolved', resolved_at = now() WHERE threshold_id = $1 AND state <> 'resolved'")
                    .bind(th.id)
                    .execute(&mut *t.tx)
                    .await?;
            }
            Evaluation::Inactive | Evaluation::NoChange => {}
        }
    }
    Ok(())
}

// ---------- export ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ExportRow {
    pub day: NaiveDate,
    pub person: String,
    pub email: String,
    pub project_key: Option<String>,
    pub item_key: Option<String>,
    pub item_title: String,
    pub kind: String,
    pub minutes: i32,
    pub description: String,
    pub rate_minor: Option<i64>,
    pub currency: Option<String>,
    pub cost_minor: Option<i64>,
    pub project_currency: Option<String>,
    pub project_cost_minor: Option<i64>,
    pub locked: bool,
}

pub async fn export_rows(
    t: &mut TenantTx,
    from: NaiveDate,
    to: NaiveDate,
    project: Option<Uuid>,
    user: Option<Uuid>,
) -> Result<Vec<ExportRow>> {
    Ok(sqlx::query_as::<_, ExportRow>(
        "SELECT l.started_at::date AS day, u.display_name AS person, u.email, p.key AS project_key, w.key AS item_key,
                w.title AS item_title, w.kind, l.minutes, l.description, l.rate_minor, l.currency::text AS currency, l.cost_minor,
                l.project_currency::text AS project_currency, l.project_cost_minor, l.locked
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id JOIN app_user u ON u.id = l.user_id
         LEFT JOIN project p ON p.id = w.project_id
         WHERE l.started_at::date BETWEEN $1 AND $2 AND ($3::uuid IS NULL OR w.project_id = $3) AND ($4::uuid IS NULL OR l.user_id = $4)
         ORDER BY l.started_at, u.display_name",
    )
    .bind(from)
    .bind(to)
    .bind(project)
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::fmt_money;

    #[test]
    fn money_formatting() {
        assert_eq!(fmt_money(4_120_000, "USD"), "USD 41,200.00");
        assert_eq!(fmt_money(650_000, "JPY"), "JPY 650,000");
        assert_eq!(fmt_money(-5, "CNY"), "CNY -0.05");
        assert_eq!(fmt_money(100, "EUR"), "EUR 1.00");
    }
}
