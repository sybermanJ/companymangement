# Decisions (append-only)

Add a new entry when a decision changes; do not edit old entries.

## D1 — Server product, self-hostable single binary (2026-11-04, proposed)
**Decision:** Rust (axum) server that embeds the Vue web app, with PostgreSQL and S3-compatible storage, shipped as Docker Compose plus a plain binary.
**Why:** multi-user, shared data, media uploads and background alerts need a server; a single binary keeps on-prem install simple.
**Rejected:** desktop-only Tauri app (no shared data); SQLite (weak for concurrent multi-user writes at 1,000 users).

## D2 — One `work_item` table for project tasks, routine tasks and requests (2026-11-04, proposed)
**Why:** the requirement is that projects and daily tasks are the same core unit on one timeline; one table makes Gantt, resource view, reassignment and cost rollups uniform.
**Rejected:** separate task/routine/request tables (duplicated logic for every view and rule).

## D3 — Cost frozen per work log at save time (2026-11-04, proposed)
**Why:** finance reports must not change when a rate changes later; backdated changes are explicit, audited recalculations of open periods only.

## D4 — Money as integer minor units, durations as integer minutes (2026-11-04, proposed)
**Why:** exact reconciliation with finance exports; no float rounding drift.

## D5 — Reassignment is free by default, recorded as events, time stays with whoever logged it (2026-11-04, proposed)
**Why:** matches the "free-flow" requirement; safety comes from visibility (chain, reports) rather than restriction. Optional approval mode per project.

## D6 — Threshold alerts via transactional outbox, fire once per crossing (2026-11-04, proposed)
**Why:** no missed and no duplicate alerts; nightly sweep as a safety net.

## D7 — UI mockups approved before UI code; 鉴色 colour tokens (2026-11-04, adopted from method)
**Why:** series standard; avoids rebuilding screens.

## D8 — Multi-company SaaS with row-level security (2026-11-04, Ricky)
**Decision:** every company is a tenant; `tenant_id` on all business tables, enforced by service-layer scoping and PostgreSQL RLS. Users are global identities with memberships in one or more tenants.
**Replaces:** the self-hosted single-company assumption in D1 (the binary stays deployable privately, but SaaS is the v1 target).

## D9 — Email/password login only in v1 (2026-11-04, Ricky)
With email verification, reset links, optional TOTP and rate limiting. SSO deferred.

## D10 — Git: GitHub App plus self-hosted Git webhooks, polling fallback (2026-11-04, Ricky)

## D11 — Notifications built in: inbox + SSE + Web Push + email; no IM adapters in v1 (2026-11-04, Ricky)

## D12 — Cross-department hand-offs need approval (2026-11-04, Ricky)
Approver defaults to the target's department head (project PM may also approve project items). Same-department hand-offs stay free.

## D13 — PMs see hours only; money visible to Finance and tenant Admins (2026-11-04, Ricky)
PMs still see budget percentage used.

## D14 — Multi-currency: rate in pay currency, converted to project currency at entry date, both frozen (2026-11-04, Ricky)
Finance-entered exchange rates per tenant; missing rate marks the entry *pending FX*.

## D15 — Open-source Gantt: Frappe Gantt + vis-timeline behind our own Vue components (2026-11-04, Ricky)

## D16 — Remaining defaults confirmed (2026-11-04, Ricky: "OK, do it")
- Cross-department hand-off approver: the target's department head; the project PM may also approve project items.
- Exchange rates: entered by Finance per tenant.
- Product name and hosting region: not chosen yet. Working name **Hengtu (衡图)** is used in code and mockups only as a placeholder; region is configured at deploy time.

## D17 — Mockups approved (2026-11-04, Ricky: "ok, do it")
The five screens in docs/v1/mockups are the visual baseline for the web app.

## D18 — Account emails via a transactional outbox (2026-11-04)
Verification, reset and invitation emails are written to `outbox_email` in the same transaction as the change and delivered by a worker over SMTP (`SMTP_URL`); without SMTP they are logged. Tokens are single-use and stored only as SHA-256.

