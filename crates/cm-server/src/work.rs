//! Projects, work items, dependencies, work logs, attachments and the timer.

use crate::storage::StorageError;
use crate::{ApiError, AppState, Session};
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use cm_domain::work_item::{self as wi, WorkItemKind};
use cm_store::work::{self as w, ItemPatch, NewItem, NewProject, ProjectRow};
use cm_store::{StoreError, TenantTx};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use uuid::Uuid;

pub const MAX_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Distinguish "field absent" (None) from "field set to null" (Some(None)).
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

pub(crate) struct Caller {
    pub user: Uuid,
    pub is_admin: bool,
    pub is_finance: bool,
}

impl Caller {
    pub fn sees_money(&self) -> bool {
        self.is_admin || self.is_finance
    }
}

pub(crate) async fn begin(s: &AppState, session: &Session) -> Result<(TenantTx, Caller), ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    let (is_admin, is_finance) = cm_store::member_flags(&mut t, session.user_id).await?;
    Ok((
        t,
        Caller {
            user: session.user_id,
            is_admin,
            is_finance,
        },
    ))
}

pub(crate) fn can_manage(c: &Caller, p: &ProjectRow) -> bool {
    c.is_admin || p.owner_id == c.user
}

/// Budget amounts are visible to Finance and Admins only (PMs see hours and
/// the share of budget used).
pub(crate) async fn project_json(
    t: &mut TenantTx,
    c: &Caller,
    p: &ProjectRow,
) -> Result<Value, ApiError> {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    let cost = cm_store::finance::project_cost(t, p.id).await?;
    let open_alerts = cm_store::finance::list_alerts(t, Some(p.id), true)
        .await?
        .len();
    if let Some(o) = v.as_object_mut() {
        o.insert("budget_used_pct".into(), json!(cost.used_pct));
        o.insert("open_alerts".into(), json!(open_alerts));
        if c.sees_money() {
            o.insert("cost".into(), json!(cost));
        } else {
            o.remove("labour_budget_minor");
        }
    }
    Ok(v)
}

/// Who may plan an item: the project's managers; for items outside projects,
/// Admins, whoever asked for a request, the routine's author, and the
/// holder's department head.
pub(crate) async fn manage_item(
    t: &mut TenantTx,
    c: &Caller,
    item: &w::ItemRow,
) -> Result<bool, ApiError> {
    if let Some(pid) = item.project_id {
        if can_manage(c, &w::get_project(t, pid).await?) {
            return Ok(true);
        }
    }
    if c.is_admin || item.requested_by == Some(c.user) {
        return Ok(true);
    }
    if let Some(tpl) = item.routine_template_id {
        if cm_store::flow::get_template(t, tpl).await?.created_by == c.user {
            return Ok(true);
        }
    }
    if let Some(h) = item.holder_user_id {
        if let Some(d) = cm_store::flow::home_department(t, h).await? {
            return Ok(cm_store::flow::heads_department(t, c.user, d).await?);
        }
    }
    if let (Some(_), Some(d)) = (item.target_role_id, item.target_department_id) {
        return Ok(cm_store::flow::heads_department(t, c.user, d).await?);
    }
    Ok(false)
}

// ---------- projects ----------

