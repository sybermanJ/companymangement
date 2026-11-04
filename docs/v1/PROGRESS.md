# Progress (append-only)

| Time (UTC) | Phase | Done | Next |
| --- | --- | --- | --- |
| 2026-11-04 13:30 | Plan | v1 plan, decisions and checkpoint script committed | Ricky reviews PLAN.md §10 open questions; then phase 0 (workspace, CI, auth, org admin, UI mockups) |
| 2026-11-04 13:40 | Plan | Rev 2: folded in answers on SaaS, login, Git, notifications, approval, cost visibility, multi-currency, Gantt | Confirm the three remaining defaults in PLAN §10; then phase 0 |
| 2026-11-04 13:57 | Phase 0 | Mockups, cm-domain rules (24 tests), RLS tenancy, signup/login/departments API, CI and Docker Compose | Ricky reviews mockups; then email verification, password reset, invitations, roles admin, web app shell |
| 2026-11-04 14:10 | Phase 0 | Email verification, password reset, login rate limiting, invitations, members, roles, department heads (API + tests) | Web app shell: Vue 3 sign-up/login/org admin with i18n and 鉴色 tokens |
| 2026-11-04 14:15 | Phase 0 | Web app: sign-up, login, verify, reset, accept invite, org admin (departments, members, roles) in 4 languages, light/dark; screenshots; CI web job | Phase 0 done pending CI. Phase 1: projects, work items, Gantt view, work logs with attachments |
| 2026-11-04 14:21 | Phase 0 | Fix rustfmt in cm-server main.rs (CI fmt check) | Phase 1: projects, work items, Gantt view, work logs |
| 2026-11-04 15:23 | Phase 1 | Projects, work items, dependencies (cycle check), history, work logs, attachments (safe download), timer: API + tests | Web: project list, Gantt with drag-to-reschedule, item detail with logging and uploads |
| 2026-11-04 15:30 | Phase 1 | Web: project list, Gantt (tree, day/week, dependency arrows, drag to reschedule), item panel with work logs, uploads, timer, history; My work; D19; screenshots | Phase 2: daily tasks for roles/people with required deliverables, free-flow hand-offs with cross-department approval, Git commit linking |
| 2026-11-04 15:58 | Phases 2-4 API | Routine work for people/roles with deliverables, requests, hand-offs with cross-department approval, inbox + SSE; rates, FX, frozen cost, periods, thresholds/alerts, CSV export; reports; Git webhooks; tests | Web screens for routines, inbox, hand-offs, finance, overview, team, Git; then single executable |
