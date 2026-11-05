//! Projects, work items, dependencies, history, work logs, attachments, timers.
//!
//! Foreign keys are checked by PostgreSQL without row-level security, so every
//! id taken from a request is first loaded through the tenant-scoped
//! transaction (`ensure_*`) before it is referenced.

use crate::{map_unique, Result, StoreError, TenantTx};
use chrono::{DateTime, NaiveDate, Utc};
use cm_domain::work_item::{Status, WorkItemKind};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Todo => "todo",
        Status::InProgress => "in_progress",
        Status::Blocked => "blocked",
        Status::InReview => "in_review",
        Status::Done => "done",
        Status::Cancelled => "cancelled",
    }
}

pub fn parse_status(s: &str) -> Option<Status> {
    Some(match s {
        "todo" => Status::Todo,
        "in_progress" => Status::InProgress,
        "blocked" => Status::Blocked,
        "in_review" => Status::InReview,
        "done" => Status::Done,
        "cancelled" => Status::Cancelled,
        _ => return None,
    })
}

fn kind_str(k: WorkItemKind) -> &'static str {
    match k {
        WorkItemKind::Phase => "phase",
        WorkItemKind::Task => "task",
        WorkItemKind::Milestone => "milestone",
        WorkItemKind::Routine => "routine",
        WorkItemKind::Request => "request",
    }
}

pub(crate) async fn event(
    t: &mut TenantTx,
    item: Uuid,
    actor: Uuid,
    kind: &str,
    payload: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO work_item_event (id, tenant_id, work_item_id, actor_id, kind, payload) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(Uuid::now_v7())
        .bind(t.tenant_id)
        .bind(item)
        .bind(actor)
        .bind(kind)
        .bind(payload)
        .execute(&mut *t.tx)
        .await?;
    Ok(())
}

pub async fn ensure_active_member(t: &mut TenantTx, user: Uuid) -> Result<()> {
    if crate::accounts::is_active_member(t, user).await? {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

// ---------- projects ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub department_id: Option<Uuid>,
    pub status: String,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub currency: String,
    /// Money: only for Finance/Admin. The API strips it for everyone else.
    pub labour_budget_minor: Option<i64>,
    pub expense_budget_minor: Option<i64>,
    pub customer: String,
    pub estimate_minutes: i64,
    pub logged_minutes: i64,
    pub open_items: i64,
    pub total_items: i64,
}

const PROJECT_SELECT: &str = "
    SELECT p.id, p.key, p.name, p.owner_id, u.display_name AS owner_name, p.department_id, p.status,
           p.planned_start, p.planned_end, p.currency, p.labour_budget_minor, p.expense_budget_minor, p.customer,
           COALESCE((SELECT sum(estimate_minutes) FROM work_item w WHERE w.project_id = p.id AND w.kind <> 'phase'), 0)::bigint AS estimate_minutes,
           COALESCE((SELECT sum(l.minutes) FROM work_log l JOIN work_item w ON w.id = l.work_item_id WHERE w.project_id = p.id), 0)::bigint AS logged_minutes,
           (SELECT count(*) FROM work_item w WHERE w.project_id = p.id AND w.kind NOT IN ('phase', 'milestone') AND w.status NOT IN ('done', 'cancelled')) AS open_items,
           (SELECT count(*) FROM work_item w WHERE w.project_id = p.id AND w.kind NOT IN ('phase', 'milestone')) AS total_items
    FROM project p JOIN app_user u ON u.id = p.owner_id";

pub struct NewProject<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub owner_id: Uuid,
    pub department_id: Option<Uuid>,
    pub currency: &'a str,
    pub labour_budget_minor: Option<i64>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
}

pub async fn create_project(t: &mut TenantTx, p: NewProject<'_>) -> Result<ProjectRow> {
    ensure_active_member(t, p.owner_id).await?;
    if let Some(d) = p.department_id {
        crate::get_department(t, d).await?;
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project (id, tenant_id, key, name, owner_id, department_id, currency, labour_budget_minor, planned_start, planned_end)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(p.key)
    .bind(p.name)
    .bind(p.owner_id)
    .bind(p.department_id)
    .bind(p.currency)
    .bind(p.labour_budget_minor)
    .bind(p.planned_start)
    .bind(p.planned_end)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| map_unique(e, "project key"))?;
    get_project(t, id).await
}

pub async fn list_projects(t: &mut TenantTx) -> Result<Vec<ProjectRow>> {
    Ok(
        sqlx::query_as::<_, ProjectRow>(&format!("{PROJECT_SELECT} ORDER BY p.created_at DESC"))
            .fetch_all(&mut *t.tx)
            .await?,
    )
}

