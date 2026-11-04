//! Phase 1 API: projects, items, dependencies, logs, attachments, timer.
//! Needs DATABASE_URL (non-superuser); skipped otherwise.

mod common;
use common::*;

#[tokio::test]
async fn plan_a_project_with_items_and_dependencies() {
    let Some(app) = app().await else { return };
    let (admin, member, dev_id) = company(&app).await;
    let (other, _, _) = company(&app).await;

    let r = call(&app, "POST", "/api/v1/projects", Some(&admin), Some(json!({ "key": "pay", "name": "支付系统重构", "labour_budget_minor": 6_000_000, "planned_start": "2026-10-19", "planned_end": "2026-11-15" }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    assert_eq!(r.json["key"], "PAY");
    assert_eq!(r.json["currency"], "USD");
    let pid = r.json["id"].as_str().expect("id").to_owned();

    // Only admins create projects.
    let r = call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&member),
        Some(json!({ "key": "XX", "name": "x" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    let phase = call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&admin), Some(json!({ "kind": "phase", "title": "开发", "planned_start": "2026-10-26", "planned_end": "2026-11-08" }))).await;
    assert_eq!(phase.json["key"], "PAY-1");
    let phase_id = phase.json["id"].as_str().expect("id").to_owned();
    let a = call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&admin), Some(json!({ "kind": "task", "title": "订单服务改造", "parent_id": phase_id, "holder_user_id": dev_id, "planned_start": "2026-10-26", "planned_end": "2026-11-06", "estimate_minutes": 4800 }))).await;
    assert_eq!(a.status, StatusCode::CREATED, "{:?}", a.json);
    assert_eq!(a.json["key"], "PAY-2");
    let a_id = a.json["id"].as_str().expect("id").to_owned();
    let b = call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&admin), Some(json!({ "kind": "task", "title": "对账模块", "planned_start": "2026-11-02", "planned_end": "2026-11-12" }))).await;
    let b_id = b.json["id"].as_str().expect("id").to_owned();
    let m = call(
        &app,
        "POST",
        &format!("/api/v1/projects/{pid}/items"),
        Some(&admin),
        Some(json!({ "kind": "milestone", "title": "上线", "planned_end": "2026-11-15" })),
    )
    .await;
    assert_eq!(
        m.json["planned_start"], "2026-11-15",
        "milestone is one day"
    );

    // Bad dates are refused.
    let r = call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&admin), Some(json!({ "kind": "task", "title": "x", "planned_start": "2026-11-10", "planned_end": "2026-11-01" }))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let r = call(
        &app,
        "POST",
        "/api/v1/dependencies",
        Some(&admin),
        Some(json!({ "from_item": a_id, "to_item": b_id })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let r = call(
        &app,
        "POST",
        "/api/v1/dependencies",
        Some(&admin),
        Some(json!({ "from_item": b_id, "to_item": a_id })),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "cycle must be refused"
    );

    let r = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(r.json["items"].as_array().expect("items").len(), 4);
    assert_eq!(r.json["dependencies"].as_array().expect("deps").len(), 1);
    assert_eq!(r.json["can_manage"], false);
    assert!(
        r.json["project"].get("labour_budget_minor").is_none(),
        "members never see budget amounts"
    );
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(r.json["project"]["labour_budget_minor"], 6_000_000);
    assert_eq!(r.json["project"]["estimate_minutes"], 4800);

    // Reschedule (drag on the Gantt) with optimistic versioning.
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/items/{b_id}"),
        Some(&admin),
        Some(json!({ "version": 1, "planned_start": "2026-11-03", "planned_end": "2026-11-13" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/items/{b_id}"),
        Some(&admin),
        Some(json!({ "version": 1, "planned_end": "2026-11-14" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "stale version");

    // Another company sees nothing and changes nothing.
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/projects/{pid}"),
            Some(&other),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/items/{a_id}"),
            Some(&other),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/items/{a_id}/logs"),
            Some(&other),
            Some(json!({ "minutes": 60 }))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let r = call(&app, "GET", "/api/v1/projects", Some(&other), None).await;
    assert!(r.json.as_array().expect("list").is_empty());
}

#[tokio::test]
async fn holders_update_status_and_log_work_with_attachments() {
    let Some(app) = app().await else { return };
    let (admin, member, dev_id) = company(&app).await;
    let pid = call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&admin),
        Some(json!({ "key": "OPS", "name": "运维平台" })),
    )
    .await
    .json["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let item = call(
        &app,
        "POST",
        &format!("/api/v1/projects/{pid}/items"),
        Some(&admin),
        Some(json!({ "kind": "task", "title": "监控告警", "holder_user_id": dev_id })),
    )
    .await;
    let id = item.json["id"].as_str().expect("id").to_owned();

    // Holder may move status and progress, but not planning fields.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{id}/status"),
        Some(&member),
        Some(json!({ "version": 1, "status": "in_progress" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/items/{id}"),
        Some(&member),
        Some(json!({ "version": 2, "progress_pct": 40 })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = call(
        &app,
        "PATCH",
        &format!("/api/v1/items/{id}"),
        Some(&member),
        Some(json!({ "version": 3, "planned_end": "2026-12-01" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{id}/status"),
        Some(&member),
        Some(json!({ "version": 3, "status": "blocked" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{id}/status"),
        Some(&member),
        Some(json!({ "version": 4, "status": "done" })),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "blocked cannot jump to done"
    );

    // Log 3 hours with a screenshot and an SVG.
    let log = call(
        &app,
        "POST",
        &format!("/api/v1/items/{id}/logs"),
        Some(&member),
        Some(json!({ "minutes": 180, "description": "完成告警规则" })),
    )
    .await;
    assert_eq!(log.status, StatusCode::CREATED, "{:?}", log.json);
    let log_id = log.json["id"].as_str().expect("id").to_owned();
    let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
    let r = upload(
        &app,
        &member,
        &id,
        "截图.png",
        "image/png",
        &png,
        Some(&log_id),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let png_id = r.json[0]["id"].as_str().expect("id").to_owned();
    assert_eq!(r.json[0]["kind"], "image");
    assert_eq!(r.json[0]["size_bytes"], 7);
    let r = upload(
        &app,
        &member,
        &id,
        "evil.svg",
        "image/svg+xml",
        b"<svg onload=alert(1)>",
        None,
    )
    .await;
    let svg_id = r.json[0]["id"].as_str().expect("id").to_owned();

    let r = call(
        &app,
        "GET",
        &format!("/api/v1/attachments/{png_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.bytes, png);
    assert_eq!(r.headers[header::CONTENT_TYPE], "image/png");
    let r = call(
        &app,
        "GET",
        &format!("/api/v1/attachments/{svg_id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(
        r.headers[header::CONTENT_TYPE],
        "application/octet-stream",
        "SVG never renders inline"
    );
    assert!(r.headers[header::CONTENT_DISPOSITION]
        .to_str()
        .expect("str")
        .starts_with("attachment"));

    let detail = call(
        &app,
        "GET",
        &format!("/api/v1/items/{id}"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(detail.json["item"]["logged_minutes"], 180);
    assert_eq!(detail.json["attachments"].as_array().expect("att").len(), 2);
    let kinds: Vec<&str> = detail.json["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"status_changed")
            && kinds.contains(&"logged")
            && kinds.contains(&"attachment_added"),
        "{kinds:?}"
    );

    // Only the author deletes a log.
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/logs/{log_id}"),
            Some(&admin),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/api/v1/logs/{log_id}"),
            Some(&member),
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );

    // Timer: one at a time; stopping creates a log.
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/timer/start",
            Some(&member),
            Some(json!({ "work_item_id": id }))
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/timer/start",
            Some(&member),
            Some(json!({ "work_item_id": id }))
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    let mine = call(&app, "GET", "/api/v1/my/items", Some(&member), None).await;
    assert_eq!(mine.json["timer"]["work_item_id"], id.as_str());
    let r = call(
        &app,
        "POST",
        "/api/v1/timer/stop",
        Some(&member),
        Some(json!({ "description": "排查" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        (r.json["source"].as_str(), r.json["minutes"].as_i64()),
        (Some("timer"), Some(1))
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/timer/stop",
            Some(&member),
            Some(json!({}))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}
