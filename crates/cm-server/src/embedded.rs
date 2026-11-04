//! Single-file mode (`--features bundle`): the built web app and a PostgreSQL 17
//! server are compiled into the binary. On first start PostgreSQL is unpacked
//! into the data folder, a cluster is created, and the app gets its own
//! non-superuser role so row-level security still applies (D25).

use anyhow::{bail, Context};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use cm_store::sqlx::{self, Connection};
use include_dir::{include_dir, Dir};
use rand::RngCore;
use std::io::Cursor;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

static WEB: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../web/dist");
// Absolute path to the platform's PostgreSQL archive, set by scripts/build-bundle.sh.
static PG_TXZ: &[u8] = include_bytes!(env!("CM_PG_TXZ"));
const PG_DIR: &str = "postgres-17.6";

/// Serves the embedded single-page app; unknown paths get index.html.
pub async fn web(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = WEB.get_file(path).filter(|_| !path.is_empty());
    let (file, path) = match file {
        Some(f) => (f, path),
        None if path.starts_with("api/") || path.starts_with("assets/") => {
            return StatusCode::NOT_FOUND.into_response()
        }
        None => match WEB.get_file("index.html") {
            Some(f) => (f, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    let mime = match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "json" | "webmanifest" => "application/json",
        _ => "application/octet-stream",
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        file.contents(),
    )
        .into_response()
}

/// True when running as root (Linux reads procfs; elsewhere a root-owned home is the hint).
pub fn is_root() -> bool {
    std::fs::metadata("/proc/self")
        .map(|m| m.uid() == 0)
        .unwrap_or_else(|_| std::env::var("USER").map(|u| u == "root").unwrap_or(false))
}

/// A running embedded PostgreSQL server.
pub struct Postgres {
    pg_ctl: PathBuf,
    data: PathBuf,
    run_as: Option<u32>,
    pub app_url: String,
}

fn secret(path: &Path) -> anyhow::Result<String> {
    if let Ok(s) = std::fs::read_to_string(path) {
        return Ok(s.trim().to_owned());
    }
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    let s: String = b.iter().map(|x| format!("{x:02x}")).collect();
    std::fs::write(path, &s).with_context(|| format!("write {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(s)
}

fn chown_tree(path: &Path, uid: u32) -> anyhow::Result<()> {
    std::os::unix::fs::lchown(path, Some(uid), Some(uid))
        .with_context(|| format!("chown {}", path.display()))?;
    if path.is_dir() && !path.is_symlink() {
        for e in std::fs::read_dir(path)? {
            chown_tree(&e?.path(), uid)?;
        }
    }
    Ok(())
}

fn run(cmd: &mut Command, what: &str, log: &Path) -> anyhow::Result<()> {
    let out = cmd.output().with_context(|| format!("start {what}"))?;
    if !out.status.success() {
        let tail = std::fs::read_to_string(log).unwrap_or_default();
        let tail: Vec<&str> = tail.lines().rev().take(15).collect();
        bail!(
            "{what} failed: {}{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(())
}

impl Postgres {
    /// Unpacks (once), initialises (once) and starts PostgreSQL under `root`.
    pub async fn start(root: &Path, port: u16) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
        let root = root.canonicalize()?;
        // PostgreSQL refuses to run as root; run it as an unprivileged user instead.
        let is_root = is_root();
        let run_as = is_root.then(|| {
            std::env::var("CM_PG_UID")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(65534)
        });

        let pg = root.join(PG_DIR);
        if !pg.join("bin/postgres").exists() {
            tracing::info!(dir = %pg.display(), "unpacking PostgreSQL (first start only)");
            let tmp = root.join(".pg-unpack");
            let _ = std::fs::remove_dir_all(&tmp);
            let tmp2 = tmp.clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut tar = Vec::with_capacity(64 << 20);
                lzma_rs::xz_decompress(&mut Cursor::new(PG_TXZ), &mut tar)
                    .map_err(|e| anyhow::anyhow!("unpack PostgreSQL: {e:?}"))?;
                let mut a = tar::Archive::new(Cursor::new(tar));
                a.set_preserve_permissions(true);
                a.unpack(&tmp2).context("unpack PostgreSQL")?;
                Ok(())
            })
            .await??;
            std::fs::rename(&tmp, &pg)?;
        }

        let su_pw = secret(&root.join("db-superuser.secret"))?;
        let app_pw = secret(&root.join("db-app.secret"))?;
        let data = root.join("db");
        let log = root.join("postgres.log");
        if let Some(uid) = run_as {
            chown_tree(&pg, uid)?;
            std::fs::create_dir_all(&data)?;
            chown_tree(&data, uid)?;
            if !log.exists() {
                std::fs::write(&log, "")?;
            }
            chown_tree(&log, uid)?;
            std::os::unix::fs::lchown(&root, Some(uid), None)?;
        }
        let cmd = |name: &str| {
            let mut c = Command::new(pg.join("bin").join(name));
            if let Some(uid) = run_as {
                c.uid(uid).gid(uid);
            }
            c.env("LC_ALL", "C");
            c
        };

        if !data.join("PG_VERSION").exists() {
            tracing::info!(dir = %data.display(), "creating database cluster (first start only)");
            let pwfile = root.join(".pwfile");
            std::fs::write(&pwfile, &su_pw)?;
            std::fs::set_permissions(&pwfile, std::fs::Permissions::from_mode(0o600))?;
            if let Some(uid) = run_as {
                chown_tree(&pwfile, uid)?;
            }
            let r = run(
                cmd("initdb")
                    .arg("-D")
                    .arg(&data)
                    .args(["-U", "postgres", "-E", "UTF8", "--locale=C"])
                    .args(["--auth=scram-sha-256"])
                    .arg(format!("--pwfile={}", pwfile.display())),
                "initdb",
                &log,
            );
            let _ = std::fs::remove_file(&pwfile);
            if let Err(e) = r {
                if run_as.is_some() {
                    bail!("{e:#}\nHint: when started as root, PostgreSQL runs as uid {} (set CM_PG_UID to change). Every parent folder of {} must be readable by that user; choose another folder with CM_DATA_DIR.", run_as.unwrap_or(65534), root.display());
                }
                return Err(e);
            }
            std::fs::write(
                data.join("postgresql.auto.conf"),
                "listen_addresses = '127.0.0.1'\nunix_socket_directories = ''\nmax_connections = 50\nshared_buffers = 128MB\n",
            )?;
            if let Some(uid) = run_as {
                chown_tree(&data, uid)?;
            }
        }

        let pg_ctl = pg.join("bin/pg_ctl");
        let running = cmd("pg_ctl")
            .arg("-D")
            .arg(&data)
            .arg("status")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !running {
            tracing::info!(port, "starting PostgreSQL");
            run(
                cmd("pg_ctl")
                    .arg("-D")
                    .arg(&data)
                    .arg("-l")
                    .arg(&log)
                    .args(["-w", "-t", "120", "-o"])
                    .arg(format!("-p {port}"))
                    .arg("start"),
                "PostgreSQL",
                &log,
            )?;
        }

        // Create the app's own role and database (owner, never superuser).
        let su = format!("postgres://postgres:{su_pw}@127.0.0.1:{port}/postgres");
        let mut c = sqlx::PgConnection::connect(&su)
            .await
            .context("connect to embedded PostgreSQL")?;
        let has_role: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cm_app')")
                .fetch_one(&mut c)
                .await?;
        if !has_role {
            sqlx::query(&format!(
                "CREATE ROLE cm_app LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '{app_pw}'"
            ))
            .execute(&mut c)
            .await?;
        }
        let has_db: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = 'cm')")
                .fetch_one(&mut c)
                .await?;
        if !has_db {
            sqlx::query("CREATE DATABASE cm OWNER cm_app")
                .execute(&mut c)
                .await?;
        }
        c.close().await?;

        Ok(Self {
            pg_ctl,
            data,
            run_as,
            app_url: format!("postgres://cm_app:{app_pw}@127.0.0.1:{port}/cm"),
        })
    }

    /// Stops the server (fast shutdown: open transactions roll back).
    pub fn stop(&self) {
        let mut c = Command::new(&self.pg_ctl);
        if let Some(uid) = self.run_as {
            c.uid(uid).gid(uid);
        }
        let _ = c
            .arg("-D")
            .arg(&self.data)
            .args(["-m", "fast", "-w", "stop"])
            .output();
        tracing::info!("PostgreSQL stopped");
    }
}
