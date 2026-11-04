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
