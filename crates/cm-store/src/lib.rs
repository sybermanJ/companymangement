//! PostgreSQL persistence. All tenant data is read and written through
//! [`TenantTx`], which pins the transaction to one tenant so PostgreSQL
//! row-level security enforces isolation even if a query forgets a filter.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub mod accounts;

pub use sqlx;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("already exists: {0}")]
    Conflict(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub(crate) fn map_unique(e: sqlx::Error, what: &'static str) -> StoreError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => StoreError::Conflict(what),
        _ => StoreError::Db(e),
    }
}

pub async fn connect(url: &str) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(20)
        .connect(url)
        .await?)
}

pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

/// A transaction scoped to one tenant (and optionally one acting user).
pub struct TenantTx {
    pub tx: Transaction<'static, Postgres>,
    pub tenant_id: Uuid,
}

impl TenantTx {
    pub async fn begin(pool: &PgPool, tenant_id: Uuid, user_id: Option<Uuid>) -> Result<Self> {
        let mut tx = pool.begin().await?;
        sqlx::query(
            "SELECT set_config('app.tenant_id', $1, true), set_config('app.user_id', $2, true)",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.map(|u| u.to_string()).unwrap_or_default())
        .execute(&mut *tx)
        .await?;
        Ok(Self { tx, tenant_id })
    }

    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DepartmentRow {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub head_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MembershipRow {
    pub tenant_id: Uuid,
    pub tenant_name: String,
    pub is_admin: bool,
    pub is_finance: bool,
}

pub struct NewTenant<'a> {
    pub company_name: &'a str,
    pub slug: &'a str,
    pub default_currency: &'a str,
    pub owner_email: &'a str,
    pub owner_name: &'a str,
    pub owner_password_hash: &'a str,
}

/// Sign-up: create a company, its first (admin + finance) member and a root department.
pub async fn create_tenant_with_owner(pool: &PgPool, n: NewTenant<'_>) -> Result<(Uuid, Uuid)> {
    let tenant_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO tenant (id, name, slug, default_currency) VALUES ($1, $2, $3, $4)")
        .bind(tenant_id)
        .bind(n.company_name)
        .bind(n.slug)
        .bind(n.default_currency)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "company slug"))?;
    sqlx::query(
        "INSERT INTO app_user (id, email, password_hash, display_name) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(n.owner_email)
    .bind(n.owner_password_hash)
    .bind(n.owner_name)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "email"))?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO membership (tenant_id, user_id, is_admin, is_finance) VALUES ($1, $2, true, true)")
        .bind(tenant_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO department (id, tenant_id, name, head_user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(tenant_id)
    .bind(n.company_name)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((tenant_id, user_id))
}

pub async fn find_user_by_email(pool: &PgPool, email: &str) -> Result<Option<UserRow>> {
    Ok(sqlx::query_as::<_, UserRow>(
        "SELECT id, email, display_name, password_hash FROM app_user WHERE lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?)
}

/// Companies a user belongs to (uses the "own memberships" policy branch).
pub async fn memberships_of(pool: &PgPool, user_id: Uuid) -> Result<Vec<MembershipRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query_as::<_, MembershipRow>(
        "SELECT m.tenant_id, t.name AS tenant_name, m.is_admin, m.is_finance
         FROM membership m JOIN tenant t ON t.id = m.tenant_id
         WHERE m.user_id = $1 AND m.status = 'active' AND t.status <> 'suspended'
         ORDER BY m.created_at",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn create_session(
    pool: &PgPool,
    token_hash: &[u8],
    user_id: Uuid,
    tenant_id: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO session (token_hash, user_id, tenant_id, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(tenant_id)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve a session to (user, tenant), only while the membership is active.
pub async fn session_lookup(pool: &PgPool, token_hash: &[u8]) -> Result<Option<(Uuid, Uuid)>> {
    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT user_id, tenant_id FROM session WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, tenant_id)) = row else {
        return Ok(None);
    };
    let mut tx = TenantTx::begin(pool, tenant_id, Some(user_id)).await?;
    let active: Option<(bool,)> = sqlx::query_as(
        "SELECT true FROM membership WHERE user_id = $1 AND tenant_id = $2 AND status = 'active'",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx.tx)
    .await?;
    tx.commit().await?;
    Ok(active.map(|_| (user_id, tenant_id)))
}

pub async fn delete_session(pool: &PgPool, token_hash: &[u8]) -> Result<()> {
    sqlx::query("DELETE FROM session WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_departments(t: &mut TenantTx) -> Result<Vec<DepartmentRow>> {
    Ok(sqlx::query_as::<_, DepartmentRow>(
        "SELECT id, parent_id, name, head_user_id FROM department ORDER BY created_at",
    )
    .fetch_all(&mut *t.tx)
    .await?)
}

pub async fn get_department(t: &mut TenantTx, id: Uuid) -> Result<DepartmentRow> {
    sqlx::query_as::<_, DepartmentRow>(
        "SELECT id, parent_id, name, head_user_id FROM department WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *t.tx)
    .await?
    .ok_or(StoreError::NotFound)
}

pub async fn create_department(
    t: &mut TenantTx,
    name: &str,
    parent_id: Option<Uuid>,
    head_user_id: Option<Uuid>,
) -> Result<DepartmentRow> {
    if let Some(p) = parent_id {
        // RLS hides other tenants' rows, so a foreign parent id is simply "not found".
        get_department(t, p).await?;
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO department (id, tenant_id, parent_id, name, head_user_id) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(t.tenant_id)
        .bind(parent_id)
        .bind(name)
        .bind(head_user_id)
        .execute(&mut *t.tx)
        .await
        .map_err(|e| map_unique(e, "department name"))?;
    get_department(t, id).await
}

/// Admin and finance flags of a member in the current tenant.
pub async fn member_flags(t: &mut TenantTx, user_id: Uuid) -> Result<(bool, bool)> {
    sqlx::query_as::<_, (bool, bool)>(
        "SELECT is_admin, is_finance FROM membership WHERE user_id = $1 AND tenant_id = $2 AND status = 'active'",
    )
    .bind(user_id)
    .bind(t.tenant_id)
    .fetch_optional(&mut *t.tx)
    .await?
    .ok_or(StoreError::NotFound)
}
