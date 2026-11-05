//! Account tokens, email outbox, invitations, members and job roles.

use crate::{map_unique, Result, StoreError, TenantTx};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

pub const NO_PASSWORD: &str = "!";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPurpose {
    VerifyEmail,
    ResetPassword,
    Invite,
}

impl TokenPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
            Self::Invite => "invite",
        }
    }
}

pub async fn create_token(
    conn: &mut sqlx::PgConnection,
    token_hash: &[u8],
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    purpose: TokenPurpose,
    expires_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("INSERT INTO auth_token (token_hash, user_id, tenant_id, purpose, expires_at) VALUES ($1, $2, $3, $4, $5)")
        .bind(token_hash)
        .bind(user_id)
        .bind(tenant_id)
        .bind(purpose.as_str())
        .bind(expires_at)
        .execute(conn)
        .await?;
    Ok(())
}

/// Mark a token used and return (user, tenant). Fails if unknown, expired or already used.
pub async fn consume_token(
    conn: &mut sqlx::PgConnection,
    token_hash: &[u8],
    purpose: TokenPurpose,
) -> Result<(Uuid, Option<Uuid>)> {
    sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "UPDATE auth_token SET used_at = now()
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id, tenant_id",
    )
    .bind(token_hash)
    .bind(purpose.as_str())
    .fetch_optional(conn)
    .await?
    .ok_or(StoreError::NotFound)
}

