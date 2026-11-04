//! HTTP API. Handlers parse and authorise; every tenant query runs inside a
//! `TenantTx` taken from the caller's session, never from request input.

mod auth;
mod error;

use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cm_store::{sqlx::PgPool, TenantTx};
use serde::Deserialize;
use uuid::Uuid;

pub use auth::Session;
pub use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cookie_secure: bool,
}

pub const CSRF_HEADER: &str = "x-requested-with";

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/signup", post(auth::signup))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route(
            "/departments",
            get(list_departments).post(create_department),
        )
        .layer(middleware::from_fn(csrf_guard));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Writes must carry a custom header that cross-site forms cannot set.
/// Together with SameSite=Lax session cookies this blocks CSRF.
async fn csrf_guard(req: axum::extract::Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !req.headers().contains_key(CSRF_HEADER) {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "csrf",
            "missing X-Requested-With header",
        )
        .into_response();
    }
    next.run(req).await
}

async fn list_departments(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    let rows = cm_store::list_departments(&mut t).await?;
    t.commit().await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
struct NewDepartment {
    name: String,
    parent_id: Option<Uuid>,
    head_user_id: Option<Uuid>,
}

async fn create_department(
    State(s): State<AppState>,
    session: Session,
    Json(body): Json<NewDepartment>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("name must be 1–100 characters"));
    }
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    let (is_admin, _) = cm_store::member_flags(&mut t, session.user_id).await?;
    if !is_admin {
        return Err(ApiError::forbidden());
    }
    let row = cm_store::create_department(&mut t, name, body.parent_id, body.head_user_id).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}
