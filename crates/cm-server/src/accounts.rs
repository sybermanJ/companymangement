//! Email verification, password reset and invitations.

use crate::auth::{check_password, hash_password, new_token, token_hash, valid_email};
use crate::{ApiError, AppState, Session};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, Utc};
use cm_store::accounts::{self as acc, TokenPurpose};
use cm_store::{StoreError, TenantTx};
use serde::Deserialize;
use uuid::Uuid;

const VERIFY_HOURS: i64 = 48;
const RESET_HOURS: i64 = 1;
const INVITE_DAYS: i64 = 7;

fn invalid_token() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_token",
        "this link is invalid or has expired",
    )
}

fn map_token(e: StoreError) -> ApiError {
    match e {
        StoreError::NotFound => invalid_token(),
        other => other.into(),
    }
}

pub(crate) async fn send_verification(
    s: &AppState,
    user_id: Uuid,
    email: &str,
) -> Result<(), ApiError> {
    let (token, hash) = new_token();
    let mut tx = s.pool.begin().await.map_err(StoreError::from)?;
    acc::create_token(
        &mut tx,
        &hash,
        user_id,
        None,
        TokenPurpose::VerifyEmail,
        Utc::now() + Duration::hours(VERIFY_HOURS),
    )
    .await?;
    let link = format!("{}/verify-email?token={token}", s.base_url);
    acc::enqueue_email(
        &mut tx,
        email,
        "请验证您的邮箱 / Verify your email",
        &format!("请打开以下链接验证邮箱（48 小时内有效）：\n{link}\n\nOpen this link to verify your email (valid for 48 hours):\n{link}\n"),
    )
    .await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(())
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
}

pub async fn verify_email(
    State(s): State<AppState>,
    Json(b): Json<TokenBody>,
) -> Result<impl IntoResponse, ApiError> {
    let mut tx = s.pool.begin().await.map_err(StoreError::from)?;
    let (user_id, _) = acc::consume_token(
        &mut tx,
        &token_hash(b.token.trim()),
        TokenPurpose::VerifyEmail,
    )
    .await
    .map_err(map_token)?;
    acc::mark_email_verified(&mut tx, user_id).await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn resend_verification(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (email, _, verified, _) = acc::user_profile(&s.pool, session.user_id).await?;
    if !verified {
        send_verification(&s, session.user_id, &email).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct EmailBody {
    email: String,
}

/// Always answers 204 so the response does not reveal whether an account exists.
pub async fn request_password_reset(
    State(s): State<AppState>,
    Json(b): Json<EmailBody>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(user) = cm_store::find_user_by_email(&s.pool, b.email.trim()).await? {
        if user.password_hash != acc::NO_PASSWORD {
            let (token, hash) = new_token();
            let mut tx = s.pool.begin().await.map_err(StoreError::from)?;
            acc::create_token(
                &mut tx,
                &hash,
                user.id,
                None,
                TokenPurpose::ResetPassword,
                Utc::now() + Duration::hours(RESET_HOURS),
            )
            .await?;
            let link = format!("{}/reset-password?token={token}", s.base_url);
            acc::enqueue_email(
                &mut tx,
                &user.email,
                "重置密码 / Reset your password",
                &format!("如果是您本人申请重置密码，请在 1 小时内打开：\n{link}\n如非本人操作请忽略。\n\nIf you asked to reset your password, open this link within 1 hour:\n{link}\nIf not, ignore this email.\n"),
            )
            .await?;
            tx.commit().await.map_err(StoreError::from)?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResetBody {
    token: String,
    password: String,
}

pub async fn reset_password(
    State(s): State<AppState>,
    Json(b): Json<ResetBody>,
) -> Result<impl IntoResponse, ApiError> {
    check_password(&b.password)?;
    let hash = hash_password(&b.password)?;
    let mut tx = s.pool.begin().await.map_err(StoreError::from)?;
    let (user_id, _) = acc::consume_token(
        &mut tx,
        &token_hash(b.token.trim()),
        TokenPurpose::ResetPassword,
    )
    .await
    .map_err(map_token)?;
    acc::set_password(&mut tx, user_id, &hash).await?;
    // Receiving the reset email proves the address.
    acc::mark_email_verified(&mut tx, user_id).await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct InviteBody {
    email: String,
    name: String,
    #[serde(default)]
    is_admin: bool,
    #[serde(default)]
    is_finance: bool,
}

pub async fn invite(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<InviteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let email = b.email.trim();
    let name = b.name.trim();
    if !valid_email(email) || name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "a valid email and a name are required",
        ));
    }
    let mut t = TenantTx::begin(&s.pool, session.tenant_id, Some(session.user_id)).await?;
    crate::require_admin(&mut t, session.user_id).await?;
    let user_id = acc::invite_member(
        &mut t,
        acc::Invite {
            email,
            name,
            is_admin: b.is_admin,
            is_finance: b.is_finance,
            invited_by: session.user_id,
        },
    )
    .await?;
    let company = acc::tenant_name(&mut t.tx, session.tenant_id).await?;
    let (token, hash) = new_token();
    acc::create_token(
        &mut t.tx,
        &hash,
        user_id,
        Some(session.tenant_id),
        TokenPurpose::Invite,
        Utc::now() + Duration::days(INVITE_DAYS),
    )
    .await?;
    let link = format!("{}/accept-invite?token={token}", s.base_url);
    acc::enqueue_email(
        &mut t.tx,
        email,
        &format!("邀请您加入 {company} / You're invited to {company}"),
        &format!("{name}，您好：\n您被邀请加入 {company}。请在 7 天内打开链接接受邀请：\n{link}\n\nHi {name},\nYou have been invited to join {company}. Accept within 7 days:\n{link}\n"),
    )
    .await?;
    t.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "user_id": user_id })),
    ))
}

#[derive(Deserialize)]
pub struct AcceptBody {
    token: String,
    /// Required when the person has no password yet (new to the platform).
    password: Option<String>,
}

pub async fn accept_invite(
    State(s): State<AppState>,
    Json(b): Json<AcceptBody>,
) -> Result<impl IntoResponse, ApiError> {
    let new_hash = match b.password.as_deref() {
        Some(pw) => {
            check_password(pw)?;
            Some(hash_password(pw)?)
        }
        None => None,
    };
    let mut tx = s.pool.begin().await.map_err(StoreError::from)?;
    let (user_id, tenant_id) =
        acc::consume_token(&mut tx, &token_hash(b.token.trim()), TokenPurpose::Invite)
            .await
            .map_err(map_token)?;
    let tenant_id = tenant_id.ok_or_else(invalid_token)?;
    let has_password: (bool,) =
        cm_store::sqlx::query_as("SELECT password_hash <> $2 FROM app_user WHERE id = $1")
            .bind(user_id)
            .bind(acc::NO_PASSWORD)
            .fetch_one(&mut *tx)
            .await
            .map_err(StoreError::from)?;
    match (has_password.0, new_hash) {
        (false, None) => return Err(ApiError::bad_request("choose a password to finish joining")),
        (false, Some(h)) => acc::set_password(&mut tx, user_id, &h).await?,
        // An existing account keeps its password; never let an invite link change it.
        (true, _) => {}
    }
    acc::mark_email_verified(&mut tx, user_id).await?;
    cm_store::sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(StoreError::from)?;
    let mut t = TenantTx { tx, tenant_id };
    acc::activate_membership(&mut t, user_id)
        .await
        .map_err(map_token)?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
