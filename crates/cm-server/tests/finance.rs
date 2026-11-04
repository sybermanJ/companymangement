//! Phases 3 and 4 API: rates, multi-currency frozen cost, thresholds and
//! alerts, closed periods, export, hours-only views, reports, Git webhooks.
//! Needs DATABASE_URL (non-superuser); skipped otherwise.

mod common;
use common::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;

async fn log(app: &axum::Router, who: &str, item: &str, minutes: i64, day: &str) -> Res {
    call(app, "POST", &format!("/api/v1/items/{item}/logs"), Some(who), Some(json!({ "minutes": minutes, "started_at": format!("{day}T03:00:00Z"), "description": "work" }))).await
}

async fn spent(app: &axum::Router, admin: &str, project: &str) -> i64 {
    call(
        app,
        "GET",
        &format!("/api/v1/finance/projects/{project}"),
        Some(admin),
        None,
    )
    .await
    .json["cost"]["spent_minor"]
        .as_i64()
        .expect("spent")
}

#[tokio::test]
async fn multi_currency_cost_thresholds_and_periods() {
    let Some(app) = app().await else { return };
    let (admin, dev, dev_id) = company(&app).await;
    let (pm, pm_id) = member(&app, &admin, "林晓").await;
    let (jp, jp_id) = member(&app, &admin, "佐藤").await;

    let p = call(&app, "POST", "/api/v1/projects", Some(&admin), Some(json!({ "key": "PAY", "name": "支付", "owner_id": pm_id, "currency": "USD", "labour_budget_minor": 100_000 }))).await;
    assert_eq!(p.status, StatusCode::CREATED, "{:?}", p.json);
    let pid = id(&p);
    let task = id(&call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&pm), Some(json!({ "kind": "task", "title": "订单", "holder_user_id": dev_id, "estimate_minutes": 600 }))).await);

    // Money is for Finance/Admin only.
    assert_eq!(
        call(&app, "GET", "/api/v1/finance/overview", Some(&pm), None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/finance/rates", Some(&dev), None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let r = call(&app, "POST", "/api/v1/finance/rates", Some(&pm), Some(json!({ "user_id": dev_id, "rate_minor": 1, "currency": "CNY", "effective_from": "2026-01-01" }))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    for (u, rate, ccy) in [(&dev_id, 20_000, "CNY"), (&jp_id, 6_500, "JPY")] {
        let r = call(&app, "POST", "/api/v1/finance/rates", Some(&admin), Some(json!({ "user_id": u, "rate_minor": rate, "currency": ccy, "effective_from": "2026-01-01" }))).await;
        assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    }
    let r = call(&app, "POST", "/api/v1/finance/fx", Some(&admin), Some(json!({ "from_ccy": "CNY", "to_ccy": "USD", "rate": "0.14", "effective_date": "2026-01-01" }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);

    // 2 h × CNY 200 = CNY 400 → USD 56.00, frozen on the entry.
    assert_eq!(
        log(&app, &dev, &task, 120, "2026-11-02").await.status,
        StatusCode::CREATED
    );
    assert_eq!(spent(&app, &admin, &pid).await, 5_600);
    // JPY has no rate to USD yet: saved, flagged as pending FX, not counted.
    let jl = log(&app, &jp, &task, 60, "2026-11-02").await;
    assert_eq!(jl.status, StatusCode::CREATED);
    let ov = call(&app, "GET", "/api/v1/finance/overview", Some(&admin), None).await;
    let pending = ov.json["pending_fx"].as_array().expect("pending");
    assert!(
        pending
            .iter()
            .any(|x| x["from"] == "JPY" && x["to"] == "USD"),
        "{pending:?}"
    );
    assert_eq!(spent(&app, &admin, &pid).await, 5_600);
    let r = call(&app, "POST", "/api/v1/finance/fx", Some(&admin), Some(json!({ "from_ccy": "USD", "to_ccy": "JPY", "rate": "150", "effective_date": "2026-10-01" }))).await;
    assert_eq!(
        r.json["filled"], 1,
        "inverse rate converts the waiting entry: {:?}",
        r.json
    );
    // JPY 6,500 / 150 = USD 43.333… → 4,333 cents (half-even).
    assert_eq!(spent(&app, &admin, &pid).await, 9_933);

    // PMs see hours and the share of budget, never amounts.
    let pv = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}"),
        Some(&pm),
        None,
    )
    .await;
    assert!(pv.json["project"].get("labour_budget_minor").is_none());
    assert!(pv.json["project"].get("cost").is_none());
    assert_eq!(pv.json["project"]["budget_used_pct"], 9);
    let av = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(av.json["project"]["cost"]["spent_minor"], 9_933);

    // Threshold at 10 % of budget: fires once on crossing, notifies Finance with
    // amounts and the PM with a percentage only.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/finance/projects/{pid}/thresholds"),
        Some(&admin),
        Some(json!({ "mode": "percent", "percent": 10, "level": "warn" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    assert!(call(
        &app,
        "GET",
        &format!("/api/v1/finance/projects/{pid}"),
        Some(&admin),
        None
    )
    .await
    .json["alerts"]
        .as_array()
        .expect("a")
        .is_empty());
    let l2 = log(&app, &dev, &task, 60, "2026-11-03").await;
    let fin = call(
        &app,
        "GET",
        &format!("/api/v1/finance/projects/{pid}"),
        Some(&admin),
        None,
    )
    .await;
    let alerts = fin.json["alerts"].as_array().expect("a").clone();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0]["state"], "open");
    log(&app, &dev, &task, 30, "2026-11-03").await;
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/finance/projects/{pid}"),
            Some(&admin),
            None
        )
        .await
        .json["alerts"]
            .as_array()
            .expect("a")
            .len(),
        1,
        "fires once"
    );
    let inbox = call(&app, "GET", "/api/v1/inbox", Some(&pm), None).await;
    let n = inbox.json["notifications"]
        .as_array()
        .expect("n")
        .iter()
        .find(|n| n["kind"] == "alert")
        .expect("pm alert")
        .clone();
    assert!(n["body"].as_str().expect("b").contains('%'));
    assert!(!n["body"].as_str().expect("b").contains("USD"), "{n:?}");
    let inbox = call(&app, "GET", "/api/v1/inbox", Some(&admin), None).await;
    let n = inbox.json["notifications"]
        .as_array()
        .expect("n")
        .iter()
        .find(|n| n["kind"] == "alert")
        .expect("finance alert")
        .clone();
    assert!(n["body"].as_str().expect("b").contains("USD"), "{n:?}");
    // PM sees the alert without amounts and may acknowledge it.
    let pa = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}/alerts"),
        Some(&pm),
        None,
    )
    .await;
    assert!(pa.json[0].get("observed_minor").is_none());
    let aid = pa.json[0]["id"].as_str().expect("id").to_owned();
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/alerts/{aid}/ack"),
            Some(&dev),
            Some(json!({}))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/alerts/{aid}/ack"),
            Some(&pm),
            Some(json!({ "note": "已知" }))
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    // Corrections below the line resolve it.
    let l2id = id(&l2);
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/logs/{l2id}"),
            Some(&dev),
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let all = call(
        &app,
        "GET",
        &format!("/api/v1/finance/projects/{pid}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(
        all.json["alerts"][0]["state"], "acknowledged",
        "still above: {:?}",
        all.json["cost"]
    );

    // Closing a period locks its entries.
    let r = call(
        &app,
        "POST",
        "/api/v1/finance/periods",
        Some(&admin),
        Some(json!({ "starts_on": "2026-11-01", "ends_on": "2026-11-02" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    assert_eq!(r.json["locked"], 2);
    assert_eq!(
        log(&app, &dev, &task, 30, "2026-11-02").await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let first = call(
        &app,
        "GET",
        &format!("/api/v1/items/{task}"),
        Some(&dev),
        None,
    )
    .await
    .json["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .find(|l| l["locked"] == true && l["user_id"] == dev_id.as_str())
        .expect("locked")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/logs/{first}"),
            Some(&dev),
            None
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Recalculation never touches closed periods.
    let before = spent(&app, &admin, &pid).await;
    call(&app, "POST", "/api/v1/finance/rates", Some(&admin), Some(json!({ "user_id": dev_id, "rate_minor": 30_000, "currency": "CNY", "effective_from": "2026-11-01" }))).await;
    let r = call(
        &app,
        "POST",
        "/api/v1/finance/recalculate",
        Some(&admin),
        Some(json!({ "from": "2026-11-01" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    // Only the open 30-minute entry on 11-03 moves: CNY 100 → 150 = +USD 7.00.
    assert_eq!(spent(&app, &admin, &pid).await, before + 700);

    // Export: Finance gets costs; members get their own hours only.
    let csv = call(
        &app,
        "GET",
        "/api/v1/export/timesheet.csv?from=2026-11-01&to=2026-11-30",
        Some(&admin),
        None,
    )
    .await;
    let text = String::from_utf8(csv.bytes.clone()).expect("utf8");
    assert!(
        text.contains("project_cost") && text.contains("USD"),
        "{text}"
    );
    let csv = call(
        &app,
        "GET",
        "/api/v1/export/timesheet.csv?from=2026-11-01&to=2026-11-30",
        Some(&jp),
        None,
    )
    .await;
    let text = String::from_utf8(csv.bytes.clone()).expect("utf8");
    assert!(!text.contains("cost"));
    assert_eq!(text.lines().count(), 2, "header + own entry: {text}");

    // Reports: hours for everyone, costs for Finance only.
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/reports/people/{dev_id}?from=2026-11-01&to=2026-11-30"),
        Some(&pm),
        None,
    )
    .await;
    assert!(
        r.json["data"]["by_project"][0].get("cost_minor").is_none(),
        "{:?}",
        r.json
    );
    assert!(
        r.json["data"]["by_project"][0]["minutes"]
            .as_i64()
            .expect("m")
            > 0
    );
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/reports/people/{dev_id}?from=2026-11-01&to=2026-11-30"),
        Some(&admin),
        None,
    )
    .await;
    assert!(
        r.json["data"]["by_project"][0]["cost_minor"]
            .as_i64()
            .expect("c")
            > 0
    );
    let w = call(
        &app,
        "GET",
        "/api/v1/reports/workload?from=2026-10-26&to=2026-11-08",
        Some(&pm),
        None,
    )
    .await;
    assert!(w.json["data"]["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .any(|c| c["user_id"] == dev_id.as_str() && c["bucket"] == "PAY"));
    let pf = call(&app, "GET", "/api/v1/reports/portfolio", Some(&pm), None).await;
    assert!(pf.json["projects"][0].get("cost").is_none());

    // The audit log records rate and threshold changes.
    let a = call(&app, "GET", "/api/v1/finance/audit", Some(&admin), None).await;
    let actions: Vec<&str> = a
        .json
        .as_array()
        .expect("a")
        .iter()
        .map(|x| x["action"].as_str().expect("s"))
        .collect();
    for want in [
        "rate.set",
        "fx.set",
        "threshold.create",
        "period.close",
        "cost.recalculate",
    ] {
        assert!(actions.contains(&want), "{want} missing from {actions:?}");
    }

    // Admins can make someone Finance.
    let r = call(
        &app,
        "PUT",
        &format!("/api/v1/members/{pm_id}/flags"),
        Some(&admin),
        Some(json!({ "is_admin": false, "is_finance": true })),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/v1/finance/overview", Some(&pm), None)
            .await
            .status,
        StatusCode::OK
    );
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("key");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn git_webhooks_link_commits_to_items() {
    let Some(app) = app().await else { return };
    let (admin, dev, dev_id) = company(&app).await;
    let pid = id(&call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&admin),
        Some(json!({ "key": "GIT", "name": "Git" })),
    )
    .await);
    let item = call(
        &app,
        "POST",
        &format!("/api/v1/projects/{pid}/items"),
        Some(&admin),
        Some(json!({ "kind": "task", "title": "退款", "holder_user_id": dev_id })),
    )
    .await;
    let item_id = id(&item);
    assert_eq!(item.json["key"], "GIT-1");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item_id}/deliverables"),
        Some(&admin),
        Some(json!({ "kind": "git_commit" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);

    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/git/repos",
            Some(&dev),
            Some(json!({ "provider": "github", "name": "x" }))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let r = call(&app, "POST", "/api/v1/git/repos", Some(&admin), Some(json!({ "provider": "github", "name": "acme/pay", "url": "https://github.com/acme/pay", "project_id": pid }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let repo = r.json["repo"]["id"].as_str().expect("id").to_owned();
    let secret = r.json["secret"].as_str().expect("secret").to_owned();
    assert!(r.json["webhook_url"]
        .as_str()
        .expect("u")
        .ends_with(&format!("/api/v1/hooks/git/{repo}")));
    let r = call(
        &app,
        "POST",
        "/api/v1/git/identities",
        Some(&admin),
        Some(json!({ "email": "wanglei@dev.example", "user_id": dev_id })),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);

    let body = json!({
        "ref": "refs/heads/feature/GIT-1-refunds",
        "commits": [
            { "id": "0123456789abcdef0123456789abcdef01234567", "message": "Handle partial refunds", "url": "https://github.com/acme/pay/commit/0123456", "timestamp": "2026-11-03T10:00:00+08:00", "author": { "name": "Wang Lei", "email": "WangLei@dev.example" } },
            { "id": "89abcdef0123456789abcdef0123456789abcdef", "message": "Docs", "url": "", "timestamp": "2026-11-03T11:00:00+08:00", "author": { "name": "Someone", "email": "x@y.example" } }
        ]
    })
    .to_string();
    let hook = |sig: String, body: String| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/hooks/git/{repo}"))
            .header("x-github-event", "push")
            .header("x-hub-signature-256", sig)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("req")
    };
    // No session or CSRF header needed, but the signature must match.
    let r = send(&app, hook("sha256=00".into(), body.clone())).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(
        &app,
        hook(
            format!("sha256={}", sign(&secret, body.as_bytes())),
            body.clone(),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);
    assert_eq!(r.json["commits"], 2);
    // Redelivery is harmless.
    send(
        &app,
        hook(
            format!("sha256={}", sign(&secret, body.as_bytes())),
            body.clone(),
        ),
    )
    .await;

    let d = call(
        &app,
        "GET",
        &format!("/api/v1/items/{item_id}"),
        Some(&dev),
        None,
    )
    .await;
    let commits = d.json["commits"].as_array().expect("commits");
    assert_eq!(
        commits.len(),
        2,
        "branch name links every commit in the push: {commits:?}"
    );
    assert!(commits.iter().any(|c| c["user_name"] == "王磊"));
    assert_eq!(
        d.json["specs"][0]["submitted"], 1,
        "linked commit counts as the deliverable"
    );

    // Manual link for servers without webhooks.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item_id}/commits"),
        Some(&dev),
        Some(json!({ "sha": "deadbeef", "message": "hotfix" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let l = call(&app, "GET", "/api/v1/git/repos", Some(&admin), None).await;
    assert_eq!(l.json["repos"][0]["repo"]["commits"], 2);
    assert!(l.json["unmapped"]
        .as_array()
        .expect("u")
        .iter()
        .any(|u| u["email"] == "x@y.example"));
    // Other companies cannot see the repository.
    let (other, _, _) = company(&app).await;
    assert!(call(&app, "GET", "/api/v1/git/repos", Some(&other), None)
        .await
        .json["repos"]
        .as_array()
        .expect("r")
        .is_empty());
}
