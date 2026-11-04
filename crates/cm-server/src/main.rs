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

    let base_url = std::env::var("APP_BASE_URL").unwrap_or_else(|_| format!("http://{bind}"));
    let mail_from =
        std::env::var("MAIL_FROM").unwrap_or_else(|_| "Hengtu <no-reply@localhost>".into());
    cm_server::mail::spawn_worker(pool.clone(), std::env::var("SMTP_URL").ok(), mail_from);

    cm_server::jobs::spawn_scheduler(pool.clone());
    let mut state = cm_server::AppState::new(pool.clone(), cookie_secure, base_url);
    cm_server::jobs::spawn_notify_listener(pool, state.notify.clone());
    state.storage = cm_server::storage::Storage::new(
        std::env::var("STORAGE_DIR").unwrap_or_else(|_| "data/objects".into()),
    );
    let mut app = cm_server::router(state);
    // Serve the built web app (single-page app: unknown paths get index.html).
    let web_dir =
        std::path::PathBuf::from(std::env::var("WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    if web_dir.join("index.html").exists() {
        let spa = tower_http::services::ServeDir::new(&web_dir).fallback(
            tower_http::services::ServeFile::new(web_dir.join("index.html")),
        );
        app = app.fallback_service(spa);
    } else {
        tracing::warn!(dir = %web_dir.display(), "web app not built; serving API only");
    }
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
