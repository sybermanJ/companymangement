//! Phase 2 API: routine work for roles, required deliverables, requests,
//! free-flow hand-offs with cross-department approval, notifications.
//! Needs DATABASE_URL (non-superuser); skipped otherwise.

mod common;
use common::*;

struct Org {
    admin: String,
    rd_head: (String, String),
    qa_head: (String, String),
    dev: (String, String),
    qa1: (String, String),
    qa2: (String, String),
    qa_dept: String,
    qa_role: String,
}

/// 研发 (head rd_head; dev is a backend engineer) and 测试 (head qa_head; qa1, qa2 are QA).
async fn org(app: &axum::Router) -> Org {
    let (admin, _, _) = company(app).await;
    let rd_head = member(app, &admin, "研发主管").await;
    let qa_head = member(app, &admin, "测试主管").await;
    let dev = member(app, &admin, "王磊").await;
    let qa1 = member(app, &admin, "赵敏").await;
    let qa2 = member(app, &admin, "陈默").await;
    let rd = call(
        app,
        "POST",
        "/api/v1/departments",
        Some(&admin),
        Some(json!({ "name": "研发", "head_user_id": rd_head.1 })),
    )
    .await;
    assert_eq!(rd.status, StatusCode::CREATED, "{:?}", rd.json);
    let qa = call(
        app,
        "POST",
        "/api/v1/departments",
        Some(&admin),
        Some(json!({ "name": "测试", "head_user_id": qa_head.1 })),
    )
    .await;
    let (rd, qa) = (id(&rd), id(&qa));
    let be = id(&call(
        app,
        "POST",
        "/api/v1/roles",
        Some(&admin),
        Some(json!({ "name": "后端工程师", "department_id": rd })),
    )
    .await);
    let qa_role = id(&call(
        app,
        "POST",
        "/api/v1/roles",
        Some(&admin),
        Some(json!({ "name": "QA", "department_id": qa })),
    )
    .await);
    for (u, r, d) in [
        (&dev.1, &be, &rd),
        (&qa1.1, &qa_role, &qa),
        (&qa2.1, &qa_role, &qa),
    ] {
        let res = call(
            app,
            "POST",
            "/api/v1/role-assignments",
            Some(&admin),
            Some(json!({ "user_id": u, "role_id": r, "department_id": d })),
        )
        .await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
    }
    Org {
        admin,
        rd_head,
        qa_head,
        dev,
        qa1,
        qa2,
        qa_dept: qa,
        qa_role,
    }
}

async fn mine(app: &axum::Router, who: &str) -> Value {
    call(app, "GET", "/api/v1/my/items", Some(who), None)
        .await
        .json
}

fn routine_items<'a>(v: &'a Value, template_title: &str) -> Vec<&'a Value> {
    v["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|i| {
            i["title"]
                .as_str()
                .is_some_and(|t| t.starts_with(template_title))
        })
        .collect()
}

