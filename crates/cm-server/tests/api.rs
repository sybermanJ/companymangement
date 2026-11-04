//! End-to-end API checks against a real database. Needs DATABASE_URL; skipped otherwise.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use cm_server::{router, AppState, CSRF_HEADER};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn app() -> Option<axum::Router> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = cm_store::connect(&url).await.expect("connect");
    cm_store::migrate(&pool).await.expect("migrate");
    Some(router(AppState {
        pool,
        cookie_secure: false,
    }))
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
    csrf: bool,
) -> (StatusCode, Option<String>, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if csrf {
        req = req.header(CSRF_HEADER, "cm");
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let set_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::to_owned);
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, set_cookie, json)
}

async fn company(app: &axum::Router, name: &str) -> String {
    let tag = &Uuid::now_v7().simple().to_string()[20..];
    let email = format!("owner-{tag}@example.test");
    let (s, _, _) = call(
        app,
        "POST",
        "/api/v1/signup",
        None,
        Some(json!({
            "company_name": name, "slug": format!("c-{tag}"), "default_currency": "USD",
            "name": "Owner", "email": email, "password": "long enough password"
        })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, cookie, _) = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "long enough password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    cookie.expect("session cookie")
}

#[tokio::test]
async fn signup_login_and_isolated_departments() {
    let Some(app) = app().await else {
        eprintln!("DATABASE_URL not set; skipping");
        return;
    };
    let a = company(&app, "Alpha Co").await;
    let b = company(&app, "Beta Co").await;

    let (s, _, dept) = call(
        &app,
        "POST",
        "/api/v1/departments",
        Some(&a),
        Some(json!({ "name": "R&D" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let dept_id = dept["id"].as_str().expect("id").to_owned();

    let (_, _, list_b) = call(&app, "GET", "/api/v1/departments", Some(&b), None, false).await;
    let names: Vec<&str> = list_b
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();
    assert_eq!(names, vec!["Beta Co"]);

    // B cannot hang a department under A's department.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/departments",
        Some(&b),
        Some(json!({ "name": "x", "parent_id": dept_id })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn auth_and_csrf_guards() {
    let Some(app) = app().await else { return };
    let (s, _, _) = call(&app, "GET", "/api/v1/departments", None, None, false).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let a = company(&app, "Gamma Co").await;
    let (s, _, body) = call(
        &app,
        "POST",
        "/api/v1/departments",
        Some(&a),
        Some(json!({ "name": "QA" })),
        false,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "csrf");

    let (s, _, body) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": "nobody@example.test", "password": "whatever123" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "bad_credentials");

    let (s, _, _) = call(&app, "POST", "/api/v1/logout", Some(&a), None, true).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(&app, "GET", "/api/v1/me", Some(&a), None, false).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
