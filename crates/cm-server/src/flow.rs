//! Routine templates, deliverables, requests, hand-offs and notifications.

use crate::work::{begin, manage_item, Caller};
use crate::{ApiError, AppState, Session};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveDate;
use cm_domain::handoff::HandoffAction;
use cm_store::flow::{self as f, NewHandoff, NewRequest, NewSpec, NewTemplate};
use cm_store::TenantTx;
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;

fn text(v: &str, max: usize, what: &str) -> Result<String, ApiError> {
    let v = v.trim();
    if v.is_empty() || v.chars().count() > max {
        return Err(ApiError::bad_request(format!(
            "{what} must be 1–{max} characters"
        )));
    }
    Ok(v.to_owned())
}

// ---------- routine templates ----------

#[derive(Deserialize)]
pub struct SpecBody {
    kind: String,
    #[serde(default = "one")]
    min_count: i32,
    #[serde(default = "yes")]
    required: bool,
    #[serde(default)]
    instructions: String,
}
fn one() -> i32 {
    1
}
fn yes() -> bool {
    true
}

fn specs_from(b: &[SpecBody]) -> Result<Vec<NewSpec>, ApiError> {
    if b.len() > 20 {
        return Err(ApiError::bad_request("at most 20 deliverables"));
    }
    b.iter()
        .map(|s| {
            Ok(NewSpec {
                kind: f::parse_kind(&s.kind)
                    .ok_or_else(|| ApiError::bad_request("unknown deliverable kind"))?,
                min_count: s.min_count.clamp(1, 100),
                required: s.required,
                instructions: s.instructions.trim().chars().take(1000).collect(),
            })
        })
        .collect()
}

#[derive(Deserialize)]
pub struct TemplateBody {
    title: String,
    #[serde(default)]
    description: String,
    recurrence: String,
    #[serde(default)]
    weekdays: Vec<i16>,
    target_user_id: Option<Uuid>,
    target_role_id: Option<Uuid>,
    target_department_id: Option<Uuid>,
    #[serde(default = "each")]
    mode: String,
    estimate_minutes: Option<i32>,
    project_id: Option<Uuid>,
    #[serde(default)]
    deliverables: Vec<SpecBody>,
}
fn each() -> String {
    "each_member".into()
}

/// Supervisors (department heads) set routine work for their department;
/// Admins for anyone.
async fn may_set_routine(
    t: &mut TenantTx,
    c: &Caller,
    user: Option<Uuid>,
    dept: Option<Uuid>,
) -> Result<bool, ApiError> {
    if c.is_admin {
        return Ok(true);
    }
    let dept = match (user, dept) {
        (_, Some(d)) => Some(d),
        (Some(u), None) => f::home_department(t, u).await?,
        _ => None,
    };
    match dept {
        Some(d) => Ok(f::heads_department(t, c.user, d).await?),
        None => Ok(false),
    }
}