pub async fn list_projects(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let rows = w::list_projects(&mut t).await?;
    let mut out = Vec::with_capacity(rows.len());
    for p in &rows {
        out.push(project_json(&mut t, &c, p).await?);
    }
    t.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct NewProjectBody {
    key: String,
    name: String,
    owner_id: Option<Uuid>,
    department_id: Option<Uuid>,
    currency: Option<String>,
    labour_budget_minor: Option<i64>,
    planned_start: Option<NaiveDate>,
    planned_end: Option<NaiveDate>,
}

pub async fn create_project(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<NewProjectBody>,
) -> Result<impl IntoResponse, ApiError> {
    let key = b.key.trim().to_ascii_uppercase();
    if !(2..=10).contains(&key.len())
        || !key.starts_with(|c: char| c.is_ascii_uppercase())
        || !key.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(ApiError::bad_request(
            "key: 2–10 letters or digits, starting with a letter",
        ));
    }
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::bad_request("name must be 1–200 characters"));
    }
    if let (Some(a), Some(z)) = (b.planned_start, b.planned_end) {
        if z < a {
            return Err(ApiError::bad_request("end date is before start date"));
        }
    }
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    let tenant_ccy: (String,) =
        cm_store::sqlx::query_as("SELECT default_currency FROM tenant WHERE id = $1")
            .bind(session.tenant_id)
            .fetch_one(&mut *t.tx)
            .await
            .map_err(StoreError::from)?;
    let ccy = cm_domain::Currency::parse(b.currency.as_deref().unwrap_or(&tenant_ccy.0).trim())
        .map_err(|_| ApiError::bad_request("currency must be an ISO 4217 code"))?;
    let p = w::create_project(
        &mut t,
        NewProject {
            key: &key,
            name,
            owner_id: b.owner_id.unwrap_or(c.user),
            department_id: b.department_id,
            currency: ccy.as_str(),
            labour_budget_minor: if c.is_finance || c.is_admin {
                b.labour_budget_minor
            } else {
                None
            },
            planned_start: b.planned_start,
            planned_end: b.planned_end,
        },
    )
    .await?;
    let v = project_json(&mut t, &c, &p).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(v)))
}

pub async fn get_project(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let p = w::get_project(&mut t, id).await?;
    let items = w::list_items(&mut t, id).await?;
    let deps = w::list_dependencies(&mut t, id).await?;
    let pj = project_json(&mut t, &c, &p).await?;
    t.commit().await?;
    Ok(Json(
        json!({ "project": pj, "items": items, "dependencies": deps, "can_manage": can_manage(&c, &p) }),
    ))
}

// ---------- items ----------

fn parse_kind(k: &str) -> Option<WorkItemKind> {
    Some(match k {
        "phase" => WorkItemKind::Phase,
        "task" => WorkItemKind::Task,
        "milestone" => WorkItemKind::Milestone,
        _ => return None,
    })
}

#[derive(Deserialize)]
pub struct NewItemBody {
    kind: String,
    title: String,
    #[serde(default)]
    description: String,
    parent_id: Option<Uuid>,
    holder_user_id: Option<Uuid>,
    planned_start: Option<NaiveDate>,
    planned_end: Option<NaiveDate>,
    estimate_minutes: Option<i32>,
}

