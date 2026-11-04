//! Finance: rates, exchange rates, periods, budgets, thresholds, alerts, export.
//! Every handler here requires the Finance or Admin flag.

use crate::work::{begin, Caller};
use crate::{ApiError, AppState, Session};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveDate;
use cm_store::finance as fin;
use cm_store::TenantTx;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;
use uuid::Uuid;

async fn money_tx(s: &AppState, session: &Session) -> Result<(TenantTx, Caller), ApiError> {
    let (t, c) = begin(s, session).await?;
    if !c.sees_money() {
        return Err(ApiError::forbidden());
    }
    Ok((t, c))
}

fn currency(code: &str) -> Result<String, ApiError> {
    cm_domain::Currency::parse(code.trim())
        .map(|c| c.as_str().to_owned())
        .map_err(|_| ApiError::bad_request("currency must be an ISO 4217 code"))
}

pub async fn overview(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let projects = cm_store::work::list_projects(&mut t).await?;
    let mut rows = Vec::new();
    for p in &projects {
        let cost = fin::project_cost(&mut t, p.id).await?;
        rows.push(json!({ "project": p, "cost": cost }));
    }
    let alerts = fin::list_alerts(&mut t, None, true).await?;
    let pending_fx = fin::pending_fx(&mut t).await?;
    let missing_rates = fin::missing_rates(&mut t).await?;
    t.commit().await?;
    Ok(Json(json!({
        "projects": rows, "alerts": alerts, "pending_fx": pending_fx, "missing_rates": missing_rates,
    })))
}

pub async fn project(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let p = cm_store::work::get_project(&mut t, id).await?;
    let cost = fin::project_cost(&mut t, id).await?;
    let people = fin::project_people(&mut t, id).await?;
    let thresholds = fin::list_thresholds(&mut t, id).await?;
    let alerts = fin::list_alerts(&mut t, Some(id), false).await?;
    t.commit().await?;
    Ok(Json(json!({
        "project": p, "cost": cost, "people": people, "thresholds": thresholds, "alerts": alerts,
    })))
}

#[derive(Deserialize)]
pub struct BudgetBody {
    labour_budget_minor: Option<i64>,
}

pub async fn set_budget(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<BudgetBody>,
) -> Result<impl IntoResponse, ApiError> {
    if b.labour_budget_minor.is_some_and(|v| v < 0) {
        return Err(ApiError::bad_request("budget cannot be negative"));
    }
    let (mut t, c) = money_tx(&s, &session).await?;
    fin::set_budget(&mut t, c.user, id, b.labour_budget_minor).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- rates ----------

pub async fn list_rates(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let rows = fin::list_rates(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct RateBody {
    user_id: Uuid,
    rate_minor: i64,
    currency: String,
    effective_from: NaiveDate,
    #[serde(default)]
    note: String,
}

pub async fn add_rate(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<RateBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !(0..=1_000_000_000).contains(&b.rate_minor) {
        return Err(ApiError::bad_request("rate is out of range"));
    }
    let ccy = currency(&b.currency)?;
    let (mut t, c) = money_tx(&s, &session).await?;
    let affected = fin::affected_by_backdate(&mut t, Some(b.user_id), b.effective_from).await?;
    let (id, filled) = fin::add_rate(
        &mut t,
        c.user,
        b.user_id,
        b.rate_minor,
        &ccy,
        b.effective_from,
        b.note.trim(),
    )
    .await?;
    t.commit().await?;
    // `affected`: entries that already had a cost from that date on; they keep
    // it until Finance explicitly recalculates.
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "filled": filled, "affected": affected })),
    ))
}

// ---------- exchange rates ----------

pub async fn list_fx(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let rows = fin::list_fx(&mut t).await?;
    let pending = fin::pending_fx(&mut t).await?;
    t.commit().await?;
    Ok(Json(json!({ "rates": rows, "pending": pending })))
}

#[derive(Deserialize)]
pub struct FxBody {
    from_ccy: String,
    to_ccy: String,
    rate: String,
    effective_date: NaiveDate,
}

