//! End-to-end API checks against a real database. Needs DATABASE_URL; skipped otherwise.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use cm_server::{router, AppState, CSRF_HEADER};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

static POOL: tokio::sync::OnceCell<Option<sqlx::PgPool>> = tokio::sync::OnceCell::const_new();

async fn db() -> Option<sqlx::PgPool> {
    POOL.get_or_init(|| async {
        let url = std::env::var("DATABASE_URL").ok()?;
        let pool = cm_store::connect(&url).await.expect("connect");
        cm_store::migrate(&pool).await.expect("migrate");
        Some(pool)
    })
    .await
    .clone()
}

/// The raw token from the newest email sent to `to`.
async fn token_from_mail(to: &str) -> String {
    let pool = db().await.expect("db");
    let (body,): (String,) = sqlx::query_as(
        "SELECT body_text FROM outbox_email WHERE to_addr = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(to)
    .fetch_one(&pool)
    .await
    .expect("email queued");
    let i = body.find("token=").expect("link") + 6;
    body[i..i + 64].to_owned()
}

async fn app() -> Option<axum::Router> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = cm_store::connect(&url).await.expect("connect");
    cm_store::migrate(&pool).await.expect("migrate");
    Some(router(AppState::new(pool, false, "http://test")))
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

fn tag() -> String {
    Uuid::now_v7().simple().to_string()[20..].to_owned()
}

/// Sign up and return (cookie, email).
async fn company_with_email(app: &axum::Router, name: &str) -> (String, String) {
    let t = tag();
    let email = format!("boss-{t}@example.test");
    let (s, _, _) = call(app, "POST", "/api/v1/signup", None, Some(json!({
        "company_name": name, "slug": format!("k-{t}"), "name": "Boss", "email": email, "password": "long enough password"
    })), true).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, cookie, _) = call(
        app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "long enough password" })),
        true,
    )
    .await;
    (cookie.expect("cookie"), email)
}

#[tokio::test]
async fn email_verification_and_password_reset() {
    let Some(app) = app().await else { return };
    let (cookie, email) = company_with_email(&app, "Verify Co").await;
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None, false).await;
    assert_eq!(me["email_verified"], false);

    let token = token_from_mail(&email).await;
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/verify-email",
        None,
        Some(json!({ "token": token })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, body) = call(
        &app,
        "POST",
        "/api/v1/verify-email",
        None,
        Some(json!({ "token": token })),
        true,
    )
    .await;
    assert_eq!(
        (s, body["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_token")),
        "single use"
    );
    let (_, _, me) = call(&app, "GET", "/api/v1/me", Some(&cookie), None, false).await;
    assert_eq!(me["email_verified"], true);

    // Unknown emails get the same answer as known ones.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/password-reset/request",
        None,
        Some(json!({ "email": "ghost@example.test" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/password-reset/request",
        None,
        Some(json!({ "email": email })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let token = token_from_mail(&email).await;
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/password-reset/confirm",
        None,
        Some(json!({ "token": token, "password": "a brand new password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Old sessions end, old password stops working, new one works.
    let (s, _, _) = call(&app, "GET", "/api/v1/me", Some(&cookie), None, false).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "long enough password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "a brand new password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn login_is_rate_limited() {
    let Some(app) = app().await else { return };
    let (_, email) = company_with_email(&app, "Limit Co").await;
    for _ in 0..5 {
        let (s, _, _) = call(
            &app,
            "POST",
            "/api/v1/login",
            None,
            Some(json!({ "email": email, "password": "wrong password!" })),
            true,
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused while blocked.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": email, "password": "long enough password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn invite_accept_roles_and_heads() {
    let Some(app) = app().await else { return };
    let (boss, _) = company_with_email(&app, "Invite Co").await;
    let (other, _) = company_with_email(&app, "Other Co").await;
    let new_email = format!("dev-{}@example.test", tag());

    let (s, _, inv) = call(
        &app,
        "POST",
        "/api/v1/invitations",
        Some(&boss),
        Some(json!({ "email": new_email, "name": "王磊" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let dev_id = inv["user_id"].as_str().expect("id").to_owned();

    // Invited people cannot log in before accepting.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": new_email, "password": "whatever password" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let token = token_from_mail(&new_email).await;
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/invitations/accept",
        None,
        Some(json!({ "token": token })),
        true,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "new people must choose a password"
    );
    // That failed attempt rolled back, so the same link still works.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/invitations/accept",
        None,
        Some(json!({ "token": token, "password": "dev password 123" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, dev, _) = call(
        &app,
        "POST",
        "/api/v1/login",
        None,
        Some(json!({ "email": new_email, "password": "dev password 123" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let dev = dev.expect("cookie");

    // Non-admins cannot invite or create roles.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/invitations",
        Some(&dev),
        Some(json!({ "email": "x@example.test", "name": "x" })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (_, _, d) = call(
        &app,
        "POST",
        "/api/v1/departments",
        Some(&boss),
        Some(json!({ "name": "研发部" })),
        true,
    )
    .await;
    let dept = d["id"].as_str().expect("dept").to_owned();
    let (s, _, r) = call(
        &app,
        "POST",
        "/api/v1/roles",
        Some(&boss),
        Some(json!({ "name": "后端工程师", "department_id": dept })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let role = r["id"].as_str().expect("role").to_owned();
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/role-assignments",
        Some(&boss),
        Some(json!({ "user_id": dev_id, "role_id": role, "department_id": dept })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/departments/{dept}/head"),
        Some(&boss),
        Some(json!({ "head_user_id": dev_id })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, _, roles) = call(&app, "GET", "/api/v1/roles", Some(&dev), None, false).await;
    assert_eq!(roles["assignments"][0]["role_name"], "后端工程师");
    let (_, _, members) = call(&app, "GET", "/api/v1/members", Some(&boss), None, false).await;
    assert_eq!(members.as_array().expect("members").len(), 2);

    // Another company's admin cannot touch these ids.
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/v1/role-assignments",
        Some(&other),
        Some(json!({ "user_id": dev_id, "role_id": role, "department_id": dept })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/v1/departments/{dept}/head"),
        Some(&other),
        Some(json!({ "head_user_id": null })),
        true,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, _, members) = call(&app, "GET", "/api/v1/members", Some(&other), None, false).await;
    assert_eq!(members.as_array().expect("members").len(), 1);
}
