//! Routine work (templates for people or roles), required deliverables,
//! requests, free-flow hand-offs and the built-in notification centre.

use crate::work::{ensure_active_member, event, get_item, get_project, ItemRow, ITEM_SELECT};
use crate::{Result, StoreError, TenantTx};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use cm_domain::deliverable::{self, DeliverableKind, DeliverableSpec, Shortfall};
use cm_domain::handoff::{self, Approvers, HandoffAction, HandoffState, Person};
use cm_domain::recurrence::Recurrence;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

pub async fn next_counter(t: &mut TenantTx, name: &str) -> Result<i32> {
    let (n,): (i32,) = sqlx::query_as(
        "INSERT INTO tenant_counter (tenant_id, name, next) VALUES ($1, $2, 2)
         ON CONFLICT (tenant_id, name) DO UPDATE SET next = tenant_counter.next + 1
         RETURNING next - 1",
    )
    .bind(t.tenant_id)
    .bind(name)
    .fetch_one(&mut *t.tx)
    .await?;
    Ok(n)
}

/// Insert an item that is not created through the project planner
/// (routine instances and requests). Returns the new id.
#[allow(clippy::too_many_arguments)]
async fn insert_loose_item(
    t: &mut TenantTx,
    actor: Uuid,
    kind: &str,
    key: &str,
    title: &str,
    description: &str,
    project_id: Option<Uuid>,
    holder: Option<Uuid>,
    target_role: Option<(Uuid, Uuid)>,
    template: Option<(Uuid, NaiveDate)>,
    due: Option<NaiveDate>,
    estimate: Option<i32>,
    requested_by: Option<Uuid>,
) -> Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO work_item (id, tenant_id, project_id, kind, key, title, description, holder_user_id,
                                target_role_id, target_department_id, routine_template_id, occurrence_date,
                                planned_start, planned_end, estimate_minutes, created_by, requested_by, sort_order)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 100000)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(project_id)
    .bind(kind)
    .bind(key)
    .bind(title)
    .bind(description)
    .bind(holder)
    .bind(target_role.map(|r| r.0))
    .bind(target_role.map(|r| r.1))
    .bind(template.map(|x| x.0))
    .bind(template.map(|x| x.1))
    .bind(template.map(|x| x.1).or(due))
    .bind(due)
    .bind(estimate)
    .bind(actor)
    .bind(requested_by)
    .execute(&mut *t.tx)
    .await?;
    event(
        t,
        id,
        actor,
        "created",
        json!({ "title": title, "holder": holder }),
    )
    .await?;
    Ok(id)
}

// ---------- notifications ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub kind: String,
    pub subject: String,
    pub body: String,
    pub link: String,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Add a notification to a person's inbox; important kinds are also emailed.
pub async fn notify(
    t: &mut TenantTx,
    user: Uuid,
    kind: &str,
    subject: &str,
    body: &str,
    link: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO notification (id, tenant_id, user_id, kind, subject, body, link) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::now_v7())
    .bind(t.tenant_id)
    .bind(user)
    .bind(kind)
    .bind(subject)
    .bind(body)
    .bind(link)
    .execute(&mut *t.tx)
    .await?;
    if matches!(kind, "handoff_approval" | "alert" | "handoff_offer") {
        let email: Option<(String,)> = sqlx::query_as("SELECT email FROM app_user WHERE id = $1")
            .bind(user)
            .fetch_optional(&mut *t.tx)
            .await?;
        if let Some((to,)) = email {
            crate::accounts::enqueue_email(&mut t.tx, &to, subject, body).await?;
        }
    }
    sqlx::query("SELECT pg_notify('cm_notify', $1)")
        .bind(user.to_string())
        .execute(&mut *t.tx)
        .await?;
    Ok(())
}

pub async fn list_notifications(t: &mut TenantTx, user: Uuid) -> Result<Vec<NotificationRow>> {
    Ok(sqlx::query_as::<_, NotificationRow>(
        "SELECT id, kind, subject, body, link, read_at, created_at FROM notification
         WHERE user_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn unread_count(t: &mut TenantTx, user: Uuid) -> Result<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM notification WHERE user_id = $1 AND read_at IS NULL")
            .bind(user)
            .fetch_one(&mut *t.tx)
            .await?;
    Ok(n)
}

pub async fn mark_read(t: &mut TenantTx, user: Uuid, id: Option<Uuid>) -> Result<()> {
    sqlx::query(
        "UPDATE notification SET read_at = now()
         WHERE user_id = $1 AND read_at IS NULL AND ($2::uuid IS NULL OR id = $2)",
    )
    .bind(user)
    .bind(id)
    .execute(&mut *t.tx)
    .await?;
    Ok(())
}

async fn admins(t: &mut TenantTx) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "SELECT user_id FROM membership WHERE tenant_id = $1 AND is_admin AND status = 'active'",
    )
    .bind(t.tenant_id)
    .fetch_all(&mut *t.tx)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect())
}

pub(crate) async fn finance_people(t: &mut TenantTx) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "SELECT user_id FROM membership WHERE tenant_id = $1 AND is_finance AND status = 'active'",
    )
    .bind(t.tenant_id)
    .fetch_all(&mut *t.tx)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect())
}

pub(crate) async fn admin_people(t: &mut TenantTx) -> Result<Vec<Uuid>> {
    admins(t).await
}

async fn display_name(t: &mut TenantTx, user: Uuid) -> Result<String> {
    let r: Option<(String,)> = sqlx::query_as("SELECT display_name FROM app_user WHERE id = $1")
        .bind(user)
        .fetch_optional(&mut *t.tx)
        .await?;
    Ok(r.map(|x| x.0).unwrap_or_default())
}