pub async fn add_fx(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<FxBody>,
) -> Result<impl IntoResponse, ApiError> {
    let from = currency(&b.from_ccy)?;
    let to = currency(&b.to_ccy)?;
    if from == to {
        return Err(ApiError::bad_request("currencies must differ"));
    }
    let rate = Decimal::from_str(b.rate.trim())
        .map_err(|_| ApiError::bad_request("rate must be a decimal number"))?;
    let (mut t, c) = money_tx(&s, &session).await?;
    let (id, filled) = fin::add_fx(&mut t, c.user, &from, &to, rate, b.effective_date).await?;
    t.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "filled": filled })),
    ))
}

#[derive(Deserialize)]
pub struct RecalcBody {
    from: NaiveDate,
}

pub async fn recalculate(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<RecalcBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = money_tx(&s, &session).await?;
    let n = fin::recalculate(&mut t, c.user, b.from).await?;
    t.commit().await?;
    Ok(Json(json!({ "entries": n })))
}

// ---------- periods ----------

pub async fn list_periods(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let rows = fin::list_periods(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct PeriodBody {
    starts_on: NaiveDate,
    ends_on: NaiveDate,
}

pub async fn close_period(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<PeriodBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = money_tx(&s, &session).await?;
    let n = fin::close_period(&mut t, c.user, b.starts_on, b.ends_on).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "locked": n }))))
}

// ---------- thresholds and alerts ----------

#[derive(Deserialize)]
pub struct ThresholdBody {
    #[serde(default = "labour")]
    metric: String,
    mode: String,
    amount_minor: Option<i64>,
    percent: Option<i32>,
    level: String,
    #[serde(default = "yes")]
    notify_pm: bool,
    #[serde(default = "yes")]
    notify_finance: bool,
    #[serde(default)]
    notify_admins: bool,
}
fn labour() -> String {
    "labour_cost".into()
}
fn yes() -> bool {
    true
}

