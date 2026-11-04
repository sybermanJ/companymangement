//! Background jobs: routine generation (today and tomorrow, in each
//! company's time zone), hand-off reminders, and the notification fan-out
//! that wakes server-sent-event streams.

use cm_store::sqlx::postgres::PgListener;
use cm_store::sqlx::PgPool;
use cm_store::TenantTx;
use std::time::Duration;
use tokio::sync::broadcast;

pub fn spawn_scheduler(pool: PgPool) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_once(&pool).await {
                tracing::error!(error = %e, "scheduler run failed");
            }
            tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        }
    });
}

/// One pass over every active company. Safe to run at any time: generation
/// is idempotent and reminders are sent once.
pub async fn run_once(pool: &PgPool) -> Result<usize, cm_store::StoreError> {
    let tenants: Vec<(uuid::Uuid,)> =
        cm_store::sqlx::query_as("SELECT id FROM tenant WHERE status <> 'suspended'")
            .fetch_all(pool)
            .await?;
    let mut created = 0;
    for (tenant,) in tenants {
        let mut t = TenantTx::begin(pool, tenant, None).await?;
        let today = cm_store::flow::tenant_today(&mut t).await?;
        created += cm_store::flow::generate_for_date(&mut t, today).await?;
        created +=
            cm_store::flow::generate_for_date(&mut t, today + chrono::Duration::days(1)).await?;
        cm_store::flow::remind_stale_handoffs(&mut t).await?;
        t.commit().await?;
    }
    if created > 0 {
        tracing::info!(created, "routine items generated");
    }
    Ok(created)
}

/// Relay PostgreSQL `cm_notify` messages (user ids) to in-process subscribers.
pub fn spawn_notify_listener(pool: PgPool, tx: broadcast::Sender<String>) {
    tokio::spawn(async move {
        loop {
            match PgListener::connect_with(&pool).await {
                Ok(mut l) => {
                    if let Err(e) = l.listen("cm_notify").await {
                        tracing::warn!(error = %e, "listen failed");
                    } else {
                        while let Ok(n) = l.recv().await {
                            let _ = tx.send(n.payload().to_owned());
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "notify listener connect failed"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
