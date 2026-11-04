//! Company administration: members, job roles, role assignments, department heads.

use crate::{ApiError, AppState, Session};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cm_store::accounts as acc;
use cm_store::TenantTx;
use serde::Deserialize;
use uuid::Uuid;

pub async fn list_members(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    let rows = acc::list_members(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

pub async fn list_roles(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    let roles = acc::list_roles(&mut t).await?;
    let assignments = acc::list_user_roles(&mut t).await?;
    t.commit().await?;
    Ok(Json(
        serde_json::json!({ "roles": roles, "assignments": assignments }),
    ))
}

#[derive(Deserialize)]
pub struct NewRole {
    name: String,
    department_id: Option<Uuid>,
}

pub async fn create_role(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<NewRole>,
) -> Result<impl IntoResponse, ApiError> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("name must be 1–100 characters"));
    }
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    crate::require_admin(&mut t, session.user_id).await?;
    let row = acc::create_role(&mut t, name, b.department_id).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize)]
pub struct Assign {
    user_id: Uuid,
    role_id: Uuid,
    department_id: Uuid,
}

pub async fn assign_role(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<Assign>,
) -> Result<impl IntoResponse, ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    crate::require_admin(&mut t, session.user_id).await?;
    acc::assign_role(&mut t, b.user_id, b.role_id, b.department_id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct Head {
    head_user_id: Option<Uuid>,
}

pub async fn set_head(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    Json(b): Json<Head>,
) -> Result<impl IntoResponse, ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    crate::require_admin(&mut t, session.user_id).await?;
    acc::set_department_head(&mut t, id, b.head_user_id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