pub async fn create_threshold(
    State(s): State<AppState>,
    session: Session,
    Path(project): Path<Uuid>,
    Json(b): Json<ThresholdBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !matches!(b.metric.as_str(), "labour_cost" | "forecast")
        || !matches!(b.mode.as_str(), "absolute" | "percent")
        || !matches!(b.level.as_str(), "info" | "warn" | "critical")
    {
        return Err(ApiError::bad_request(
            "metric labour_cost|forecast, mode absolute|percent, level info|warn|critical",
        ));
    }
    let (amount, percent) = match b.mode.as_str() {
        "absolute" => (
            Some(
                b.amount_minor
                    .filter(|a| *a >= 0)
                    .ok_or_else(|| ApiError::bad_request("amount is required"))?,
            ),
            None,
        ),
        _ => (
            None,
            Some(
                b.percent
                    .filter(|p| (1..=1000).contains(p))
                    .ok_or_else(|| ApiError::bad_request("percent must be 1–1000"))?,
            ),
        ),
    };
    let (mut t, c) = money_tx(&s, &session).await?;
    let id = fin::create_threshold(
        &mut t,
        c.user,
        fin::NewThreshold {
            project_id: project,
            metric: b.metric,
            mode: b.mode,
            amount_minor: amount,
            percent,
            level: b.level,
            notify_pm: b.notify_pm,
            notify_finance: b.notify_finance,
            notify_admins: b.notify_admins,
        },
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn delete_threshold(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = money_tx(&s, &session).await?;
    fin::delete_threshold(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AckBody {
    #[serde(default)]
    note: String,
}

/// Recipients acknowledge alerts. PMs may acknowledge their projects' alerts
/// too, but never see the amounts.
pub async fn ack_alert(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<AckBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    if !c.sees_money() {
        let alerts = fin::list_alerts(&mut t, None, true).await?;
        let a = alerts
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(ApiError::forbidden)?;
        if cm_store::work::get_project(&mut t, a.project_id)
            .await?
            .owner_id
            != c.user
        {
            return Err(ApiError::forbidden());
        }
    }
    fin::ack_alert(&mut t, c.user, id, &b.note).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Alerts for a project, as a PM sees them: level, state and percentage, no amounts.
pub async fn project_alerts_hours(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let p = cm_store::work::get_project(&mut t, id).await?;
    if !(c.sees_money() || p.owner_id == c.user) {
        return Err(ApiError::forbidden());
    }
    let alerts = fin::list_alerts(&mut t, Some(id), true).await?;
    t.commit().await?;
    Ok(Json(
        alerts
            .iter()
            .map(|a| {
                let mut v = json!({
                    "id": a.id, "level": a.level, "state": a.state, "metric": a.metric,
                    "mode": a.mode, "percent": a.percent, "fired_at": a.fired_at,
                });
                if c.sees_money() {
                    v["observed_minor"] = json!(a.observed_minor);
                    v["limit_minor"] = json!(a.limit_minor);
                }
                v
            })
            .collect::<Vec<_>>(),
    ))
}

pub async fn audit(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = money_tx(&s, &session).await?;
    let rows = fin::list_audit(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

// ---------- export ----------

#[derive(Deserialize)]
pub struct ExportQuery {
    from: NaiveDate,
    to: NaiveDate,
    project: Option<Uuid>,
    user: Option<Uuid>,
}

fn csv_field(s: &str) -> String {
    // Quote every field; neutralise spreadsheet formulas.
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn major(minor: Option<i64>, ccy: Option<&str>) -> String {
    match (minor, ccy) {
        (Some(m), Some(c)) => {
            let exp = cm_domain::Currency::parse(c)
                .map(|c| c.minor_exponent())
                .unwrap_or(2);
            let d = Decimal::new(m, exp);
            d.to_string()
        }
        _ => String::new(),
    }
}

/// Timesheet export. Finance and Admins get costs; everyone else gets hours
/// for their own entries, and PMs for their projects.
pub async fn export_csv(
    State(s): State<AppState>,
    session: Session,
    Query(q): Query<ExportQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if q.to < q.from || (q.to - q.from).num_days() > 400 {
        return Err(ApiError::bad_request("range must be at most 400 days"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let money = c.sees_money();
    let user = if money {
        q.user
    } else {
        match q.project {
            Some(p) if cm_store::work::get_project(&mut t, p).await?.owner_id == c.user => q.user,
            _ => Some(c.user),
        }
    };
    let rows = fin::export_rows(&mut t, q.from, q.to, q.project, user).await?;
    t.commit().await?;
    let mut out = String::from("\u{feff}");
    out.push_str(if money {
        "date,person,email,project,item,title,kind,hours,description,rate,rate_currency,cost,project_currency,project_cost,locked\r\n"
    } else {
        "date,person,email,project,item,title,kind,hours,description\r\n"
    });
    for r in rows {
        let hours = Decimal::new(i64::from(r.minutes) * 100 / 60, 2).to_string();
        let mut f = vec![
            r.day.to_string(),
            csv_field(&r.person),
            csv_field(&r.email),
            csv_field(r.project_key.as_deref().unwrap_or("")),
            csv_field(r.item_key.as_deref().unwrap_or("")),
            csv_field(&r.item_title),
            r.kind.clone(),
            hours,
            csv_field(&r.description),
        ];
        if money {
            f.extend([
                major(r.rate_minor, r.currency.as_deref()),
                r.currency.clone().unwrap_or_default(),
                major(r.cost_minor, r.currency.as_deref()),
                r.project_currency.clone().unwrap_or_default(),
                major(r.project_cost_minor, r.project_currency.as_deref()),
                r.locked.to_string(),
            ]);
        }
        out.push_str(&f.join(","));
        out.push_str("\r\n");
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"timesheet-{}-{}.csv\"", q.from, q.to),
            ),
        ],
        out,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_is_quoted_and_formula_safe() {
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("=SUM(A1)"), "\"'=SUM(A1)\"");
        assert_eq!(major(Some(12345), Some("USD")), "123.45");
        assert_eq!(major(Some(650), Some("JPY")), "650");
        assert_eq!(major(None, Some("USD")), "");
    }
}