pub async fn get_project(t: &mut TenantTx, id: Uuid) -> Result<ProjectRow> {
    sqlx::query_as::<_, ProjectRow>(&format!("{PROJECT_SELECT} WHERE p.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

// ---------- work items ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ItemRow {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub kind: String,
    pub key: Option<String>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub holder_user_id: Option<Uuid>,
    pub holder_name: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub estimate_minutes: Option<i32>,
    pub progress_pct: i16,
    pub sort_order: i32,
    pub version: i32,
    pub logged_minutes: i64,
    pub target_role_id: Option<Uuid>,
    pub target_department_id: Option<Uuid>,
    pub routine_template_id: Option<Uuid>,
    pub occurrence_date: Option<NaiveDate>,
    pub requested_by: Option<Uuid>,
    pub handoff_count: i64,
    pub open_handoff: Option<String>,
    /// Expense budget of a phase.
    pub budget_minor: Option<i64>,
    /// The request (e.g. a trip) this item was created for.
    pub claim_id: Option<Uuid>,
}

pub(crate) const ITEM_SELECT: &str = "
    SELECT w.id, w.project_id, w.parent_id, w.kind, w.key, w.title, w.description, w.status,
           w.holder_user_id, u.display_name AS holder_name, w.planned_start, w.planned_end,
           w.estimate_minutes, w.progress_pct, w.sort_order, w.version,
           COALESCE((SELECT sum(minutes) FROM work_log l WHERE l.work_item_id = w.id), 0)::bigint AS logged_minutes,
           w.target_role_id, w.target_department_id, w.routine_template_id, w.occurrence_date, w.requested_by,
           (SELECT count(*) FROM handoff h WHERE h.work_item_id = w.id AND h.state = 'accepted') AS handoff_count,
           (SELECT h.state FROM handoff h WHERE h.work_item_id = w.id AND h.state IN ('pending_approval', 'pending_accept')) AS open_handoff,
           w.budget_minor, w.claim_id
    FROM work_item w LEFT JOIN app_user u ON u.id = w.holder_user_id";

pub struct NewItem<'a> {
    pub project_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub kind: WorkItemKind,
    pub title: &'a str,
    pub description: &'a str,
    pub holder_user_id: Option<Uuid>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub estimate_minutes: Option<i32>,
}

pub async fn create_item(t: &mut TenantTx, actor: Uuid, n: NewItem<'_>) -> Result<ItemRow> {
    let project = get_project(t, n.project_id).await?;
    if let Some(p) = n.parent_id {
        let parent = get_item(t, p).await?;
        if parent.project_id != Some(n.project_id) {
            return Err(StoreError::NotFound);
        }
    }
    if let Some(h) = n.holder_user_id {
        ensure_active_member(t, h).await?;
    }
    let (num,): (i32,) = sqlx::query_as(
        "UPDATE project SET next_item_number = next_item_number + 1 WHERE id = $1 RETURNING next_item_number - 1",
    )
    .bind(n.project_id)
    .fetch_one(&mut *t.tx)
    .await?;
    let key = format!("{}-{num}", project.key);
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO work_item (id, tenant_id, project_id, parent_id, kind, key, title, description, holder_user_id,
                                planned_start, planned_end, estimate_minutes, sort_order, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(n.project_id)
    .bind(n.parent_id)
    .bind(kind_str(n.kind))
    .bind(&key)
    .bind(n.title)
    .bind(n.description)
    .bind(n.holder_user_id)
    .bind(n.planned_start)
    .bind(n.planned_end)
    .bind(n.estimate_minutes)
    .bind(num)
    .bind(actor)
    .execute(&mut *t.tx)
    .await?;
    event(
        t,
        id,
        actor,
        "created",
        json!({ "title": n.title, "holder": n.holder_user_id }),
    )
    .await?;
    get_item(t, id).await
}

pub async fn list_items(t: &mut TenantTx, project_id: Uuid) -> Result<Vec<ItemRow>> {
    Ok(sqlx::query_as::<_, ItemRow>(&format!(
        "{ITEM_SELECT} WHERE w.project_id = $1 ORDER BY w.sort_order"
    ))
    .bind(project_id)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Open items held by a person, across projects ("my work").
pub async fn list_items_held_by(t: &mut TenantTx, user: Uuid) -> Result<Vec<ItemRow>> {
    Ok(sqlx::query_as::<_, ItemRow>(&format!(
        "{ITEM_SELECT} WHERE w.holder_user_id = $1 AND w.status NOT IN ('done', 'cancelled') ORDER BY w.planned_end NULLS LAST"
    ))
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn get_item(t: &mut TenantTx, id: Uuid) -> Result<ItemRow> {
    sqlx::query_as::<_, ItemRow>(&format!("{ITEM_SELECT} WHERE w.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

/// Fields that may change in one edit. `None` leaves a field as it is; for
/// nullable fields, `Some(None)` clears it.
#[derive(Default)]
pub struct ItemPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub planned_start: Option<Option<NaiveDate>>,
    pub planned_end: Option<Option<NaiveDate>>,
    pub estimate_minutes: Option<Option<i32>>,
    pub progress_pct: Option<i16>,
    pub holder_user_id: Option<Option<Uuid>>,
}

/// Apply an edit with an optimistic version check; records one event per change kind.
pub async fn update_item(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    version: i32,
    p: ItemPatch,
) -> Result<ItemRow> {
    let cur = get_item(t, id).await?;
    if cur.version != version {
        return Err(StoreError::Conflict("a newer version of this item"));
    }
    if let Some(Some(h)) = p.holder_user_id {
        ensure_active_member(t, h).await?;
    }
    let start = p.planned_start.unwrap_or(cur.planned_start);
    let end = p.planned_end.unwrap_or(cur.planned_end);
    if let (Some(s), Some(e)) = (start, end) {
        if e < s {
            return Err(StoreError::Invalid("end date is before start date"));
        }
    }
    let holder = p.holder_user_id.unwrap_or(cur.holder_user_id);
    sqlx::query(
        "UPDATE work_item SET title = $2, description = $3, planned_start = $4, planned_end = $5,
                estimate_minutes = $6, progress_pct = $7, holder_user_id = $8, version = version + 1, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(p.title.as_deref().unwrap_or(&cur.title))
    .bind(p.description.as_deref().unwrap_or(&cur.description))
    .bind(start)
    .bind(end)
    .bind(p.estimate_minutes.unwrap_or(cur.estimate_minutes))
    .bind(p.progress_pct.unwrap_or(cur.progress_pct))
    .bind(holder)
    .execute(&mut *t.tx)
    .await?;
    if start != cur.planned_start || end != cur.planned_end {
        event(
            t,
            id,
            actor,
            "rescheduled",
            json!({ "from": [cur.planned_start, cur.planned_end], "to": [start, end] }),
        )
        .await?;
    }
    if holder != cur.holder_user_id {
        event(
            t,
            id,
            actor,
            "assigned",
            json!({ "from": cur.holder_user_id, "to": holder }),
        )
        .await?;
    }
    if p.title.as_ref().is_some_and(|x| *x != cur.title)
        || p.description
            .as_ref()
            .is_some_and(|x| *x != cur.description)
    {
        event(t, id, actor, "edited", json!({})).await?;
    }
    get_item(t, id).await
}

/// Change status after the domain rules approved it.
pub async fn set_status(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    version: i32,
    to: Status,
    note: Option<&str>,
) -> Result<ItemRow> {
    let cur = get_item(t, id).await?;
    if cur.version != version {
        return Err(StoreError::Conflict("a newer version of this item"));
    }
    let progress = if to == Status::Done {
        100
    } else {
        cur.progress_pct
    };
    sqlx::query("UPDATE work_item SET status = $2, progress_pct = $3, version = version + 1, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status_str(to))
        .bind(progress)
        .execute(&mut *t.tx)
        .await?;
    event(
        t,
        id,
        actor,
        "status_changed",
        json!({ "from": cur.status, "to": status_str(to), "note": note }),
    )
    .await?;
    get_item(t, id).await
}

// ---------- dependencies ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DependencyRow {
    pub from_item: Uuid,
    pub to_item: Uuid,
    pub kind: String,
    pub lag_days: i32,
}

pub async fn add_dependency(
    t: &mut TenantTx,
    actor: Uuid,
    from: Uuid,
    to: Uuid,
    kind: &str,
    lag_days: i32,
) -> Result<()> {
    if from == to {
        return Err(StoreError::Invalid("an item cannot depend on itself"));
    }
    let a = get_item(t, from).await?;
    let b = get_item(t, to).await?;
    if a.project_id.is_none() || a.project_id != b.project_id {
        return Err(StoreError::Invalid(
            "dependencies must be inside one project",
        ));
    }
    // Reject cycles: is `from` reachable from `to`?
    let (cycle,): (bool,) = sqlx::query_as(
        "WITH RECURSIVE reach(id) AS (
             SELECT to_item FROM dependency WHERE from_item = $1
             UNION SELECT d.to_item FROM dependency d JOIN reach r ON d.from_item = r.id)
         SELECT EXISTS (SELECT 1 FROM reach WHERE id = $2)",
    )
    .bind(to)
    .bind(from)
    .fetch_one(&mut *t.tx)
    .await?;
    if cycle {
        return Err(StoreError::Invalid("this dependency would create a loop"));
    }
    sqlx::query("INSERT INTO dependency (tenant_id, from_item, to_item, kind, lag_days) VALUES ($1, $2, $3, $4, $5)")
        .bind(t.tenant_id)
        .bind(from)
        .bind(to)
        .bind(kind)
        .bind(lag_days)
        .execute(&mut *t.tx)
        .await
        .map_err(|e| map_unique(e, "dependency"))?;
    event(
        t,
        to,
        actor,
        "dependency_added",
        json!({ "after": from, "kind": kind }),
    )
    .await?;
    Ok(())
}

pub async fn remove_dependency(t: &mut TenantTx, from: Uuid, to: Uuid) -> Result<()> {
    let r = sqlx::query("DELETE FROM dependency WHERE from_item = $1 AND to_item = $2")
        .bind(from)
        .bind(to)
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn list_dependencies(t: &mut TenantTx, project_id: Uuid) -> Result<Vec<DependencyRow>> {
    Ok(sqlx::query_as::<_, DependencyRow>(
        "SELECT d.from_item, d.to_item, d.kind, d.lag_days FROM dependency d
         JOIN work_item w ON w.id = d.to_item WHERE w.project_id = $1",
    )
    .bind(project_id)
    .fetch_all(&mut *t.tx)
    .await?)
}

// ---------- history ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EventRow {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub actor_name: String,
    pub kind: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub async fn list_events(t: &mut TenantTx, item: Uuid) -> Result<Vec<EventRow>> {
    Ok(sqlx::query_as::<_, EventRow>(
        "SELECT e.id, e.actor_id, u.display_name AS actor_name, e.kind, e.payload, e.created_at
         FROM work_item_event e JOIN app_user u ON u.id = e.actor_id
         WHERE e.work_item_id = $1 ORDER BY e.created_at",
    )
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

// ---------- work logs ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct LogRow {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub user_id: Uuid,
    pub user_name: String,
    pub started_at: DateTime<Utc>,
    pub minutes: i32,
    pub description: String,
    pub source: String,
    pub locked: bool,
}

const LOG_SELECT: &str = "
    SELECT l.id, l.work_item_id, l.user_id, u.display_name AS user_name, l.started_at, l.minutes,
           l.description, l.source, l.locked
    FROM work_log l JOIN app_user u ON u.id = l.user_id";

pub async fn create_log(
    t: &mut TenantTx,
    user: Uuid,
    item: Uuid,
    started_at: DateTime<Utc>,
    minutes: i32,
    description: &str,
    source: &str,
) -> Result<LogRow> {
    let it = get_item(t, item).await?;
    if matches!(it.status.as_str(), "cancelled") {
        return Err(StoreError::Invalid("cannot log work on a cancelled item"));
    }
    if crate::finance::in_closed_period(t, started_at.date_naive()).await? {
        return Err(StoreError::Invalid(
            "that date is in a closed timesheet period",
        ));
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO work_log (id, tenant_id, work_item_id, user_id, started_at, minutes, description, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(item)
    .bind(user)
    .bind(started_at)
    .bind(minutes)
    .bind(description)
    .bind(source)
    .execute(&mut *t.tx)
    .await?;
    event(
        t,
        item,
        user,
        "logged",
        json!({ "minutes": minutes, "log": id }),
    )
    .await?;
    crate::finance::freeze_cost(t, id).await?;
    if let Some(p) = it.project_id {
        crate::finance::evaluate_project(t, p).await?;
    }
    get_log(t, id).await
}

pub async fn get_log(t: &mut TenantTx, id: Uuid) -> Result<LogRow> {
    sqlx::query_as::<_, LogRow>(&format!("{LOG_SELECT} WHERE l.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn list_logs(t: &mut TenantTx, item: Uuid) -> Result<Vec<LogRow>> {
    Ok(sqlx::query_as::<_, LogRow>(&format!(
        "{LOG_SELECT} WHERE l.work_item_id = $1 ORDER BY l.started_at DESC"
    ))
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// People can delete their own logs while the period is open.
pub async fn delete_own_log(t: &mut TenantTx, user: Uuid, id: Uuid) -> Result<()> {
    let log = get_log(t, id).await?;
    if log.user_id != user {
        return Err(StoreError::NotFound);
    }
    if log.locked {
        return Err(StoreError::Invalid("this entry is in a closed period"));
    }
    sqlx::query("DELETE FROM work_log WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    event(
        t,
        log.work_item_id,
        user,
        "log_deleted",
        json!({ "minutes": log.minutes }),
    )
    .await?;
    if let Some(p) = get_item(t, log.work_item_id).await?.project_id {
        crate::finance::evaluate_project(t, p).await?;
    }
    Ok(())
}

// ---------- attachments ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub work_log_id: Option<Uuid>,
    pub kind: String,
    pub file_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(skip)]
    pub object_key: String,
    pub uploaded_by: Uuid,
    pub created_at: DateTime<Utc>,
}

const ATT_SELECT: &str = "SELECT id, work_item_id, work_log_id, kind, file_name, mime, size_bytes, sha256, object_key, uploaded_by, created_at FROM attachment";

pub struct NewAttachment<'a> {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub work_log_id: Option<Uuid>,
    pub kind: &'a str,
    pub file_name: &'a str,
    pub mime: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub object_key: &'a str,
    pub uploaded_by: Uuid,
}

pub async fn create_attachment(t: &mut TenantTx, a: NewAttachment<'_>) -> Result<AttachmentRow> {
    get_item(t, a.work_item_id).await?;
    if let Some(l) = a.work_log_id {
        let log = get_log(t, l).await?;
        if log.work_item_id != a.work_item_id {
            return Err(StoreError::NotFound);
        }
    }
    sqlx::query(
        "INSERT INTO attachment (id, tenant_id, work_item_id, work_log_id, kind, file_name, mime, size_bytes, sha256, object_key, uploaded_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(a.id)
    .bind(t.tenant_id)
    .bind(a.work_item_id)
    .bind(a.work_log_id)
    .bind(a.kind)
    .bind(a.file_name)
    .bind(a.mime)
    .bind(a.size_bytes)
    .bind(a.sha256)
    .bind(a.object_key)
    .bind(a.uploaded_by)
    .execute(&mut *t.tx)
    .await?;
    event(
        t,
        a.work_item_id,
        a.uploaded_by,
        "attachment_added",
        json!({ "name": a.file_name, "kind": a.kind }),
    )
    .await?;
    get_attachment(t, a.id).await
}

pub async fn get_attachment(t: &mut TenantTx, id: Uuid) -> Result<AttachmentRow> {
    sqlx::query_as::<_, AttachmentRow>(&format!("{ATT_SELECT} WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn list_attachments(t: &mut TenantTx, item: Uuid) -> Result<Vec<AttachmentRow>> {
    Ok(sqlx::query_as::<_, AttachmentRow>(&format!(
        "{ATT_SELECT} WHERE work_item_id = $1 ORDER BY created_at"
    ))
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

// ---------- timer ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TimerRow {
    pub work_item_id: Uuid,
    pub started_at: DateTime<Utc>,
}

pub async fn get_timer(t: &mut TenantTx, user: Uuid) -> Result<Option<TimerRow>> {
    Ok(sqlx::query_as::<_, TimerRow>(
        "SELECT work_item_id, started_at FROM active_timer WHERE user_id = $1",
    )
    .bind(user)
    .fetch_optional(&mut *t.tx)
    .await?)
}

pub async fn start_timer(t: &mut TenantTx, user: Uuid, item: Uuid) -> Result<TimerRow> {
    get_item(t, item).await?;
    sqlx::query("INSERT INTO active_timer (tenant_id, user_id, work_item_id) VALUES ($1, $2, $3)")
        .bind(t.tenant_id)
        .bind(user)
        .bind(item)
        .execute(&mut *t.tx)
        .await
        .map_err(|e| map_unique(e, "running timer"))?;
    get_timer(t, user).await?.ok_or(StoreError::NotFound)
}

pub async fn take_timer(t: &mut TenantTx, user: Uuid) -> Result<TimerRow> {
    sqlx::query_as::<_, TimerRow>(
        "DELETE FROM active_timer WHERE user_id = $1 RETURNING work_item_id, started_at",
    )
    .bind(user)
    .fetch_optional(&mut *t.tx)
    .await?
    .ok_or(StoreError::NotFound)
}