fn item_label(it: &ItemRow) -> String {
    match &it.key {
        Some(k) => format!("{k} {}", it.title),
        None => it.title.clone(),
    }
}

fn item_link(it: &ItemRow) -> String {
    match it.project_id {
        Some(p) => format!("/projects/{p}?item={}", it.id),
        None => format!("/my?item={}", it.id),
    }
}

// ---------- departments of people ----------

/// A person's home department: the department of their earliest current role,
/// otherwise a department they head. `None` when they have neither.
pub async fn home_department(t: &mut TenantTx, user: Uuid) -> Result<Option<Uuid>> {
    let r: Option<(Uuid,)> = sqlx::query_as(
        "SELECT department_id FROM (
             SELECT department_id, valid_from AS o FROM user_role
             WHERE user_id = $1 AND valid_from <= current_date AND (valid_to IS NULL OR valid_to >= current_date)
             UNION ALL
             SELECT id, '9999-12-31'::date FROM department WHERE head_user_id = $1
         ) x ORDER BY o LIMIT 1",
    )
    .bind(user)
    .fetch_optional(&mut *t.tx)
    .await?;
    Ok(r.map(|x| x.0))
}

/// Is `user` the head (or a deputy) of `dept`?
pub async fn heads_department(t: &mut TenantTx, user: Uuid, dept: Uuid) -> Result<bool> {
    let r: Option<(bool,)> = sqlx::query_as(
        "SELECT true FROM department WHERE id = $2 AND head_user_id = $1
         UNION ALL SELECT true FROM department_deputy WHERE department_id = $2 AND user_id = $1 LIMIT 1",
    )
    .bind(user)
    .bind(dept)
    .fetch_optional(&mut *t.tx)
    .await?;
    Ok(r.is_some())
}

/// Departments someone heads or deputises for.
pub async fn headed_departments(t: &mut TenantTx, user: Uuid) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM department WHERE head_user_id = $1
         UNION SELECT department_id FROM department_deputy WHERE user_id = $1",
    )
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect())
}

async fn person(t: &mut TenantTx, user: Uuid) -> Result<Person> {
    let active = crate::accounts::is_active_member(t, user).await?;
    let dept = home_department(t, user).await?;
    Ok(Person {
        user_id: user,
        department_id: dept.unwrap_or(Uuid::nil()),
        active,
    })
}

/// Who may approve a hand-off to `target`: the head and deputies of the
/// target's department (Admins when there is no head), plus the project PM.
async fn approvers_for(t: &mut TenantTx, target: Uuid, project: Option<Uuid>) -> Result<Approvers> {
    let mut a = Approvers::default();
    if let Some(d) = home_department(t, target).await? {
        let head: Option<(Option<Uuid>,)> =
            sqlx::query_as("SELECT head_user_id FROM department WHERE id = $1")
                .bind(d)
                .fetch_optional(&mut *t.tx)
                .await?;
        a.target_department_head = head.and_then(|h| h.0);
        a.target_department_deputies = sqlx::query_as::<_, (Uuid,)>(
            "SELECT user_id FROM department_deputy WHERE department_id = $1",
        )
        .bind(d)
        .fetch_all(&mut *t.tx)
        .await?
        .into_iter()
        .map(|r| r.0)
        .collect();
    }
    if a.target_department_head.is_none() {
        a.target_department_deputies.extend(admins(t).await?);
    }
    if let Some(p) = project {
        a.project_pm = Some(get_project(t, p).await?.owner_id);
    }
    Ok(a)
}

fn approver_list(a: &Approvers) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = a
        .target_department_head
        .iter()
        .chain(a.target_department_deputies.iter())
        .chain(a.project_pm.iter())
        .copied()
        .collect();
    v.sort();
    v.dedup();
    v
}

// ---------- routine templates ----------

pub struct NewSpec {
    pub kind: DeliverableKind,
    pub min_count: i32,
    pub required: bool,
    pub instructions: String,
}

pub fn kind_str(k: DeliverableKind) -> &'static str {
    match k {
        DeliverableKind::Text => "text",
        DeliverableKind::Image => "image",
        DeliverableKind::Audio => "audio",
        DeliverableKind::Video => "video",
        DeliverableKind::File => "file",
        DeliverableKind::GitCommit => "git_commit",
        DeliverableKind::Link => "link",
        DeliverableKind::Checklist => "checklist",
    }
}

pub fn parse_kind(s: &str) -> Option<DeliverableKind> {
    Some(match s {
        "text" => DeliverableKind::Text,
        "image" => DeliverableKind::Image,
        "audio" => DeliverableKind::Audio,
        "video" => DeliverableKind::Video,
        "file" => DeliverableKind::File,
        "git_commit" => DeliverableKind::GitCommit,
        "link" => DeliverableKind::Link,
        "checklist" => DeliverableKind::Checklist,
        _ => return None,
    })
}

pub struct NewTemplate<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub recurrence: &'a str,
    pub weekdays: Vec<i16>,
    pub target_user_id: Option<Uuid>,
    pub target_role_id: Option<Uuid>,
    pub target_department_id: Option<Uuid>,
    pub mode: &'a str,
    pub estimate_minutes: Option<i32>,
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TemplateRow {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub recurrence: String,
    pub weekdays: Vec<i16>,
    pub target_user_id: Option<Uuid>,
    pub target_user_name: Option<String>,
    pub target_role_id: Option<Uuid>,
    pub target_role_name: Option<String>,
    pub target_department_id: Option<Uuid>,
    pub target_department_name: Option<String>,
    pub mode: String,
    pub estimate_minutes: Option<i32>,
    pub project_id: Option<Uuid>,
    pub active: bool,
    pub created_by: Uuid,
    pub created_by_name: String,
    pub open_items: i64,
}

