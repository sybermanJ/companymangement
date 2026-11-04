//! Shared helpers for API integration tests. Needs DATABASE_URL (non-superuser).
#![allow(dead_code)]

pub use axum::body::Body;
pub use axum::http::{header, Request, StatusCode};
pub use cm_server::{router, AppState, CSRF_HEADER};
use http_body_util::BodyExt;
pub use serde_json::{json, Value};
use tower::ServiceExt;
pub use uuid::Uuid;

/// A pool per test: each `#[tokio::test]` has its own runtime, and a pool
/// shared across runtimes breaks when the first test's runtime shuts down.
pub async fn db() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    cm_store::migrate(&pool).await.expect("migrate");
    Some(pool)
}

pub async fn app() -> Option<axum::Router> {
    let pool = db().await?;
    let mut state = AppState::new(pool, false, "http://test");
    state.storage = cm_server::storage::Storage::new(
        std::env::temp_dir().join(format!("cm-test-{}", Uuid::now_v7())),
    );
    Some(router(state))
}

pub struct Res {
    pub status: StatusCode,
    pub cookie: Option<String>,
    pub headers: axum::http::HeaderMap,
    pub json: Value,
    pub bytes: Vec<u8>,
}

pub async fn send(app: &axum::Router, req: Request<Body>) -> Res {
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::to_owned);
    let bytes = res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Res {
        status,
        cookie,
        headers,
        json,
        bytes,
    }
}

pub async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(CSRF_HEADER, "cm");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    send(app, req).await
}

pub async fn upload(
    app: &axum::Router,
    cookie: &str,
    item: &str,
    name: &str,
    mime: &str,
    data: &[u8],
    log: Option<&str>,
) -> Res {
    let boundary = "XBOUNDARYX";
    let mut body = Vec::new();
    if let Some(l) = log {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"work_log_id\"\r\n\r\n{l}\r\n").as_bytes());
    }
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend(data);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/items/{item}/attachments"))
        .header(CSRF_HEADER, "cm")
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("request");
    send(app, req).await
}

pub fn tag() -> String {
    Uuid::now_v7().simple().to_string()[20..].to_owned()
}

pub async fn token_from_mail(to: &str) -> String {
    let pool = db().await.expect("db");
    let (body,): (String,) = sqlx::query_as(
        "SELECT body_text FROM outbox_email WHERE to_addr = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(to)
    .fetch_one(&pool)
    .await
    .expect("email");
    let i = body.find("token=").expect("link") + 6;
    body[i..i + 64].to_owned()
}

/// A company with an admin (also finance) and one plain member. Returns (admin, member, member_id).
pub async fn company(app: &axum::Router) -> (String, String, String) {
    let t = tag();
    let boss = format!("boss-{t}@example.test");
    let r = call(app, "POST", "/api/v1/signup", None, Some(json!({ "company_name": "Work Co", "slug": format!("w-{t}"), "default_currency": "USD", "name": "Boss", "email": boss, "password": "long enough password" }))).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let admin = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": boss, "password": "long enough password" })),
    )
    .await
    .cookie
    .expect("cookie");
    let dev = format!("dev-{t}@example.test");
    let r = call(
        app,
        "POST",
        "/api/v1/invitations",
        Some(&admin),
        Some(json!({ "email": dev, "name": "王磊" })),
    )
    .await;
    let dev_id = r.json["user_id"].as_str().expect("id").to_owned();
    let token = token_from_mail(&dev).await;
    call(
        app,
        "POST",
        "/api/v1/invitations/accept",
        None,
        Some(json!({ "token": token, "password": "dev password 123" })),
    )
    .await;
    let member = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": dev, "password": "dev password 123" })),
    )
    .await
    .cookie
    .expect("cookie");
    (admin, member, dev_id)
}

/// Invite and activate one more member; returns (cookie, user id).
pub async fn member(app: &axum::Router, admin: &str, name: &str) -> (String, String) {
    let email = format!(
        "{}-{}@example.test",
        name.to_lowercase().replace(' ', ""),
        tag()
    );
    let r = call(
        app,
        "POST",
        "/api/v1/invitations",
        Some(admin),
        Some(json!({ "email": email, "name": name })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let id = r.json["user_id"].as_str().expect("id").to_owned();
    let token = token_from_mail(&email).await;
    let a = call(
        app,
        "POST",
        "/api/v1/invitations/accept",
        None,
        Some(json!({ "token": token, "password": "member password 1" })),
    )
    .await;
    assert!(
        a.status.is_success(),
        "accept {email}: {:?} {:?}",
        a.status,
        a.json
    );
    let l = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "member password 1" })),
    )
    .await;
    let cookie = l
        .cookie
        .unwrap_or_else(|| panic!("login {email}: {:?} {:?}", l.status, l.json));
    (cookie, id)
}

pub fn id(r: &Res) -> String {
    r.json["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id in {:?}", r.json))
        .to_owned()
}
