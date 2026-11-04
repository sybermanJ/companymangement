# companymangement

A Gantt-based management framework for whole companies, delivered as a multi-company SaaS: projects and routine daily tasks on one timeline, output logging with attachments and Git commits, role-based routine assignment, free-flow reassignment with cross-department approval, hourly-rate costing in multiple currencies, and finance threshold alerts.

Status: **v1 feature-complete (phases 0–5)**, single executable available. Working name: Hengtu (衡图), placeholder until the product name is chosen.

- Plan: [docs/v1/PLAN.md](docs/v1/PLAN.md) · Decisions: [docs/v1/DECISIONS.md](docs/v1/DECISIONS.md) · Progress: [docs/v1/PROGRESS.md](docs/v1/PROGRESS.md)
- Early screen mockups: [docs/v1/mockups/](docs/v1/mockups/)

## Layout

| Path | What |
| --- | --- |
| `crates/cm-domain` | Pure rules: money and labour cost, currency conversion, cost thresholds, hand-offs, work-item lifecycle, deliverables |
| `crates/cm-store` | PostgreSQL with per-company row-level security; migrations |
| `crates/cm-server` | axum HTTP API: accounts, organisation, projects and Gantt items, routine work, hand-offs, inbox, finance, reports, Git webhooks; serves the web app (embedded with `--features bundle`) |
| `web/` | Vue 3 + TypeScript web app (zh-CN, zh-TW, en, ja; light and dark) |
| `deploy/` | Docker Compose dev stack (Postgres, MinIO, Mailpit, server) and Dockerfile |

## Run it (one file)

```sh
scripts/build-bundle.sh            # → dist/hengtu-linux-x86_64 (static, ~26 MB, PostgreSQL inside)
./hengtu-linux-x86_64              # open http://localhost:8080 and register: the first account creates the company
./hengtu-linux-x86_64 --help       # settings: BIND, APP_BASE_URL, CM_DATA_DIR, SMTP_URL, DATABASE_URL …
```

Data lives in `./hengtu-data` (or `/var/lib/hengtu` when run as root). Back up that folder while the program is stopped. Without `SMTP_URL`, invitation and password links are written to the program's log instead of emailed.

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
| ![Gantt](docs/screenshots/gantt-light.png) Project Gantt with dependencies and milestones | ![Item panel](docs/screenshots/gantt-item-panel-light.png) Item panel: log work, attachments, history |
| ![Week scale, dark](docs/screenshots/gantt-week-dark.png) Week scale, dark theme | ![Projects](docs/screenshots/projects-light.png) Project list with hours and progress |
| ![My work, phone](docs/screenshots/my-work-phone.png) My work on a phone | ![Login](docs/screenshots/login-light.png) Login |
| ![Members](docs/screenshots/org-members-light.png) Members and invitations | ![Roles, dark](docs/screenshots/org-roles-dark.png) Roles and assignments, dark theme |
| ![Departments](docs/screenshots/org-departments-light.png) Department tree with heads | ![English](docs/screenshots/org-members-en-dark.png) English, dark theme |
| ![Phone, Japanese](docs/screenshots/org-departments-phone-ja.png) Phone width, Japanese | ![Sign-up, phone](docs/screenshots/signup-phone.png) Company sign-up on a phone |
| ![Overview](docs/screenshots/overview-light.png) Company overview: timeline and health | ![Team](docs/screenshots/team-light.png) Who works on what, per week, with one person's breakdown |
| ![Routines](docs/screenshots/routines-light.png) Routine work for roles and people with required deliverables | ![Inbox](docs/screenshots/inbox-light.png) Inbox: cross-department hand-off approval and alerts |
| ![Finance](docs/screenshots/finance-costs-light.png) Project cost, budget bar, thresholds and alerts | ![Rates, dark](docs/screenshots/finance-rates-dark.png) Hourly rates in several currencies |
| ![Commits](docs/screenshots/item-commits-light.png) Item with linked Git commits and history | ![Git](docs/screenshots/org-git-light.png) Git repositories and author mapping |

Save progress during long work: `scripts/checkpoint.sh "<phase>" "<done>" "<next>"`.

© M&J Group · 473276@qq.com