#[tokio::test]
async fn routine_work_for_a_role_with_required_deliverables() {
    let Some(app) = app().await else { return };
    let o = org(&app).await;

    // A plain member cannot set routine work for another department.
    let body = json!({
        "title": "每日冒烟测试", "recurrence": "daily", "target_role_id": o.qa_role, "target_department_id": o.qa_dept,
        "mode": "each_member", "estimate_minutes": 60,
        "deliverables": [{ "kind": "text", "instructions": "测试结论" }, { "kind": "image", "instructions": "截图" }],
    });
    let r = call(
        &app,
        "POST",
        "/api/v1/routines",
        Some(&o.dev.0),
        Some(body.clone()),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // The department head can; today's items are created at once, one per role holder.
    let r = call(
        &app,
        "POST",
        "/api/v1/routines",
        Some(&o.qa_head.0),
        Some(body),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let q1 = mine(&app, &o.qa1.0).await;
    let q2 = mine(&app, &o.qa2.0).await;
    assert_eq!(routine_items(&q1, "每日冒烟测试").len(), 1);
    assert_eq!(routine_items(&q2, "每日冒烟测试").len(), 1);
    assert!(routine_items(&mine(&app, &o.dev.0).await, "每日冒烟测试").is_empty());
    let item = routine_items(&q1, "每日冒烟测试")[0].clone();
    assert!(item["key"].as_str().expect("key").starts_with("RT-"));
    let item_id = item["id"].as_str().expect("id").to_owned();

    // Generation is idempotent.
    let r = call(
        &app,
        "POST",
        "/api/v1/routines/generate",
        Some(&o.admin),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.json["created"], 0, "{:?}", r.json);

    // Done is refused until the deliverables are in.
    let d = call(
        &app,
        "GET",
        &format!("/api/v1/items/{item_id}"),
        Some(&o.qa1.0),
        None,
    )
    .await;
    let specs = d.json["specs"].as_array().expect("specs").clone();
    assert_eq!(specs.len(), 2);
    let text_spec = specs.iter().find(|s| s["kind"] == "text").expect("text")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let image_spec = specs.iter().find(|s| s["kind"] == "image").expect("image")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let version = d.json["item"]["version"].as_i64().expect("v");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item_id}/status"),
        Some(&o.qa1.0),
        Some(json!({ "version": version, "status": "done" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", r.json);

    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item_id}/submissions"),
        Some(&o.qa1.0),
        Some(json!({ "spec_id": text_spec, "value": "全部通过" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    // A text file does not satisfy an image deliverable.
    let boundary = "B0UNDARY";
    let mk = |mime: &str, name: &str| {
        let mut b = Vec::new();
        b.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"spec_id\"\r\n\r\n{image_spec}\r\n").as_bytes());
        b.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
        b.extend(b"\x89PNG\r\n\x1a\nfake");
        b.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/items/{item_id}/attachments"))
            .header(CSRF_HEADER, "cm")
            .header(header::COOKIE, &o.qa1.0)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(b))
            .expect("req")
    };
    let r = send(&app, mk("text/plain", "notes.txt")).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{:?}", r.json);
    let r = send(&app, mk("image/png", "shot.png")).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let d = call(
        &app,
        "GET",
        &format!("/api/v1/items/{item_id}"),
        Some(&o.qa1.0),
        None,
    )
    .await;
    let version = d.json["item"]["version"].as_i64().expect("v");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item_id}/status"),
        Some(&o.qa1.0),
        Some(json!({ "version": version, "status": "done" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);

    // The supervisor may close an item without deliverables, with a reason; the holder may not.
    let other = routine_items(&q2, "每日冒烟测试")[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let v = call(
        &app,
        "GET",
        &format!("/api/v1/items/{other}"),
        Some(&o.qa2.0),
        None,
    )
    .await
    .json["item"]["version"]
        .as_i64()
        .expect("v");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{other}/status"),
        Some(&o.qa2.0),
        Some(json!({ "version": v, "status": "done", "override_deliverables": true, "note": "x" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{other}/status"),
        Some(&o.qa_head.0),
        Some(json!({ "version": v, "status": "done", "override_deliverables": true })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "reason required");
    let r = call(&app, "POST", &format!("/api/v1/items/{other}/status"), Some(&o.qa_head.0), Some(json!({ "version": v, "status": "done", "override_deliverables": true, "note": "环境故障" }))).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);

    // Pool mode: one item for the role; the first holder to claim takes it.
    let r = call(&app, "POST", "/api/v1/routines", Some(&o.qa_head.0), Some(json!({
        "title": "值班巡检", "recurrence": "daily", "target_role_id": o.qa_role, "target_department_id": o.qa_dept, "mode": "pool",
    }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let pool = mine(&app, &o.qa2.0).await["claimable"]
        .as_array()
        .expect("claimable")
        .clone();
    assert_eq!(pool.len(), 1, "{pool:?}");
    let pid = pool[0]["id"].as_str().expect("id").to_owned();
    assert!(mine(&app, &o.dev.0).await["claimable"]
        .as_array()
        .expect("c")
        .is_empty());
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{pid}/claim"),
        Some(&o.qa2.0),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{pid}/claim"),
        Some(&o.qa1.0),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Supervisors see who is missing deliverables.
    let l = call(&app, "GET", "/api/v1/routines", Some(&o.qa_head.0), None).await;
    assert_eq!(l.json["can_create"], true);
    let l = call(&app, "GET", "/api/v1/routines", Some(&o.dev.0), None).await;
    assert_eq!(l.json["can_create"], false);
    assert!(l.json["missing"].as_array().expect("m").is_empty());
}

#[tokio::test]
async fn requests_and_free_flow_handoffs() {
    let Some(app) = app().await else { return };
    let o = org(&app).await;
    let (outsider, _, _) = company(&app).await;

    // Anyone can ask anyone for something; it becomes a work item.
    let r = call(
        &app,
        "POST",
        "/api/v1/requests",
        Some(&o.dev.0),
        Some(json!({ "to_user": o.qa1.1, "title": "帮忙回归登录", "description": "改了会话逻辑" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    assert!(r.json["key"].as_str().expect("key").starts_with("REQ-"));
    assert_eq!(r.json["holder_user_id"], o.qa1.1.as_str());
    let item = id(&r);
    let inbox = call(&app, "GET", "/api/v1/inbox", Some(&o.qa1.0), None).await;
    assert!(inbox.json["unread"].as_i64().expect("n") >= 1);
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/items/{item}"),
            Some(&outsider),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    let version = |r: &Res| {
        r.json["item"]["version"]
            .as_i64()
            .unwrap_or_else(|| panic!("{:?} {:?}", r.status, r.json))
    };
    let get = |who: String| {
        let app = app.clone();
        let item = item.clone();
        async move {
            call(
                &app,
                "GET",
                &format!("/api/v1/items/{item}"),
                Some(&who),
                None,
            )
            .await
        }
    };

    // Same department: straight to the receiver.
    let v = version(&get(o.qa1.0.clone()).await);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/handoffs"),
        Some(&o.qa1.0),
        Some(json!({ "version": v, "to_user": o.qa2.1, "reason": "我今天请假" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    assert_eq!(r.json["state"], "pending_accept");
    let h = id(&r);
    // Stale version conflicts.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/handoffs"),
        Some(&o.qa1.0),
        Some(json!({ "version": v, "to_user": o.dev.1, "reason": "x" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    // Only the receiver accepts.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.dev.0),
        Some(json!({ "action": "accept" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.qa2.0),
        Some(json!({ "action": "accept" })),
    )
    .await;
    assert_eq!(r.json["state"], "accepted", "{:?}", r.json);
    let d = get(o.qa2.0.clone()).await;
    assert_eq!(d.json["item"]["holder_user_id"], o.qa2.1.as_str());
    assert_eq!(d.json["can_return"], true);
    // Return to sender in one step.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/return"),
        Some(&o.qa2.0),
        Some(json!({ "reason": "不是我的模块" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.json);
    assert_eq!(r.json["holder_user_id"], o.qa1.1.as_str());

    // Cross department: the target's head approves first.
    let v = version(&get(o.qa1.0.clone()).await);
    let r = call(&app, "POST", &format!("/api/v1/items/{item}/handoffs"), Some(&o.qa1.0), Some(json!({ "version": v, "to_user": o.dev.1, "reason": "需要后端确认", "new_due": "2026-12-01" }))).await;
    assert_eq!(r.json["state"], "pending_approval", "{:?}", r.json);
    let h = id(&r);
    let inbox = call(&app, "GET", "/api/v1/inbox", Some(&o.rd_head.0), None).await;
    assert_eq!(
        inbox.json["to_approve"].as_array().expect("a").len(),
        1,
        "{:?}",
        inbox.json
    );
    assert!(call(&app, "GET", "/api/v1/inbox", Some(&o.qa_head.0), None)
        .await
        .json["to_approve"]
        .as_array()
        .expect("a")
        .is_empty());
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.dev.0),
        Some(json!({ "action": "accept" })),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "cannot skip approval"
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.qa_head.0),
        Some(json!({ "action": "approve" })),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "sender's head is not an approver"
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.rd_head.0),
        Some(json!({ "action": "approve" })),
    )
    .await;
    assert_eq!(r.json["state"], "pending_accept", "{:?}", r.json);
    // Declining needs a reason and leaves the item with the sender.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.dev.0),
        Some(json!({ "action": "decline" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.dev.0),
        Some(json!({ "action": "decline", "note": "这周排满了" })),
    )
    .await;
    assert_eq!(r.json["state"], "declined");
    assert_eq!(
        get(o.qa1.0.clone()).await.json["item"]["holder_user_id"],
        o.qa1.1.as_str()
    );

    // Ask again; this time the head rejects.
    let v = version(&get(o.qa1.0.clone()).await);
    let h = id(&call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/handoffs"),
        Some(&o.qa1.0),
        Some(json!({ "version": v, "to_user": o.dev.1, "reason": "再试一次" })),
    )
    .await);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.rd_head.0),
        Some(json!({ "action": "reject", "note": "找别人" })),
    )
    .await;
    assert_eq!(r.json["state"], "rejected");

    // Approved and accepted: the item moves and takes the new due date.
    let v = version(&get(o.qa1.0.clone()).await);
    let h = id(&call(&app, "POST", &format!("/api/v1/items/{item}/handoffs"), Some(&o.qa1.0), Some(json!({ "version": v, "to_user": o.dev.1, "reason": "确认接口", "new_due": "2026-12-01" }))).await);
    call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.rd_head.0),
        Some(json!({ "action": "approve" })),
    )
    .await;
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/handoffs/{h}/decide"),
        Some(&o.dev.0),
        Some(json!({ "action": "accept" })),
    )
    .await;
    assert_eq!(r.json["state"], "accepted");
    let d = get(o.dev.0.clone()).await;
    assert_eq!(d.json["item"]["holder_user_id"], o.dev.1.as_str());
    assert_eq!(d.json["item"]["planned_end"], "2026-12-01");
    assert_eq!(d.json["item"]["handoff_count"], 3);
    let chain: Vec<&str> = d.json["handoffs"]
        .as_array()
        .expect("h")
        .iter()
        .map(|h| h["state"].as_str().expect("s"))
        .collect();
    assert_eq!(
        chain,
        ["accepted", "accepted", "declined", "rejected", "accepted"]
    );
    // Logged time stays with whoever did it.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/logs"),
        Some(&o.dev.0),
        Some(json!({ "minutes": 30, "description": "看了接口" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED);

    // A non-holder outside the chain cannot hand it off.
    let v = version(&d);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/items/{item}/handoffs"),
        Some(&o.qa2.0),
        Some(json!({ "version": v, "to_user": o.qa2.1, "reason": "x" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Notifications: read all clears the count.
    let r = call(
        &app,
        "POST",
        "/api/v1/notifications/read",
        Some(&o.qa1.0),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/v1/inbox", Some(&o.qa1.0), None)
            .await
            .json["unread"],
        0
    );
    let _ = o.admin;
}