const TEMPLATE_SELECT: &str = "
    SELECT r.id, r.title, r.description, r.recurrence, r.weekdays, r.target_user_id, u.display_name AS target_user_name,
           r.target_role_id, jr.name AS target_role_name, r.target_department_id, d.name AS target_department_name,
           r.mode, r.estimate_minutes, r.project_id, r.active, r.created_by, c.display_name AS created_by_name,
           (SELECT count(*) FROM work_item w WHERE w.routine_template_id = r.id AND w.status NOT IN ('done', 'cancelled')) AS open_items
    FROM routine_template r
    LEFT JOIN app_user u ON u.id = r.target_user_id
    LEFT JOIN job_role jr ON jr.id = r.target_role_id
    LEFT JOIN department d ON d.id = r.target_department_id
    JOIN app_user c ON c.id = r.created_by";

pub async fn create_template(
    t: &mut TenantTx,
    actor: Uuid,
    n: NewTemplate<'_>,
    specs: &[NewSpec],
) -> Result<TemplateRow> {
    if Recurrence::parse(n.recurrence, &n.weekdays).is_none() {
        return Err(StoreError::Invalid(
            "recurrence: daily, weekdays, or weekly with days",
        ));
    }
    if let Some(u) = n.target_user_id {
        ensure_active_member(t, u).await?;
    }
    if let Some(r) = n.target_role_id {
        let found: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM job_role WHERE id = $1")
            .bind(r)
            .fetch_optional(&mut *t.tx)
            .await?;
        found.ok_or(StoreError::NotFound)?;
    }
    if let Some(d) = n.target_department_id {
        crate::get_department(t, d).await?;
    }
    if let Some(p) = n.project_id {
        get_project(t, p).await?;
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO routine_template (id, tenant_id, title, description, recurrence, weekdays, target_user_id,
                                       target_role_id, target_department_id, mode, estimate_minutes, project_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(n.title)
    .bind(n.description)
    .bind(n.recurrence)
    .bind(&n.weekdays)
    .bind(n.target_user_id)
    .bind(n.target_role_id)
    .bind(n.target_department_id)
    .bind(n.mode)
    .bind(n.estimate_minutes)
    .bind(n.project_id)
    .bind(actor)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(d) if d.is_check_violation() => {
            StoreError::Invalid("target: a person, or a role in a department")
        }
        _ => StoreError::Db(e),
    })?;
    for (i, s) in specs.iter().enumerate() {
        insert_spec(t, Some(id), None, s, i).await?;
    }
    get_template(t, id).await
}

async fn insert_spec(
    t: &mut TenantTx,
    template: Option<Uuid>,
    item: Option<Uuid>,
    s: &NewSpec,
    order: usize,
) -> Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO deliverable_spec (id, tenant_id, template_id, work_item_id, kind, min_count, required, instructions, sort_order)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(template)
    .bind(item)
    .bind(kind_str(s.kind))
    .bind(s.min_count)
    .bind(s.required)
    .bind(&s.instructions)
    .bind(i32::try_from(order).unwrap_or(0))
    .execute(&mut *t.tx)
    .await?;
    Ok(id)
}

