# Company Management Framework — v1 Plan

Status: **draft for review, rev 2** (2026-11-04). No code yet. Ricky's answers to the open questions are recorded in §10 and folded into the sections below; decisions taken so far are in [DECISIONS.md](DECISIONS.md); progress is in [PROGRESS.md](PROGRESS.md).

Planned with the `lucky-rust-fullstack` method: real context first, smallest complete vertical slice, core rules separated from platform adapters, evidence-graded verification, and UI mockups approved before UI code.

---

## 1. Goal and acceptance chain

Run the whole company the way a project manager runs a Gantt chart. Every project task and every routine daily task is one kind of thing (a **work item**) on one timeline. People log what they produced against it, and the system turns that into "who is on what", "where did each person's time go", and "what has this project cost".

The chain that proves v1 works, end to end:

1. A supervisor creates a project with a budget and a Gantt plan (phases, tasks, dependencies), and a routine "daily stand-up report" template assigned to the role *Backend Engineer* in the *R&D* department.
2. Each engineer holding that role gets today's routine item automatically, with the required deliverables listed (e.g. "one text summary + at least one commit link").
3. An engineer logs 3 h against a project task with a description, a screenshot and a voice note; a pushed Git commit referencing the task key appears on the same item automatically.
4. The engineer reassigns a blocked task to a colleague in another department with a reason. The colleague accepts, logs time, and hands it back. The full chain is visible on the item and on the Gantt bar.
5. Finance has set the engineer's rate at ¥200/h (the project budget is in USD, converted at the exchange rate finance entered) and a project threshold of "warn at 80 % of labour budget, critical above ¥50,000". When logged cost crosses 80 %, finance and the project manager get an alert exactly once.
6. The portfolio view shows all parallel projects, each person's allocation across them for the week, and an aggregate time and cost breakdown per person and per project, exportable to CSV/XLSX.

### Measurable targets (v1)

| Target | Value |
| --- | --- |
| Tenant size | 50–100 users per company (design ceiling 500) |
| Platform scale for v1 | 200 companies on one deployment, 20 M work-log rows total |
| Tenant isolation | zero cross-tenant reads, proven by automated tests on every API route |
| Gantt view load (300 items) | < 1.0 s p95 on LAN |
| Portfolio/resource view (200 people × 4 weeks) | < 1.5 s p95 |
| Logging a work entry on mobile web | ≤ 3 taps + text, attachments upload in background |
| Threshold alert latency after the crossing entry is saved | < 60 s |
| Cost accuracy | integer cents, rate frozen per entry, reconciles to the cent with finance export |

---

## 2. Product shape

This is a **multi-company SaaS** (multi-tenant, server-backed), not an offline desktop tool. Companies sign up, invite their people and manage their own data; we run one platform for all of them. Deliver it as:

