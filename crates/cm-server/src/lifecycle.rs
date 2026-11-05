//! Project lifecycle API: expense and fund requests, trips and delegated
//! bookings, approval routes, contracts with files and milestones, and the
//! project file.

use crate::work::{begin, can_manage, serve_file, store_field, Caller};
use crate::{ApiError, AppState, Session};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::NaiveDate;
use cm_store::lifecycle::{self as lc, ClaimFilter, LineRow, RouteStep, Viewer};
use cm_store::{StoreError, TenantTx};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

async fn open(s: &AppState, session: &Session) -> Result<(TenantTx, Caller, Viewer), ApiError> {
    let (mut t, c) = begin(s, session).await?;
    let v = lc::viewer(&mut t, c.user).await?;
    Ok((t, c, v))
}

fn currency(code: &str) -> Result<String, ApiError> {
    cm_domain::Currency::parse(code.trim())
        .map(|c| c.as_str().to_owned())
        .map_err(|_| ApiError::bad_request("currency must be an ISO 4217 code"))
}

// ---------- routes ----------

pub async fn list_routes(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, _) = open(&s, &session).await?;
    let rows = lc::list_routes(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct RouteBody {
    subject: String,
    name: String,
    #[serde(default)]
    min_amount_minor: i64,
    steps: Vec<RouteStep>,
    #[serde(default = "yes")]
    active: bool,
}
fn yes() -> bool {
    true
}

pub async fn create_route(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<RouteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    let id = lc::create_route(
        &mut t,
        c.user,
        &b.subject,
        b.name.trim(),
        b.min_amount_minor,
        &b.steps,
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_route(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<RouteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    lc::update_route(
        &mut t,
        c.user,
        id,
        b.name.trim(),
        b.min_amount_minor,
        &b.steps,
        b.active,
    )
    .await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_route(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    lc::delete_route(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct FlagBody {
    on: bool,
}

pub async fn set_contract_viewer(
    State(s): State<AppState>,
    session: Session,
    Path(user): Path<Uuid>,
    Json(b): Json<FlagBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    lc::set_contract_viewer(&mut t, c.user, user, b.on).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- requests ----------

#[derive(Deserialize)]
pub struct ClaimQuery {
    #[serde(default)]
    view: String,
    project: Option<Uuid>,
}

pub async fn list_claims(
    State(s): State<AppState>,
    session: Session,
    Query(q): Query<ClaimQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let f = match (q.view.as_str(), q.project) {
        (_, Some(p)) => ClaimFilter::Project(p),
        ("to_approve", _) => ClaimFilter::ToApprove,
        ("to_pay", _) => ClaimFilter::ToPay,
        ("all", _) => ClaimFilter::All,
        _ => ClaimFilter::Mine,
    };
    let rows = lc::list_claims(&mut t, &v, f).await?;
    t.commit().await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct ClaimBody {
    kind: String,
    project_id: Option<Uuid>,
    phase_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    title: String,
    #[serde(default)]
    justification: String,
    currency: String,
    #[serde(default)]
    payee: String,
    #[serde(default)]
    destination: String,
    depart_on: Option<NaiveDate>,
    return_on: Option<NaiveDate>,
    #[serde(default)]
    travellers: Vec<Uuid>,
    #[serde(default)]
    lines: Vec<LineRow>,
    /// Submit right after saving.
    #[serde(default)]
    submit: bool,
}

impl ClaimBody {
    fn as_new<'a>(&'a self, ccy: &'a str) -> lc::NewClaim<'a> {
        lc::NewClaim {
            kind: &self.kind,
            project_id: self.project_id,
            phase_id: self.phase_id,
            parent_id: self.parent_id,
            title: &self.title,
            justification: &self.justification,
            currency: ccy,
            payee: &self.payee,
            destination: &self.destination,
            depart_on: self.depart_on,
            return_on: self.return_on,
            travellers: &self.travellers,
            lines: &self.lines,
        }
    }
}

pub async fn create_claim(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<ClaimBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ccy = currency(&b.currency)?;
    let (mut t, c, _) = open(&s, &session).await?;
    let mut row = lc::create_claim(&mut t, c.user, b.as_new(&ccy)).await?;
    if b.submit {
        row = lc::submit_claim(&mut t, c.user, row.id).await?;
    }
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn update_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<ClaimBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ccy = currency(&b.currency)?;
    let (mut t, c, _) = open(&s, &session).await?;
    let mut row = lc::update_claim(&mut t, c.user, id, b.as_new(&ccy)).await?;
    if b.submit {
        row = lc::submit_claim(&mut t, c.user, id).await?;
    }
    t.commit().await?;
    Ok(Json(row))
}

async fn visible_claim(t: &mut TenantTx, v: &Viewer, id: Uuid) -> Result<lc::ClaimRow, ApiError> {
    let c = lc::get_claim(t, id).await?;
    if !lc::can_see_claim(t, v, &c).await? {
        return Err(StoreError::NotFound.into());
    }
    Ok(c)
}

pub async fn get_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let c = visible_claim(&mut t, &v, id).await?;
    let lines = lc::claim_lines(&mut t, id).await?;
    let documents = lc::list_documents(&mut t, id).await?;
    let actions = lc::list_actions(&mut t, id).await?;
    let approval = lc::approval_view(&mut t, c.approval_id).await?;
    let (children, bookings) = lc::claim_children(&mut t, id).await?;
    let mut travellers = Vec::new();
    for u in &c.travellers {
        travellers
            .push(json!({ "id": u, "name": cm_store::flow::display_name(&mut t, *u).await? }));
    }
    t.commit().await?;
    Ok(Json(json!({
        "claim": c, "lines": lines, "documents": documents, "actions": actions, "approval": approval,
        "children": children, "bookings": bookings, "travellers": travellers,
    })))
}

#[derive(Deserialize)]
pub struct DecideBody {
    action: String,
    to_user: Option<Uuid>,
    #[serde(default)]
    note: String,
}

pub async fn decide_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<DecideBody>,
) -> Result<impl IntoResponse, ApiError> {
    let action = lc::parse_action(&b.action, b.to_user).ok_or_else(|| {
        ApiError::bad_request("action approve|return|reject|delegate|forward (with to_user)")
    })?;
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::decide_claim(&mut t, c.user, id, action, b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct NoteBody {
    #[serde(default)]
    note: String,
    #[serde(default)]
    reference: String,
}

pub async fn submit_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::submit_claim(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn cancel_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<NoteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::cancel_claim(&mut t, c.user, id, b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn pay_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<NoteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.sees_money() {
        return Err(ApiError::forbidden());
    }
    let row = lc::pay_claim(&mut t, c.user, id, b.reference.trim(), b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn close_claim(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<NoteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::close_claim(&mut t, c.user, id, b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct BookingBody {
    to_user: Uuid,
    title: String,
    #[serde(default)]
    note: String,
    due_on: Option<NaiveDate>,
}

/// The traveller, the project manager, Finance or an Admin hands a booking to someone.
pub async fn delegate_booking(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<BookingBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, v) = open(&s, &session).await?;
    let claim = visible_claim(&mut t, &v, id).await?;
    let pm = match claim.project_id {
        Some(p) => can_manage(&c, &cm_store::work::get_project(&mut t, p).await?),
        None => false,
    };
    if !(claim.applicant_id == c.user || claim.travellers.contains(&c.user) || pm || c.sees_money())
    {
        return Err(ApiError::forbidden());
    }
    if b.title.trim().is_empty() {
        return Err(ApiError::bad_request("title is required"));
    }
    let it = lc::delegate_booking(
        &mut t,
        c.user,
        id,
        b.to_user,
        b.title.trim(),
        b.note.trim(),
        b.due_on,
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(it)))
}

// ---------- documents ----------

/// Upload files to a request or contract. Field `label` names the document
/// ("receipt", "signed copy"...); uploading under the same label adds a version.
pub async fn upload_document(
    State(s): State<AppState>,
    session: Session,
    Path((owner_kind, owner)): Path<(String, Uuid)>,
    mut form: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let kind = match owner_kind.as_str() {
        "claims" => "claim",
        "contracts" => "contract",
        _ => return Err(StoreError::NotFound.into()),
    };
    let mut label = String::new();
    let mut uploaded = Vec::new();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        match field.name() {
            Some("label") => {
                label = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?
                    .trim()
                    .chars()
                    .take(100)
                    .collect();
            }
            Some("file") => uploaded.push(store_field(&s, &session, field).await?),
            _ => {}
        }
    }
    if uploaded.is_empty() {
        return Err(ApiError::bad_request("no file in the upload"));
    }
    let result: Result<Vec<lc::DocumentRow>, ApiError> = async {
        let (mut t, c, v) = open(&s, &session).await?;
        let allowed = if kind == "claim" {
            let cl = visible_claim(&mut t, &v, owner).await?;
            cl.applicant_id == c.user || cl.travellers.contains(&c.user) || c.sees_money()
        } else {
            let ct = visible_contract(&mut t, &v, owner).await?;
            ct.owner_id == c.user || ct.created_by == c.user || c.sees_money()
        };
        if !allowed {
            return Err(ApiError::forbidden());
        }
        let mut rows = Vec::new();
        for (id, name, mime, st) in &uploaded {
            rows.push(
                lc::create_document(
                    &mut t,
                    lc::NewDocument {
                        id: *id,
                        owner_kind: kind,
                        owner_id: owner,
                        label: &label,
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

pub async fn download_document(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let d = lc::get_document(&mut t, id).await?;
    if d.owner_kind == "claim" {
        visible_claim(&mut t, &v, d.owner_id).await?;
    } else {
        visible_contract(&mut t, &v, d.owner_id).await?;
    }
    t.commit().await?;
    serve_file(&s, &d.object_key, &d.mime, &d.file_name).await
}

// ---------- contracts ----------

async fn visible_contract(
    t: &mut TenantTx,
    v: &Viewer,
    id: Uuid,
) -> Result<lc::ContractRow, ApiError> {
    let c = lc::get_contract(t, id).await?;
    if !lc::can_see_contract(t, v, &c).await? {
        return Err(StoreError::NotFound.into());
    }
    Ok(c)
}

#[derive(Deserialize)]
pub struct ContractQuery {
    project: Option<Uuid>,
}

pub async fn list_contracts(
    State(s): State<AppState>,
    session: Session,
    Query(q): Query<ContractQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let rows = lc::list_contracts(&mut t, &v, q.project).await?;
    t.commit().await?;
    Ok(Json(
        json!({ "contracts": rows, "sees_all": v.is_admin || v.is_finance || v.can_view_contracts }),
    ))
}

#[derive(Deserialize)]
pub struct ContractBody {
    project_id: Option<Uuid>,
    title: String,
    #[serde(default)]
    counterparty: String,
    kind: String,
    currency: String,
    #[serde(default)]
    amount_minor: i64,
    owner_id: Option<Uuid>,
    starts_on: Option<NaiveDate>,
    ends_on: Option<NaiveDate>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    submit: bool,
}

impl ContractBody {
    fn as_new<'a>(&'a self, ccy: &'a str, me: Uuid) -> lc::NewContract<'a> {
        lc::NewContract {
            project_id: self.project_id,
            title: &self.title,
            counterparty: self.counterparty.trim(),
            kind: &self.kind,
            currency: ccy,
            amount_minor: self.amount_minor,
            owner_id: self.owner_id.unwrap_or(me),
            starts_on: self.starts_on,
            ends_on: self.ends_on,
            summary: &self.summary,
        }
    }
}

pub async fn create_contract(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<ContractBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ccy = currency(&b.currency)?;
    let (mut t, c, _) = open(&s, &session).await?;
    let mut row = lc::create_contract(&mut t, c.user, b.as_new(&ccy, c.user)).await?;
    if b.submit {
        row = lc::submit_contract(&mut t, c.user, row.id).await?;
    }
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn update_contract(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<ContractBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ccy = currency(&b.currency)?;
    let (mut t, c, _) = open(&s, &session).await?;
    let mut row = lc::update_contract(&mut t, c.user, id, b.as_new(&ccy, c.user)).await?;
    if b.submit {
        row = lc::submit_contract(&mut t, c.user, id).await?;
    }
    t.commit().await?;
    Ok(Json(row))
}

pub async fn get_contract(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let c = visible_contract(&mut t, &v, id).await?;
    let milestones = lc::list_milestones(&mut t, id).await?;
    let documents = lc::list_documents(&mut t, id).await?;
    let actions = lc::list_actions(&mut t, id).await?;
    let approval = lc::approval_view(&mut t, c.approval_id).await?;
    t.commit().await?;
    Ok(Json(json!({
        "contract": c, "milestones": milestones, "documents": documents, "actions": actions, "approval": approval,
    })))
}

pub async fn submit_contract(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::submit_contract(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(Json(row))
}

pub async fn decide_contract(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<DecideBody>,
) -> Result<impl IntoResponse, ApiError> {
    let action = lc::parse_action(&b.action, b.to_user).ok_or_else(|| {
        ApiError::bad_request("action approve|return|reject|delegate|forward (with to_user)")
    })?;
    let (mut t, c, _) = open(&s, &session).await?;
    let row = lc::decide_contract(&mut t, c.user, id, action, b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct ContractStatusBody {
    status: String,
    signed_on: Option<NaiveDate>,
    #[serde(default)]
    note: String,
}

fn may_run_contract(c: &Caller, ct: &lc::ContractRow) -> bool {
    ct.owner_id == c.user || ct.created_by == c.user || c.sees_money()
}

pub async fn set_contract_status(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<ContractStatusBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, v) = open(&s, &session).await?;
    let ct = visible_contract(&mut t, &v, id).await?;
    if !may_run_contract(&c, &ct) {
        return Err(ApiError::forbidden());
    }
    let row =
        lc::set_contract_status(&mut t, c.user, id, &b.status, b.signed_on, b.note.trim()).await?;
    t.commit().await?;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct MilestoneBody {
    phase_id: Option<Uuid>,
    title: String,
    due_on: Option<NaiveDate>,
    #[serde(default)]
    amount_minor: i64,
}

pub async fn add_milestone(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<MilestoneBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, v) = open(&s, &session).await?;
    let ct = visible_contract(&mut t, &v, id).await?;
    if !may_run_contract(&c, &ct) {
        return Err(ApiError::forbidden());
    }
    let mid = lc::add_milestone(
        &mut t,
        c.user,
        id,
        b.phase_id,
        &b.title,
        b.due_on,
        b.amount_minor,
    )
    .await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": mid }))))
}

#[derive(Deserialize)]
pub struct MilestoneStatusBody {
    status: String,
    settled_on: Option<NaiveDate>,
}

pub async fn set_milestone_status(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<MilestoneStatusBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, v) = open(&s, &session).await?;
    let contract = lc::milestone_contract(&mut t, id).await?;
    let ct = visible_contract(&mut t, &v, contract).await?;
    if !may_run_contract(&c, &ct) {
        return Err(ApiError::forbidden());
    }
    lc::set_milestone_status(&mut t, c.user, id, &b.status, b.settled_on).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- project view ----------

/// The project file: timeline, expense summary and contracts. The project
/// manager and money people see the expense summary.
pub async fn project_file(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, v) = open(&s, &session).await?;
    let p = cm_store::work::get_project(&mut t, id).await?;
    let timeline = lc::project_file(&mut t, &v, id).await?;
    let expenses: Value = if can_manage(&c, &p) || c.sees_money() {
        json!(lc::project_expenses(&mut t, id).await?)
    } else {
        Value::Null
    };
    let labour: Value = if c.sees_money() {
        json!(cm_store::finance::project_cost(&mut t, id).await?)
    } else {
        Value::Null
    };
    let contracts = lc::list_contracts(&mut t, &v, Some(id)).await?;
    let claims = lc::list_claims(&mut t, &v, ClaimFilter::Project(id)).await?;
    t.commit().await?;
    Ok(Json(json!({
        "timeline": timeline, "expenses": expenses, "cost": labour, "contracts": contracts, "claims": claims,
        "can_set_budget": c.sees_money(),
    })))
}

#[derive(Deserialize)]
pub struct BudgetBody {
    budget_minor: Option<i64>,
}

pub async fn set_expense_budget(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<BudgetBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.sees_money() {
        return Err(ApiError::forbidden());
    }
    if b.budget_minor.is_some_and(|x| x < 0) {
        return Err(ApiError::bad_request("budget cannot be negative"));
    }
    lc::set_expense_budget(&mut t, c.user, id, b.budget_minor).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_phase_budget(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<BudgetBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c, _) = open(&s, &session).await?;
    if !c.sees_money() {
        return Err(ApiError::forbidden());
    }
    if b.budget_minor.is_some_and(|x| x < 0) {
        return Err(ApiError::bad_request("budget cannot be negative"));
    }
    lc::set_phase_budget(&mut t, c.user, id, b.budget_minor).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Everything waiting for the caller: approvals, payments, receipts to confirm.
pub async fn waiting(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, _, v) = open(&s, &session).await?;
    let w = lc::waiting_on(&mut t, &v).await?;
    t.commit().await?;
    Ok(Json(w))
}
