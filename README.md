# companymangement

A Gantt-based management framework for whole companies, delivered as a multi-company SaaS: projects and routine daily tasks on one timeline, output logging with attachments and Git commits, role-based routine assignment, free-flow reassignment with cross-department approval, hourly-rate costing in multiple currencies, and finance threshold alerts.

Status: **phase 0 (foundation)**. Working name: Hengtu (衡图), placeholder until the product name is chosen.

- Plan: [docs/v1/PLAN.md](docs/v1/PLAN.md) · Decisions: [docs/v1/DECISIONS.md](docs/v1/DECISIONS.md) · Progress: [docs/v1/PROGRESS.md](docs/v1/PROGRESS.md)
- Screen mockups (pending approval): [docs/v1/mockups/](docs/v1/mockups/)

## Layout

| Path | What |
| --- | --- |
| `crates/cm-domain` | Pure rules: money and labour cost, currency conversion, cost thresholds, hand-offs, work-item lifecycle, deliverables |
| `crates/cm-store` | PostgreSQL with per-company row-level security; migrations |
| `crates/cm-server` | axum HTTP API: sign-up, login, email verification, password reset, invitations, members, roles, departments; serves the web app |
| `web/` | Vue 3 + TypeScript web app (zh-CN, zh-TW, en, ja; light and dark) |
| `deploy/` | Docker Compose dev stack (Postgres, MinIO, Mailpit, server) and Dockerfile |

## Develop

```sh
docker compose -f deploy/docker-compose.yml up -d postgres
export DATABASE_URL=postgres://cm_app:cm_app_dev_only@localhost/cm   # never a superuser: it bypasses RLS
cargo test --workspace          # DB tests are skipped when DATABASE_URL is unset
(cd web && pnpm install && pnpm build)   # or `pnpm dev` on :5173 with /api proxied to :8080
cargo run -p cm-server          # serves API and web/dist on :8080; SMTP_URL=smtp://localhost:1025 to send via Mailpit
node web/scripts/screenshots.mjs   # seeds a demo company and refreshes docs/screenshots
```

## Screenshots

| | |
| --- | --- |
| ![Members](docs/screenshots/org-members-light.png) Members and invitations | ![Roles, dark](docs/screenshots/org-roles-dark.png) Roles and assignments, dark theme |
| ![Departments](docs/screenshots/org-departments-light.png) Department tree with heads | ![English](docs/screenshots/org-members-en-dark.png) English, dark theme |
| ![Phone, Japanese](docs/screenshots/org-departments-phone-ja.png) Phone width, Japanese | ![Sign-up, phone](docs/screenshots/signup-phone.png) Company sign-up on a phone |

Save progress during long work: `scripts/checkpoint.sh "<phase>" "<done>" "<next>"`.

© M&J Group · 473276@qq.com
