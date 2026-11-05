//! Project lifecycle: expense and fund requests (with trips and delegated
//! bookings), configurable approval routes shared with contracts, contracts
//! with files and payment milestones, and the project file — one timeline of
//! what happened on a project, why, and what it cost.

use crate::finance::{audit, fmt_money, fx_on};
use crate::flow::{
    admin_people, display_name, finance_people, home_department, insert_loose_item, next_counter,
    notify,
};
use crate::work::{ensure_active_member, get_item, get_project, ItemRow, ProjectRow};
use crate::{Result, StoreError, TenantTx};
use chrono::{DateTime, NaiveDate, Utc};
use cm_domain::approval::{self, Action, Outcome, Step};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

pub const CLAIM_KINDS: [&str; 5] = ["reimbursement", "advance", "payment", "purchase", "trip"];
pub const LINE_CATEGORIES: [&str; 12] = [
    "flight",
    "train",
    "hotel",
    "meals",
    "local_transport",
    "allowance",
    "equipment",
    "materials",
    "software",
    "outsourcing",
    "entertainment",
    "other",
];
pub const CONTRACT_KINDS: [&str; 6] = [
    "sales",
    "purchase",
    "outsourcing",
    "nda",
    "framework",
    "other",
];
pub const STEP_KINDS: [&str; 6] = [
    "phase_manager",
    "project_manager",
    "department_head",
    "finance",
    "admin",
    "user",
];
/// Request kinds whose approved amount is a project cost. Advances are cash
/// handed out ahead of time and settled by a reimbursement, so they are not.
const COST_KINDS: [&str; 3] = ["reimbursement", "payment", "purchase"];

/// Who is asking, with the permissions that widen what they may see.
#[derive(Debug, Clone, Copy)]
pub struct Viewer {
    pub user: Uuid,
    pub is_admin: bool,
    pub is_finance: bool,
    pub can_view_contracts: bool,
}

impl Viewer {
    fn sees_all_claims(&self) -> bool {
        self.is_admin || self.is_finance
    }
    fn sees_all_contracts(&self) -> bool {
        self.is_admin || self.is_finance || self.can_view_contracts
    }
}

