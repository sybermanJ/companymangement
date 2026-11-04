use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let bind = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());

    // Without DATABASE_URL the bundled build runs its own PostgreSQL.
    #[cfg(feature = "bundle")]
    let mut embedded = None;
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        #[cfg(feature = "bundle")]
        Err(_) => {
            let root = std::env::var("CM_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    let is_root = std::os::unix::fs::MetadataExt::uid(
                        &std::fs::metadata("/proc/self").expect("procfs"),
                    ) == 0;
                    if is_root {
                        "/var/lib/hengtu".into()
                    } else {
                        "hengtu-data".into()
                    }
                });
            let port = std::env::var("CM_PG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(54329);
            let pg = cm_server::embedded::Postgres::start(&root, port)
                .await
                .context("start embedded PostgreSQL")?;
            let url = pg.app_url.clone();
            if std::env::var("STORAGE_DIR").is_err() {
                std::env::set_var("STORAGE_DIR", root.join("objects"));
            }
            tracing::info!(data = %root.display(), "using embedded PostgreSQL");
            embedded = Some(pg);
            url
        }
        #[cfg(not(feature = "bundle"))]
        Err(_) => anyhow::bail!("DATABASE_URL is required"),
    };
    #[cfg(feature = "bundle")]
    let local = embedded.is_some();
    #[cfg(not(feature = "bundle"))]
    let local = false;
    // A self-hosted single-file install is usually reached over plain http first.
    let cookie_secure = std::env::var("COOKIE_SECURE")
        .map(|v| v != "false")
        .unwrap_or(!local);

    let pool = cm_store::connect(&database_url)
        .await
        .context("connect to database")?;
    cm_store::migrate(&pool).await.context("run migrations")?;

    let base_url = std::env::var("APP_BASE_URL")
        .unwrap_or_else(|_| format!("http://{}", bind.replace("0.0.0.0", "localhost")));
    let mail_from =
        std::env::var("MAIL_FROM").unwrap_or_else(|_| "Hengtu <no-reply@localhost>".into());
    cm_server::mail::spawn_worker(pool.clone(), std::env::var("SMTP_URL").ok(), mail_from);

    cm_server::jobs::spawn_scheduler(pool.clone());
    let mut state = cm_server::AppState::new(pool.clone(), cookie_secure, base_url.clone());
    cm_server::jobs::spawn_notify_listener(pool, state.notify.clone());
    state.storage = cm_server::storage::Storage::new(
        std::env::var("STORAGE_DIR").unwrap_or_else(|_| "data/objects".into()),
    );
    #[allow(unused_mut)]
    let mut app = cm_server::router(state);
    #[cfg(feature = "bundle")]
    if std::env::var("WEB_DIR").is_err() {
        app = app.fallback(cm_server::embedded::web);
    }
    // Serve the built web app (single-page app: unknown paths get index.html).
    let web_dir =
        std::path::PathBuf::from(std::env::var("WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    if cfg!(feature = "bundle") && std::env::var("WEB_DIR").is_err() {
        // served from the binary
    } else if web_dir.join("index.html").exists() {
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
    tracing::info!(%bind, url = %base_url, "Hengtu is running");
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        })
        .await;
    #[cfg(feature = "bundle")]
    if let Some(pg) = embedded {
        pg.stop();
    }
    served?;
    Ok(())
}

const HELP: &str = "Hengtu: projects, daily work, hand-offs and costs on one Gantt chart.

Run it and open http://localhost:8080. The first account you register creates your company.

Environment (all optional):
  BIND           address to listen on (default 0.0.0.0:8080)
  APP_BASE_URL   public URL used in emails and webhook addresses (default http://localhost:8080)
  CM_DATA_DIR    data folder for the built-in database and files
                 (default ./hengtu-data, or /var/lib/hengtu when run as root)
  CM_PG_PORT     local port of the built-in PostgreSQL (default 54329, 127.0.0.1 only)
  DATABASE_URL   use an external PostgreSQL instead (a non-superuser role)
  SMTP_URL       send email, e.g. smtps://user:pass@smtp.example.com (otherwise links are logged)
  MAIL_FROM      sender address
  COOKIE_SECURE  true when served over https (default false for the built-in database setup)
  STORAGE_DIR    where attachments are stored (default <data>/objects)";
