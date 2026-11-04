//! Git repositories (GitHub, Gitea, GitLab, other servers), commit authors
//! mapped to people by an Admin, and links between commits and work items.

use crate::work::{event, get_item};
use crate::{Result, StoreError, TenantTx};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoRow {
    pub id: Uuid,
    pub provider: String,
    pub name: String,
    pub url: String,
    pub project_id: Option<Uuid>,
    pub commits: i64,
    pub created_at: DateTime<Utc>,
}

const REPO_SELECT: &str = "SELECT r.id, r.provider, r.name, r.url, r.project_id,
    (SELECT count(*) FROM git_commit c WHERE c.repo_id = r.id) AS commits, r.created_at FROM git_repository r";

pub async fn create_repo(
    t: &mut TenantTx,
    actor: Uuid,
    provider: &str,
    name: &str,
    url: &str,
    project: Option<Uuid>,
    secret: &str,
) -> Result<RepoRow> {
    if let Some(p) = project {
        crate::work::get_project(t, p).await?;
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO git_repository (id, tenant_id, provider, name, url, project_id, webhook_secret, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(provider)
    .bind(name)
    .bind(url)
    .bind(project)
    .bind(secret)
    .bind(actor)
    .execute(&mut *t.tx)
    .await?;
    sqlx::query("INSERT INTO git_hook_route (repo_id, tenant_id) VALUES ($1, $2)")
        .bind(id)
        .bind(t.tenant_id)
        .execute(&mut *t.tx)
        .await?;
    crate::finance::audit(
        t,
        actor,
        "git.repo.create",
        json!({ "repo": id, "name": name }),
    )
    .await?;
    get_repo(t, id).await
}

pub async fn get_repo(t: &mut TenantTx, id: Uuid) -> Result<RepoRow> {
    sqlx::query_as::<_, RepoRow>(&format!("{REPO_SELECT} WHERE r.id = $1"))
        .bind(id)
        .fetch_optional(&mut *t.tx)
        .await?
        .ok_or(StoreError::NotFound)
}

pub async fn list_repos(t: &mut TenantTx) -> Result<Vec<RepoRow>> {
    Ok(
        sqlx::query_as::<_, RepoRow>(&format!("{REPO_SELECT} ORDER BY r.name"))
            .fetch_all(&mut *t.tx)
            .await?,
    )
}

pub async fn delete_repo(t: &mut TenantTx, actor: Uuid, id: Uuid) -> Result<()> {
    get_repo(t, id).await?;
    sqlx::query("DELETE FROM git_hook_route WHERE repo_id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(t.tenant_id)
        .execute(&mut *t.tx)
        .await?;
    sqlx::query("DELETE FROM git_repository WHERE id = $1")
        .bind(id)
        .execute(&mut *t.tx)
        .await?;
    crate::finance::audit(t, actor, "git.repo.delete", json!({ "repo": id })).await
}

/// Tenant of a repository, for webhooks that arrive without a session.
pub async fn hook_tenant(pool: &PgPool, repo: Uuid) -> Result<Option<Uuid>> {
    Ok(
        sqlx::query_as::<_, (Uuid,)>("SELECT tenant_id FROM git_hook_route WHERE repo_id = $1")
            .bind(repo)
            .fetch_optional(pool)
            .await?
            .map(|r| r.0),
    )
}

pub async fn repo_secret(t: &mut TenantTx, repo: Uuid) -> Result<String> {
    sqlx::query_as::<_, (String,)>("SELECT webhook_secret FROM git_repository WHERE id = $1")
        .bind(repo)
        .fetch_optional(&mut *t.tx)
        .await?
        .map(|r| r.0)
        .ok_or(StoreError::NotFound)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct IdentityRow {
    pub email: String,
    pub user_id: Uuid,
    pub user_name: String,
}

pub async fn list_identities(t: &mut TenantTx) -> Result<Vec<IdentityRow>> {
    Ok(sqlx::query_as::<_, IdentityRow>(
        "SELECT g.email, g.user_id, u.display_name AS user_name FROM git_identity g JOIN app_user u ON u.id = g.user_id ORDER BY g.email",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Map a commit author email to a person; earlier commits from it are attributed too.
pub async fn set_identity(t: &mut TenantTx, actor: Uuid, email: &str, user: Uuid) -> Result<()> {
    crate::work::ensure_active_member(t, user).await?;
    let email = email.trim().to_lowercase();
    sqlx::query(
        "INSERT INTO git_identity (tenant_id, email, user_id) VALUES ($1, $2, $3)
         ON CONFLICT (tenant_id, email) DO UPDATE SET user_id = EXCLUDED.user_id",
    )
    .bind(t.tenant_id)
    .bind(&email)
    .bind(user)
    .execute(&mut *t.tx)
    .await?;
    sqlx::query("UPDATE git_commit SET user_id = $2 WHERE lower(author_email) = $1")
        .bind(&email)
        .bind(user)
        .execute(&mut *t.tx)
        .await?;
    crate::finance::audit(
        t,
        actor,
        "git.identity.set",
        json!({ "email": email, "user": user }),
    )
    .await
}

pub async fn unmapped_emails(t: &mut TenantTx) -> Result<Vec<(String, String, i64)>> {
    Ok(sqlx::query_as::<_, (String, String, i64)>(
        "SELECT lower(author_email), max(author_name), count(*) FROM git_commit
         WHERE user_id IS NULL AND author_email <> '' GROUP BY lower(author_email) ORDER BY 3 DESC LIMIT 100",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub struct IncomingCommit<'a> {
    pub sha: &'a str,
    pub author_name: &'a str,
    pub author_email: &'a str,
    pub message: &'a str,
    pub url: &'a str,
    pub committed_at: Option<DateTime<Utc>>,
}

/// Store a commit (once) and link it to every item key in its message (and
/// `extra_keys`, e.g. from the branch name). Returns linked item keys.
pub async fn ingest_commit(
    t: &mut TenantTx,
    repo: Option<Uuid>,
    c: &IncomingCommit<'_>,
    extra_keys: &[String],
    linked_by: Option<Uuid>,
) -> Result<(Uuid, Vec<String>)> {
    let sha = c.sha.trim().to_lowercase();
    if !(7..=64).contains(&sha.len()) || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(StoreError::Invalid("commit SHA must be 7–64 hex digits"));
    }
    let mapped: Option<(Uuid,)> =
        sqlx::query_as("SELECT user_id FROM git_identity WHERE email = $1")
            .bind(c.author_email.trim().to_lowercase())
            .fetch_optional(&mut *t.tx)
            .await?;
    let user = mapped.map(|m| m.0);
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM git_commit WHERE COALESCE(repo_id, '00000000-0000-0000-0000-000000000000'::uuid) = COALESCE($1, '00000000-0000-0000-0000-000000000000'::uuid) AND sha = $2",
    )
    .bind(repo)
    .bind(&sha)
    .fetch_optional(&mut *t.tx)
    .await?;
    let id = match existing {
        Some((id,)) => id,
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO git_commit (id, tenant_id, repo_id, sha, author_name, author_email, user_id, message, url, committed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(id)
            .bind(t.tenant_id)
            .bind(repo)
            .bind(&sha)
            .bind(c.author_name.chars().take(200).collect::<String>())
            .bind(c.author_email.chars().take(254).collect::<String>())
            .bind(user)
            .bind(c.message.chars().take(10_000).collect::<String>())
            .bind(c.url.chars().take(2_000).collect::<String>())
            .bind(c.committed_at)
            .execute(&mut *t.tx)
            .await?;
            id
        }
    };
    let mut keys = cm_domain::gitref::item_keys(c.message);
    for k in extra_keys {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    let mut linked = Vec::new();
    for k in keys {
        let item: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM work_item WHERE key = $1")
            .bind(&k)
            .fetch_optional(&mut *t.tx)
            .await?;
        if let Some((item,)) = item {
            if link(t, id, item, linked_by.or(user), &sha).await? {
                linked.push(k);
            }
        }
    }
    Ok((id, linked))
}

async fn link(
    t: &mut TenantTx,
    commit: Uuid,
    item: Uuid,
    by: Option<Uuid>,
    sha: &str,
) -> Result<bool> {
    let r = sqlx::query(
        "INSERT INTO git_commit_link (tenant_id, commit_id, work_item_id, linked_by) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(t.tenant_id)
    .bind(commit)
    .bind(item)
    .bind(by)
    .execute(&mut *t.tx)
    .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    let it = get_item(t, item).await?;
    let actor = by.or(it.holder_user_id);
    if let Some(actor) = actor {
        event(
            t,
            item,
            actor,
            "commit_linked",
            json!({ "sha": &sha[..sha.len().min(10)] }),
        )
        .await?;
        // A linked commit counts towards a git_commit deliverable, if the item asks for one.
        let spec = crate::flow::item_specs(t, &it)
            .await?
            .into_iter()
            .find(|s| s.kind == "git_commit" && s.submitted < i64::from(s.min_count));
        if let Some(s) = spec {
            sqlx::query(
                "INSERT INTO deliverable_submission (id, tenant_id, spec_id, work_item_id, value, submitted_by)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(Uuid::now_v7())
            .bind(t.tenant_id)
            .bind(s.id)
            .bind(item)
            .bind(sha)
            .bind(actor)
            .execute(&mut *t.tx)
            .await?;
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CommitRow {
    pub id: Uuid,
    pub repo_name: Option<String>,
    pub sha: String,
    pub author_name: String,
    pub user_name: Option<String>,
    pub message: String,
    pub url: String,
    pub committed_at: Option<DateTime<Utc>>,
}

pub async fn item_commits(t: &mut TenantTx, item: Uuid) -> Result<Vec<CommitRow>> {
    Ok(sqlx::query_as::<_, CommitRow>(
        "SELECT c.id, r.name AS repo_name, c.sha, c.author_name, u.display_name AS user_name, c.message, c.url, c.committed_at
         FROM git_commit_link l JOIN git_commit c ON c.id = l.commit_id
         LEFT JOIN git_repository r ON r.id = c.repo_id LEFT JOIN app_user u ON u.id = c.user_id
         WHERE l.work_item_id = $1 ORDER BY c.committed_at DESC NULLS LAST",
    )
    .bind(item)
    .fetch_all(&mut *t.tx)
    .await?)
}

/// Link a commit to an item by hand (for servers that cannot send webhooks).
pub async fn link_manual(
    t: &mut TenantTx,
    actor: Uuid,
    item: Uuid,
    c: &IncomingCommit<'_>,
) -> Result<()> {
    let it = get_item(t, item).await?;
    let (id, _) = ingest_commit(t, None, c, &[], Some(actor)).await?;
    link(t, id, it.id, Some(actor), &c.sha.trim().to_lowercase()).await?;
    Ok(())
}
