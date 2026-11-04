//! Git integration: repositories, signed push webhooks from GitHub, Gitea
//! and GitLab (or anything that sends the same payload), author mapping,
//! and manual commit links.

use crate::work::{begin, manage_item};
use crate::{ApiError, AppState, Session};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use cm_store::git::{self as g, IncomingCommit};
use cm_store::TenantTx;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

pub async fn list(
    State(s): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    let repos = g::list_repos(&mut t).await?;
    let identities = g::list_identities(&mut t).await?;
    let unmapped: Vec<Value> = g::unmapped_emails(&mut t)
        .await?
        .into_iter()
        .map(|(e, n, c)| json!({ "email": e, "name": n, "commits": c }))
        .collect();
    t.commit().await?;
    Ok(Json(json!({
        "repos": repos.iter().map(|r| json!({ "repo": r, "webhook_url": format!("{}/api/v1/hooks/git/{}", s.base_url, r.id) })).collect::<Vec<_>>(),
        "identities": identities, "unmapped": unmapped,
    })))
}

#[derive(Deserialize)]
pub struct RepoBody {
    provider: String,
    name: String,
    #[serde(default)]
    url: String,
    project_id: Option<Uuid>,
}

pub async fn create(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<RepoBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !matches!(
        b.provider.as_str(),
        "github" | "gitea" | "gitlab" | "generic"
    ) {
        return Err(ApiError::bad_request(
            "provider must be github, gitea, gitlab or generic",
        ));
    }
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ApiError::bad_request("name must be 1–200 characters"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    let (secret, _) = crate::auth::new_token();
    let repo = g::create_repo(
        &mut t,
        c.user,
        &b.provider,
        name,
        b.url.trim(),
        b.project_id,
        &secret,
    )
    .await?;
    t.commit().await?;
    // The secret is shown once, for pasting into the Git server's webhook settings.
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "repo": repo, "secret": secret,
            "webhook_url": format!("{}/api/v1/hooks/git/{}", s.base_url, repo.id),
        })),
    ))
}

pub async fn delete(
    State(s): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    g::delete_repo(&mut t, c.user, id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct IdentityBody {
    email: String,
    user_id: Uuid,
}

pub async fn set_identity(
    State(s): State<AppState>,
    session: Session,
    Json(b): Json<IdentityBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !crate::auth::valid_email(&b.email) {
        return Err(ApiError::bad_request("a valid email is required"));
    }
    let (mut t, c) = begin(&s, &session).await?;
    if !c.is_admin {
        return Err(ApiError::forbidden());
    }
    g::set_identity(&mut t, c.user, &b.email, b.user_id).await?;
    t.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ManualCommit {
    sha: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    url: String,
}

pub async fn link_manual(
    State(s): State<AppState>,
    session: Session,
    Path(item): Path<Uuid>,
    Json(b): Json<ManualCommit>,
) -> Result<impl IntoResponse, ApiError> {
    if !b.url.is_empty() && !(b.url.starts_with("https://") || b.url.starts_with("http://")) {
        return Err(ApiError::bad_request(
            "url must start with http:// or https://",
        ));
    }
    let (mut t, c) = begin(&s, &session).await?;
    let it = cm_store::work::get_item(&mut t, item).await?;
    if !(it.holder_user_id == Some(c.user) || manage_item(&mut t, &c, &it).await?) {
        return Err(ApiError::forbidden());
    }
    let (name, email) = cm_store::accounts::user_profile(&s.pool, c.user)
        .await
        .map(|(e, n, _, _)| (n, e))?;
    g::link_manual(
        &mut t,
        c.user,
        item,
        &IncomingCommit {
            sha: &b.sha,
            author_name: &name,
            author_email: &email,
            message: &b.message,
            url: &b.url,
            committed_at: Some(Utc::now()),
        },
    )
    .await?;
    t.commit().await?;
    Ok(StatusCode::CREATED)
}

// ---------- webhook ----------

fn verify(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    let header = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    // GitLab sends the shared secret itself.
    if let Some(tok) = header("x-gitlab-token") {
        return constant_eq(tok.as_bytes(), secret.as_bytes());
    }
    // GitHub: "sha256=<hex>"; Gitea/Forgejo/Gogs: "<hex>".
    let sig = header("x-hub-signature-256")
        .map(|v| v.trim_start_matches("sha256="))
        .or_else(|| header("x-gitea-signature"))
        .or_else(|| header("x-gogs-signature"));
    let Some(sig) = sig else { return false };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let want: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    constant_eq(want.as_bytes(), sig.trim().to_ascii_lowercase().as_bytes())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Push webhook. Accepts the common push payload shape:
/// `{ ref, commits: [{ id, message, url, timestamp, author: { name, email } }] }`.
pub async fn webhook(
    State(s): State<AppState>,
    Path(repo): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let tenant = g::hook_tenant(&s.pool, repo)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown repository"))?;
    let mut t = TenantTx::begin(&s.pool, tenant, None).await?;
    let secret = g::repo_secret(&mut t, repo).await?;
    if !verify(&headers, &body, &secret) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "bad_signature",
            "webhook signature does not match",
        ));
    }
    let event = headers
        .get("x-github-event")
        .or_else(|| headers.get("x-gitea-event"))
        .or_else(|| headers.get("x-gitlab-event"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("push")
        .to_ascii_lowercase();
    if event == "ping" {
        return Ok(Json(json!({ "ok": true, "pong": true })));
    }
    let v: Value =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("body is not JSON"))?;
    let branch = v["ref"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("refs/heads/");
    let branch_keys = cm_domain::gitref::item_keys(branch);
    let mut stored = 0;
    let mut linked = Vec::new();
    for c in v["commits"].as_array().into_iter().flatten().take(500) {
        let sha = c["id"].as_str().unwrap_or("");
        let when = c["timestamp"]
            .as_str()
            .and_then(|x| DateTime::parse_from_rfc3339(x).ok())
            .map(|d| d.with_timezone(&Utc));
        let (_, keys) = g::ingest_commit(
            &mut t,
            Some(repo),
            &IncomingCommit {
                sha,
                author_name: c["author"]["name"].as_str().unwrap_or(""),
                author_email: c["author"]["email"].as_str().unwrap_or(""),
                message: c["message"].as_str().unwrap_or(""),
                url: c["url"].as_str().unwrap_or(""),
                committed_at: when,
            },
            &branch_keys,
            None,
        )
        .await?;
        stored += 1;
        linked.extend(keys);
    }
    t.commit().await?;
    Ok(Json(
        json!({ "ok": true, "commits": stored, "linked": linked }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("key");
        mac.update(body);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn signatures() {
        let body = br#"{"commits":[]}"#;
        let mut h = HeaderMap::new();
        h.insert(
            "x-hub-signature-256",
            HeaderValue::from_str(&format!("sha256={}", sign("s3cret", body))).expect("hv"),
        );
        assert!(verify(&h, body, "s3cret"));
        assert!(!verify(&h, body, "other"));
        assert!(!verify(&h, b"tampered", "s3cret"));

        let mut g = HeaderMap::new();
        g.insert(
            "x-gitea-signature",
            HeaderValue::from_str(&sign("k", body)).expect("hv"),
        );
        assert!(verify(&g, body, "k"));

        let mut l = HeaderMap::new();
        l.insert("x-gitlab-token", HeaderValue::from_static("tok"));
        assert!(verify(&l, body, "tok"));
        assert!(!verify(&l, body, "tok2"));
        assert!(!verify(&HeaderMap::new(), body, "k"));
    }
}
