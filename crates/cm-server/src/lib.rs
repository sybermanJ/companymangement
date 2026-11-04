//! HTTP API. Handlers parse and authorise; every tenant query runs inside a
//! `TenantTx` taken from the caller's session, never from request input.

mod accounts;
mod admin;
mod auth;
mod error;
mod limiter;
pub mod mail;

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
    /// Public URL of the web app, used in email links (no trailing slash).
    pub base_url: String,
    pub limiter: std::sync::Arc<limiter::LoginLimiter>,
}

impl AppState {
    pub fn new(pool: PgPool, cookie_secure: bool, base_url: impl Into<String>) -> Self {
        Self {
            pool,
            cookie_secure,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            limiter: Default::default(),
        }
    }
}

pub const CSRF_HEADER: &str = "x-requested-with";

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/signup", post(auth::signup))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/verify-email", post(accounts::verify_email))
        .route("/verify-email/resend", post(accounts::resend_verification))
        .route(
            "/password-reset/request",
            post(accounts::request_password_reset),
        )
        .route("/password-reset/confirm", post(accounts::reset_password))
        .route("/invitations", post(accounts::invite))
        .route("/invitations/accept", post(accounts::accept_invite))
        .route("/members", get(admin::list_members))
        .route("/roles", get(admin::list_roles).post(admin::create_role))
        .route("/role-assignments", post(admin::assign_role))
        .route(
            "/departments",
            get(list_departments).post(create_department),
        )
        .route(
            "/departments/{id}/head",
            axum::routing::put(admin::set_head),
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
    require_admin(&mut t, session.user_id).await?;
    let row = cm_store::create_department(&mut t, name, body.parent_id, body.head_user_id).await?;
    t.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub(crate) async fn require_admin(t: &mut TenantTx, user_id: Uuid) -> Result<(), ApiError> {
    let (is_admin, _) = cm_store::member_flags(t, user_id).await?;
    if is_admin {
        Ok(())
    } else {
        Err(ApiError::forbidden())
    }
}
