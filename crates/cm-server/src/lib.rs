//! HTTP API. Handlers parse and authorise; every tenant query runs inside a
//! `TenantTx` taken from the caller's session, never from request input.

mod accounts;
mod admin;
mod auth;
mod error;
mod finance;
mod flow;
mod git;
pub mod jobs;
mod limiter;
pub mod mail;
mod reports;
pub mod storage;
mod work;

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
    pub storage: storage::Storage,
    /// User ids with new notifications (fed by `jobs::spawn_notify_listener`).
    pub notify: tokio::sync::broadcast::Sender<String>,
}

impl AppState {
    pub fn new(pool: PgPool, cookie_secure: bool, base_url: impl Into<String>) -> Self {
        Self {
            pool,
            cookie_secure,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            limiter: Default::default(),
            storage: storage::Storage::new(std::env::temp_dir().join("cm-objects")),
            notify: tokio::sync::broadcast::channel(256).0,
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
        .route("/members/{id}/flags", axum::routing::put(admin::set_flags))
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
        .route(
            "/projects",
            get(work::list_projects).post(work::create_project),
        )
        .route("/projects/{id}", get(work::get_project))
        .route("/projects/{id}/items", post(work::create_item))
        .route("/items/{id}", get(work::get_item).patch(work::patch_item))
        .route("/items/{id}/status", post(work::set_status))
        .route("/items/{id}/logs", post(work::create_log))
        .route(
            "/items/{id}/attachments",
            post(work::upload_attachment).layer(axum::extract::DefaultBodyLimit::max(
                work::MAX_UPLOAD_BYTES as usize + 1024 * 1024,
            )),
        )
        .route("/attachments/{id}", get(work::download_attachment))
        .route("/logs/{id}", axum::routing::delete(work::delete_log))
        .route("/dependencies", post(work::add_dependency))
        .route(
            "/dependencies/{from}/{to}",
            axum::routing::delete(work::remove_dependency),
        )
        .route("/my/items", get(work::my_items))
        .route("/timer/start", post(work::start_timer))
        .route("/timer/stop", post(work::stop_timer))
        // phase 2: routine work, deliverables, requests, hand-offs, inbox
        .route(
            "/routines",
            get(flow::list_templates).post(flow::create_template),
        )
        .route("/routines/{id}/active", post(flow::set_template_active))
        .route("/routines/generate", post(flow::generate))
        .route("/items/{id}/claim", post(flow::claim))
        .route("/items/{id}/deliverables", post(flow::add_spec))
        .route("/items/{id}/submissions", post(flow::submit))
        .route("/items/{id}/handoffs", post(flow::request_handoff))
        .route("/items/{id}/return", post(flow::return_item))
        .route("/items/{id}/commits", post(git::link_manual))
        .route("/handoffs/{id}/decide", post(flow::decide_handoff))
        .route("/requests", post(flow::create_request))
        .route("/inbox", get(flow::inbox))
        .route("/notifications/read", post(flow::mark_read))
        .route("/notifications/stream", get(flow::stream))
        // phase 3: finance
        .route("/finance/overview", get(finance::overview))
        .route("/finance/projects/{id}", get(finance::project))
        .route(
            "/finance/projects/{id}/budget",
            axum::routing::put(finance::set_budget),
        )
        .route(
            "/finance/projects/{id}/thresholds",
            post(finance::create_threshold),
        )
        .route(
            "/finance/thresholds/{id}",
            axum::routing::delete(finance::delete_threshold),
        )
        .route(
            "/finance/rates",
            get(finance::list_rates).post(finance::add_rate),
        )
        .route("/finance/fx", get(finance::list_fx).post(finance::add_fx))
        .route("/finance/recalculate", post(finance::recalculate))
        .route(
            "/finance/periods",
            get(finance::list_periods).post(finance::close_period),
        )
        .route("/finance/audit", get(finance::audit))
        .route("/alerts/{id}/ack", post(finance::ack_alert))
        .route("/projects/{id}/alerts", get(finance::project_alerts_hours))
        .route("/export/timesheet.csv", get(finance::export_csv))
        // phase 4: overview and Git
        .route("/reports/portfolio", get(reports::portfolio))
        .route("/reports/workload", get(reports::workload))
        .route("/reports/people/{id}", get(reports::person))
        .route("/reports/handoffs", get(flow::handoff_stats))
        .route("/git/repos", get(git::list).post(git::create))
        .route("/git/repos/{id}", axum::routing::delete(git::delete))
        .route("/git/identities", post(git::set_identity))
        .layer(middleware::from_fn(csrf_guard));
    // Webhooks come from Git servers, not browsers: no session, no CSRF
    // header; they are authenticated by their signature instead.
    let hooks = Router::new().route(
        "/hooks/git/{repo}",
        post(git::webhook).layer(axum::extract::DefaultBodyLimit::max(5 * 1024 * 1024)),
    );
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api/v1", api.merge(hooks))
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