pub async fn create_item(
    State(s): State<AppState>,
    session: Session,
    Path(project_id): Path<Uuid>,
    Json(b): Json<NewItemBody>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = parse_kind(&b.kind)
        .ok_or_else(|| ApiError::bad_request("kind must be phase, task or milestone"))?;
    let title = b.title.trim();
    if title.is_empty() || title.chars().count() > 300 {
        return Err(ApiError::bad_request("title must be 1–300 characters"));
    }
    if b.estimate_minutes
        .is_some_and(|m| !(0..=100_000).contains(&m))
    {
        return Err(ApiError::bad_request("estimate is out of range"));
    }
    if let (Some(a), Some(z)) = (b.planned_start, b.planned_end) {
        if z < a {
            return Err(ApiError::bad_request("end date is before start date"));
        }
    }
    let (mut t, c) = begin(&s, &session).await?;
    let p = w::get_project(&mut t, project_id).await?;
    if !can_manage(&c, &p) {
        return Err(ApiError::forbidden());
    }
    let (start, end) = match kind {
        // A milestone is a single day.
        WorkItemKind::Milestone => (
            b.planned_start.or(b.planned_end),
            b.planned_start.or(b.planned_end),
        ),
        _ => (b.planned_start, b.planned_end),
    };
    let item = w::create_item(
        &mut t,
        c.user,
        NewItem {
            project_id,
            parent_id: b.parent_id,
            kind,
            title,
            description: &b.description,
            holder_user_id: b.holder_user_id,
            planned_start: start,
            planned_end: end,
            estimate_minutes: b.estimate_minutes,
        },
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn get_item(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let item = w::get_item(&mut t, id).await?;
    let logs = w::list_logs(&mut t, id).await?;
    let attachments = w::list_attachments(&mut t, id).await?;
    let events = w::list_events(&mut t, id).await?;
    let manage = manage_item(&mut t, &c, &item).await?;
    let specs = cm_store::flow::item_specs(&mut t, &item).await?;
    let submissions = cm_store::flow::list_submissions(&mut t, id).await?;
    let handoffs = cm_store::flow::list_handoffs(&mut t, id).await?;
    let commits = cm_store::git::item_commits(&mut t, id).await?;
    let can_reassign = cm_store::flow::may_reassign(&mut t, c.user, c.is_admin, &item).await?;
    let can_return = item.holder_user_id == Some(c.user)
        && handoffs
            .iter()
            .rev()
            .find(|h| h.state == "accepted")
            .is_some_and(|h| h.to_user == c.user);
    t.commit().await?;
    Ok(Json(json!({
        "item": item, "logs": logs, "attachments": attachments, "events": events, "can_manage": manage,
        "specs": specs, "submissions": submissions, "handoffs": handoffs, "commits": commits,
        "can_reassign": can_reassign, "can_return": can_return,
    })))
}

#[derive(Deserialize)]
pub struct PatchBody {
    version: i32,
    title: Option<String>,
    description: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    planned_start: Option<Option<NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    planned_end: Option<Option<NaiveDate>>,
    #[serde(default, deserialize_with = "double_option")]
    estimate_minutes: Option<Option<i32>>,
    progress_pct: Option<i16>,
    #[serde(default, deserialize_with = "double_option")]
    holder_user_id: Option<Option<Uuid>>,
}

pub async fn patch_item(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<PatchBody>,
) -> Result<impl IntoResponse, ApiError> {
    if b.title
        .as_deref()
        .is_some_and(|x| x.trim().is_empty() || x.chars().count() > 300)
    {
        return Err(ApiError::bad_request("title must be 1–300 characters"));
    }
    if b.progress_pct.is_some_and(|p| !(0..=100).contains(&p)) {
        return Err(ApiError::bad_request("progress must be 0–100"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let item = w::get_item(&mut t, id).await?;
    let manage = manage_item(&mut t, &c, &item).await?;
    let is_holder = item.holder_user_id == Some(c.user);
    // Holders may update progress and the description; planning fields need a manager.
    let planning = b.title.is_some()
        || b.planned_start.is_some()
        || b.planned_end.is_some()
        || b.estimate_minutes.is_some()
        || b.holder_user_id.is_some();
    if !(manage || (is_holder && !planning)) {
        return Err(ApiError::forbidden());
    }
    let row = w::update_item(
        &mut t,
        c.user,
        id,
        b.version,
        ItemPatch {
            title: b.title.map(|x| x.trim().to_owned()),
            description: b.description,
            planned_start: b.planned_start,
            planned_end: b.planned_end,
            estimate_minutes: b.estimate_minutes,
            progress_pct: b.progress_pct,
            holder_user_id: b.holder_user_id,
        },
    )
    .await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct StatusBody {
    version: i32,
    status: String,
    note: Option<String>,
    /// A manager may close an item with missing deliverables, giving a reason.
    #[serde(default)]
    override_deliverables: bool,
}

pub async fn set_status(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<StatusBody>,
) -> Result<impl IntoResponse, ApiError> {
    let to = w::parse_status(&b.status).ok_or_else(|| ApiError::bad_request("unknown status"))?;
    let (mut t, c) = begin(&s, &session).await?;
    let item = w::get_item(&mut t, id).await?;
    let manage = manage_item(&mut t, &c, &item).await?;
    if !(manage || item.holder_user_id == Some(c.user)) {
        return Err(ApiError::forbidden());
    }
    let from =
        w::parse_status(&item.status).ok_or_else(|| ApiError::bad_request("unknown status"))?;
    let overriding = b.override_deliverables && manage;
    if b.override_deliverables && !manage {
        return Err(ApiError::forbidden());
    }
    if overriding && b.note.as_deref().is_none_or(|n| n.trim().is_empty()) {
        return Err(ApiError::bad_request(
            "give a reason for closing without the deliverables",
        ));
    }
    let shortfalls = cm_store::flow::shortfalls(&mut t, &item).await?;
    wi::transition(from, to, shortfalls, overriding).map_err(|e| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_transition",
            e.to_string(),
        )
    })?;
    let row = w::set_status(&mut t, c.user, id, b.version, to, b.note.as_deref()).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn my_items(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let rows = w::list_items_held_by(&mut t, c.user).await?;
    let timer = w::get_timer(&mut t, c.user).await?;
    let claimable = cm_store::flow::claimable_items(&mut t, c.user).await?;
    let sent = cm_store::flow::sent_handoffs(&mut t, c.user).await?;
    t.commit().await?;
    Ok(Json(
        json!({ "items": rows, "timer": timer, "claimable": claimable, "sent_handoffs": sent }),
    ))
}

// ---------- dependencies ----------

#[derive(Deserialize)]
pub struct DepBody {
    from_item: Uuid,
    to_item: Uuid,
    #[serde(default = "fs")]
    kind: String,
    #[serde(default)]
    lag_days: i32,
}
fn fs() -> String {
    "FS".into()
}

async fn manage_item_project(t: &mut TenantTx, c: &Caller, item: Uuid) -> Result<(), ApiError> {
    let it = w::get_item(t, item).await?;
    let pid = it.project_id.ok_or_else(ApiError::forbidden)?;
    if can_manage(c, &w::get_project(t, pid).await?) {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}

pub async fn add_dependency(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<DepBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !matches!(b.kind.as_str(), "FS" | "SS" | "FF" | "SF") || !(-365..=365).contains(&b.lag_days)
    {
        return Err(ApiError::bad_request(
            "kind must be FS, SS, FF or SF; lag within a year",
        ));
    }
    let (mut t, c) = begin(&s, &session).await?;
    manage_item_project(&mut t, &c, b.to_item).await?;
    w::add_dependency(&mut t, c.user, b.from_item, b.to_item, &b.kind, b.lag_days).await?;
    t.commit().await?;
    Ok(StatusCode::CREATED)
}

pub async fn remove_dependency(
    State(s): State<AppState>,
    session: Session,
    Path((from, to)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    manage_item_project(&mut t, &c, to).await?;
    w::remove_dependency(&mut t, from, to).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- work logs ----------

#[derive(Deserialize)]
pub struct LogBody {
    minutes: i32,
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    description: String,
}

pub async fn create_log(
    State(s): State<AppState>,
    session: Session,
    Path(item): Path<Uuid>,
    Json(b): Json<LogBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !(1..=24 * 60).contains(&b.minutes) {
        return Err(ApiError::bad_request("minutes must be 1–1440"));
    }
    if b.description.chars().count() > 20_000 {
        return Err(ApiError::bad_request("description is too long"));
    }
    let started = b
        .started_at
        .unwrap_or_else(|| Utc::now() - chrono::Duration::minutes(i64::from(b.minutes)));
    if started > Utc::now() + chrono::Duration::minutes(5) {
        return Err(ApiError::bad_request("cannot log work in the future"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let row = w::create_log(
        &mut t,
        c.user,
        item,
        started,
        b.minutes,
        b.description.trim(),
        "manual",
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn delete_log(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    w::delete_own_log(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- timer ----------

#[derive(Deserialize)]
pub struct TimerStart {
    work_item_id: Uuid,
}

pub async fn start_timer(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<TimerStart>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let row = w::start_timer(&mut t, c.user, b.work_item_id).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
pub struct TimerStop {
    #[serde(default)]
    description: String,
}

/// Stop the running timer and turn it into a work log (at least 1 minute, at most 24 hours).
pub async fn stop_timer(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<TimerStop>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    let timer = w::take_timer(&mut t, c.user).await?;
    let elapsed = (Utc::now() - timer.started_at).num_seconds().max(0);
    let minutes = i32::try_from(((elapsed + 59) / 60).clamp(1, 24 * 60)).unwrap_or(24 * 60);
    let row = w::create_log(
        &mut t,
        c.user,
        timer.work_item_id,
        timer.started_at,
        minutes,
        b.description.trim(),
        "timer",
    )
    .await?;
    t.commit().await?;
    Ok(Json(row))
}

// ---------- attachments ----------

fn kind_for(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("audio/") {
        "audio"
    } else if mime.starts_with("video/") {
        "video"
    } else {
        "file"
    }
}

/// Types we are willing to render inline. Everything else downloads as a file,
/// so an uploaded HTML or SVG page can never run script in our origin.
fn inline_safe(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf"
    ) || (mime.starts_with("audio/") || mime.starts_with("video/")) && !mime.contains(['+', ';'])
}

fn clean_name(name: &str) -> String {
    let n: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | '"'))
        .take(200)
        .collect();
    if n.trim().is_empty() {
        "file".into()
    } else {
        n
    }
}

pub async fn upload_attachment(
    State(s): State<AppState>,
    session: Session,
    Path(item): Path<Uuid>,
    mut form: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut work_log_id: Option<Uuid> = None;
    let mut spec_id: Option<Uuid> = None;
    let mut uploaded = Vec::new();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        match field.name() {
            Some("work_log_id") => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
                work_log_id = Some(
                    v.trim()
                        .parse()
                        .map_err(|_| ApiError::bad_request("bad work_log_id"))?,
                );
            }
            Some("spec_id") => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
                spec_id = Some(
                    v.trim()
                        .parse()
                        .map_err(|_| ApiError::bad_request("bad spec_id"))?,
                );
            }
            Some("file") => {
                let file_name = clean_name(field.file_name().unwrap_or("file"));
                let mime = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_ascii_lowercase();
                let id = Uuid::now_v7();
                let stored = s
                    .storage
                    .put(session.tenant_id, id, field, MAX_UPLOAD_BYTES)
                    .await
                    .map_err(|e| match e {
                        StorageError::TooLarge => ApiError::new(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "too_large",
                            "file is larger than 200 MB",
                        ),
                        StorageError::Upload(m) => ApiError::bad_request(m),
                        StorageError::Io(e) => {
                            tracing::error!(error = %e, "storage write failed");
                            ApiError::new(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "internal",
                                "could not store the file",
                            )
                        }
                    })?;
                uploaded.push((id, file_name, mime, stored));
            }
            _ => {}
        }
    }
    if uploaded.is_empty() {
        return Err(ApiError::bad_request("no file in the upload"));
    }
    let result: Result<Vec<w::AttachmentRow>, ApiError> = async {
        let (mut t, c) = begin(&s, &session).await?;
        let mut rows = Vec::new();
        for (id, name, mime, st) in &uploaded {
            rows.push(
                w::create_attachment(
                    &mut t,
                    w::NewAttachment {
                        id: *id,
                        work_item_id: item,
                        work_log_id,
                        kind: kind_for(mime),
                        file_name: name,
                        mime,
                        size_bytes: st.size,
                        sha256: &st.sha256,
                        object_key: &st.key,
                        uploaded_by: c.user,
                    },
                )
                .await?,
            );
        }
        if let Some(spec) = spec_id {
            for r in &rows {
                cm_store::flow::submit(&mut t, c.user, item, spec, "", Some(r.id), work_log_id)
                    .await?;
            }
        }
        t.commit().await?;
        Ok(rows)
    }
    .await;
    if result.is_err() {
        for (_, _, _, st) in &uploaded {
            s.storage.remove(&st.key).await;
        }
    }
    Ok((StatusCode::CREATED, Json(result?)))
}

pub async fn download_attachment(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (mut t, _) = begin(&s, &session).await?;
    let a = w::get_attachment(&mut t, id).await?;
    t.commit().await?;
    let file = s
        .storage
        .open(&a.object_key)
        .await
        .map_err(|_| ApiError::from(StoreError::NotFound))?;
    let inline = inline_safe(&a.mime);
    let ctype = if inline {
        a.mime.as_str()
    } else {
        "application/octet-stream"
    };
    let disposition = format!(
        "{}; filename*=UTF-8''{}",
        if inline { "inline" } else { "attachment" },
        a.file_name
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            })
            .collect::<String>()
    );
    let mut res = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(file)));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(ctype)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).unwrap_or(HeaderValue::from_static("attachment")),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_safe_types_render_inline() {
        assert!(inline_safe("image/png"));
        assert!(inline_safe("audio/webm"));
        assert!(!inline_safe("image/svg+xml"));
        assert!(!inline_safe("text/html"));
        assert!(!inline_safe("application/octet-stream"));
    }

    #[test]
    fn file_names_are_cleaned() {
        assert_eq!(clean_name("../../etc/passwd"), "....etcpasswd");
        assert_eq!(clean_name("a\"b\nc.png"), "abc.png");
        assert_eq!(clean_name("  "), "file");
    }
}