## D19 — Gantt is our own Vue component (2026-11-04)
Instead of embedding Frappe Gantt, `web/src/pages/Gantt.vue` draws the chart: a task tree on the left (as in the approved mockup), day/week scales, dependency arrows, drag to move or resize with day snapping, and the same design tokens in light and dark. It is open-source code in this repo (Ricky's answer 9), has no third-party runtime dependency, and keeps item keys, holders and progress labels in one row.

## D20 — Routine recurrence: daily, weekdays, or chosen weekdays (2026-11-04)
Instead of full RFC 5545 RRULEs, a routine template fires daily, Monday–Friday, or on chosen ISO weekdays. This covers daily and weekly routine work, is easy to set in the UI, and keeps generation simple and idempotent (unique per template, day and holder). Generation runs every 15 minutes for today and tomorrow in each company's time zone, and immediately when a template is created.

## D21 — Cost thresholds are evaluated in the same transaction as the change (2026-11-04)
Every saved or deleted work log, rate, exchange rate, budget change, recalculation and new threshold re-evaluates the affected projects inside the same database transaction. That gives the same guarantee the plan's outbox gave (no change without evaluation) without a separate queue. "Fire once per crossing" is also enforced by a unique index on unresolved alerts per threshold.

## D22 — Notifications: inbox, live count over SSE, email for decisions (2026-11-04)
Notifications are rows in the built-in inbox. A browser tab keeps a server-sent-events stream open and gets the unread count pushed through PostgreSQL LISTEN/NOTIFY. Hand-off approvals and offers, and cost alerts, are also emailed through the outbox. Browser Web Push is deferred: it needs VAPID keys and a service worker, and the major push services are unreliable from mainland China.

## D23 — Git through signed webhooks for every provider (2026-11-04)
GitHub, Gitea/Forgejo, GitLab and other servers all connect with a per-repository webhook URL and secret (GitHub/Gitea HMAC-SHA256 signatures, GitLab token). This needs no GitHub App registration and works the same for self-hosted servers. Commits link to items by key in the message or branch name; authors map to people only through Admin-confirmed email mappings. A manual "link commit" action covers servers that cannot reach us. The GitHub App and polling mode are left for later.

## D24 — Exports are CSV (2026-11-04)
Timesheet and cost exports are UTF-8 CSV with a BOM, which Excel and WPS open directly with Chinese text intact. Formula-like cells are neutralised. XLSX is not needed for v1.

## D25 — One self-contained executable for self-hosting (2026-11-04)
`scripts/build-bundle.sh` builds `dist/hengtu-linux-x86_64`: a static (musl) binary with the web app and a PostgreSQL 17.6 server compiled in (binaries from the zonky embedded-postgres project, Apache 2.0, PostgreSQL licence). Without `DATABASE_URL` it unpacks PostgreSQL into its data folder on first start, creates a cluster with a random superuser password, and runs the app as its own non-superuser role (`cm_app`), so row-level security behaves exactly as in the SaaS setup. PostgreSQL listens on 127.0.0.1 only. When started as root it runs PostgreSQL as `nobody` (PostgreSQL refuses root) and keeps data in `/var/lib/hengtu`. Cookies default to non-secure in this mode because a fresh install is usually reached over plain http; set `COOKIE_SECURE=true` behind https. Setting `DATABASE_URL` keeps the external-database behaviour. Linux x86-64 only for now; Windows and macOS would need their own PostgreSQL bundles.

## D26 — One approval engine for requests and contracts (2026-11-05)
Ricky asked for spending routed by project phase: phase manager, then Finance, then back to the staff member. Routes are data (`approval_route`): an ordered list of steps (phase manager, project manager, department head, Finance, Admin, a named person), with amount tiers per kind. At submission the route is resolved to people and frozen on the `approval` row, so later org changes do not move a running approval. Approvers can approve, return (with reason), reject (with reason), pass the step to someone, or approve and add an approver after themselves. Nobody approves their own request: their step is skipped; an empty step falls to the Admins. Returned requests restart the route as a new round.

## D27 — Expenses are project cost once approved (2026-11-05)
Reimbursements, supplier payments and purchases count toward project and phase cost when approved, converted to the project currency at approval and frozen. Trips are plans (their actual costs are filed as child requests) and cash advances are cash flow, so neither counts. New threshold metrics `expense` and `total_cost` (labour + expenses) reuse the alert machinery. Project managers see the amounts of expenses on their projects (they approve them) but still not hourly rates.

## D28 — Contract visibility follows the work (2026-11-05)
Staff see contracts of projects they worked on (held an item or logged time), ones they own, created or helped approve. Finance, Admins and members with the new "contract viewer" flag see all. Files are versioned by label (draft, signed copy, amendment...). Payment milestones can be tied to phases.