pub async fn list_templates(t: &mut TenantTx) -> Result<Vec<TemplateRow>> {
    Ok(sqlx::query_as::<_, TemplateRow>(&format!(
        "{TEMPLATE_SELECT} ORDER BY r.active DESC, r.created_at DESC"
    ))
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn get_template(t: &mut TenantTx, id: Uuid) -> Result<TemplateRow> {
    sqlx::query_as::<_, TemplateRow>(&format!("{TEMPLATE_SELECT} WHERE r.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn set_template_active(t: &mut TenantTx, id: Uuid, active: bool) -> Result<TemplateRow> {
    let r = sqlx::query("UPDATE routine_template SET active = $2 WHERE id = $1")
        .bind(id)
        .bind(active)
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    get_template(t, id).await
}

/// People holding a role in a department on a date (active members only).
pub async fn role_holders(
    t: &mut TenantTx,
    role: Uuid,
    dept: Uuid,
    on: NaiveDate,
) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "SELECT DISTINCT ur.user_id FROM user_role ur
         JOIN membership m ON m.user_id = ur.user_id AND m.tenant_id = ur.tenant_id AND m.status = 'active'
         WHERE ur.role_id = $1 AND ur.department_id = $2 AND ur.valid_from <= $3 AND (ur.valid_to IS NULL OR ur.valid_to >= $3)",
    )
    .bind(role)
    .bind(dept)
    .bind(on)
    .fetch_all(&mut *t.tx)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect())
}

/// The tenant's local date today (routine generation runs in company time).
pub async fn tenant_today(t: &mut TenantTx) -> Result<NaiveDate> {
    let (d,): (NaiveDate,) =
        sqlx::query_as("SELECT (now() AT TIME ZONE timezone)::date FROM tenant WHERE id = $1")
            .bind(t.tenant_id)
            .fetch_one(&mut *t.tx)
            .await?;
    Ok(d)
}

/// Create the items for every active template that fires on `date`.
/// Idempotent: running it twice creates nothing new. Returns the number created.
pub async fn generate_for_date(t: &mut TenantTx, date: NaiveDate) -> Result<usize> {
    let templates: Vec<TemplateRow> =
        sqlx::query_as::<_, TemplateRow>(&format!("{TEMPLATE_SELECT} WHERE r.active"))
            .fetch_all(&mut *t.tx)
            .await?;
    let weekday = u8::try_from(date.weekday().number_from_monday()).unwrap_or(1);
    let mut created = 0;
    for tpl in templates {
        let Some(rec) = Recurrence::parse(&tpl.recurrence, &tpl.weekdays) else {
            continue;
        };
        if !rec.fires_on(weekday) {
            continue;
        }
        let title = format!("{} · {}", tpl.title, date.format("%m-%d"));
        match (
            tpl.target_user_id,
            tpl.target_role_id,
            tpl.target_department_id,
        ) {
            (Some(u), _, _) => {
                if crate::accounts::is_active_member(t, u).await?
                    && create_instance(t, &tpl, date, &title, Some(u), None).await?
                {
                    created += 1;
                }
            }
            (None, Some(role), Some(dept)) if tpl.mode == "pool" => {
                if create_instance(t, &tpl, date, &title, None, Some((role, dept))).await? {
                    created += 1;
                    for u in role_holders(t, role, dept, date).await? {
                        notify(
                            t,
                            u,
                            "pool_item",
                            &format!(
                                "{title} — {}",
                                tpl.target_role_name.as_deref().unwrap_or("")
                            ),
                            "",
                            "/my",
                        )
                        .await?;
                    }
                }
            }
            (None, Some(role), Some(dept)) => {
                for u in role_holders(t, role, dept, date).await? {
                    if create_instance(t, &tpl, date, &title, Some(u), None).await? {
                        created += 1;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(created)
}

async fn create_instance(
    t: &mut TenantTx,
    tpl: &TemplateRow,
    date: NaiveDate,
    title: &str,
    holder: Option<Uuid>,
    pool: Option<(Uuid, Uuid)>,
) -> Result<bool> {
    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM work_item WHERE routine_template_id = $1 AND occurrence_date = $2
           AND (($3::uuid IS NULL AND target_role_id IS NOT NULL) OR holder_user_id = $3)",
    )
    .bind(tpl.id)
    .bind(date)
    .bind(holder)
    .fetch_optional(&mut *t.tx)
    .await?;
    if exists.is_some() {
        return Ok(false);
    }
    let key = format!("RT-{}", next_counter(t, "RT").await?);
    insert_loose_item(
        t,
        tpl.created_by,
        "routine",
        &key,
        title,
        &tpl.description,
        tpl.project_id,
        holder,
        pool,
        Some((tpl.id, date)),
        Some(date),
        tpl.estimate_minutes,
        None,
    )
    .await?;
    Ok(true)
}

/// Pool items the person may claim: open, unheld, for a role they hold.
pub async fn claimable_items(t: &mut TenantTx, user: Uuid) -> Result<Vec<ItemRow>> {
    Ok(sqlx::query_as::<_, ItemRow>(&format!(
        "{ITEM_SELECT} WHERE w.holder_user_id IS NULL AND w.target_role_id IS NOT NULL
           AND w.status NOT IN ('done', 'cancelled')
           AND EXISTS (SELECT 1 FROM user_role ur WHERE ur.user_id = $1 AND ur.role_id = w.target_role_id
                       AND ur.department_id = w.target_department_id
                       AND (ur.valid_to IS NULL OR ur.valid_to >= current_date))
         ORDER BY w.occurrence_date"
    ))
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// First holder of the role to claim a pool item takes it.
pub async fn claim(t: &mut TenantTx, user: Uuid, item: Uuid) -> Result<ItemRow> {
    let ok = claimable_items(t, user).await?.iter().any(|i| i.id == item);
    if !ok {
        return Err(StoreError::Invalid(
            "this item is not open for you to claim",
        ));
    }
    let r = sqlx::query(
        "UPDATE work_item SET holder_user_id = $2, version = version + 1, updated_at = now()
         WHERE id = $1 AND holder_user_id IS NULL",
    )
    .bind(item)
    .bind(user)
    .execute(&mut *t.tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::Conflict("a claim on this item"));
    }
    event(t, item, user, "claimed", json!({})).await?;
    get_item(t, item).await
}

// ---------- deliverables ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SpecRow {
    pub id: Uuid,
    pub kind: String,
    pub min_count: i32,
    pub required: bool,
    pub instructions: String,
    pub submitted: i64,
}

/// Specs that apply to an item (its own, plus its routine template's), with counts.
pub async fn item_specs(t: &mut TenantTx, item: &ItemRow) -> Result<Vec<SpecRow>> {
    Ok(sqlx::query_as::<_, SpecRow>(
        "SELECT s.id, s.kind, s.min_count, s.required, s.instructions,
                (SELECT count(*) FROM deliverable_submission d WHERE d.spec_id = s.id AND d.work_item_id = $1) AS submitted
         FROM deliverable_spec s
         WHERE s.work_item_id = $1 OR ($2::uuid IS NOT NULL AND s.template_id = $2)
         ORDER BY s.sort_order, s.id",
    )
    .bind(item.id)
    .bind(item.routine_template_id)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn template_specs(t: &mut TenantTx, template: Uuid) -> Result<Vec<SpecRow>> {
    Ok(sqlx::query_as::<_, SpecRow>(
        "SELECT id, kind, min_count, required, instructions, 0::bigint AS submitted
         FROM deliverable_spec WHERE template_id = $1 ORDER BY sort_order",
    )
    .bind(template)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn add_item_spec(t: &mut TenantTx, item: Uuid, s: &NewSpec) -> Result<Uuid> {
    get_item(t, item).await?;
    insert_spec(t, None, Some(item), s, 1000).await
}

pub async fn shortfalls(t: &mut TenantTx, item: &ItemRow) -> Result<Vec<Shortfall>> {
    let rows = item_specs(t, item).await?;
    let specs: Vec<DeliverableSpec> = rows
        .iter()
        .filter_map(|r| {
            Some(DeliverableSpec {
                kind: parse_kind(&r.kind)?,
                min_count: u32::try_from(r.min_count).unwrap_or(1),
                required: r.required,
            })
        })
        .collect();
    let counts: Vec<u32> = rows
        .iter()
        .filter(|r| parse_kind(&r.kind).is_some())
        .map(|r| u32::try_from(r.submitted).unwrap_or(0))
        .collect();
    Ok(deliverable::shortfalls(&specs, &counts))
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SubmissionRow {
    pub id: Uuid,
    pub spec_id: Uuid,
    pub work_log_id: Option<Uuid>,
    pub attachment_id: Option<Uuid>,
    pub value: String,
    pub submitted_by: Uuid,
    pub submitted_by_name: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_submissions(t: &mut TenantTx, item: Uuid) -> Result<Vec<SubmissionRow>> {
    Ok(sqlx::query_as::<_, SubmissionRow>(
        "SELECT d.id, d.spec_id, d.work_log_id, d.attachment_id, d.value, d.submitted_by, u.display_name AS submitted_by_name, d.created_at
         FROM deliverable_submission d JOIN app_user u ON u.id = d.submitted_by
         WHERE d.work_item_id = $1 ORDER BY d.created_at",
    )
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Record one deliverable for a spec of an item. Files must already be
/// attachments of the same item and match the spec's media kind.
pub async fn submit(
    t: &mut TenantTx,
    user: Uuid,
    item: Uuid,
    spec: Uuid,
    value: &str,
    attachment: Option<Uuid>,
    log: Option<Uuid>,
) -> Result<Uuid> {
    let it = get_item(t, item).await?;
    let specs = item_specs(t, &it).await?;
    let s = specs
        .iter()
        .find(|s| s.id == spec)
        .ok_or(StoreError::NotFound)?;
    let media = matches!(s.kind.as_str(), "image" | "audio" | "video" | "file");
    if media {
        let a = crate::work::get_attachment(
            t,
            attachment.ok_or(StoreError::Invalid("a file is required"))?,
        )
        .await?;
        if a.work_item_id != item {
            return Err(StoreError::NotFound);
        }
        if s.kind != "file" && a.kind != s.kind {
            return Err(StoreError::Invalid(
                "the file type does not match this deliverable",
            ));
        }
    } else {
        let v = value.trim();
        if v.is_empty() {
            return Err(StoreError::Invalid("a value is required"));
        }
        if s.kind == "link" && !(v.starts_with("https://") || v.starts_with("http://")) {
            return Err(StoreError::Invalid(
                "a link must start with http:// or https://",
            ));
        }
        if s.kind == "git_commit" && !(7..=64).contains(&v.len()) && !v.contains("/commit/") {
            return Err(StoreError::Invalid("give a commit SHA or commit URL"));
        }
    }
    if let Some(l) = log {
        if crate::work::get_log(t, l).await?.work_item_id != item {
            return Err(StoreError::NotFound);
        }
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO deliverable_submission (id, tenant_id, spec_id, work_item_id, work_log_id, attachment_id, value, submitted_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(spec)
    .bind(item)
    .bind(log)
    .bind(attachment)
    .bind(value.trim())
    .bind(user)
    .execute(&mut *t.tx)
    .await?;
    event(
        t,
        item,
        user,
        "deliverable_submitted",
        json!({ "kind": s.kind }),
    )
    .await?;
    Ok(id)
}

/// Routine items whose required deliverables are missing, for supervisors.
pub async fn missing_deliverables(
    t: &mut TenantTx,
    since: NaiveDate,
) -> Result<Vec<serde_json::Value>> {
    let items: Vec<ItemRow> = sqlx::query_as::<_, ItemRow>(&format!(
        "{ITEM_SELECT} WHERE w.routine_template_id IS NOT NULL AND w.occurrence_date >= $1 AND w.status <> 'cancelled'
         ORDER BY w.occurrence_date DESC"
    ))
    .bind(since)
    .fetch_all(&mut *t.tx)
    .await?;
    let mut out = Vec::new();
    for it in items {
        let sf = shortfalls(t, &it).await?;
        if !sf.is_empty() {
            out.push(json!({
                "item": it,
                "missing": sf.iter().map(|s| json!({ "kind": kind_str(s.kind), "needed": s.needed, "have": s.have })).collect::<Vec<_>>(),
            }));
        }
    }
    Ok(out)
}

// ---------- requests ----------

pub struct NewRequest<'a> {
    pub to_user: Uuid,
    pub title: &'a str,
    pub description: &'a str,
    pub due: Option<NaiveDate>,
    pub project_id: Option<Uuid>,
    pub estimate_minutes: Option<i32>,
}

pub async fn create_request(t: &mut TenantTx, actor: Uuid, r: NewRequest<'_>) -> Result<ItemRow> {
    ensure_active_member(t, r.to_user).await?;
    if let Some(p) = r.project_id {
        get_project(t, p).await?;
    }
    let key = format!("REQ-{}", next_counter(t, "REQ").await?);
    let id = insert_loose_item(
        t,
        actor,
        "request",
        &key,
        r.title,
        r.description,
        r.project_id,
        Some(r.to_user),
        None,
        None,
        r.due,
        r.estimate_minutes,
        Some(actor),
    )
    .await?;
    let it = get_item(t, id).await?;
    if r.to_user != actor {
        let from = display_name(t, actor).await?;
        notify(
            t,
            r.to_user,
            "request",
            &format!("{from}: {}", item_label(&it)),
            r.description,
            &item_link(&it),
        )
        .await?;
    }
    Ok(it)
}

// ---------- hand-offs ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HandoffRow {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub item_key: Option<String>,
    pub item_title: String,
    pub project_id: Option<Uuid>,
    pub from_user: Uuid,
    pub from_name: String,
    pub to_user: Uuid,
    pub to_name: String,
    pub requested_by: Uuid,
    pub reason: String,
    pub note: String,
    pub new_due: Option<NaiveDate>,
    pub needs_approval: bool,
    pub approved_by: Option<Uuid>,
    pub state: String,
    pub decision_note: String,
    pub is_return: bool,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

const HANDOFF_SELECT: &str = "
    SELECT h.id, h.work_item_id, w.key AS item_key, w.title AS item_title, w.project_id,
           h.from_user, fu.display_name AS from_name, h.to_user, tu.display_name AS to_name, h.requested_by,
           h.reason, h.note, h.new_due, h.needs_approval, h.approved_by, h.state, h.decision_note, h.is_return,
           h.created_at, h.decided_at
    FROM handoff h JOIN work_item w ON w.id = h.work_item_id
    JOIN app_user fu ON fu.id = h.from_user JOIN app_user tu ON tu.id = h.to_user";

fn state_str(s: HandoffState) -> &'static str {
    match s {
        HandoffState::PendingApproval => "pending_approval",
        HandoffState::PendingAccept => "pending_accept",
        HandoffState::Accepted => "accepted",
        HandoffState::Declined => "declined",
        HandoffState::Rejected => "rejected",
        HandoffState::Cancelled => "cancelled",
    }
}

fn parse_state(s: &str) -> Option<HandoffState> {
    Some(match s {
        "pending_approval" => HandoffState::PendingApproval,
        "pending_accept" => HandoffState::PendingAccept,
        "accepted" => HandoffState::Accepted,
        "declined" => HandoffState::Declined,
        "rejected" => HandoffState::Rejected,
        "cancelled" => HandoffState::Cancelled,
        _ => return None,
    })
}

pub async fn get_handoff(t: &mut TenantTx, id: Uuid) -> Result<HandoffRow> {
    sqlx::query_as::<_, HandoffRow>(&format!("{HANDOFF_SELECT} WHERE h.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn list_handoffs(t: &mut TenantTx, item: Uuid) -> Result<Vec<HandoffRow>> {
    Ok(sqlx::query_as::<_, HandoffRow>(&format!(
        "{HANDOFF_SELECT} WHERE h.work_item_id = $1 ORDER BY h.created_at"
    ))
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Who may start a hand-off of an item: its holder, the project PM, the
/// holder's department head, or an Admin.
pub async fn may_reassign(
    t: &mut TenantTx,
    actor: Uuid,
    is_admin: bool,
    it: &ItemRow,
) -> Result<bool> {
    if is_admin
        || it.holder_user_id == Some(actor)
        || it.requested_by == Some(actor) && it.holder_user_id.is_none()
    {
        return Ok(true);
    }
    if let Some(p) = it.project_id {
        if get_project(t, p).await?.owner_id == actor {
            return Ok(true);
        }
    }
    if let Some(h) = it.holder_user_id {
        if let Some(d) = home_department(t, h).await? {
            return heads_department(t, actor, d).await;
        }
    }
    Ok(false)
}

pub struct NewHandoff<'a> {
    pub item: Uuid,
    pub version: i32,
    pub to_user: Uuid,
    pub reason: &'a str,
    pub note: &'a str,
    pub new_due: Option<NaiveDate>,
}

pub async fn request_handoff(
    t: &mut TenantTx,
    actor: Uuid,
    n: NewHandoff<'_>,
) -> Result<HandoffRow> {
    let it = get_item(t, n.item).await?;
    if it.version != n.version {
        return Err(StoreError::Conflict("a newer version of this item"));
    }
    let open = !matches!(it.status.as_str(), "done" | "cancelled");
    let sender = it.holder_user_id.unwrap_or(actor);
    let from = person(t, sender).await?;
    let to = person(t, n.to_user).await?;
    let state = handoff::request(open, sender, &from, &to, n.reason).map_err(|e| match e {
        handoff::HandoffError::ItemClosed => StoreError::Invalid("the work item is closed"),
        handoff::HandoffError::TargetInactive => {
            StoreError::Invalid("that person is not active in this company")
        }
        handoff::HandoffError::SameHolder => {
            StoreError::Invalid("that person already holds this item")
        }
        handoff::HandoffError::MissingReason => StoreError::Invalid("a reason is required"),
        _ => StoreError::Invalid("hand-off not allowed"),
    })?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO handoff (id, tenant_id, work_item_id, from_user, to_user, requested_by, reason, note, new_due, needs_approval, state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(n.item)
    .bind(sender)
    .bind(n.to_user)
    .bind(actor)
    .bind(n.reason.trim())
    .bind(n.note.trim())
    .bind(n.new_due)
    .bind(state == HandoffState::PendingApproval)
    .bind(state_str(state))
    .execute(&mut *t.tx)
    .await
    .map_err(|e| crate::map_unique(e, "an open hand-off for this item"))?;
    // Bump the version so concurrent hand-offs of the same item conflict.
    sqlx::query("UPDATE work_item SET version = version + 1 WHERE id = $1")
        .bind(n.item)
        .execute(&mut *t.tx)
        .await?;
    let to_name = display_name(t, n.to_user).await?;
    event(
        t,
        n.item,
        actor,
        "handoff_requested",
        json!({ "to": n.to_user, "to_name": to_name, "reason": n.reason.trim(), "needs_approval": state == HandoffState::PendingApproval }),
    )
    .await?;
    let from_name = display_name(t, sender).await?;
    let label = item_label(&it);
    let link = item_link(&it);
    if state == HandoffState::PendingApproval {
        let a = approvers_for(t, n.to_user, it.project_id).await?;
        for u in approver_list(&a) {
            notify(
                t,
                u,
                "handoff_approval",
                &format!("{label}: {from_name} → {to_name}"),
                n.reason,
                "/inbox",
            )
            .await?;
        }
    } else {
        notify(
            t,
            n.to_user,
            "handoff_offer",
            &format!("{from_name} → {label}"),
            n.reason,
            &link,
        )
        .await?;
    }
    get_handoff(t, id).await
}

pub async fn decide_handoff(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    action: HandoffAction,
    note: &str,
) -> Result<HandoffRow> {
    let h = get_handoff(t, id).await?;
    let it = get_item(t, h.work_item_id).await?;
    let state = parse_state(&h.state).ok_or(StoreError::Invalid("unknown state"))?;
    let approvers = if state == HandoffState::PendingApproval {
        approvers_for(t, h.to_user, it.project_id).await?
    } else {
        Approvers::default()
    };
    // The sender of record is the holder at request time; the person who
    // asked may also cancel.
    let sender = if action == HandoffAction::Cancel && actor == h.requested_by {
        actor
    } else {
        h.from_user
    };
    let next = handoff::apply(state, action, actor, sender, h.to_user, &approvers).map_err(
        |e| match e {
            handoff::HandoffError::NotAnApprover => {
                StoreError::Invalid("you may not approve this hand-off")
            }
            handoff::HandoffError::NotTheTarget => {
                StoreError::Invalid("only the receiver can accept or decline")
            }
            handoff::HandoffError::NotTheSender => {
                StoreError::Invalid("only the sender can cancel")
            }
            _ => StoreError::Invalid("this hand-off is no longer open for that action"),
        },
    )?;
    let approved_by = if action == HandoffAction::Approve {
        Some(actor)
    } else {
        h.approved_by
    };
    let done = !next.is_open();
    sqlx::query(
        "UPDATE handoff SET state = $2, approved_by = $3, decision_note = CASE WHEN $4 = '' THEN decision_note ELSE $4 END,
                decided_at = CASE WHEN $5 THEN now() ELSE decided_at END
         WHERE id = $1",
    )
    .bind(id)
    .bind(state_str(next))
    .bind(approved_by)
    .bind(note.trim())
    .bind(done)
    .execute(&mut *t.tx)
    .await?;
    let label = item_label(&it);
    let link = item_link(&it);
    let actor_name = display_name(t, actor).await?;
    match next {
        HandoffState::Accepted => {
            move_item(t, actor, &it, h.from_user, h.to_user, h.new_due, &h.reason).await?;
            notify(
                t,
                h.from_user,
                "handoff_accepted",
                &format!("{actor_name} ✓ {label}"),
                note,
                &link,
            )
            .await?;
        }
        HandoffState::PendingAccept => {
            event(
                t,
                it.id,
                actor,
                "handoff_approved",
                json!({ "note": note.trim() }),
            )
            .await?;
            notify(
                t,
                h.to_user,
                "handoff_offer",
                &format!("{} → {label}", h.from_name),
                &h.reason,
                &link,
            )
            .await?;
            notify(
                t,
                h.from_user,
                "handoff_approved",
                &format!("{actor_name} ✓ {label}"),
                note,
                &link,
            )
            .await?;
        }
        HandoffState::Rejected | HandoffState::Declined => {
            let kind = if next == HandoffState::Rejected {
                "handoff_rejected"
            } else {
                "handoff_declined"
            };
            event(t, it.id, actor, kind, json!({ "note": note.trim() })).await?;
            notify(
                t,
                h.from_user,
                kind,
                &format!("{actor_name} ✗ {label}"),
                note,
                &link,
            )
            .await?;
        }
        HandoffState::Cancelled => {
            event(t, it.id, actor, "handoff_cancelled", json!({})).await?;
        }
        HandoffState::PendingApproval => {}
    }
    get_handoff(t, id).await
}

async fn move_item(
    t: &mut TenantTx,
    actor: Uuid,
    it: &ItemRow,
    from: Uuid,
    to: Uuid,
    due: Option<NaiveDate>,
    reason: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE work_item SET holder_user_id = $2, planned_end = COALESCE($3, planned_end),
                planned_start = CASE WHEN $3 IS NOT NULL AND planned_start > $3 THEN $3 ELSE planned_start END,
                version = version + 1, updated_at = now()
         WHERE id = $1",
    )
    .bind(it.id)
    .bind(to)
    .bind(due)
    .execute(&mut *t.tx)
    .await?;
    let from_name = display_name(t, from).await?;
    let to_name = display_name(t, to).await?;
    event(
        t,
        it.id,
        actor,
        "reassigned",
        json!({ "from": from, "to": to, "from_name": from_name, "to_name": to_name, "reason": reason }),
    )
    .await
}

/// Send an item back to whoever handed it to the current holder. Returning
/// needs no approval: the item came from there.
pub async fn return_to_sender(
    t: &mut TenantTx,
    actor: Uuid,
    item: Uuid,
    reason: &str,
) -> Result<ItemRow> {
    let it = get_item(t, item).await?;
    let holder = it
        .holder_user_id
        .ok_or(StoreError::Invalid("nobody holds this item"))?;
    if holder != actor {
        return Err(StoreError::Invalid("only the holder can return an item"));
    }
    if it.open_handoff.is_some() {
        return Err(StoreError::Conflict("an open hand-off for this item"));
    }
    let last: Option<(Uuid,)> = sqlx::query_as(
        "SELECT from_user FROM handoff WHERE work_item_id = $1 AND state = 'accepted' AND to_user = $2
         ORDER BY decided_at DESC LIMIT 1",
    )
    .bind(item)
    .bind(holder)
    .fetch_optional(&mut *t.tx)
    .await?;
    let (back,) = last.ok_or(StoreError::Invalid("this item was not handed to you"))?;
    let reason = if reason.trim().is_empty() {
        "returned"
    } else {
        reason.trim()
    };
    sqlx::query(
        "INSERT INTO handoff (id, tenant_id, work_item_id, from_user, to_user, requested_by, reason, needs_approval, state, is_return, decided_at)
         VALUES ($1, $2, $3, $4, $5, $4, $6, false, 'accepted', true, now())",
    )
    .bind(Uuid::now_v7())
    .bind(t.tenant_id)
    .bind(item)
    .bind(holder)
    .bind(back)
    .bind(reason)
    .execute(&mut *t.tx)
    .await?;
    move_item(t, actor, &it, holder, back, None, reason).await?;
    let name = display_name(t, actor).await?;
    notify(
        t,
        back,
        "handoff_returned",
        &format!("{name} ↩ {}", item_label(&it)),
        reason,
        &item_link(&it),
    )
    .await?;
    get_item(t, item).await
}

/// Hand-offs waiting on this person: offers to accept and approvals to give.
pub async fn inbox_handoffs(
    t: &mut TenantTx,
    user: Uuid,
) -> Result<(Vec<HandoffRow>, Vec<HandoffRow>)> {
    let to_accept: Vec<HandoffRow> = sqlx::query_as::<_, HandoffRow>(&format!(
        "{HANDOFF_SELECT} WHERE h.to_user = $1 AND h.state = 'pending_accept' ORDER BY h.created_at"
    ))
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?;
    let pending: Vec<HandoffRow> = sqlx::query_as::<_, HandoffRow>(&format!(
        "{HANDOFF_SELECT} WHERE h.state = 'pending_approval' ORDER BY h.created_at"
    ))
    .fetch_all(&mut *t.tx)
    .await?;
    let mut to_approve = Vec::new();
    for h in pending {
        if approvers_for(t, h.to_user, h.project_id)
            .await?
            .allows(user)
        {
            to_approve.push(h);
        }
    }
    Ok((to_accept, to_approve))
}

/// Hand-offs this person sent that are still open.
pub async fn sent_handoffs(t: &mut TenantTx, user: Uuid) -> Result<Vec<HandoffRow>> {
    Ok(sqlx::query_as::<_, HandoffRow>(&format!(
        "{HANDOFF_SELECT} WHERE (h.from_user = $1 OR h.requested_by = $1) AND h.state IN ('pending_approval', 'pending_accept')
         ORDER BY h.created_at"
    ))
    .bind(user)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Remind once about hand-offs waiting more than 24 hours. Approvals that
/// stay unanswered also go to the Admins.
pub async fn remind_stale_handoffs(t: &mut TenantTx) -> Result<usize> {
    let stale: Vec<HandoffRow> = sqlx::query_as::<_, HandoffRow>(&format!(
        "{HANDOFF_SELECT} WHERE h.state IN ('pending_approval', 'pending_accept') AND h.reminded_at IS NULL
           AND h.created_at < now() - interval '24 hours'"
    ))
    .fetch_all(&mut *t.tx)
    .await?;
    for h in &stale {
        let subject = format!(
            "⏰ {} {}",
            h.item_key.as_deref().unwrap_or(""),
            h.item_title
        );
        let mut who = if h.state == "pending_approval" {
            let mut v = approver_list(&approvers_for(t, h.to_user, h.project_id).await?);
            v.extend(admins(t).await?);
            v
        } else {
            vec![h.to_user]
        };
        who.sort();
        who.dedup();
        for u in who {
            notify(t, u, "handoff_reminder", &subject, &h.reason, "/inbox").await?;
        }
        sqlx::query("UPDATE handoff SET reminded_at = now() WHERE id = $1")
            .bind(h.id)
            .execute(&mut *t.tx)
            .await?;
    }
    Ok(stale.len())
}

/// Hand-off chain statistics for managers ("ping-pong" detection).
pub async fn handoff_stats(t: &mut TenantTx) -> Result<serde_json::Value> {
    let bounced: Vec<ItemRow> = sqlx::query_as::<_, ItemRow>(&format!(
        "{ITEM_SELECT} WHERE (SELECT count(*) FROM handoff h WHERE h.work_item_id = w.id AND h.state = 'accepted') > 3
         ORDER BY w.updated_at DESC LIMIT 50"
    ))
    .fetch_all(&mut *t.tx)
    .await?;
    let (avg_wait,): (Option<f64>,) = sqlx::query_as(
        "SELECT avg(extract(epoch FROM decided_at - created_at) / 3600)::float8 FROM handoff
         WHERE state = 'accepted' AND NOT is_return AND decided_at > now() - interval '90 days'",
    )
    .fetch_one(&mut *t.tx)
    .await?;
    Ok(json!({ "passed_more_than_3": bounced, "avg_hours_to_accept": avg_wait }))
}