pub async fn create_template(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<TemplateBody>,
) -> Result<impl IntoResponse, ApiError> {
    let title = text(&b.title, 200, "title")?;
    if !matches!(b.mode.as_str(), "each_member" | "pool") {
        return Err(ApiError::bad_request("mode must be each_member or pool"));
    }
    let specs = specs_from(&b.deliverables)?;
    let (mut t, c) = begin(&s, &session).await?;
    if !may_set_routine(&mut t, &c, b.target_user_id, b.target_department_id).await? {
        return Err(ApiError::forbidden());
    }
    let row = f::create_template(
        &mut t,
        c.user,
        NewTemplate {
            title: &title,
            description: b.description.trim(),
            recurrence: &b.recurrence,
            weekdays: b.weekdays,
            target_user_id: b.target_user_id,
            target_role_id: b.target_role_id,
            target_department_id: b.target_department_id,
            mode: &b.mode,
            estimate_minutes: b.estimate_minutes,
            project_id: b.project_id,
        },
        &specs,
    )
    .await?;
    // Create today's items straight away so the new routine shows up now.
    let today = f::tenant_today(&mut t).await?;
    f::generate_for_date(&mut t, today).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn list_templates(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let rows = f::list_templates(&mut t).await?;
    let mut out = Vec::new();
    for r in rows {
        let specs = f::template_specs(&mut t, r.id).await?;
        let editable = c.is_admin || r.created_by == c.user;
        out.push(json!({ "template": r, "deliverables": specs, "can_edit": editable }));
    }
    let headed = f::headed_departments(&mut t, c.user).await?;
    let since = f::tenant_today(&mut t).await? - chrono::Duration::days(7);
    let missing = if c.is_admin || !headed.is_empty() {
        f::missing_deliverables(&mut t, since).await?
    } else {
        vec![]
    };
    t.commit().await?;
    Ok(Json(json!({
        "templates": out,
        "can_create": c.is_admin || !headed.is_empty(),
        "headed_departments": headed,
        "missing": missing,
    })))
}

#[derive(Deserialize)]
pub struct ActiveBody {
    active: bool,
}

pub async fn set_template_active(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<ActiveBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let tpl = f::get_template(&mut t, id).await?;
    if !(c.is_admin || tpl.created_by == c.user) {
        return Err(ApiError::forbidden());
    }
    let row = f::set_template_active(&mut t, id, b.active).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct GenerateBody {
    date: Option<NaiveDate>,
}

pub async fn generate(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<GenerateBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    let date = match b.date {
        Some(d) => d,
        None => f::tenant_today(&mut t).await?,
    };
    let n = f::generate_for_date(&mut t, date).await?;
    t.commit().await?;
    Ok(Json(json!({ "created": n, "date": date })))
}

pub async fn claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let row = f::claim(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(Json(row))
}

// ---------- deliverables ----------

pub async fn add_spec(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<SpecBody>,
) -> Result<impl IntoResponse, ApiError> {
    let spec = specs_from(std::slice::from_ref(&b))?
        .pop()
        .ok_or_else(|| ApiError::bad_request("deliverable required"))?;
    let (mut t, c) = begin(&s, &session).await?;
    let item = cm_store::work::get_item(&mut t, id).await?;
    if !manage_item(&mut t, &c, &item).await? {
        return Err(ApiError::forbidden());
    }
    let sid = f::add_item_spec(&mut t, id, &spec).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": sid }))))
}

#[derive(Deserialize)]
pub struct SubmitBody {
    spec_id: Uuid,
    #[serde(default)]
    value: String,
    attachment_id: Option<Uuid>,
    work_log_id: Option<Uuid>,
}

pub async fn submit(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<SubmitBody>,
) -> Result<impl IntoResponse, ApiError> {
    if b.value.chars().count() > 5000 {
        return Err(ApiError::bad_request("value is too long"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let item = cm_store::work::get_item(&mut t, id).await?;
    if !(item.holder_user_id == Some(c.user) || manage_item(&mut t, &c, &item).await?) {
        return Err(ApiError::forbidden());
    }
    let sid = f::submit(
        &mut t,
        c.user,
        id,
        b.spec_id,
        &b.value,
        b.attachment_id,
        b.work_log_id,
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": sid }))))
}

// ---------- requests ----------

#[derive(Deserialize)]
pub struct RequestBody {
    to_user: Uuid,
    title: String,
    #[serde(default)]
    description: String,
    due: Option<NaiveDate>,
    project_id: Option<Uuid>,
    estimate_minutes: Option<i32>,
}

pub async fn create_request(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let title = text(&b.title, 300, "title")?;
    if b.description.chars().count() > 20_000 {
        return Err(ApiError::bad_request("description is too long"));
    }
    if b.estimate_minutes
        .is_some_and(|m| !(0..=100_000).contains(&m))
    {
        return Err(ApiError::bad_request("estimate is out of range"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let row = f::create_request(
        &mut t,
        c.user,
        NewRequest {
            to_user: b.to_user,
            title: &title,
            description: b.description.trim(),
            due: b.due,
            project_id: b.project_id,
            estimate_minutes: b.estimate_minutes,
        },
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

// ---------- hand-offs ----------

#[derive(Deserialize)]
pub struct HandoffBody {
    version: i32,
    to_user: Uuid,
    reason: String,
    #[serde(default)]
    note: String,
    new_due: Option<NaiveDate>,
}

pub async fn request_handoff(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<HandoffBody>,
) -> Result<impl IntoResponse, ApiError> {
    let reason = text(&b.reason, 500, "reason")?;
    if b.note.chars().count() > 5000 {
        return Err(ApiError::bad_request("note is too long"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let item = cm_store::work::get_item(&mut t, id).await?;
    if !f::may_reassign(&mut t, c.user, c.is_admin, &item).await? {
        return Err(ApiError::forbidden());
    }
    let row = f::request_handoff(
        &mut t,
        c.user,
        NewHandoff {
            item: id,
            version: b.version,
            to_user: b.to_user,
            reason: &reason,
            note: &b.note,
            new_due: b.new_due,
        },
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
pub struct DecideBody {
    action: String,
    #[serde(default)]
    note: String,
}

pub async fn decide_handoff(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<DecideBody>,
) -> Result<impl IntoResponse, ApiError> {
    let action = match b.action.as_str() {
        "approve" => HandoffAction::Approve,
        "reject" => HandoffAction::Reject,
        "accept" => HandoffAction::Accept,
        "decline" => HandoffAction::Decline,
        "cancel" => HandoffAction::Cancel,
        _ => return Err(ApiError::bad_request("unknown action")),
    };
    if matches!(action, HandoffAction::Reject | HandoffAction::Decline) && b.note.trim().is_empty()
    {
        return Err(ApiError::bad_request("give a reason"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let row = f::decide_handoff(&mut t, c.user, id, action, &b.note).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct ReturnBody {
    #[serde(default)]
    reason: String,
}

pub async fn return_item(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<ReturnBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let row = f::return_to_sender(&mut t, c.user, id, &b.reason).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn handoff_stats(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _) = begin(&s, &session).await?;
    let v = f::handoff_stats(&mut t).await?;
    t.commit().await?;
    Ok(Json(v))
}

// ---------- inbox and notifications ----------

pub async fn inbox(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let (to_accept, to_approve) = f::inbox_handoffs(&mut t, c.user).await?;
    let notifications = f::list_notifications(&mut t, c.user).await?;
    let unread = f::unread_count(&mut t, c.user).await?;
    t.commit().await?;
    Ok(Json(json!({
        "to_accept": to_accept, "to_approve": to_approve,
        "notifications": notifications, "unread": unread,
    })))
}

#[derive(Deserialize)]
pub struct ReadBody {
    id: Option<Uuid>,
}

pub async fn mark_read(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<ReadBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    f::mark_read(&mut t, c.user, b.id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    _v: Option<String>,
}

async fn unread(s: &AppState, session: &Session) -> i64 {
    async {
        let (mut t, c) = begin(s, session).await?;
        let n = f::unread_count(&mut t, c.user).await?;
        t.commit().await?;
        Ok::<_, ApiError>(n)
    }
    .await
    .unwrap_or(0)
}

/// Server-sent events: the unread count, pushed whenever a notification for
/// this person is committed (PostgreSQL NOTIFY) and at least every 30 s.
pub async fn stream(
    State(s): State<AppState>,
    session: Session,
    Query(_): Query<StreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = s.notify.subscribe();
    let me = session.user_id.to_string();
    let (tx, out) = tokio::sync::mpsc::channel::<Event>(8);
    tokio::spawn(async move {
        let mut last = -1;
        loop {
            let n = unread(&s, &session).await;
            if n != last {
                last = n;
                if tx
                    .send(Event::default().event("unread").data(n.to_string()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            tokio::select! {
                r = rx.recv() => match r {
                    Ok(u) if u != me => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                },
                _ = tx.closed() => break,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    });
    let stream =
        futures_util::stream::unfold(
            out,
            |mut rx| async move { rx.recv().await.map(|e| (Ok(e), rx)) },
        );
    Sse::new(stream).keep_alive(KeepAlive::default())
}
