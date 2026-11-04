//! Tenant isolation at the database layer. Needs DATABASE_URL (a non-superuser
//! role that owns the schema); skipped otherwise.

use cm_store::*;
use uuid::Uuid;

async fn pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = connect(&url).await.expect("connect");
    migrate(&pool).await.expect("migrate");
    Some(pool)
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[20..])
}

async fn signup(pool: &sqlx::PgPool, name: &str) -> (Uuid, Uuid) {
    let slug = unique("t");
    let email = format!("{}@example.test", unique("u"));
    create_tenant_with_owner(
        pool,
        NewTenant {
            company_name: name,
            slug: &slug,
            default_currency: "CNY",
            owner_email: &email,
            owner_name: "Owner",
            owner_password_hash: "x",
        },
    )
    .await
    .expect("signup")
}

#[tokio::test]
async fn tenants_cannot_see_or_write_each_other() {
    let Some(pool) = pool().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let (ta, ua) = signup(&pool, "Alpha").await;
    let (tb, ub) = signup(&pool, "Beta").await;

    let mut a = TenantTx::begin(&pool, ta, Some(ua)).await.expect("tx");
    let rd = create_department(&mut a, "R&D", None, None)
        .await
        .expect("create");
    a.commit().await.expect("commit");

    // B sees only its own root department, never A's.
    let mut b = TenantTx::begin(&pool, tb, Some(ub)).await.expect("tx");
    let names: Vec<String> = list_departments(&mut b)
        .await
        .expect("list")
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, vec!["Beta".to_string()]);
    // A's department id is invisible to B.
    assert!(matches!(
        get_department(&mut b, rd.id).await,
        Err(StoreError::NotFound)
    ));
    // B cannot attach a child to A's department.
    assert!(matches!(
        create_department(&mut b, "Sneaky", Some(rd.id), None).await,
        Err(StoreError::NotFound)
    ));
    b.commit().await.expect("commit");

    // A raw insert with another tenant's id is rejected by WITH CHECK.
    let mut b = TenantTx::begin(&pool, tb, Some(ub)).await.expect("tx");
    let err = sqlx::query("INSERT INTO department (id, tenant_id, name) VALUES ($1, $2, 'x')")
        .bind(Uuid::now_v7())
        .bind(ta)
        .execute(&mut *b.tx)
        .await;
    assert!(err.is_err(), "cross-tenant insert must fail");

    // Without a tenant set, nothing is visible.
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM department")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(n.0, 0);
}

#[tokio::test]
async fn user_lists_own_memberships_only() {
    let Some(pool) = pool().await else { return };
    let (ta, ua) = signup(&pool, "Gamma").await;
    let _ = signup(&pool, "Delta").await;
    let m = memberships_of(&pool, ua).await.expect("memberships");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].tenant_id, ta);
    assert!(m[0].is_admin && m[0].is_finance);
}
