-- Phase 1: projects, work items (one table for tasks, phases, milestones,
-- routine items and requests), dependencies, per-item event history,
-- work logs (time + output) and attachments.

CREATE TABLE project (
    id                  uuid PRIMARY KEY,
    tenant_id           uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    key                 text NOT NULL CHECK (key ~ '^[A-Z][A-Z0-9]{1,9}$'),
    name                text NOT NULL CHECK (length(trim(name)) > 0),
    owner_id            uuid NOT NULL REFERENCES app_user (id),
    department_id       uuid REFERENCES department (id),
    status              text NOT NULL DEFAULT 'active' CHECK (status IN ('planning', 'active', 'on_hold', 'done', 'cancelled')),
    planned_start       date,
    planned_end         date,
    labour_budget_minor bigint CHECK (labour_budget_minor IS NULL OR labour_budget_minor >= 0),
    currency            char(3) NOT NULL,
    handoff_policy      text NOT NULL DEFAULT 'approval_cross_department',
    next_item_number    int NOT NULL DEFAULT 1,
    created_at          timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, key),
    CHECK (planned_end IS NULL OR planned_start IS NULL OR planned_end >= planned_start)
);

CREATE TABLE work_item (
    id               uuid PRIMARY KEY,
    tenant_id        uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    project_id       uuid REFERENCES project (id) ON DELETE CASCADE,
    parent_id        uuid REFERENCES work_item (id) ON DELETE CASCADE,
    kind             text NOT NULL CHECK (kind IN ('phase', 'task', 'milestone', 'routine', 'request')),
    key              text,
    title            text NOT NULL CHECK (length(trim(title)) > 0),
    description      text NOT NULL DEFAULT '',
    status           text NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'blocked', 'in_review', 'done', 'cancelled')),
    holder_user_id   uuid REFERENCES app_user (id),
    target_role_id   uuid REFERENCES job_role (id),
    planned_start    date,
    planned_end      date,
    estimate_minutes int CHECK (estimate_minutes IS NULL OR estimate_minutes >= 0),
    progress_pct     smallint NOT NULL DEFAULT 0 CHECK (progress_pct BETWEEN 0 AND 100),
    sort_order       int NOT NULL DEFAULT 0,
    version          int NOT NULL DEFAULT 1,
    created_by       uuid NOT NULL REFERENCES app_user (id),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, key),
    CHECK (planned_end IS NULL OR planned_start IS NULL OR planned_end >= planned_start)
);
CREATE INDEX work_item_project ON work_item (project_id, sort_order);
CREATE INDEX work_item_holder ON work_item (holder_user_id) WHERE status NOT IN ('done', 'cancelled');

CREATE TABLE dependency (
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    from_item    uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    to_item      uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    kind         text NOT NULL DEFAULT 'FS' CHECK (kind IN ('FS', 'SS', 'FF', 'SF')),
    lag_days     int NOT NULL DEFAULT 0,
    PRIMARY KEY (from_item, to_item),
    CHECK (from_item <> to_item)
);

CREATE TABLE work_item_event (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    work_item_id uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    actor_id     uuid NOT NULL REFERENCES app_user (id),
    kind         text NOT NULL,
    payload      jsonb NOT NULL DEFAULT '{}',
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX work_item_event_item ON work_item_event (work_item_id, created_at);

CREATE TABLE work_log (
    id                 uuid PRIMARY KEY,
    tenant_id          uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    work_item_id       uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    user_id            uuid NOT NULL REFERENCES app_user (id),
    started_at         timestamptz NOT NULL,
    minutes            int NOT NULL CHECK (minutes > 0 AND minutes <= 24 * 60),
    description        text NOT NULL DEFAULT '',
    source             text NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'timer', 'git', 'import')),
    -- Frozen cost (filled from phase 3: rate cards and FX).
    rate_minor         bigint,
    cost_minor         bigint,
    currency           char(3),
    project_cost_minor bigint,
    locked             boolean NOT NULL DEFAULT false,
    created_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX work_log_item ON work_log (work_item_id, started_at);
CREATE INDEX work_log_user ON work_log (user_id, started_at);

CREATE TABLE attachment (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    work_item_id uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    work_log_id  uuid REFERENCES work_log (id) ON DELETE CASCADE,
    kind         text NOT NULL CHECK (kind IN ('image', 'audio', 'video', 'file')),
    file_name    text NOT NULL,
    mime         text NOT NULL,
    size_bytes   bigint NOT NULL CHECK (size_bytes >= 0),
    sha256       text NOT NULL,
    object_key   text NOT NULL,
    uploaded_by  uuid NOT NULL REFERENCES app_user (id),
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX attachment_item ON attachment (work_item_id);

-- One running timer per person per company.
CREATE TABLE active_timer (
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    user_id      uuid NOT NULL REFERENCES app_user (id),
    work_item_id uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    started_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['project', 'work_item', 'dependency', 'work_item_event', 'work_log', 'attachment', 'active_timer'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
