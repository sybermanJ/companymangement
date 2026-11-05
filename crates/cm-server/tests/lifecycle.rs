//! Phase 6 API: expense requests through phase manager → Finance → payment →
//! receipt, configurable routes, trips with delegated bookings, contracts with
//! files and milestones, visibility, expense budgets and alerts, project file.
//! Needs DATABASE_URL (non-superuser); skipped otherwise.

mod common;
use common::*;

async fn doc(app: &axum::Router, cookie: &str, owner: &str, label: &str, name: &str) -> Res {
    let boundary = "XBOUNDARYX";
    let mut body = Vec::new();
    body.extend(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"label\"\r\n\r\n{label}\r\n"
        )
        .as_bytes(),
    );
    body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.4 test\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/{owner}/documents"))
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

fn waiting(r: &Res) -> Vec<String> {
    r.json["waiting_for"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

async fn decide(app: &axum::Router, who: &str, claim: &str, body: Value) -> Res {
    call(
        app,
        "POST",
        &format!("/api/v1/claims/{claim}/decide"),
        Some(who),
        Some(body),
    )
    .await
}

#[tokio::test]
async fn expense_request_lifecycle() {
    let Some(app) = app().await else { return };
    let (admin, dev, dev_id) = company(&app).await;
    let (pm, pm_id) = member(&app, &admin, "林晓").await;
    let (lead, lead_id) = member(&app, &admin, "陈工").await;
    let (fin, fin_id) = member(&app, &admin, "财务赵").await;
    let (assistant, assistant_id) = member(&app, &admin, "行政周").await;
    let (stranger, stranger_id) = member(&app, &admin, "路人").await;
    let r = call(
        &app,
        "PUT",
        &format!("/api/v1/members/{fin_id}/flags"),
        Some(&admin),
        Some(json!({ "is_admin": false, "is_finance": true })),
    )
    .await;
    assert!(r.status.is_success(), "{:?}", r.json);

    let p = call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&admin),
        Some(json!({ "key": "ERP", "name": "定制 ERP", "owner_id": pm_id, "currency": "CNY" })),
    )
    .await;
    assert_eq!(p.status, StatusCode::CREATED, "{:?}", p.json);
    let pid = id(&p);
    let phase = id(&call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&pm), Some(json!({ "kind": "phase", "title": "需求调研", "holder_user_id": lead_id, "planned_start": "2026-11-01", "planned_end": "2026-11-30" }))).await);
    let bare_phase = id(&call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&pm), Some(json!({ "kind": "phase", "title": "实施", "planned_start": "2026-12-01", "planned_end": "2026-12-30" }))).await);

    // Expense budget and an alert at 50 % (Finance only).
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/projects/{pid}/expense-budget"),
            Some(&pm),
            Some(json!({ "budget_minor": 300_000 }))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/projects/{pid}/expense-budget"),
            Some(&fin),
            Some(json!({ "budget_minor": 300_000 }))
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/phases/{phase}/budget"),
            Some(&fin),
            Some(json!({ "budget_minor": 200_000 }))
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let th = call(
        &app,
        "POST",
        &format!("/api/v1/finance/projects/{pid}/thresholds"),
        Some(&fin),
        Some(json!({ "metric": "expense", "mode": "percent", "percent": 50, "level": "warn" })),
    )
    .await;
    assert_eq!(th.status, StatusCode::CREATED, "{:?}", th.json);

    // The developer files a reimbursement against the phase.
    let c = call(&app, "POST", "/api/v1/claims", Some(&dev), Some(json!({
        "kind": "reimbursement", "project_id": pid, "phase_id": phase, "title": "客户现场调研差旅",
        "justification": "客户要求现场梳理业务流程", "currency": "CNY",
        "lines": [
            { "spent_on": "2026-11-03", "category": "flight", "description": "上海-成都", "amount_minor": 120_000 },
            { "spent_on": "2026-11-04", "category": "hotel", "description": "2 晚", "amount_minor": 60_000 }
        ],
        "submit": true
    }))).await;
    assert_eq!(c.status, StatusCode::CREATED, "{:?}", c.json);
    let cid = id(&c);
    assert_eq!(c.json["status"], "in_approval");
    assert_eq!(c.json["amount_minor"], 180_000);
    assert_eq!(waiting(&c), vec![lead_id.clone()]);
    assert!(c.json["key"].as_str().unwrap_or("").starts_with("EXPR-"));

    // Visibility: the PM sees it, a stranger does not.
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/claims/{cid}"),
            Some(&pm),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/claims/{cid}"),
            Some(&stranger),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    // Receipts, versioned by label.
    assert_eq!(
        doc(&app, &dev, &format!("claims/{cid}"), "receipt", "a.pdf")
            .await
            .status,
        StatusCode::CREATED
    );
    let d2 = doc(&app, &dev, &format!("claims/{cid}"), "receipt", "b.pdf").await;
    assert_eq!(d2.json[0]["version"], 2, "{:?}", d2.json);
    assert_eq!(
        doc(
            &app,
            &stranger,
            &format!("claims/{cid}"),
            "receipt",
            "x.pdf"
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // Not your turn; the phase manager adds the PM after themselves.
    assert_eq!(
        decide(&app, &dev, &cid, json!({ "action": "approve" }))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let to_approve = call(
        &app,
        "GET",
        "/api/v1/claims?view=to_approve",
        Some(&lead),
        None,
    )
    .await;
    assert_eq!(to_approve.json.as_array().map(Vec::len), Some(1));
    let r = decide(
        &app,
        &lead,
        &cid,
        json!({ "action": "forward", "to_user": pm_id, "note": "金额较大，请项目经理确认" }),
    )
    .await;
    assert_eq!(waiting(&r), vec![pm_id.clone()], "{:?}", r.json);
    let r = decide(&app, &pm, &cid, json!({ "action": "approve" })).await;
    let mut fin_step = vec![
        fin_id.clone(),
        waiting(&r)
            .into_iter()
            .find(|u| *u != fin_id)
            .unwrap_or_default(),
    ];
    fin_step.sort();
    let mut got = waiting(&r);
    got.sort();
    assert_eq!(
        got, fin_step,
        "finance step = Finance people (incl. the admin)"
    );

    // Finance sends it back: a reason is required.
    assert_eq!(
        decide(&app, &fin, &cid, json!({ "action": "return" }))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let r = decide(
        &app,
        &fin,
        &cid,
        json!({ "action": "return", "note": "缺少酒店发票" }),
    )
    .await;
    assert_eq!(r.json["status"], "returned");
    let inbox = call(&app, "GET", "/api/v1/inbox", Some(&dev), None).await;
    assert!(
        inbox.json.to_string().contains("缺少酒店发票"),
        "{}",
        inbox.json
    );

    // Fix and resubmit: the route starts again (round 2).
    let r = call(&app, "PUT", &format!("/api/v1/claims/{cid}"), Some(&dev), Some(json!({
        "kind": "reimbursement", "project_id": pid, "phase_id": phase, "title": "客户现场调研差旅",
        "justification": "客户要求现场梳理业务流程", "currency": "CNY",
        "lines": [
            { "spent_on": "2026-11-03", "category": "flight", "amount_minor": 120_000 },
            { "spent_on": "2026-11-04", "category": "hotel", "description": "2 晚，附发票", "amount_minor": 60_000 }
        ],
        "submit": true
    }))).await;
    assert_eq!(r.json["status"], "in_approval", "{:?}", r.json);
    assert_eq!(waiting(&r), vec![lead_id.clone()]);
    decide(&app, &lead, &cid, json!({ "action": "approve" })).await;
    let r = decide(
        &app,
        &fin,
        &cid,
        json!({ "action": "approve", "note": "ok" }),
    )
    .await;
    assert_eq!(r.json["status"], "approved", "{:?}", r.json);
    assert_eq!(r.json["project_amount_minor"], 180_000);

    // Expenses count toward cost and the 50 % alert fired.
    let f = call(
        &app,
        "GET",
        &format!("/api/v1/finance/projects/{pid}"),
        Some(&fin),
        None,
    )
    .await;
    assert_eq!(
        f.json["cost"]["expense_minor"], 180_000,
        "{:?}",
        f.json["cost"]
    );
    assert_eq!(f.json["cost"]["total_minor"], 180_000);
    assert!(
        f.json["alerts"].as_array().is_some_and(|a| !a.is_empty()),
        "{:?}",
        f.json["alerts"]
    );
    let pf = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}/file"),
        Some(&pm),
        None,
    )
    .await;
    assert_eq!(
        pf.json["expenses"]["total_minor"], 180_000,
        "{:?}",
        pf.json["expenses"]
    );
    let by_phase = pf.json["expenses"]["by_phase"]
        .as_array()
        .expect("phases")
        .clone();
    let first = by_phase
        .iter()
        .find(|x| x["phase_id"] == phase.as_str())
        .expect("phase row");
    assert_eq!(first["spent_minor"], 180_000);
    assert_eq!(first["budget_minor"], 200_000);
    assert!(pf.json["expenses"]["by_category"]
        .to_string()
        .contains("flight"));
    let tl = pf.json["timeline"].to_string();
    assert!(
        tl.contains("缺少酒店发票") && tl.contains("forward"),
        "{tl}"
    );
    // A plain member sees the timeline but not the expense summary.
    let pf = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}/file"),
        Some(&stranger),
        None,
    )
    .await;
    assert!(pf.json["expenses"].is_null());
    assert!(!pf.json["timeline"].to_string().contains("缺少酒店发票"));

    // Finance pays; the applicant confirms receipt.
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/claims/{cid}/pay"),
            Some(&pm),
            Some(json!({ "reference": "x" }))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let to_pay = call(&app, "GET", "/api/v1/claims?view=to_pay", Some(&fin), None).await;
    assert_eq!(to_pay.json.as_array().map(Vec::len), Some(1));
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/claims/{cid}/pay"),
        Some(&fin),
        Some(json!({ "reference": "BANK-20261105-01" })),
    )
    .await;
    assert_eq!(r.json["status"], "paid", "{:?}", r.json);
    let w = call(&app, "GET", "/api/v1/approvals/waiting", Some(&dev), None).await;
    assert_eq!(w.json["to_confirm"].as_array().map(Vec::len), Some(1));
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/claims/{cid}/close"),
        Some(&dev),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.json["status"], "closed");

    // Trip: approved by the phase manager, booking delegated to the assistant.
    let trip = call(&app, "POST", "/api/v1/claims", Some(&dev), Some(json!({
        "kind": "trip", "project_id": pid, "phase_id": phase, "title": "第二次现场调研", "justification": "确认原型",
        "currency": "CNY", "destination": "成都", "depart_on": "2026-11-20", "return_on": "2026-11-22",
        "travellers": [dev_id, lead_id], "lines": [{ "category": "flight", "amount_minor": 240_000 }], "submit": true
    }))).await;
    let tid = id(&trip);
    assert!(trip.json["key"]
        .as_str()
        .unwrap_or("")
        .starts_with("TRIPR-"));
    // The traveller lead is the phase manager and not the applicant, so they approve.
    let r = decide(&app, &lead, &tid, json!({ "action": "approve" })).await;
    assert_eq!(r.json["status"], "approved", "{:?}", r.json);
    let b = call(
        &app,
        "POST",
        &format!("/api/v1/claims/{tid}/bookings"),
        Some(&dev),
        Some(json!({ "to_user": assistant_id, "title": "订 11/20 上海-成都机票 2 张" })),
    )
    .await;
    assert_eq!(b.status, StatusCode::CREATED, "{:?}", b.json);
    assert_eq!(b.json["holder_user_id"], assistant_id.as_str());
    assert_eq!(b.json["claim_id"], tid.as_str());
    // The assistant can open the trip they book for; the stranger cannot ask for bookings.
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/claims/{tid}"),
            Some(&assistant),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/claims/{tid}/bookings"),
            Some(&stranger),
            Some(json!({ "to_user": assistant_id, "title": "x" }))
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    // Actual costs filed against the trip.
    let child = call(&app, "POST", "/api/v1/claims", Some(&assistant), Some(json!({
        "kind": "payment", "project_id": pid, "phase_id": phase, "parent_id": tid, "title": "机票款", "justification": "TRIP 机票",
        "currency": "CNY", "payee": "航旅公司", "lines": [{ "category": "flight", "amount_minor": 236_000 }]
    }))).await;
    assert_eq!(child.status, StatusCode::CREATED, "{:?}", child.json);
    let t = call(
        &app,
        "GET",
        &format!("/api/v1/claims/{tid}"),
        Some(&dev),
        None,
    )
    .await;
    assert_eq!(t.json["bookings"].as_array().map(Vec::len), Some(1));
    assert_eq!(t.json["children"].as_array().map(Vec::len), Some(1));
    // A trip is not itself cost.
    let pf = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}/file"),
        Some(&fin),
        None,
    )
    .await;
    assert_eq!(pf.json["expenses"]["total_minor"], 180_000);

    // The PM files on a phase without a manager: their own step is passed, Finance decides.
    let own = call(&app, "POST", "/api/v1/claims", Some(&pm), Some(json!({
        "kind": "purchase", "project_id": pid, "phase_id": bare_phase, "title": "测试服务器", "justification": "实施环境",
        "currency": "CNY", "lines": [{ "category": "equipment", "amount_minor": 50_000 }], "submit": true
    }))).await;
    assert!(!waiting(&own).contains(&pm_id), "{:?}", own.json);
    assert!(waiting(&own).contains(&fin_id));

    // A big-payment route: above 1,000,000.00 the stranger signs off before Finance.
    let r = call(&app, "POST", "/api/v1/approval-routes", Some(&fin), Some(json!({ "subject": "payment", "name": "大额", "min_amount_minor": 100_000_000, "steps": [{ "kind": "user", "user_id": stranger_id }, { "kind": "finance" }] }))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(&app, "POST", "/api/v1/approval-routes", Some(&admin), Some(json!({ "subject": "payment", "name": "大额", "min_amount_minor": 100_000_000, "steps": [{ "kind": "user", "user_id": stranger_id }, { "kind": "finance" }] }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.json);
    let big = call(&app, "POST", "/api/v1/claims", Some(&dev), Some(json!({
        "kind": "payment", "project_id": pid, "title": "外包开发首付款", "justification": "合同约定", "currency": "CNY",
        "payee": "某软件公司", "lines": [{ "category": "outsourcing", "amount_minor": 150_000_000 }], "submit": true
    }))).await;
    assert_eq!(waiting(&big), vec![stranger_id.clone()], "{:?}", big.json);
    let small = call(&app, "POST", "/api/v1/claims", Some(&dev), Some(json!({
        "kind": "payment", "project_id": pid, "phase_id": phase, "title": "云服务器", "justification": "测试", "currency": "CNY",
        "lines": [{ "category": "software", "amount_minor": 10_000 }], "submit": true
    }))).await;
    assert_eq!(waiting(&small), vec![lead_id.clone()]);
    // Withdraw.
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/claims/{}/cancel", id(&small)),
        Some(&dev),
        Some(json!({})),
    )
    .await;
    assert_eq!(r.json["status"], "cancelled");
    let routes = call(&app, "GET", "/api/v1/approval-routes", Some(&dev), None).await;
    assert!(
        routes.json.as_array().is_some_and(|a| a.len() >= 7),
        "{:?}",
        routes.json
    );
}

#[tokio::test]
async fn contracts_with_files_milestones_and_visibility() {
    let Some(app) = app().await else { return };
    let (admin, dev, dev_id) = company(&app).await;
    let (pm, pm_id) = member(&app, &admin, "林晓").await;
    let (viewer, viewer_id) = member(&app, &admin, "法务钱").await;
    let p = call(
        &app,
        "POST",
        "/api/v1/projects",
        Some(&admin),
        Some(json!({ "key": "MES", "name": "MES 定制", "owner_id": pm_id, "currency": "CNY" })),
    )
    .await;
    let pid = id(&p);
    let phase = id(&call(&app, "POST", &format!("/api/v1/projects/{pid}/items"), Some(&pm), Some(json!({ "kind": "phase", "title": "上线", "planned_start": "2026-11-01", "planned_end": "2026-12-31" }))).await);
    // The developer works on the project.
    call(
        &app,
        "POST",
        &format!("/api/v1/projects/{pid}/items"),
        Some(&pm),
        Some(json!({ "kind": "task", "title": "接口", "holder_user_id": dev_id })),
    )
    .await;

    let c = call(&app, "POST", "/api/v1/contracts", Some(&pm), Some(json!({
        "project_id": pid, "title": "MES 系统开发合同", "counterparty": "某制造公司", "kind": "sales", "currency": "CNY",
        "amount_minor": 80_000_000, "starts_on": "2026-11-01", "ends_on": "2027-03-31", "summary": "三期付款", "submit": true
    }))).await;
    assert_eq!(c.status, StatusCode::CREATED, "{:?}", c.json);
    let ct = id(&c);
    assert!(c.json["key"].as_str().unwrap_or("").starts_with("CT-"));
    // The PM is the owner: the PM step is passed; Finance (the admin) decides, then the admin step.
    assert_eq!(c.json["status"], "in_approval");
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/contracts/{ct}/decide"),
        Some(&admin),
        Some(json!({ "action": "approve" })),
    )
    .await;
    assert_eq!(r.json["status"], "in_approval", "{:?}", r.json);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/contracts/{ct}/decide"),
        Some(&admin),
        Some(json!({ "action": "approve" })),
    )
    .await;
    assert_eq!(r.json["status"], "approved", "{:?}", r.json);
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/contracts/{ct}/status"),
            Some(&pm),
            Some(json!({ "status": "signed" }))
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/contracts/{ct}/status"),
        Some(&pm),
        Some(json!({ "status": "signed", "signed_on": "2026-11-05" })),
    )
    .await;
    assert_eq!(r.json["status"], "signed", "{:?}", r.json);

    assert_eq!(
        doc(
            &app,
            &pm,
            &format!("contracts/{ct}"),
            "signed copy",
            "MES.pdf"
        )
        .await
        .status,
        StatusCode::CREATED
    );
    let m = call(&app, "POST", &format!("/api/v1/contracts/{ct}/milestones"), Some(&pm), Some(json!({ "phase_id": phase, "title": "上线验收款", "due_on": "2026-12-31", "amount_minor": 24_000_000 }))).await;
    assert_eq!(m.status, StatusCode::CREATED, "{:?}", m.json);
    let r = call(
        &app,
        "POST",
        &format!("/api/v1/milestones/{}/status", id(&m)),
        Some(&pm),
        Some(json!({ "status": "settled", "settled_on": "2026-12-28" })),
    )
    .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let g = call(
        &app,
        "GET",
        &format!("/api/v1/contracts/{ct}"),
        Some(&dev),
        None,
    )
    .await;
    assert_eq!(
        g.status,
        StatusCode::OK,
        "project members see the project's contracts"
    );
    assert_eq!(g.json["contract"]["settled_minor"], 24_000_000);
    assert_eq!(g.json["milestones"][0]["phase_title"], "上线");
    assert_eq!(g.json["documents"].as_array().map(Vec::len), Some(1));
    // The developer may read but not change it.
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/v1/contracts/{ct}/status"),
            Some(&dev),
            Some(json!({ "status": "completed" }))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    // Someone outside the project does not see it until made a contract viewer.
    let l = call(&app, "GET", "/api/v1/contracts", Some(&viewer), None).await;
    assert_eq!(l.json["contracts"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("/api/v1/contracts/{ct}"),
            Some(&viewer),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/members/{viewer_id}/contract-viewer"),
            Some(&pm),
            Some(json!({ "on": true }))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/api/v1/members/{viewer_id}/contract-viewer"),
            Some(&admin),
            Some(json!({ "on": true }))
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    let l = call(&app, "GET", "/api/v1/contracts", Some(&viewer), None).await;
    assert_eq!(l.json["contracts"].as_array().map(Vec::len), Some(1));
    assert_eq!(l.json["sees_all"], true);

    let pf = call(
        &app,
        "GET",
        &format!("/api/v1/projects/{pid}/file"),
        Some(&pm),
        None,
    )
    .await;
    let tl = pf.json["timeline"].to_string();
    assert!(
        tl.contains("signed") && tl.contains("milestone_settled"),
        "{tl}"
    );
    assert_eq!(pf.json["contracts"].as_array().map(Vec::len), Some(1));
}
