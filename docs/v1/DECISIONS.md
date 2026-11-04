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