pub async fn viewer(t: &mut TenantTx, user: Uuid) -> Result<Viewer> {
    let (is_admin, is_finance, can_view_contracts): (bool, bool, bool) = sqlx::query_as(
        "SELECT is_admin, is_finance, can_view_contracts FROM membership WHERE user_id = $1 AND tenant_id = $2 AND status = 'active'",
    )
    .bind(user)
    .bind(t.tenant_id)
    .fetch_optional(&mut *t.tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(Viewer {
        user,
        is_admin,
        is_finance,
        can_view_contracts,
    })
}

pub async fn set_contract_viewer(
    t: &mut TenantTx,
    actor: Uuid,
    user: Uuid,
    on: bool,
) -> Result<()> {
    let r = sqlx::query(
        "UPDATE membership SET can_view_contracts = $2 WHERE user_id = $1 AND tenant_id = $3",
    )
    .bind(user)
    .bind(on)
    .bind(t.tenant_id)
    .execute(&mut *t.tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    audit(
        t,
        actor,
        "member.contract_viewer",
        json!({ "user": user, "on": on }),
    )
    .await
}

// ---------- approval routes ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStep {
    pub kind: String,
    #[serde(default)]
    pub user_id: Option<Uuid>,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RouteRow {
    pub id: Uuid,
    pub subject: String,
    pub name: String,
    pub min_amount_minor: i64,
    pub steps: Value,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

fn default_steps(subject: &str) -> Vec<RouteStep> {
    let s = |k: &str| RouteStep {
        kind: k.into(),
        user_id: None,
        label: String::new(),
    };
    match subject {
        "trip" => vec![s("phase_manager")],
        "contract" => vec![s("project_manager"), s("finance"), s("admin")],
        _ => vec![s("phase_manager"), s("finance")],
    }
}

/// Every company starts with one route per kind; admins change them later.
async fn ensure_default_routes(t: &mut TenantTx) -> Result<()> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM approval_route")
        .fetch_one(&mut *t.tx)
        .await?;
    if n > 0 {
        return Ok(());
    }
    let Some(owner) = admin_people(t).await?.into_iter().next() else {
        return Ok(());
    };
    for subject in CLAIM_KINDS.iter().chain(["contract"].iter()) {
        sqlx::query(
            "INSERT INTO approval_route (id, tenant_id, subject, name, steps, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::now_v7())
        .bind(t.tenant_id)
        .bind(subject)
        .bind("default")
        .bind(json!(default_steps(subject)))
        .bind(owner)
        .execute(&mut *t.tx)
        .await?;
    }
    Ok(())
}

pub async fn list_routes(t: &mut TenantTx) -> Result<Vec<RouteRow>> {
    ensure_default_routes(t).await?;
    Ok(sqlx::query_as::<_, RouteRow>(
        "SELECT id, subject, name, min_amount_minor, steps, active, created_at FROM approval_route
         ORDER BY subject, min_amount_minor, created_at",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

fn check_steps(steps: &[RouteStep]) -> Result<()> {
    if steps.is_empty() || steps.len() > 10 {
        return Err(StoreError::Invalid("a route needs 1–10 steps"));
    }
    for s in steps {
        if !STEP_KINDS.contains(&s.kind.as_str()) {
            return Err(StoreError::Invalid("unknown step kind"));
        }
        if s.kind == "user" && s.user_id.is_none() {
            return Err(StoreError::Invalid("choose the person for this step"));
        }
    }
    Ok(())
}

pub async fn create_route(
    t: &mut TenantTx,
    actor: Uuid,
    subject: &str,
    name: &str,
    min_amount_minor: i64,
    steps: &[RouteStep],
) -> Result<Uuid> {
    ensure_default_routes(t).await?;
    if !CLAIM_KINDS.contains(&subject) && subject != "contract" {
        return Err(StoreError::Invalid("unknown route subject"));
    }
    check_steps(steps)?;
    for s in steps {
        if let Some(u) = s.user_id {
            ensure_active_member(t, u).await?;
        }
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO approval_route (id, tenant_id, subject, name, min_amount_minor, steps, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(subject)
    .bind(name)
    .bind(min_amount_minor.max(0))
    .bind(json!(steps))
    .bind(actor)
    .execute(&mut *t.tx)
    .await?;
    audit(
        t,
        actor,
        "route.create",
        json!({ "route": id, "subject": subject, "min": min_amount_minor, "steps": steps }),
    )
    .await?;
    Ok(id)
}

pub async fn update_route(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    name: &str,
    min_amount_minor: i64,
    steps: &[RouteStep],
    active: bool,
) -> Result<()> {
    check_steps(steps)?;
    let r = sqlx::query(
        "UPDATE approval_route SET name = $2, min_amount_minor = $3, steps = $4, active = $5 WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(min_amount_minor.max(0))
    .bind(json!(steps))
    .bind(active)
    .execute(&mut *t.tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    audit(
        t,
        actor,
        "route.update",
        json!({ "route": id, "min": min_amount_minor, "steps": steps, "active": active }),
    )
    .await
}

pub async fn delete_route(t: &mut TenantTx, actor: Uuid, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM approval_route WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    audit(t, actor, "route.delete", json!({ "route": id })).await
}

/// Resolves a route step to people.
async fn step_people(
    t: &mut TenantTx,
    s: &RouteStep,
    applicant: Uuid,
    project: Option<&ProjectRow>,
    phase: Option<&ItemRow>,
) -> Result<Vec<Uuid>> {
    Ok(match s.kind.as_str() {
        "phase_manager" => match (phase.and_then(|p| p.holder_user_id), project) {
            (Some(h), _) => vec![h],
            (None, Some(p)) => vec![p.owner_id],
            _ => vec![],
        },
        "project_manager" => project.map(|p| vec![p.owner_id]).unwrap_or_default(),
        "department_head" => match home_department(t, applicant).await? {
            Some(d) => {
                let (h,): (Option<Uuid>,) =
                    sqlx::query_as("SELECT head_user_id FROM department WHERE id = $1")
                        .bind(d)
                        .fetch_one(&mut *t.tx)
                        .await?;
                h.into_iter().collect()
            }
            None => vec![],
        },
        "finance" => finance_people(t).await?,
        "admin" => admin_people(t).await?,
        "user" => s.user_id.into_iter().collect(),
        _ => vec![],
    })
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ApprovalRow {
    pub id: Uuid,
    pub subject: String,
    pub subject_id: Uuid,
    pub steps: Value,
    pub current_step: i32,
    pub state: String,
    pub round: i32,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

fn steps_of(a: &ApprovalRow) -> Vec<Step> {
    serde_json::from_value(a.steps.clone()).unwrap_or_default()
}

async fn get_approval(t: &mut TenantTx, id: Uuid) -> Result<ApprovalRow> {
    sqlx::query_as::<_, ApprovalRow>(
        "SELECT id, subject, subject_id, steps, current_step, state, round, created_at, decided_at FROM approval WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *t.tx)
    .await?
    .ok_or(StoreError::NotFound)
}

/// People the approval is waiting for (empty when it is finished).
pub fn waiting_for(a: &ApprovalRow) -> Vec<Uuid> {
    if a.state != "pending" {
        return vec![];
    }
    steps_of(a)
        .get(usize::try_from(a.current_step).unwrap_or(usize::MAX))
        .map(|s| s.users.clone())
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
async fn log_action(
    t: &mut TenantTx,
    subject_id: Uuid,
    approval_id: Option<Uuid>,
    step: Option<i32>,
    actor: Uuid,
    action: &str,
    to_user: Option<Uuid>,
    note: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO approval_action (id, tenant_id, subject_id, approval_id, step, actor_id, action, to_user, note)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::now_v7())
    .bind(t.tenant_id)
    .bind(subject_id)
    .bind(approval_id)
    .bind(step)
    .bind(actor)
    .bind(action)
    .bind(to_user)
    .bind(note)
    .execute(&mut *t.tx)
    .await?;
    Ok(())
}

struct Subject<'a> {
    kind: &'a str,
    id: Uuid,
    label: String,
    amount_minor: i64,
    currency: String,
    applicant: Uuid,
    project: Option<ProjectRow>,
    phase: Option<ItemRow>,
    link: String,
}

/// Starts (or restarts, after changes) the approval of a request or contract.
async fn start_approval(t: &mut TenantTx, s: &Subject<'_>) -> Result<(Uuid, Outcome)> {
    ensure_default_routes(t).await?;
    let routes: Vec<(Uuid, i64, Value)> = sqlx::query_as(
        "SELECT id, min_amount_minor, steps FROM approval_route WHERE subject = $1 AND active",
    )
    .bind(s.kind)
    .fetch_all(&mut *t.tx)
    .await?;
    let candidates: Vec<(i64, (Uuid, Value))> = routes
        .into_iter()
        .map(|(id, min, st)| (min, (id, st)))
        .collect();
    let (route_id, route_steps) = match approval::pick_route(&candidates, s.amount_minor) {
        Some((id, st)) => (
            Some(*id),
            serde_json::from_value::<Vec<RouteStep>>(st.clone()).unwrap_or_default(),
        ),
        None => (None, default_steps(s.kind)),
    };
    let mut steps = Vec::new();
    for rs in &route_steps {
        steps.push(Step {
            kind: rs.kind.clone(),
            label: rs.label.clone(),
            users: step_people(t, rs, s.applicant, s.project.as_ref(), s.phase.as_ref()).await?,
            skipped: false,
        });
    }
    let fallback = admin_people(t).await?;
    let first = approval::prepare(&mut steps, s.applicant, &fallback);
    let (round,): (i32,) =
        sqlx::query_as("SELECT COALESCE(max(round), 0) + 1 FROM approval WHERE subject_id = $1")
            .bind(s.id)
            .fetch_one(&mut *t.tx)
            .await?;
    let id = Uuid::now_v7();
    let (state, current, outcome) = match first {
        Some(i) => ("pending", i, Outcome::Next(i)),
        None => ("approved", steps.len(), Outcome::Approved),
    };
    sqlx::query(
        "INSERT INTO approval (id, tenant_id, subject, subject_id, route_id, steps, current_step, state, round, decided_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CASE WHEN $8 = 'pending' THEN NULL ELSE now() END)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(s.kind)
    .bind(s.id)
    .bind(route_id)
    .bind(json!(steps))
    .bind(i32::try_from(current).unwrap_or(0))
    .bind(state)
    .bind(round)
    .execute(&mut *t.tx)
    .await?;
    log_action(t, s.id, Some(id), None, s.applicant, "submit", None, "").await?;
    for (i, st) in steps.iter().enumerate() {
        if st.skipped {
            log_action(
                t,
                s.id,
                Some(id),
                i32::try_from(i).ok(),
                s.applicant,
                "skipped",
                None,
                "",
            )
            .await?;
        }
    }
    if let Some(i) = first {
        notify_approvers(t, &steps[i].users, s).await?;
    }
    Ok((id, outcome))
}

async fn notify_approvers(t: &mut TenantTx, users: &[Uuid], s: &Subject<'_>) -> Result<()> {
    let from = display_name(t, s.applicant).await?;
    for u in users {
        notify(
            t,
            *u,
            "approval_request",
            &format!(
                "{} · {from} · {}",
                s.label,
                fmt_money(s.amount_minor, &s.currency)
            ),
            "",
            &s.link,
        )
        .await?;
    }
    Ok(())
}

/// One decision on the waiting step. Returns the outcome.
async fn decide(
    t: &mut TenantTx,
    approval_id: Uuid,
    actor: Uuid,
    action: Action,
    note: &str,
    s: &Subject<'_>,
) -> Result<Outcome> {
    let a = get_approval(t, approval_id).await?;
    if a.state != "pending" {
        return Err(StoreError::Invalid("this approval is finished"));
    }
    let mut steps = steps_of(&a);
    let current = usize::try_from(a.current_step).unwrap_or(0);
    let outcome = approval::act(&mut steps, current, actor, action).map_err(|e| match e {
        approval::ApprovalError::NotYourTurn => {
            StoreError::Invalid("this approval is not waiting for you")
        }
        approval::ApprovalError::SelfTarget => StoreError::Invalid("choose someone else"),
        approval::ApprovalError::Finished => StoreError::Invalid("this approval is finished"),
    })?;
    if matches!(action, Action::Return | Action::Reject) && note.trim().is_empty() {
        return Err(StoreError::Invalid("a reason is required"));
    }
    let (name, to) = match action {
        Action::Approve => ("approve", None),
        Action::Return => ("return", None),
        Action::Reject => ("reject", None),
        Action::Delegate(u) => ("delegate", Some(u)),
        Action::Forward(u) => ("forward", Some(u)),
    };
    if let Some(u) = to {
        ensure_active_member(t, u).await?;
    }
    log_action(
        t,
        s.id,
        Some(a.id),
        Some(a.current_step),
        actor,
        name,
        to,
        note,
    )
    .await?;
    let (state, next) = match outcome {
        Outcome::Next(i) => ("pending", i),
        Outcome::Approved => ("approved", steps.len()),
        Outcome::Returned => ("returned", current),
        Outcome::Rejected => ("rejected", current),
    };
    sqlx::query(
        "UPDATE approval SET steps = $2, current_step = $3, state = $4,
                decided_at = CASE WHEN $4 = 'pending' THEN NULL ELSE now() END WHERE id = $1",
    )
    .bind(a.id)
    .bind(json!(steps))
    .bind(i32::try_from(next).unwrap_or(0))
    .bind(state)
    .execute(&mut *t.tx)
    .await?;
    let who = display_name(t, actor).await?;
    match outcome {
        Outcome::Next(i) => {
            let users = steps[i].users.clone();
            notify_approvers(t, &users, s).await?;
        }
        Outcome::Approved => {
            notify(
                t,
                s.applicant,
                "approval_done",
                &format!("{} ✓ {who}", s.label),
                note,
                &s.link,
            )
            .await?;
        }
        Outcome::Returned | Outcome::Rejected => {
            let mark = if outcome == Outcome::Returned {
                "↩"
            } else {
                "✗"
            };
            notify(
                t,
                s.applicant,
                "approval_done",
                &format!("{} {mark} {who}", s.label),
                note,
                &s.link,
            )
            .await?;
        }
    }
    Ok(outcome)
}

pub fn parse_action(name: &str, to: Option<Uuid>) -> Option<Action> {
    Some(match (name, to) {
        ("approve", _) => Action::Approve,
        ("return", _) => Action::Return,
        ("reject", _) => Action::Reject,
        ("delegate", Some(u)) => Action::Delegate(u),
        ("forward", Some(u)) => Action::Forward(u),
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ActionRow {
    pub id: Uuid,
    pub approval_id: Option<Uuid>,
    pub step: Option<i32>,
    pub actor_id: Uuid,
    pub actor_name: String,
    pub action: String,
    pub to_user: Option<Uuid>,
    pub to_name: Option<String>,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_actions(t: &mut TenantTx, subject_id: Uuid) -> Result<Vec<ActionRow>> {
    Ok(sqlx::query_as::<_, ActionRow>(
        "SELECT a.id, a.approval_id, a.step, a.actor_id, u.display_name AS actor_name, a.action, a.to_user,
                v.display_name AS to_name, a.note, a.created_at
         FROM approval_action a JOIN app_user u ON u.id = a.actor_id LEFT JOIN app_user v ON v.id = a.to_user
         WHERE a.subject_id = $1 ORDER BY a.created_at, a.id",
    )
    .bind(subject_id)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// The approval's steps with people's names, for display.
pub async fn approval_view(t: &mut TenantTx, id: Option<Uuid>) -> Result<Value> {
    let Some(id) = id else { return Ok(Value::Null) };
    let a = get_approval(t, id).await?;
    let mut out = Vec::new();
    for (i, s) in steps_of(&a).iter().enumerate() {
        let mut names = Vec::new();
        for u in &s.users {
            names.push(json!({ "id": u, "name": display_name(t, *u).await? }));
        }
        out.push(json!({ "index": i, "kind": s.kind, "label": s.label, "users": names, "skipped": s.skipped }));
    }
    Ok(
        json!({ "id": a.id, "state": a.state, "current_step": a.current_step, "round": a.round, "steps": out }),
    )
}

// ---------- requests (claims) ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ClaimRow {
    pub id: Uuid,
    pub key: String,
    pub kind: String,
    pub project_id: Option<Uuid>,
    pub project_key: Option<String>,
    pub project_name: Option<String>,
    pub project_currency: Option<String>,
    pub phase_id: Option<Uuid>,
    pub phase_title: Option<String>,
    pub parent_id: Option<Uuid>,
    pub parent_key: Option<String>,
    pub applicant_id: Uuid,
    pub applicant_name: String,
    pub title: String,
    pub justification: String,
    pub currency: String,
    pub amount_minor: i64,
    pub payee: String,
    pub destination: String,
    pub depart_on: Option<NaiveDate>,
    pub return_on: Option<NaiveDate>,
    pub travellers: Vec<Uuid>,
    pub status: String,
    pub approval_id: Option<Uuid>,
    pub paid_at: Option<DateTime<Utc>>,
    pub paid_by: Option<Uuid>,
    pub payment_ref: String,
    pub project_amount_minor: Option<i64>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub waiting_for: Vec<Uuid>,
}

const CLAIM_SELECT: &str = "
    SELECT c.id, c.key, c.kind, c.project_id, p.key AS project_key, p.name AS project_name, p.currency::text AS project_currency,
           c.phase_id, w.title AS phase_title, c.parent_id, pc.key AS parent_key,
           c.applicant_id, u.display_name AS applicant_name, c.title, c.justification, c.currency::text AS currency,
           c.amount_minor, c.payee, c.destination, c.depart_on, c.return_on, c.travellers, c.status, c.approval_id,
           c.paid_at, c.paid_by, c.payment_ref, c.project_amount_minor, c.submitted_at, c.created_at, c.updated_at,
           COALESCE((SELECT ARRAY(SELECT jsonb_array_elements_text(a.steps -> a.current_step -> 'users')::uuid)
                     FROM approval a WHERE a.id = c.approval_id AND a.state = 'pending'), '{}') AS waiting_for
    FROM claim c JOIN app_user u ON u.id = c.applicant_id
    LEFT JOIN project p ON p.id = c.project_id LEFT JOIN work_item w ON w.id = c.phase_id
    LEFT JOIN claim pc ON pc.id = c.parent_id";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LineRow {
    #[serde(default)]
    pub id: Uuid,
    pub spent_on: Option<NaiveDate>,
    pub category: String,
    #[serde(default)]
    pub description: String,
    pub amount_minor: i64,
}

pub struct NewClaim<'a> {
    pub kind: &'a str,
    pub project_id: Option<Uuid>,
    pub phase_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub title: &'a str,
    pub justification: &'a str,
    pub currency: &'a str,
    pub payee: &'a str,
    pub destination: &'a str,
    pub depart_on: Option<NaiveDate>,
    pub return_on: Option<NaiveDate>,
    pub travellers: &'a [Uuid],
    pub lines: &'a [LineRow],
}

pub async fn get_claim(t: &mut TenantTx, id: Uuid) -> Result<ClaimRow> {
    sqlx::query_as::<_, ClaimRow>(&format!("{CLAIM_SELECT} WHERE c.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn claim_lines(t: &mut TenantTx, id: Uuid) -> Result<Vec<LineRow>> {
    Ok(sqlx::query_as::<_, LineRow>(
        "SELECT id, spent_on, category, description, amount_minor FROM claim_line WHERE claim_id = $1 ORDER BY sort_order, id",
    )
    .bind(id)
    .fetch_all(&mut *t.tx)
    .await?)
}

async fn check_claim(t: &mut TenantTx, n: &NewClaim<'_>) -> Result<()> {
    if !CLAIM_KINDS.contains(&n.kind) {
        return Err(StoreError::Invalid("unknown request kind"));
    }
    if n.title.trim().is_empty() || n.title.chars().count() > 300 {
        return Err(StoreError::Invalid("title must be 1–300 characters"));
    }
    if n.currency.len() != 3 || !n.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(StoreError::Invalid("currency must be a 3-letter code"));
    }
    if n.lines.len() > 200 {
        return Err(StoreError::Invalid("at most 200 lines"));
    }
    for l in n.lines {
        if !LINE_CATEGORIES.contains(&l.category.as_str()) {
            return Err(StoreError::Invalid("unknown expense category"));
        }
        if l.amount_minor < 0 {
            return Err(StoreError::Invalid("amounts cannot be negative"));
        }
    }
    if let (Some(a), Some(b)) = (n.depart_on, n.return_on) {
        if b < a {
            return Err(StoreError::Invalid("return date is before departure"));
        }
    }
    if let Some(p) = n.project_id {
        get_project(t, p).await?;
    }
    if let Some(ph) = n.phase_id {
        let it = get_item(t, ph).await?;
        if it.kind != "phase" || it.project_id != n.project_id || n.project_id.is_none() {
            return Err(StoreError::Invalid(
                "the phase must belong to the chosen project",
            ));
        }
    }
    for u in n.travellers {
        ensure_active_member(t, *u).await?;
    }
    if let Some(parent) = n.parent_id {
        get_claim(t, parent).await?;
    }
    Ok(())
}

async fn write_lines(t: &mut TenantTx, claim: Uuid, lines: &[LineRow]) -> Result<i64> {
    sqlx::query("DELETE FROM claim_line WHERE claim_id = $1")
        .bind(claim)
        .execute(&mut *t.tx)
        .await?;
    let mut total: i64 = 0;
    for (i, l) in lines.iter().enumerate() {
        total = total
            .checked_add(l.amount_minor)
            .ok_or(StoreError::Invalid("amount too large"))?;
        sqlx::query(
            "INSERT INTO claim_line (id, tenant_id, claim_id, spent_on, category, description, amount_minor, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(t.tenant_id)
        .bind(claim)
        .bind(l.spent_on)
        .bind(&l.category)
        .bind(l.description.chars().take(2000).collect::<String>())
        .bind(l.amount_minor)
        .bind(i32::try_from(i).unwrap_or(0))
        .execute(&mut *t.tx)
        .await?;
    }
    sqlx::query("UPDATE claim SET amount_minor = $2, updated_at = now() WHERE id = $1")
        .bind(claim)
        .bind(total)
        .execute(&mut *t.tx)
        .await?;
    Ok(total)
}

pub async fn create_claim(t: &mut TenantTx, actor: Uuid, n: NewClaim<'_>) -> Result<ClaimRow> {
    check_claim(t, &n).await?;
    let prefix = match n.kind {
        "trip" => "TRIP",
        "advance" => "ADV",
        "payment" => "PAY",
        "purchase" => "PO",
        _ => "EXP",
    };
    // Item keys like PAY-12 share the namespace of project keys; request keys add an "R".
    let key = format!(
        "{prefix}R-{}",
        next_counter(t, &format!("claim_{prefix}")).await?
    );
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO claim (id, tenant_id, key, kind, project_id, phase_id, parent_id, applicant_id, title, justification,
                            currency, payee, destination, depart_on, return_on, travellers)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(&key)
    .bind(n.kind)
    .bind(n.project_id)
    .bind(n.phase_id)
    .bind(n.parent_id)
    .bind(actor)
    .bind(n.title.trim())
    .bind(n.justification)
    .bind(n.currency)
    .bind(n.payee)
    .bind(n.destination)
    .bind(n.depart_on)
    .bind(n.return_on)
    .bind(n.travellers)
    .execute(&mut *t.tx)
    .await?;
    write_lines(t, id, n.lines).await?;
    log_action(t, id, None, None, actor, "create", None, "").await?;
    get_claim(t, id).await
}

pub async fn update_claim(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    n: NewClaim<'_>,
) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.applicant_id != actor {
        return Err(StoreError::Invalid("only the applicant can edit a request"));
    }
    if !matches!(c.status.as_str(), "draft" | "returned") {
        return Err(StoreError::Invalid(
            "only drafts and returned requests can be edited",
        ));
    }
    check_claim(t, &n).await?;
    sqlx::query(
        "UPDATE claim SET project_id = $2, phase_id = $3, title = $4, justification = $5, currency = $6, payee = $7,
                          destination = $8, depart_on = $9, return_on = $10, travellers = $11, parent_id = $12, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(n.project_id)
    .bind(n.phase_id)
    .bind(n.title.trim())
    .bind(n.justification)
    .bind(n.currency)
    .bind(n.payee)
    .bind(n.destination)
    .bind(n.depart_on)
    .bind(n.return_on)
    .bind(n.travellers)
    .bind(n.parent_id)
    .execute(&mut *t.tx)
    .await?;
    write_lines(t, id, n.lines).await?;
    log_action(t, id, None, None, actor, "edit", None, "").await?;
    get_claim(t, id).await
}

async fn claim_subject(t: &mut TenantTx, c: &ClaimRow) -> Result<Subject<'static>> {
    let project = match c.project_id {
        Some(p) => Some(get_project(t, p).await?),
        None => None,
    };
    let phase = match c.phase_id {
        Some(p) => Some(get_item(t, p).await?),
        None => None,
    };
    let kind: &'static str = CLAIM_KINDS
        .iter()
        .find(|k| **k == c.kind)
        .copied()
        .unwrap_or("reimbursement");
    Ok(Subject {
        kind,
        id: c.id,
        label: format!("{} {}", c.key, c.title),
        amount_minor: c.amount_minor,
        currency: c.currency.clone(),
        applicant: c.applicant_id,
        project,
        phase,
        link: format!("/requests?id={}", c.id),
    })
}

pub async fn submit_claim(t: &mut TenantTx, actor: Uuid, id: Uuid) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.applicant_id != actor {
        return Err(StoreError::Invalid(
            "only the applicant can submit a request",
        ));
    }
    if !matches!(c.status.as_str(), "draft" | "returned") {
        return Err(StoreError::Invalid("this request was already submitted"));
    }
    if c.kind == "trip" {
        if c.destination.trim().is_empty() || c.depart_on.is_none() || c.return_on.is_none() {
            return Err(StoreError::Invalid("a trip needs a destination and dates"));
        }
    } else if claim_lines(t, id).await?.is_empty() {
        return Err(StoreError::Invalid("add at least one line"));
    }
    if c.justification.trim().is_empty() {
        return Err(StoreError::Invalid("write down why this is needed"));
    }
    let s = claim_subject(t, &c).await?;
    let (approval_id, outcome) = start_approval(t, &s).await?;
    let status = if outcome == Outcome::Approved {
        "approved"
    } else {
        "in_approval"
    };
    sqlx::query("UPDATE claim SET status = $2, approval_id = $3, submitted_at = now(), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .bind(approval_id)
        .execute(&mut *t.tx)
        .await?;
    if status == "approved" {
        on_claim_approved(t, id).await?;
    }
    get_claim(t, id).await
}

/// Freeze the amount in the project currency and re-check the project's alerts.
async fn on_claim_approved(t: &mut TenantTx, id: Uuid) -> Result<()> {
    let c = get_claim(t, id).await?;
    if let (Some(p), Some(pc)) = (c.project_id, c.project_currency.clone()) {
        let converted =
            convert(t, c.amount_minor, &c.currency, &pc, Utc::now().date_naive()).await?;
        sqlx::query("UPDATE claim SET project_amount_minor = $2 WHERE id = $1")
            .bind(id)
            .bind(converted)
            .execute(&mut *t.tx)
            .await?;
        crate::finance::evaluate_project(t, p).await?;
    }
    Ok(())
}

async fn convert(
    t: &mut TenantTx,
    minor: i64,
    from: &str,
    to: &str,
    on: NaiveDate,
) -> Result<Option<i64>> {
    if from == to {
        return Ok(Some(minor));
    }
    // Both currencies in this system use the same minor-unit scale for the
    // supported set except JPY/KRW; convert through major units.
    let (fe, te) = (exponent(from), exponent(to));
    Ok(fx_on(t, from, to, on).await?.and_then(|(_, rate)| {
        let major = Decimal::from(minor) / Decimal::from(10i64.pow(fe));
        (major * rate * Decimal::from(10i64.pow(te)))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointNearestEven)
            .to_i64()
    }))
}

fn exponent(ccy: &str) -> u32 {
    cm_domain::Currency::parse(ccy)
        .map(|c| c.minor_exponent())
        .unwrap_or(2)
}

pub async fn decide_claim(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    action: Action,
    note: &str,
) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.status != "in_approval" {
        return Err(StoreError::Invalid(
            "this request is not waiting for approval",
        ));
    }
    let s = claim_subject(t, &c).await?;
    let approval_id = c.approval_id.ok_or(StoreError::NotFound)?;
    let outcome = decide(t, approval_id, actor, action, note, &s).await?;
    let status = match outcome {
        Outcome::Next(_) => None,
        Outcome::Approved => Some("approved"),
        Outcome::Returned => Some("returned"),
        Outcome::Rejected => Some("rejected"),
    };
    if let Some(st) = status {
        sqlx::query("UPDATE claim SET status = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(st)
            .execute(&mut *t.tx)
            .await?;
        if st == "approved" {
            on_claim_approved(t, id).await?;
        }
    }
    get_claim(t, id).await
}

pub async fn cancel_claim(t: &mut TenantTx, actor: Uuid, id: Uuid, note: &str) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.applicant_id != actor {
        return Err(StoreError::Invalid(
            "only the applicant can withdraw a request",
        ));
    }
    if !matches!(c.status.as_str(), "draft" | "returned" | "in_approval") {
        return Err(StoreError::Invalid(
            "an approved request cannot be withdrawn",
        ));
    }
    if let Some(a) = c.approval_id {
        sqlx::query("UPDATE approval SET state = 'cancelled', decided_at = now() WHERE id = $1 AND state = 'pending'")
            .bind(a)
            .execute(&mut *t.tx)
            .await?;
    }
    sqlx::query("UPDATE claim SET status = 'cancelled', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    log_action(t, id, c.approval_id, None, actor, "cancel", None, note).await?;
    get_claim(t, id).await
}

/// Finance records the payment; the request goes back to the applicant to confirm receipt.
pub async fn pay_claim(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    reference: &str,
    note: &str,
) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.kind == "trip" {
        return Err(StoreError::Invalid(
            "a trip is paid through its expense requests",
        ));
    }
    if c.status != "approved" {
        return Err(StoreError::Invalid("only approved requests can be paid"));
    }
    sqlx::query("UPDATE claim SET status = 'paid', paid_at = now(), paid_by = $2, payment_ref = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(actor)
        .bind(reference)
        .execute(&mut *t.tx)
        .await?;
    log_action(t, id, c.approval_id, None, actor, "pay", None, note).await?;
    let who = display_name(t, actor).await?;
    notify(
        t,
        c.applicant_id,
        "claim_paid",
        &format!(
            "{} {} · {} · {who}",
            c.key,
            c.title,
            fmt_money(c.amount_minor, &c.currency)
        ),
        reference,
        &format!("/requests?id={id}"),
    )
    .await?;
    audit(t, actor, "claim.pay", json!({ "claim": id, "amount_minor": c.amount_minor, "currency": c.currency, "ref": reference })).await?;
    get_claim(t, id).await
}

/// The applicant confirms the money arrived (or a trip is finished).
pub async fn close_claim(t: &mut TenantTx, actor: Uuid, id: Uuid, note: &str) -> Result<ClaimRow> {
    let c = get_claim(t, id).await?;
    if c.applicant_id != actor {
        return Err(StoreError::Invalid(
            "only the applicant can close a request",
        ));
    }
    let ok = (c.kind == "trip" && c.status == "approved") || c.status == "paid";
    if !ok {
        return Err(StoreError::Invalid("this request cannot be closed yet"));
    }
    sqlx::query("UPDATE claim SET status = 'closed', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    let action = if c.kind == "trip" {
        "close"
    } else {
        "confirm_receipt"
    };
    log_action(t, id, c.approval_id, None, actor, action, None, note).await?;
    get_claim(t, id).await
}

/// Hand a booking (tickets, hotel...) for an approved trip to someone as a tracked task.
pub async fn delegate_booking(
    t: &mut TenantTx,
    actor: Uuid,
    trip: Uuid,
    to_user: Uuid,
    title: &str,
    note: &str,
    due: Option<NaiveDate>,
) -> Result<ItemRow> {
    let c = get_claim(t, trip).await?;
    if c.kind != "trip" || c.status != "approved" {
        return Err(StoreError::Invalid(
            "bookings can be handed out for approved trips only",
        ));
    }
    ensure_active_member(t, to_user).await?;
    let key = format!("REQ-{}", next_counter(t, "REQ").await?);
    let description = format!(
        "{} {} · {} · {} → {}\n{note}",
        c.key,
        c.destination,
        c.title,
        c.depart_on.map(|d| d.to_string()).unwrap_or_default(),
        c.return_on.map(|d| d.to_string()).unwrap_or_default()
    );
    let id = insert_loose_item(
        t,
        actor,
        "request",
        &key,
        title,
        &description,
        c.project_id,
        Some(to_user),
        None,
        None,
        due.or(c.depart_on),
        None,
        Some(actor),
    )
    .await?;
    sqlx::query("UPDATE work_item SET claim_id = $2 WHERE id = $1")
        .bind(id)
        .bind(trip)
        .execute(&mut *t.tx)
        .await?;
    log_action(
        t,
        trip,
        c.approval_id,
        None,
        actor,
        "delegate_booking",
        Some(to_user),
        title,
    )
    .await?;
    let it = get_item(t, id).await?;
    let from = display_name(t, actor).await?;
    notify(
        t,
        to_user,
        "request",
        &format!("{from}: {} {}", key, title),
        &description,
        &format!("/my?item={id}"),
    )
    .await?;
    Ok(it)
}

pub async fn claim_children(t: &mut TenantTx, id: Uuid) -> Result<(Vec<ClaimRow>, Vec<ItemRow>)> {
    let claims = sqlx::query_as::<_, ClaimRow>(&format!(
        "{CLAIM_SELECT} WHERE c.parent_id = $1 ORDER BY c.created_at"
    ))
    .bind(id)
    .fetch_all(&mut *t.tx)
    .await?;
    let items = sqlx::query_as::<_, ItemRow>(&format!(
        "{} WHERE w.claim_id = $1 ORDER BY w.created_at",
        crate::work::ITEM_SELECT
    ))
    .bind(id)
    .fetch_all(&mut *t.tx)
    .await?;
    Ok((claims, items))
}

/// May this person see the request? Applicant, travellers, anyone who acted on
/// it or is waiting to, the project's manager, the phase manager, Finance and Admins.
pub async fn can_see_claim(t: &mut TenantTx, v: &Viewer, c: &ClaimRow) -> Result<bool> {
    if v.sees_all_claims()
        || c.applicant_id == v.user
        || c.travellers.contains(&v.user)
        || c.waiting_for.contains(&v.user)
    {
        return Ok(true);
    }
    if let Some(p) = c.project_id {
        if get_project(t, p).await?.owner_id == v.user {
            return Ok(true);
        }
    }
    if let Some(ph) = c.phase_id {
        if get_item(t, ph).await?.holder_user_id == Some(v.user) {
            return Ok(true);
        }
    }
    let (acted,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM approval_action WHERE subject_id = $1 AND (actor_id = $2 OR to_user = $2))
             OR EXISTS (SELECT 1 FROM approval WHERE subject_id = $1 AND steps::text LIKE '%' || $2::text || '%')",
    )
    .bind(c.id)
    .bind(v.user)
    .fetch_one(&mut *t.tx)
    .await?;
    Ok(acted)
}

pub enum ClaimFilter {
    Mine,
    ToApprove,
    ToPay,
    Project(Uuid),
    All,
}

pub async fn list_claims(t: &mut TenantTx, v: &Viewer, f: ClaimFilter) -> Result<Vec<ClaimRow>> {
    let rows = match f {
        ClaimFilter::Mine => sqlx::query_as::<_, ClaimRow>(&format!(
            "{CLAIM_SELECT} WHERE c.applicant_id = $1 OR $1 = ANY (c.travellers) ORDER BY c.created_at DESC LIMIT 300"
        ))
        .bind(v.user)
        .fetch_all(&mut *t.tx)
        .await?,
        ClaimFilter::ToApprove => sqlx::query_as::<_, ClaimRow>(&format!(
            "SELECT * FROM ({CLAIM_SELECT} WHERE c.status = 'in_approval') x WHERE $1 = ANY (x.waiting_for) ORDER BY x.submitted_at"
        ))
        .bind(v.user)
        .fetch_all(&mut *t.tx)
        .await?,
        ClaimFilter::ToPay => {
            if !v.is_finance && !v.is_admin {
                return Ok(vec![]);
            }
            sqlx::query_as::<_, ClaimRow>(&format!(
                "{CLAIM_SELECT} WHERE c.status = 'approved' AND c.kind <> 'trip' ORDER BY c.updated_at"
            ))
            .fetch_all(&mut *t.tx)
            .await?
        }
        ClaimFilter::Project(p) => sqlx::query_as::<_, ClaimRow>(&format!(
            "{CLAIM_SELECT} WHERE c.project_id = $1 AND c.status <> 'draft' ORDER BY c.created_at DESC"
        ))
        .bind(p)
        .fetch_all(&mut *t.tx)
        .await?,
        ClaimFilter::All => sqlx::query_as::<_, ClaimRow>(&format!(
            "{CLAIM_SELECT} WHERE c.status <> 'draft' ORDER BY c.created_at DESC LIMIT 500"
        ))
        .fetch_all(&mut *t.tx)
        .await?,
    };
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if can_see_claim(t, v, &r).await? {
            out.push(r);
        }
    }
    Ok(out)
}

// ---------- documents ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DocumentRow {
    pub id: Uuid,
    pub owner_kind: String,
    pub owner_id: Uuid,
    pub claim_line_id: Option<Uuid>,
    pub label: String,
    pub version: i32,
    pub file_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(skip)]
    pub object_key: String,
    pub uploaded_by: Uuid,
    pub uploaded_by_name: String,
    pub created_at: DateTime<Utc>,
}

pub struct NewDocument<'a> {
    pub id: Uuid,
    pub owner_kind: &'a str,
    pub owner_id: Uuid,
    pub label: &'a str,
    pub file_name: &'a str,
    pub mime: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub object_key: &'a str,
    pub uploaded_by: Uuid,
}

const DOC_SELECT: &str = "SELECT d.id, d.owner_kind, d.owner_id, d.claim_line_id, d.label, d.version, d.file_name, d.mime,
    d.size_bytes, d.sha256, d.object_key, d.uploaded_by, u.display_name AS uploaded_by_name, d.created_at
    FROM document d JOIN app_user u ON u.id = d.uploaded_by";

pub async fn create_document(t: &mut TenantTx, d: NewDocument<'_>) -> Result<DocumentRow> {
    let (version,): (i32,) = sqlx::query_as(
        "SELECT COALESCE(max(version), 0) + 1 FROM document WHERE owner_id = $1 AND label = $2 AND label <> ''",
    )
    .bind(d.owner_id)
    .bind(d.label)
    .fetch_one(&mut *t.tx)
    .await?;
    sqlx::query(
        "INSERT INTO document (id, tenant_id, owner_kind, owner_id, label, version, file_name, mime, size_bytes, sha256, object_key, uploaded_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(d.id)
    .bind(t.tenant_id)
    .bind(d.owner_kind)
    .bind(d.owner_id)
    .bind(d.label)
    .bind(version)
    .bind(d.file_name)
    .bind(d.mime)
    .bind(d.size_bytes)
    .bind(d.sha256)
    .bind(d.object_key)
    .bind(d.uploaded_by)
    .execute(&mut *t.tx)
    .await?;
    log_action(
        t,
        d.owner_id,
        None,
        None,
        d.uploaded_by,
        "upload",
        None,
        d.file_name,
    )
    .await?;
    get_document(t, d.id).await
}

pub async fn get_document(t: &mut TenantTx, id: Uuid) -> Result<DocumentRow> {
    sqlx::query_as::<_, DocumentRow>(&format!("{DOC_SELECT} WHERE d.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn list_documents(t: &mut TenantTx, owner: Uuid) -> Result<Vec<DocumentRow>> {
    Ok(sqlx::query_as::<_, DocumentRow>(&format!(
        "{DOC_SELECT} WHERE d.owner_id = $1 ORDER BY d.created_at"
    ))
    .bind(owner)
    .fetch_all(&mut *t.tx)
    .await?)
}

// ---------- contracts ----------

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ContractRow {
    pub id: Uuid,
    pub key: String,
    pub project_id: Option<Uuid>,
    pub project_key: Option<String>,
    pub project_name: Option<String>,
    pub title: String,
    pub counterparty: String,
    pub kind: String,
    pub currency: String,
    pub amount_minor: i64,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub status: String,
    pub signed_on: Option<NaiveDate>,
    pub starts_on: Option<NaiveDate>,
    pub ends_on: Option<NaiveDate>,
    pub summary: String,
    pub approval_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub waiting_for: Vec<Uuid>,
    pub settled_minor: i64,
}

const CONTRACT_SELECT: &str = "
    SELECT c.id, c.key, c.project_id, p.key AS project_key, p.name AS project_name, c.title, c.counterparty, c.kind,
           c.currency::text AS currency, c.amount_minor, c.owner_id, u.display_name AS owner_name, c.status, c.signed_on,
           c.starts_on, c.ends_on, c.summary, c.approval_id, c.created_by, c.created_at, c.updated_at,
           COALESCE((SELECT ARRAY(SELECT jsonb_array_elements_text(a.steps -> a.current_step -> 'users')::uuid)
                     FROM approval a WHERE a.id = c.approval_id AND a.state = 'pending'), '{}') AS waiting_for,
           COALESCE((SELECT sum(m.amount_minor) FROM contract_milestone m WHERE m.contract_id = c.id AND m.status = 'settled'), 0)::bigint AS settled_minor
    FROM contract c JOIN app_user u ON u.id = c.owner_id LEFT JOIN project p ON p.id = c.project_id";

pub struct NewContract<'a> {
    pub project_id: Option<Uuid>,
    pub title: &'a str,
    pub counterparty: &'a str,
    pub kind: &'a str,
    pub currency: &'a str,
    pub amount_minor: i64,
    pub owner_id: Uuid,
    pub starts_on: Option<NaiveDate>,
    pub ends_on: Option<NaiveDate>,
    pub summary: &'a str,
}

async fn check_contract(t: &mut TenantTx, n: &NewContract<'_>) -> Result<()> {
    if !CONTRACT_KINDS.contains(&n.kind) {
        return Err(StoreError::Invalid("unknown contract type"));
    }
    if n.title.trim().is_empty() || n.title.chars().count() > 300 {
        return Err(StoreError::Invalid("title must be 1–300 characters"));
    }
    if n.currency.len() != 3 || !n.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(StoreError::Invalid("currency must be a 3-letter code"));
    }
    if n.amount_minor < 0 {
        return Err(StoreError::Invalid("amounts cannot be negative"));
    }
    if let (Some(a), Some(b)) = (n.starts_on, n.ends_on) {
        if b < a {
            return Err(StoreError::Invalid("end date is before start date"));
        }
    }
    if let Some(p) = n.project_id {
        get_project(t, p).await?;
    }
    ensure_active_member(t, n.owner_id).await
}

pub async fn get_contract(t: &mut TenantTx, id: Uuid) -> Result<ContractRow> {
    sqlx::query_as::<_, ContractRow>(&format!("{CONTRACT_SELECT} WHERE c.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn create_contract(
    t: &mut TenantTx,
    actor: Uuid,
    n: NewContract<'_>,
) -> Result<ContractRow> {
    check_contract(t, &n).await?;
    let key = format!("CT-{}", next_counter(t, "CT").await?);
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO contract (id, tenant_id, key, project_id, title, counterparty, kind, currency, amount_minor, owner_id,
                               starts_on, ends_on, summary, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(&key)
    .bind(n.project_id)
    .bind(n.title.trim())
    .bind(n.counterparty)
    .bind(n.kind)
    .bind(n.currency)
    .bind(n.amount_minor)
    .bind(n.owner_id)
    .bind(n.starts_on)
    .bind(n.ends_on)
    .bind(n.summary)
    .bind(actor)
    .execute(&mut *t.tx)
    .await?;
    log_action(t, id, None, None, actor, "create", None, "").await?;
    get_contract(t, id).await
}

fn may_edit_contract(c: &ContractRow, actor: Uuid) -> bool {
    c.owner_id == actor || c.created_by == actor
}

pub async fn update_contract(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    n: NewContract<'_>,
) -> Result<ContractRow> {
    let c = get_contract(t, id).await?;
    if !may_edit_contract(&c, actor) {
        return Err(StoreError::Invalid("only the contract owner can edit it"));
    }
    if !matches!(c.status.as_str(), "draft" | "returned") {
        return Err(StoreError::Invalid(
            "only drafts and returned contracts can be edited",
        ));
    }
    check_contract(t, &n).await?;
    sqlx::query(
        "UPDATE contract SET project_id = $2, title = $3, counterparty = $4, kind = $5, currency = $6, amount_minor = $7,
                             owner_id = $8, starts_on = $9, ends_on = $10, summary = $11, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(n.project_id)
    .bind(n.title.trim())
    .bind(n.counterparty)
    .bind(n.kind)
    .bind(n.currency)
    .bind(n.amount_minor)
    .bind(n.owner_id)
    .bind(n.starts_on)
    .bind(n.ends_on)
    .bind(n.summary)
    .execute(&mut *t.tx)
    .await?;
    log_action(t, id, None, None, actor, "edit", None, "").await?;
    get_contract(t, id).await
}

async fn contract_subject(t: &mut TenantTx, c: &ContractRow) -> Result<Subject<'static>> {
    let project = match c.project_id {
        Some(p) => Some(get_project(t, p).await?),
        None => None,
    };
    Ok(Subject {
        kind: "contract",
        id: c.id,
        label: format!("{} {}", c.key, c.title),
        amount_minor: c.amount_minor,
        currency: c.currency.clone(),
        applicant: c.owner_id,
        project,
        phase: None,
        link: format!("/contracts?id={}", c.id),
    })
}

pub async fn submit_contract(t: &mut TenantTx, actor: Uuid, id: Uuid) -> Result<ContractRow> {
    let c = get_contract(t, id).await?;
    if !may_edit_contract(&c, actor) {
        return Err(StoreError::Invalid("only the contract owner can submit it"));
    }
    if !matches!(c.status.as_str(), "draft" | "returned") {
        return Err(StoreError::Invalid("this contract was already submitted"));
    }
    let s = contract_subject(t, &c).await?;
    let (approval_id, outcome) = start_approval(t, &s).await?;
    let status = if outcome == Outcome::Approved {
        "approved"
    } else {
        "in_approval"
    };
    sqlx::query(
        "UPDATE contract SET status = $2, approval_id = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(approval_id)
    .execute(&mut *t.tx)
    .await?;
    get_contract(t, id).await
}

pub async fn decide_contract(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    action: Action,
    note: &str,
) -> Result<ContractRow> {
    let c = get_contract(t, id).await?;
    if c.status != "in_approval" {
        return Err(StoreError::Invalid(
            "this contract is not waiting for approval",
        ));
    }
    let s = contract_subject(t, &c).await?;
    let outcome = decide(
        t,
        c.approval_id.ok_or(StoreError::NotFound)?,
        actor,
        action,
        note,
        &s,
    )
    .await?;
    let status = match outcome {
        Outcome::Next(_) => None,
        Outcome::Approved => Some("approved"),
        Outcome::Returned => Some("returned"),
        Outcome::Rejected => Some("rejected"),
    };
    if let Some(st) = status {
        sqlx::query("UPDATE contract SET status = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(st)
            .execute(&mut *t.tx)
            .await?;
    }
    get_contract(t, id).await
}

/// After approval: record signing, then completion or termination.
pub async fn set_contract_status(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    status: &str,
    signed_on: Option<NaiveDate>,
    note: &str,
) -> Result<ContractRow> {
    let c = get_contract(t, id).await?;
    let ok = match (c.status.as_str(), status) {
        ("approved", "signed") => signed_on.is_some(),
        ("signed", "completed" | "terminated") => true,
        _ => false,
    };
    if !ok {
        return Err(StoreError::Invalid("this status change is not allowed"));
    }
    sqlx::query("UPDATE contract SET status = $2, signed_on = COALESCE($3, signed_on), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .bind(signed_on)
        .execute(&mut *t.tx)
        .await?;
    log_action(t, id, c.approval_id, None, actor, status, None, note).await?;
    audit(
        t,
        actor,
        &format!("contract.{status}"),
        json!({ "contract": id, "signed_on": signed_on }),
    )
    .await?;
    get_contract(t, id).await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MilestoneRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub phase_id: Option<Uuid>,
    pub phase_title: Option<String>,
    pub title: String,
    pub due_on: Option<NaiveDate>,
    pub amount_minor: i64,
    pub status: String,
    pub settled_on: Option<NaiveDate>,
}

pub async fn list_milestones(t: &mut TenantTx, contract: Uuid) -> Result<Vec<MilestoneRow>> {
    Ok(sqlx::query_as::<_, MilestoneRow>(
        "SELECT m.id, m.contract_id, m.phase_id, w.title AS phase_title, m.title, m.due_on, m.amount_minor, m.status, m.settled_on
         FROM contract_milestone m LEFT JOIN work_item w ON w.id = m.phase_id WHERE m.contract_id = $1 ORDER BY m.sort_order, m.due_on",
    )
    .bind(contract)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn add_milestone(
    t: &mut TenantTx,
    actor: Uuid,
    contract: Uuid,
    phase: Option<Uuid>,
    title: &str,
    due_on: Option<NaiveDate>,
    amount_minor: i64,
) -> Result<Uuid> {
    let c = get_contract(t, contract).await?;
    if title.trim().is_empty() || amount_minor < 0 {
        return Err(StoreError::Invalid(
            "a milestone needs a title and a non-negative amount",
        ));
    }
    if let Some(ph) = phase {
        let it = get_item(t, ph).await?;
        if it.kind != "phase" || it.project_id != c.project_id {
            return Err(StoreError::Invalid(
                "the phase must belong to the contract's project",
            ));
        }
    }
    let id = Uuid::now_v7();
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM contract_milestone WHERE contract_id = $1")
            .bind(contract)
            .fetch_one(&mut *t.tx)
            .await?;
    sqlx::query(
        "INSERT INTO contract_milestone (id, tenant_id, contract_id, phase_id, title, due_on, amount_minor, sort_order)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(contract)
    .bind(phase)
    .bind(title.trim())
    .bind(due_on)
    .bind(amount_minor)
    .bind(i32::try_from(n).unwrap_or(0))
    .execute(&mut *t.tx)
    .await?;
    log_action(t, contract, None, None, actor, "milestone_add", None, title).await?;
    Ok(id)
}

pub async fn milestone_contract(t: &mut TenantTx, id: Uuid) -> Result<Uuid> {
    let r: Option<(Uuid,)> =
        sqlx::query_as("SELECT contract_id FROM contract_milestone WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *t.tx)
            .await?;
    r.map(|x| x.0).ok_or(StoreError::NotFound)
}

pub async fn set_milestone_status(
    t: &mut TenantTx,
    actor: Uuid,
    id: Uuid,
    status: &str,
    settled_on: Option<NaiveDate>,
) -> Result<Uuid> {
    if !matches!(status, "planned" | "invoiced" | "settled") {
        return Err(StoreError::Invalid("unknown milestone status"));
    }
    let r: Option<(Uuid, String)> = sqlx::query_as(
        "UPDATE contract_milestone SET status = $2, settled_on = CASE WHEN $2 = 'settled' THEN COALESCE($3, current_date) ELSE NULL END
         WHERE id = $1 RETURNING contract_id, title",
    )
    .bind(id)
    .bind(status)
    .bind(settled_on)
    .fetch_optional(&mut *t.tx)
    .await?;
    let (contract, title) = r.ok_or(StoreError::NotFound)?;
    log_action(
        t,
        contract,
        None,
        None,
        actor,
        &format!("milestone_{status}"),
        None,
        &title,
    )
    .await?;
    Ok(contract)
}

/// Staff see contracts of projects they worked on (held an item or logged
/// time), ones they own or created, and ones they approve. Finance, Admins
/// and contract viewers see every contract.
pub async fn can_see_contract(t: &mut TenantTx, v: &Viewer, c: &ContractRow) -> Result<bool> {
    if v.sees_all_contracts()
        || c.owner_id == v.user
        || c.created_by == v.user
        || c.waiting_for.contains(&v.user)
    {
        return Ok(true);
    }
    let (ok,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM approval_action WHERE subject_id = $1 AND (actor_id = $2 OR to_user = $2))
             OR EXISTS (SELECT 1 FROM project p WHERE p.id = $3 AND p.owner_id = $2)
             OR EXISTS (SELECT 1 FROM work_item w WHERE w.project_id = $3 AND w.holder_user_id = $2)
             OR EXISTS (SELECT 1 FROM work_log l JOIN work_item w ON w.id = l.work_item_id WHERE w.project_id = $3 AND l.user_id = $2)",
    )
    .bind(c.id)
    .bind(v.user)
    .bind(c.project_id)
    .fetch_one(&mut *t.tx)
    .await?;
    Ok(ok)
}

pub async fn list_contracts(
    t: &mut TenantTx,
    v: &Viewer,
    project: Option<Uuid>,
) -> Result<Vec<ContractRow>> {
    let rows = sqlx::query_as::<_, ContractRow>(&format!(
        "{CONTRACT_SELECT} WHERE ($1::uuid IS NULL OR c.project_id = $1) ORDER BY c.created_at DESC LIMIT 500"
    ))
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if can_see_contract(t, v, &r).await? {
            out.push(r);
        }
    }
    Ok(out)
}

// ---------- project expenses and the project file ----------

#[derive(Debug, Clone, Default, Serialize)]
pub struct Expenses {
    pub currency: String,
    /// Approved cost in the project currency.
    pub total_minor: i64,
    /// Approved cost still waiting for an exchange rate (not counted).
    pub pending_fx: i64,
    pub in_approval_minor: i64,
    pub by_category: Vec<(String, i64)>,
    /// (phase id, phase title, budget, spent)
    pub by_phase: Vec<Value>,
    pub budget_minor: Option<i64>,
    pub advances_outstanding_minor: i64,
}

#[allow(clippy::type_complexity)]
pub async fn project_expenses(t: &mut TenantTx, project: Uuid) -> Result<Expenses> {
    let p = get_project(t, project).await?;
    let rows: Vec<(Uuid, String, String, i64, Option<i64>, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT id, kind, currency::text, amount_minor, project_amount_minor, phase_id, status FROM claim
         WHERE project_id = $1 AND status IN ('in_approval', 'approved', 'paid', 'closed')",
    )
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    let today = Utc::now().date_naive();
    let mut e = Expenses {
        currency: p.currency.clone(),
        budget_minor: p.expense_budget_minor,
        ..Default::default()
    };
    let mut cats: std::collections::BTreeMap<String, i64> = Default::default();
    let mut phases: std::collections::HashMap<Uuid, i64> = Default::default();
    for (id, kind, ccy, amount, frozen, phase, status) in rows {
        let converted = match frozen {
            Some(v) => Some(v),
            None => convert(t, amount, &ccy, &p.currency, today).await?,
        };
        if status == "in_approval" {
            if COST_KINDS.contains(&kind.as_str()) {
                e.in_approval_minor += converted.unwrap_or(0);
            }
            continue;
        }
        if kind == "advance" && status != "closed" {
            e.advances_outstanding_minor += converted.unwrap_or(0);
        }
        if !COST_KINDS.contains(&kind.as_str()) {
            continue;
        }
        let Some(v) = converted else {
            e.pending_fx += 1;
            continue;
        };
        e.total_minor += v;
        if let Some(ph) = phase {
            *phases.entry(ph).or_default() += v;
        }
        // Split by category in proportion to the lines.
        let lines = claim_lines(t, id).await?;
        let sum: i64 = lines.iter().map(|l| l.amount_minor).sum();
        for l in lines {
            let share = if sum > 0 {
                i128::from(v) * i128::from(l.amount_minor) / i128::from(sum)
            } else {
                0
            };
            *cats.entry(l.category).or_default() += i64::try_from(share).unwrap_or(0);
        }
    }
    e.by_category = cats.into_iter().collect();
    let phase_rows: Vec<(Uuid, String, Option<i64>, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, title, budget_minor, holder_user_id FROM work_item WHERE project_id = $1 AND kind = 'phase' ORDER BY sort_order",
    )
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    for (id, title, budget, holder) in phase_rows {
        let (labour_min,): (i64,) = sqlx::query_as(
            "WITH RECURSIVE sub AS (SELECT id FROM work_item WHERE id = $1 UNION ALL SELECT w.id FROM work_item w JOIN sub ON w.parent_id = sub.id)
             SELECT COALESCE(sum(l.minutes), 0)::bigint FROM work_log l WHERE l.work_item_id IN (SELECT id FROM sub)",
        )
        .bind(id)
        .fetch_one(&mut *t.tx)
        .await?;
        let manager = match holder {
            Some(h) => Some(display_name(t, h).await?),
            None => None,
        };
        e.by_phase.push(json!({
            "phase_id": id, "title": title, "budget_minor": budget, "spent_minor": phases.get(&id).copied().unwrap_or(0),
            "logged_minutes": labour_min, "manager": manager,
        }));
    }
    Ok(e)
}

pub async fn set_expense_budget(
    t: &mut TenantTx,
    actor: Uuid,
    project: Uuid,
    budget: Option<i64>,
) -> Result<()> {
    get_project(t, project).await?;
    sqlx::query("UPDATE project SET expense_budget_minor = $2 WHERE id = $1")
        .bind(project)
        .bind(budget)
        .execute(&mut *t.tx)
        .await?;
    audit(
        t,
        actor,
        "expense_budget.set",
        json!({ "project": project, "budget_minor": budget }),
    )
    .await?;
    crate::finance::evaluate_project(t, project).await
}

pub async fn set_phase_budget(
    t: &mut TenantTx,
    actor: Uuid,
    phase: Uuid,
    budget: Option<i64>,
) -> Result<()> {
    let it = get_item(t, phase).await?;
    if it.kind != "phase" {
        return Err(StoreError::Invalid("budgets are set on phases"));
    }
    sqlx::query("UPDATE work_item SET budget_minor = $2 WHERE id = $1")
        .bind(phase)
        .bind(budget)
        .execute(&mut *t.tx)
        .await?;
    audit(
        t,
        actor,
        "phase_budget.set",
        json!({ "phase": phase, "budget_minor": budget }),
    )
    .await
}

/// One timeline for a project: planning and work events, requests and their
/// decisions, contracts and milestones. Newest first.
#[allow(clippy::type_complexity)]
pub async fn project_file(t: &mut TenantTx, v: &Viewer, project: Uuid) -> Result<Vec<Value>> {
    get_project(t, project).await?;
    let mut out: Vec<(DateTime<Utc>, Value)> = Vec::new();
    let events: Vec<(DateTime<Utc>, String, String, Value, Option<String>, String)> = sqlx::query_as(
        "SELECT e.created_at, u.display_name, e.kind, e.payload, w.key, w.title
         FROM work_item_event e JOIN work_item w ON w.id = e.work_item_id JOIN app_user u ON u.id = e.actor_id
         WHERE w.project_id = $1 AND e.kind IN ('created', 'status', 'reassigned', 'deliverable_submitted', 'commit_linked', 'attachment_added')
         ORDER BY e.created_at DESC LIMIT 400",
    )
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    for (at, who, kind, payload, key, title) in events {
        out.push((at, json!({ "at": at, "source": "work", "who": who, "kind": kind, "payload": payload, "ref": key, "title": title })));
    }
    let logs: Vec<(DateTime<Utc>, String, i32, String, Option<String>, String)> = sqlx::query_as(
        "SELECT l.created_at, u.display_name, l.minutes, l.description, w.key, w.title
         FROM work_log l JOIN work_item w ON w.id = l.work_item_id JOIN app_user u ON u.id = l.user_id
         WHERE w.project_id = $1 ORDER BY l.created_at DESC LIMIT 400",
    )
    .bind(project)
    .fetch_all(&mut *t.tx)
    .await?;
    for (at, who, minutes, desc, key, title) in logs {
        out.push((at, json!({ "at": at, "source": "log", "who": who, "kind": "logged", "minutes": minutes, "note": desc, "ref": key, "title": title })));
    }
    for c in list_claims(t, v, ClaimFilter::Project(project)).await? {
        for a in list_actions(t, c.id).await? {
            out.push((a.created_at, json!({
                "at": a.created_at, "source": "claim", "id": c.id, "who": a.actor_name, "kind": a.action, "to": a.to_name,
                "note": a.note, "ref": c.key, "title": c.title, "claim_kind": c.kind, "amount_minor": c.amount_minor,
                "currency": c.currency, "justification": c.justification, "phase": c.phase_title,
            })));
        }
    }
    for c in list_contracts(t, v, Some(project)).await? {
        for a in list_actions(t, c.id).await? {
            out.push((a.created_at, json!({
                "at": a.created_at, "source": "contract", "id": c.id, "who": a.actor_name, "kind": a.action, "to": a.to_name,
                "note": a.note, "ref": c.key, "title": c.title, "contract_kind": c.kind, "amount_minor": c.amount_minor,
                "currency": c.currency, "counterparty": c.counterparty,
            })));
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out.truncate(800);
    Ok(out.into_iter().map(|x| x.1).collect())
}

/// Items waiting for this person across requests and contracts (for the inbox).
pub async fn waiting_on(t: &mut TenantTx, v: &Viewer) -> Result<Value> {
    let claims = list_claims(t, v, ClaimFilter::ToApprove).await?;
    let to_pay = list_claims(t, v, ClaimFilter::ToPay).await?;
    let contracts: Vec<ContractRow> = sqlx::query_as::<_, ContractRow>(&format!(
        "SELECT * FROM ({CONTRACT_SELECT} WHERE c.status = 'in_approval') x WHERE $1 = ANY (x.waiting_for) ORDER BY x.updated_at"
    ))
    .bind(v.user)
    .fetch_all(&mut *t.tx)
    .await?;
    let to_confirm: Vec<ClaimRow> = sqlx::query_as::<_, ClaimRow>(&format!(
        "{CLAIM_SELECT} WHERE c.applicant_id = $1 AND c.status = 'paid' ORDER BY c.paid_at"
    ))
    .bind(v.user)
    .fetch_all(&mut *t.tx)
    .await?;
    Ok(
        json!({ "claims": claims, "contracts": contracts, "to_pay": to_pay, "to_confirm": to_confirm }),
    )
}