pub async fn enqueue_email(
    conn: &mut sqlx::PgConnection,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO outbox_email (id, to_addr, subject, body_text) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(to)
    .bind(subject)
    .bind(body)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutboxEmail {
    pub id: Uuid,
    pub to_addr: String,
    pub subject: String,
    pub body_text: String,
}

/// Claim up to `limit` unsent emails (skipping rows another worker holds).
pub async fn claim_emails(pool: &PgPool, limit: i64) -> Result<Vec<OutboxEmail>> {
    Ok(sqlx::query_as::<_, OutboxEmail>(
        "UPDATE outbox_email SET attempts = attempts + 1
         WHERE id IN (SELECT id FROM outbox_email
                      WHERE sent_at IS NULL AND attempts < 8
                      ORDER BY created_at LIMIT $1 FOR UPDATE SKIP LOCKED)
         RETURNING id, to_addr, subject, body_text",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn mark_email_result(pool: &PgPool, id: Uuid, error: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE outbox_email SET sent_at = CASE WHEN $2::text IS NULL THEN now() END, last_error = $2 WHERE id = $1")
        .bind(id)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_email_verified(conn: &mut sqlx::PgConnection, user_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE app_user SET email_verified = true WHERE id = $1")
        .bind(user_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Set a new password and end every existing session of the user.
pub async fn set_password(conn: &mut sqlx::PgConnection, user_id: Uuid, hash: &str) -> Result<()> {
    sqlx::query("UPDATE app_user SET password_hash = $2 WHERE id = $1")
        .bind(user_id)
        .bind(hash)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM session WHERE user_id = $1")
        .bind(user_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn user_profile(pool: &PgPool, user_id: Uuid) -> Result<(String, String, bool, bool)> {
    let r: (String, String, bool, String) = sqlx::query_as(
        "SELECT email, display_name, email_verified, password_hash FROM app_user WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok((r.0, r.1, r.2, r.3 != NO_PASSWORD))
}

pub async fn tenant_name(conn: &mut sqlx::PgConnection, tenant_id: Uuid) -> Result<String> {
    let r: (String,) = sqlx::query_as("SELECT name FROM tenant WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(conn)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(r.0)
}

pub struct Invite<'a> {
    pub email: &'a str,
    pub name: &'a str,
    pub is_admin: bool,
    pub is_finance: bool,
    pub invited_by: Uuid,
}

/// Add a person to the current tenant as `invited`. Reuses an existing global
/// identity with the same email (people can belong to several companies).
pub async fn invite_member(t: &mut TenantTx, inv: Invite<'_>) -> Result<Uuid> {
    let existing: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM app_user WHERE lower(email) = lower($1)")
            .bind(inv.email)
            .fetch_optional(&mut *t.tx)
            .await?;
    let user_id = match existing {
        Some((id,)) => id,
        None => {
            let id = Uuid::now_v7();
            sqlx::query("INSERT INTO app_user (id, email, password_hash, display_name) VALUES ($1, $2, $3, $4)")
                .bind(id)
                .bind(inv.email)
                .bind(NO_PASSWORD)
                .bind(inv.name)
                .execute(&mut *t.tx)
                .await
                .map_err(|e| map_unique(e, "email"))?;
            id
        }
    };
    sqlx::query(
        "INSERT INTO membership (tenant_id, user_id, status, is_admin, is_finance, invited_by)
         VALUES ($1, $2, 'invited', $3, $4, $5)",
    )
    .bind(t.tenant_id)
    .bind(user_id)
    .bind(inv.is_admin)
    .bind(inv.is_finance)
    .bind(inv.invited_by)
    .execute(&mut *t.tx)
    .await
    .map_err(|e| map_unique(e, "member"))?;
    Ok(user_id)
}

pub async fn activate_membership(t: &mut TenantTx, user_id: Uuid) -> Result<()> {
    let r = sqlx::query("UPDATE membership SET status = 'active' WHERE tenant_id = $1 AND user_id = $2 AND status = 'invited'")
        .bind(t.tenant_id)
        .bind(user_id)
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub status: String,
    pub is_admin: bool,
    pub is_finance: bool,
    pub can_view_contracts: bool,
}

pub async fn list_members(t: &mut TenantTx) -> Result<Vec<MemberRow>> {
    Ok(sqlx::query_as::<_, MemberRow>(
        "SELECT m.user_id, u.email, u.display_name, m.status, m.is_admin, m.is_finance, m.can_view_contracts
         FROM membership m JOIN app_user u ON u.id = m.user_id
         WHERE m.tenant_id = $1 ORDER BY m.created_at",
    )
    .bind(t.tenant_id)
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn is_active_member(t: &mut TenantTx, user_id: Uuid) -> Result<bool> {
    let r: Option<(bool,)> = sqlx::query_as(
        "SELECT true FROM membership WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'",
    )
    .bind(t.tenant_id)
    .bind(user_id)
    .fetch_optional(&mut *t.tx)
    .await?;
    Ok(r.is_some())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RoleRow {
    pub id: Uuid,
    pub department_id: Option<Uuid>,
    pub name: String,
}

pub async fn list_roles(t: &mut TenantTx) -> Result<Vec<RoleRow>> {
    Ok(
        sqlx::query_as::<_, RoleRow>("SELECT id, department_id, name FROM job_role ORDER BY name")
            .fetch_all(&mut *t.tx)
            .await?,
    )
}

pub async fn create_role(
    t: &mut TenantTx,
    name: &str,
    department_id: Option<Uuid>,
) -> Result<RoleRow> {
    if let Some(d) = department_id {
        crate::get_department(t, d).await?;
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO job_role (id, tenant_id, department_id, name) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(t.tenant_id)
    .bind(department_id)
    .bind(name)
    .execute(&mut *t.tx)
    .await?;
    Ok(RoleRow {
        id,
        department_id,
        name: name.to_owned(),
    })
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserRoleRow {
    pub user_id: Uuid,
    pub role_id: Uuid,
    pub role_name: String,
    pub department_id: Uuid,
    pub department_name: String,
}

/// Give a member a role in a department. Every id must belong to this tenant.
pub async fn assign_role(
    t: &mut TenantTx,
    user_id: Uuid,
    role_id: Uuid,
    department_id: Uuid,
) -> Result<()> {
    if !is_active_member(t, user_id).await? {
        return Err(StoreError::NotFound);
    }
    crate::get_department(t, department_id).await?;
    let role: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM job_role WHERE id = $1")
        .bind(role_id)
        .fetch_optional(&mut *t.tx)
        .await?;
    role.ok_or(StoreError::NotFound)?;
    sqlx::query("INSERT INTO user_role (tenant_id, user_id, role_id, department_id) VALUES ($1, $2, $3, $4)")
        .bind(t.tenant_id)
        .bind(user_id)
        .bind(role_id)
        .bind(department_id)
        .execute(&mut *t.tx)
        .await
        .map_err(|e| map_unique(e, "role assignment"))?;
    Ok(())
}

pub async fn list_user_roles(t: &mut TenantTx) -> Result<Vec<UserRoleRow>> {
    Ok(sqlx::query_as::<_, UserRoleRow>(
        "SELECT ur.user_id, ur.role_id, r.name AS role_name, ur.department_id, d.name AS department_name
         FROM user_role ur JOIN job_role r ON r.id = ur.role_id JOIN department d ON d.id = ur.department_id
         WHERE ur.valid_to IS NULL OR ur.valid_to >= current_date
         ORDER BY d.name, r.name",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn set_department_head(
    t: &mut TenantTx,
    department_id: Uuid,
    head: Option<Uuid>,
) -> Result<()> {
    if let Some(u) = head {
        if !is_active_member(t, u).await? {
            return Err(StoreError::NotFound);
        }
    }
    let r = sqlx::query("UPDATE department SET head_user_id = $2 WHERE id = $1")
        .bind(department_id)
        .bind(head)
        .execute(&mut *t.tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Change a member's Admin/Finance flags. The last active Admin cannot be demoted.
pub async fn set_member_flags(
    t: &mut TenantTx,
    user_id: Uuid,
    is_admin: bool,
    is_finance: bool,
) -> Result<()> {
    if !is_admin {
        let (others,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM membership WHERE tenant_id = $1 AND is_admin AND status = 'active' AND user_id <> $2",
        )
        .bind(t.tenant_id)
        .bind(user_id)
        .fetch_one(&mut *t.tx)
        .await?;
        if others == 0 {
            return Err(StoreError::Invalid("the company needs at least one Admin"));
        }
    }
    let r = sqlx::query(
        "UPDATE membership SET is_admin = $3, is_finance = $4 WHERE tenant_id = $1 AND user_id = $2 AND status = 'active'",
    )
    .bind(t.tenant_id)
    .bind(user_id)
    .bind(is_admin)
    .bind(is_finance)
    .execute(&mut *t.tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}