- **One Rust server binary** that serves the API and the embedded web app (`rust-embed`), plus PostgreSQL and S3-compatible object storage, deployed as containers (staging + production). The same image can still be installed privately for a customer later, but that is not a v1 goal.
- **Tenancy:** every company is a `tenant`. All business tables carry `tenant_id`, enforced twice: in the service layer (every query is scoped by the session's tenant) and in PostgreSQL row-level security (`SET app.tenant_id` per transaction), so a missed `WHERE` cannot leak data. Object storage keys are prefixed by tenant.
- **Platform layer:** self-service sign-up with email verification, company creation, invitations, a platform admin console (tenants, usage, suspend), plan limits (users, storage). Paid billing is a later phase; v1 records plan and usage so billing can be added without migration.
- **Responsive web app** (Vue 3 + TypeScript) that works on desktop browsers for planning and on phones for logging work (camera, microphone, file picker).
- **Optional later:** a Tauri 2 desktop/tray shell for quick time logging and notifications. If shipped, it ships with signed forced-upgrade from its first release (skill appendix E). Not in v1 scope.

Interface languages: Simplified Chinese, Traditional Chinese, English, Japanese (series baseline). Colours: the 鉴色 Chinese traditional colour tokens (default accent 苍筤), light and dark.

---

## 3. Architecture

```
companymangement/
├─ Cargo.toml                 # workspace
├─ crates/
│  ├─ cm-domain/              # pure rules, no I/O: work-item state machine, reassignment rules,
│  │                          # recurrence expansion, cost & threshold evaluation, rollups
│  ├─ cm-store/               # sqlx + PostgreSQL repositories, migrations, outbox
│  ├─ cm-storage/             # object storage adapter (object_store crate: S3/MinIO/local FS)
│  ├─ cm-integrations/        # Git: GitHub App + self-hosted Git webhooks (Gitea/GitLab/generic),
│  │                          # transactional email (SMTP / provider API)
│  ├─ cm-notify/              # built-in notification centre: inbox, realtime push (SSE),
│  │                          # browser Web Push, email digests
│  ├─ cm-jobs/                # background workers: routine generation, alert evaluation,
│  │                          # media processing, digests (Postgres SKIP LOCKED queue)
│  └─ cm-server/              # axum HTTP API, auth, RBAC, OpenAPI (utoipa), static web embed
├─ web/                       # Vue 3 + Vite + TS + Pinia; Gantt + resource timeline
├─ deploy/                    # docker-compose, systemd unit, backup scripts
└─ docs/
```

Key choices (see DECISIONS.md for reasoning):

- **Rust:** tokio, axum, tower-http, sqlx (compile-time checked queries), serde, utoipa → generated TypeScript client so DTOs never drift, argon2 for passwords, `tracing` for logs.
- **Gantt (open source):** Frappe Gantt (MIT) for the project timeline, extended by us for dependencies/drag; vis-timeline (MIT/Apache) for the people × time resource view. Wrapped in our own Vue components so either can be swapped.
- **Money and time:** money as `i64` minor units (cents) plus ISO currency code; durations as integer minutes; percentages as `rust_decimal`. No floats anywhere in cost paths.
- **Domain core is testable without a database.** Handlers parse and authorise; services run transactions; `cm-domain` decides.
- **Event log per work item** (append-only) is the source for history, the reassignment chain, audit, and notifications. Current state lives in normal tables for fast queries.
- **Transactional outbox:** saving a work log and enqueueing "re-evaluate project cost" happen in one transaction, so an alert can never be missed or double-fired.

---

## 4. Data model

All business tables have `id UUID (v7)`, `tenant_id` (row-level security), `created_at`, `updated_at`; soft delete where history matters. Simplified:

### 4.1 Organisation and people

| Table | Purpose / key fields |
| --- | --- |
| `tenant` | A customer company. `name`, `slug`, `plan`, `status` (trial/active/suspended), `default_currency`, `timezone`, `work_week`, `locale` |
| `platform_admin` | Our own operators; outside any tenant |
| `department` | Tree. `parent_id`, `name`, `head_user_id` |
| `role` | Job role, e.g. *Backend Engineer*, *Finance*. `department_id` (nullable = company-wide), `name` |
| `user` | Global identity: `email` (unique, verified), `password_hash`, `totp_secret?`, `locale` |
| `membership` | User ↔ tenant with `status` (invited/active/deactivated), `timezone`; one person can belong to several companies and switch between them |
| `auth_token` | Email verification, password reset and invitation tokens (hashed, single use, expiring) |
| `user_role` | Who holds which role in which department, with `valid_from/valid_to` so role history is kept |
| `permission_grant` | RBAC: system roles (Admin, Finance, Supervisor, PM, Member) mapped to permissions, scoped org/department/project |
| `rate_card` | **Hourly rate history.** `user_id`, `rate_minor`, `currency` (the person's pay currency), `effective_from`, `effective_to`, `set_by`, `note`. Optional overtime/weekend multipliers. Visible only to Finance/Admin |

### 4.2 Work

| Table | Purpose / key fields |
| --- | --- |
| `project` | `key` (e.g. `PAY`), `name`, `owner_id` (PM), `status`, `planned_start/end`, `labour_budget_minor`, `currency` (budget currency, may differ from people's rate currencies), `department_id`, `handoff_policy` |
| `work_item` | **The single core unit.** `kind` = `phase` / `task` / `milestone` / `routine` / `request`; `project_id` (nullable for routine/request not tied to a project); `parent_id` (WBS tree); `key` (e.g. `PAY-142`); `title`, `description`; `planned_start/end`, `estimate_minutes`; `actual_start/end`; `status` (`todo/in_progress/blocked/in_review/done/cancelled`); `holder_user_id` (current responsible person); `target_role_id` (for role-pool items); `priority`; `progress_pct` |
| `dependency` | Gantt links: `from_item`, `to_item`, `type` (FS/SS/FF/SF), `lag_minutes` |
| `routine_template` | Recurring daily/weekly work. `title`, `recurrence` (RFC 5545 RRULE), `target` = user **or** role-in-department, `mode` = `each_member` (one item per holder) or `pool` (one item, first to claim), `due_offset`, `default_estimate`, `project_id` (optional, to charge cost), `created_by` (supervisor) |
| `deliverable_spec` | Required outputs defined by a supervisor, attached to a template or a work item. `type` (text / image / audio / video / file / git_commit / link / checklist), `min_count`, `required`, `instructions` |
| `handoff` | One reassignment request: `from_user`, `to_user`, `reason`, `needs_approval`, `approver_id`, `state` (pending_approval / pending_accept / accepted / declined / rejected / cancelled) |
| `work_item_event` | Append-only: created, status_changed, reassigned, accepted, declined, returned, comment, deliverable_submitted, rescheduled. `actor_id`, `payload JSONB` |

### 4.3 Output logging and time

| Table | Purpose / key fields |
| --- | --- |
| `work_log` | One entry of work. `user_id`, `work_item_id`, `started_at`, `minutes`, `description` (Markdown), `billable`, **frozen cost**: `rate_card_id`, `rate_minor`, `cost_minor`, `currency` (rate currency) **and** `fx_rate_id`, `project_cost_minor` (converted to the project currency at entry time); `source` (manual / timer / git / import); `locked` once a period is closed |
| `attachment` | `work_log_id` or `work_item_id`, `kind` (image/audio/video/file), `object_key`, `mime`, `size`, `sha256`, `duration_s`, `thumbnail_key`, `uploaded_by`. Upload goes straight to object storage via pre-signed URL; server verifies size/type and generates thumbnails/transcodes in a job |
| `deliverable_submission` | Links a log/attachment/commit to a `deliverable_spec`, so "required deliverables met?" is computable |
| `git_repository` | Connected repo: `provider` (github / gitea / gitlab / generic), URL, credentials or GitHub App installation id, webhook secret, project mapping |
| `git_commit` | `repo_id`, `sha`, `author_email` → mapped `user_id`, `message`, `url`, `committed_at`, `additions/deletions`; linked to work items by key in message/branch (`PAY-142`) or manually |
| `timesheet_period` | Weekly/monthly period with `status` open/submitted/approved/closed; closing locks entries |

### 4.4 Finance and alerts

| Table | Purpose / key fields |
| --- | --- |
| `fx_rate` | Exchange rates per tenant: `from_ccy`, `to_ccy`, `rate` (decimal), `effective_date`, `source` (finance-entered, or optional daily reference feed later), `set_by` |
| `cost_threshold` | `project_id`, `metric` = `labour_cost` or `labour_cost_forecast`, `mode` = `absolute` (amount) or `percent_of_budget`, `value`, `level` (info/warn/critical), `recipients` (users/roles), `channels`, `created_by` (Finance) |
| `alert` | `threshold_id`, `project_id`, `fired_at`, `observed_value`, `level`, `state` (open/acknowledged/resolved), `ack_by`, `note` |
| `project_cost_snapshot` | Materialised daily totals per project × user × day (minutes, cost) for fast dashboards and trend charts |
| `notification` | Built-in inbox rows (`type`, `subject`, `link`, `read_at`), plus delivery attempts for realtime/Web Push/email |
| `notification_pref` | Per user and event type: in-app always on; Web Push and email on/off; quiet hours |
| `audit_log` | Who changed rates, thresholds, permissions, closed periods |

---

## 5. Work-item lifecycle and the flexible ("free-flow") workflow

### 5.1 States

`todo → in_progress → (blocked ↔ in_progress) → in_review → done`, plus `cancelled` from any open state. Routine items with required deliverables cannot reach `done` until all required specs have submissions (the supervisor can override with a reason, logged).

### 5.2 Reassignment (free-flow)

The rule is deliberately permissive: **any open work item can be passed from its current holder to any active user in the company.** What makes it safe is that every hop is recorded and visible, not that it is restricted.

- **Who can reassign:** the current holder, the project PM, the holder's supervisor (department head), or an Admin.
- **Same department: free.** The hand-off goes straight to the target for acceptance.
- **Cross-department: approval first (Ricky's choice).** When the target is in a different department from the sender, the hand-off waits for approval from the **target's department head** (they own that person's time). If the item belongs to a project, the project PM can also approve. Approver sees the item, reason and current workload of the target. Reject returns it to the sender with a reason. Department heads can name a deputy approver; approvals waiting > 24 h remind the approver and can escalate to Admin. Flow: `pending_approval → pending_accept → accepted`.
- **What a hand-off carries:** target user, reason (required, short), optional note and attachments, optional new due date. Deliverable specs and history stay on the item.
- **Accept / decline:** the target sees it in their inbox; the item shows `pending_handoff` until accepted. Decline returns it to the sender with a reason. Auto-accept after N hours is an org setting (default off).
- **Return to sender:** one click sends it back up the chain.
- **Time and cost stay with whoever did the work.** Each person's `work_log` rows remain theirs; the project cost is the sum of everyone's frozen-rate entries. Reassignment never moves logged hours.
- **Requests:** anyone can create a `request` item addressed to anyone, optionally linked to a project. It then behaves like any other work item (can be passed on, logged against, shown on the Gantt if it has dates). This is how ad-hoc "can you look at this?" work enters the system instead of chat.
- **Visibility:** the item detail shows the chain (A → B → C → back to B) with timestamps and reasons. The Gantt bar shows the current holder's avatar and a hand-off count. Reports show "items passed > 3 times" and "average time waiting for acceptance" so managers can spot ping-pong.
- **Guards in `cm-domain` (not just the UI):** a cross-department hand-off cannot skip approval (checked server-side from `user_role` at request time); cannot hand off a closed item; cannot hand off to a deactivated user; concurrent hand-offs resolved by optimistic version check (`work_item.version`), the loser gets a conflict error.

### 5.3 Role-based routine tasks

- A supervisor creates a `routine_template` targeting a person, or a role within a department (e.g. *QA* in *R&D*), with a recurrence and deliverable specs.
- A job runs every 15 minutes, expands each RRULE in the org's timezone for the next day, and creates instances idempotently (unique key `template_id + occurrence_date + user_id`). Holidays/leave (simple calendar table) skip generation.
- `each_member` creates one item per current role holder; `pool` creates one item with `target_role_id` that any holder can claim.
- When someone joins or leaves a role, future instances follow automatically because they are generated from `user_role` at expansion time; already-created items are not touched.
- Supervisors get a daily view: which routine items are missing deliverables, by department and role.

---

## 6. Time and cost tracking

- **Ways to log:** start/stop timer on an item; manual entry (date + duration); quick log from mobile; Git commit linked to an item (creates a zero-minute evidence entry, or prompts for time — configurable); CSV import.
- **Each `work_log` is the output record**: description + attachments (image, audio, video, any file) + linked commits. One entry can satisfy one or more deliverable specs.
- **Rate freezing:** on save, look up the user's `rate_card` effective at `started_at`, then store `rate_minor` and `cost_minor = round_half_even(minutes × rate / 60)`.
- **Multi-currency:** people are paid in their own currency; each project has a budget currency. On save, the entry is also converted to the project currency using the tenant's `fx_rate` effective on the entry date, and both amounts plus the rate used are frozen. Project totals, budgets and thresholds always use the project-currency amount. If no rate exists for that pair and date, the entry is saved with cost marked *pending FX* and Finance gets a task to enter the rate; pending entries are then converted and re-evaluated. Reports can show any total in the tenant's reporting currency, labelled with the rate date. Changing a rate later does not silently rewrite history. If Finance backdates a rate, the system shows the entries affected and asks for an explicit "recalculate open periods" action, which is audited. Closed periods never change.
- **Who sees money (Ricky's choice: PMs see hours only):** Finance and tenant Admins see rates and costs. PMs and supervisors see **hours only**: per person, per project, per item, plus the budget *percentage used* so they know when to worry, but no amounts. Members see their own hours. The API never sends money fields to non-finance roles (DTO-level, covered by tests).
- **Views:**
  - Project Gantt: bars from planned dates, overlaid with actual logged time; progress from logged vs estimated hours or manual %.
  - Portfolio: all projects on one timeline, health (schedule, cost vs budget), current open alerts.
  - Resource view: people (grouped by department/role) × days/weeks, stacked by project, with over-allocation highlighted. This is the "who is working on what" screen.
  - Person breakdown: for any range, hours and cost by project and by item kind (project vs routine vs request).
  - Exports: CSV and XLSX for timesheets and finance cost reports.

---

## 7. Finance thresholds and alerts

- Finance defines thresholds per project (and defaults per project type): **absolute** ("labour cost > ¥50,000") or **percent of budget** ("> 80 % of labour budget"), with a level and recipients. Multiple thresholds per project (e.g. 50 % info, 80 % warn, 100 % critical).
- **Evaluation:** every saved/edited/deleted work log enqueues the project in the same transaction (outbox). A worker recomputes the project total and compares it with each threshold. A nightly full sweep catches anything missed (rate recalculation, imports).
- **Fire once per crossing:** an alert opens when the value goes from below to above a threshold. It does not repeat while it stays above. If cost drops back below (entries deleted/corrected), the alert auto-resolves; a later re-crossing opens a new alert.
- **Channels (built into this system):** notification centre in the app (bell + inbox, realtime via SSE), a banner on the project, browser Web Push when the tab is closed, and email as fallback/digest. No third-party chat integration in v1. Each delivery is retried with backoff and logged.
- **Acknowledge:** recipients acknowledge with a note; the project shows open alerts until resolved.
- **Forecast threshold (phase 3):** projected cost at completion = actual cost + remaining estimate × blended rate; alert when the forecast crosses a threshold, so Finance hears early, not after the money is spent.
- **Edge rules (unit tested in `cm-domain`):** budget zero or missing makes percent thresholds inactive (shown as a warning in the UI); absolute thresholds are always in the project currency; entries with pending FX are excluded and flagged until converted; thresholds are evaluated on frozen costs only.

---

## 8. Security and privacy

- Auth: email + password (argon2id) with verified email, password reset by email link, optional TOTP, login rate limiting and lockout. HTTP-only secure session cookies, CSRF token on writes. SSO is out of v1 scope.
- Tenant isolation: RLS in PostgreSQL plus service-layer scoping; an automated test suite creates two tenants and asserts every route refuses cross-tenant ids. Tenant data export and deletion on request.
- Authorisation is checked in the service layer on every request, scoped by org → department → project; hidden buttons are not security.
- Rates and costs are the most sensitive data: separate permission, excluded from non-finance API responses at the DTO level, every read of a rate report audited.
- Attachments: pre-signed upload with size/type limits, content sniffing, virus scan hook (ClamAV optional), images stripped of EXIF location by default.
- Git: GitHub via a GitHub App (per-tenant installation, least privileges: metadata + read commits). Self-hosted Git (Gitea / GitLab / any server) via webhooks with a per-repo HMAC secret; if the self-hosted server cannot reach our SaaS, an optional polling mode reads commits with a read-only token. Webhooks are verified; commit author email mapping is explicit (admin-confirmed), never trusted blindly.
- Backups: nightly `pg_dump` + object storage sync, with a documented restore drill.
- Dependency checks in CI: `cargo deny`, `cargo audit`, `pnpm audit`.

---

## 9. Phased roadmap

Each phase ends with CI green, a PROGRESS.md entry, a demo on seeded data, and screenshots in `docs/screenshots/` for UI changes. UI work in every phase starts with a mockup that Ricky approves before UI code is written.

| Phase | Scope | Checkable result |
| --- | --- | --- |
| **0. Foundation** (≈1.5 wks) | Workspace, CI (fmt, clippy, tests, web build), Docker Compose (Postgres + MinIO + mail catcher), migrations with RLS, tenant sign-up, email verification, password reset, invitations, department/role/member admin, i18n skeleton, design tokens, cross-tenant test harness. **UI mockups** of Gantt, resource view, log entry, hand-off approval, finance thresholds | A new company can sign up, invite people and build its org tree; isolation tests pass; mockups approved |
| **1. Core vertical slice** (≈3 wks) | Projects, work items (WBS), dependencies, Gantt view (drag to reschedule), work logs with attachments, timer, item detail with event history | The acceptance chain steps 1 (project part) and 3 (without Git) work end to end |
| **2. Routine work and free-flow** (≈2 wks) | Routine templates (person/role, each/pool), deliverable specs and submissions, generation job, requests, reassignment with accept/decline/return and cross-department approval, built-in notification centre (SSE + Web Push + email) | Steps 1, 2 and 4 of the acceptance chain |
| **3. Cost and finance** (≈2.5 wks) | Rate cards with history, frozen cost, exchange rates and multi-currency conversion, timesheet periods and closing, cost dashboards (finance) and hours dashboards (PMs), thresholds (absolute/percent), alert engine with outbox, CSV/XLSX export, forecast threshold | Step 5, cost reconciles to the cent with export |
| **4. Overview and integrations** (≈2 wks) | Portfolio view, resource/allocation view, person breakdown reports, GitHub App and self-hosted Git webhooks/polling with commit linking | Steps 3 (with Git) and 6; full acceptance chain passes |
| **5. Hardening and launch** (≈1.5 wks) | Load test to §1 targets, security review incl. tenant isolation, backup/restore drill, platform admin console, plan limits, monitoring and alerting for the platform, terms/privacy pages, staging → production deploy | v1.0.0 live for the first pilot companies |
| **Later** | Paid billing and subscriptions, SSO, IM integrations, Tauri tray app for quick logging (with forced upgrade), native mobile wrapper, AI features: auto-summarise daily logs into a weekly report, suggest time entries from commits, flag stalled hand-off chains | Separate plan |

Rough total for v1: 11–12 weeks of focused work for one full-stack developer with AI assistance; phases 2 and 3 can run in parallel with two people.

---

## 10. Answers from Ricky (2026-11-04)

| # | Question | Answer | Where it changed the plan |
| --- | --- | --- | --- |
| 1 | Deployment | Multi-company SaaS | §2 tenancy and platform layer, §4 `tenant`/`membership`, §8 isolation, phases 0 and 5 |
| 2 | Size and languages | 50–100 people per company; Chinese-first with English/Japanese is fine | §1 targets |
| 3 | Login | Email/password | §8 auth, phase 0 |
| 4 | Git hosting | GitHub and self-hosted Git servers | §4 `git_repository`, §8 Git, phase 4 |
| 5 | Notification channel | Built into this system | §3 `cm-notify`, §4 notifications, §7 channels |
| 6 | Reassignment control | Approval for cross-department hand-offs | §4 `handoff`, §5.2 |
| 7 | Cost visibility | PMs see hours only | §6 who sees money |
| 8 | Currency | Multi-currency projects | §4 `fx_rate`, §6 multi-currency, §7 |
| 9 | Gantt library | Open source | §3 Frappe Gantt + vis-timeline |

### Still open (my default stands unless you say otherwise)

- **Who approves cross-department hand-offs:** the target's department head, with the project PM also allowed for project items. Alternative: the sender's head.
- **Exchange rates:** entered by Finance per tenant (default), with an optional daily reference feed later.
- **Product name and domain** for the SaaS, and whether the first pilot companies are in mainland China (affects hosting region, ICP filing and email deliverability).

## 11. Risks

| Risk | Mitigation |
| --- | --- |
| People do not log consistently, so the data is thin | Mobile-first logging, timer, Git auto-evidence, routine items with clear deliverables, supervisor "missing logs" view |
| Gantt UI complexity eats the schedule | Mockups first; start with read + drag-reschedule only; defer critical-path and baseline comparison |
| Rate data leaks | Separate permission, DTO-level exclusion, audit of reads, tests asserting non-finance responses contain no rate fields |
| Free-flow becomes ping-pong | Visible chain, ping-pong report, optional approval mode |
| Cross-tenant data leak | RLS plus service scoping, isolation test suite on every route, tenant-prefixed storage keys, security review before launch |
| Missing exchange rates block cost reports | *Pending FX* state, Finance task, nightly reminder; thresholds flag excluded entries |
| Large media uploads strain the server | Direct-to-object-storage uploads, size caps, async transcoding |
