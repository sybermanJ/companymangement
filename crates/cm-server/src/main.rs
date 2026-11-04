use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let bind = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let cookie_secure = std::env::var("COOKIE_SECURE")
        .map(|v| v != "false")
        .unwrap_or(true);

    let pool = cm_store::connect(&database_url)
        .await
        .context("connect to database")?;
    cm_store::migrate(&pool).await.context("run migrations")?;

    let app = cm_server::router(cm_server::AppState {
        pool,
        cookie_secure,
    });
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(%bind, "cm-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
