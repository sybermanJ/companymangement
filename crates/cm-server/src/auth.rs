//! Email/password sign-up and login with server-side sessions.

use crate::{ApiError, AppState};
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "cm_session";
const SESSION_DAYS: i64 = 14;

/// Authenticated caller, resolved from the session cookie.
#[derive(Clone, Copy, Debug)]
pub struct Session {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .ok_or_else(ApiError::unauthorized)?
            .value()
            .to_owned();
        let (user_id, tenant_id) = cm_store::session_lookup(&state.pool, &token_hash(&token))
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Session { user_id, tenant_id })
    }
}

fn hash_password(pw: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error",
            )
        })
}

fn verify_password(pw: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

fn valid_email(e: &str) -> bool {
    let e = e.trim();
    e.len() <= 254
        && e.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'))
}

#[derive(Deserialize)]
pub struct SignupBody {
    company_name: String,
    slug: String,
    #[serde(default = "default_currency")]
    default_currency: String,
    name: String,
    email: String,
    password: String,
}
fn default_currency() -> String {
    "CNY".into()
}

#[derive(Serialize)]
struct SignupResult {
    tenant_id: Uuid,
    user_id: Uuid,
}

pub async fn signup(
    State(s): State<AppState>,
    Json(b): Json<SignupBody>,
) -> Result<impl IntoResponse, ApiError> {
    if b.company_name.trim().is_empty() || b.name.trim().is_empty() {
        return Err(ApiError::bad_request("company and your name are required"));
    }
    let slug = b.slug.trim().to_ascii_lowercase();
    if !(2..=63).contains(&slug.len())
        || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        || slug.starts_with('-')
    {
        return Err(ApiError::bad_request(
            "slug: 2–63 lowercase letters, digits or '-'",
        ));
    }
    if !valid_email(&b.email) {
        return Err(ApiError::bad_request("a valid email is required"));
    }
    if b.password.chars().count() < 10 {
        return Err(ApiError::bad_request(
            "password must be at least 10 characters",
        ));
    }
    let ccy = cm_domain::Currency::parse(b.default_currency.trim())
        .map_err(|_| ApiError::bad_request("default_currency must be an ISO 4217 code"))?;
    let hash = hash_password(&b.password)?;
    let (tenant_id, user_id) = cm_store::create_tenant_with_owner(
        &s.pool,
        cm_store::NewTenant {
            company_name: b.company_name.trim(),
            slug: &slug,
            default_currency: ccy.as_str(),
            owner_email: b.email.trim(),
            owner_name: b.name.trim(),
            owner_password_hash: &hash,
        },
    )
    .await?;
    // TODO(phase 0): send the verification email; until then email_verified stays false.
    Ok((
        StatusCode::CREATED,
        Json(SignupResult { tenant_id, user_id }),
    ))
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
    tenant_id: Option<Uuid>,
}

pub async fn login(
    State(s): State<AppState>,
    jar: CookieJar,
    Json(b): Json<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    let bad = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "bad_credentials",
            "wrong email or password",
        )
    };
    let user = cm_store::find_user_by_email(&s.pool, b.email.trim()).await?;
    // Always run one verification so response time does not reveal whether the email exists.
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$5F7aX6ZXv0b8q2k8yYk4c2uYw8l1kq4m0v2V7n0w9xY";
    let ok = verify_password(
        &b.password,
        user.as_ref().map_or(DUMMY, |u| u.password_hash.as_str()),
    );
    let user = match (ok, user) {
        (true, Some(u)) => u,
        _ => return Err(bad()),
    };
    let memberships = cm_store::memberships_of(&s.pool, user.id).await?;
    let tenant_id = match b.tenant_id {
        Some(t) if memberships.iter().any(|m| m.tenant_id == t) => t,
        Some(_) => return Err(ApiError::forbidden()),
        None => memberships
            .first()
            .map(|m| m.tenant_id)
            .ok_or_else(ApiError::forbidden)?,
    };
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    cm_store::create_session(
        &s.pool,
        &token_hash(&token),
        user.id,
        tenant_id,
        Utc::now() + Duration::days(SESSION_DAYS),
    )
    .await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .secure(s.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time_days(SESSION_DAYS))
        .build();
    Ok((
        jar.add(cookie),
        Json(
            serde_json::json!({ "user_id": user.id, "tenant_id": tenant_id, "memberships": memberships }),
        ),
    ))
}

fn time_days(d: i64) -> time::Duration {
    time::Duration::days(d)
}

pub async fn logout(
    State(s): State<AppState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        cm_store::delete_session(&s.pool, &token_hash(c.value())).await?;
    }
    Ok((
        jar.remove(Cookie::build(SESSION_COOKIE).path("/")),
        StatusCode::NO_CONTENT,
    ))
}

pub async fn me(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let memberships = cm_store::memberships_of(&s.pool, session.user_id).await?;
    Ok(Json(serde_json::json!({
        "user_id": session.user_id,
        "tenant_id": session.tenant_id,
        "memberships": memberships,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation() {
        assert!(valid_email("a@b.co"));
        assert!(!valid_email("ab.co"));
        assert!(!valid_email("@b.co"));
        assert!(!valid_email("a@bco"));
    }

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse battery").expect("hash");
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not a hash"));
    }
}
