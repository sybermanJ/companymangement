-- Phase 2: routine (recurring) work for people or roles, required deliverables,
-- requests, free-flow hand-offs with cross-department approval, and the
-- built-in notification centre.

-- Per-tenant counters for keys of items that have no project (RT-n, REQ-n).
CREATE TABLE tenant_counter (
    tenant_id uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    name      text NOT NULL,
    next      int NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, name)
);

CREATE TABLE routine_template (
    id                   uuid PRIMARY KEY,
    tenant_id            uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    title                text NOT NULL CHECK (length(trim(title)) > 0),
    description          text NOT NULL DEFAULT '',
    -- daily | weekdays (Mon–Fri) | weekly (on the listed ISO weekdays, 1 = Monday)
    recurrence           text NOT NULL CHECK (recurrence IN ('daily', 'weekdays', 'weekly')),
    weekdays             smallint[] NOT NULL DEFAULT '{}',
    target_user_id       uuid REFERENCES app_user (id),
    target_role_id       uuid REFERENCES job_role (id),
    target_department_id uuid REFERENCES department (id),
    mode                 text NOT NULL DEFAULT 'each_member' CHECK (mode IN ('each_member', 'pool')),
    estimate_minutes     int CHECK (estimate_minutes IS NULL OR estimate_minutes BETWEEN 0 AND 1440),
    project_id           uuid REFERENCES project (id) ON DELETE SET NULL,
    active               boolean NOT NULL DEFAULT true,
    created_by           uuid NOT NULL REFERENCES app_user (id),
    created_at           timestamptz NOT NULL DEFAULT now(),
    CHECK ((target_user_id IS NOT NULL) <> (target_role_id IS NOT NULL)),
    CHECK (target_role_id IS NULL OR target_department_id IS NOT NULL)
);

ALTER TABLE work_item
    ADD COLUMN routine_template_id uuid REFERENCES routine_template (id) ON DELETE SET NULL,
    ADD COLUMN occurrence_date date,
    ADD COLUMN target_department_id uuid REFERENCES department (id),
    ADD COLUMN requested_by uuid REFERENCES app_user (id);
-- Generation is idempotent: one item per template, day and holder (or one pool item).
CREATE UNIQUE INDEX work_item_occurrence ON work_item
    (routine_template_id, occurrence_date, COALESCE(holder_user_id, '00000000-0000-0000-0000-000000000000'::uuid))
    WHERE routine_template_id IS NOT NULL AND target_role_id IS NULL;
CREATE UNIQUE INDEX work_item_pool_occurrence ON work_item (routine_template_id, occurrence_date)
    WHERE routine_template_id IS NOT NULL AND target_role_id IS NOT NULL;

CREATE TABLE deliverable_spec (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    template_id  uuid REFERENCES routine_template (id) ON DELETE CASCADE,
    work_item_id uuid REFERENCES work_item (id) ON DELETE CASCADE,
    kind         text NOT NULL CHECK (kind IN ('text', 'image', 'audio', 'video', 'file', 'git_commit', 'link', 'checklist')),
    min_count    int NOT NULL DEFAULT 1 CHECK (min_count BETWEEN 1 AND 100),
    required     boolean NOT NULL DEFAULT true,
    instructions text NOT NULL DEFAULT '',
    sort_order   int NOT NULL DEFAULT 0,
    CHECK ((template_id IS NOT NULL) <> (work_item_id IS NOT NULL))
);

CREATE TABLE deliverable_submission (
    id            uuid PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    spec_id       uuid NOT NULL REFERENCES deliverable_spec (id) ON DELETE CASCADE,
    work_item_id  uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    work_log_id   uuid REFERENCES work_log (id) ON DELETE SET NULL,
    attachment_id uuid REFERENCES attachment (id) ON DELETE CASCADE,
    value         text NOT NULL DEFAULT '',
    submitted_by  uuid NOT NULL REFERENCES app_user (id),
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX deliverable_submission_item ON deliverable_submission (work_item_id);

CREATE TABLE handoff (
    id            uuid PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    work_item_id  uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    from_user     uuid NOT NULL REFERENCES app_user (id),
    to_user       uuid NOT NULL REFERENCES app_user (id),
    requested_by  uuid NOT NULL REFERENCES app_user (id),
    reason        text NOT NULL CHECK (length(trim(reason)) > 0),
    note          text NOT NULL DEFAULT '',
    new_due       date,
    needs_approval boolean NOT NULL,
    approved_by   uuid REFERENCES app_user (id),
    state         text NOT NULL CHECK (state IN ('pending_approval', 'pending_accept', 'accepted', 'declined', 'rejected', 'cancelled')),
    decision_note text NOT NULL DEFAULT '',
    is_return     boolean NOT NULL DEFAULT false,
    reminded_at   timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    decided_at    timestamptz
);
-- At most one open hand-off per item.
CREATE UNIQUE INDEX handoff_open ON handoff (work_item_id) WHERE state IN ('pending_approval', 'pending_accept');
CREATE INDEX handoff_to ON handoff (to_user, state);

CREATE TABLE notification (
    id         uuid PRIMARY KEY,
    tenant_id  uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    kind       text NOT NULL,
    subject    text NOT NULL,
    body       text NOT NULL DEFAULT '',
    link       text NOT NULL DEFAULT '',
    read_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notification_user ON notification (user_id, created_at DESC);

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['tenant_counter', 'routine_template', 'deliverable_spec', 'deliverable_submission', 'handoff', 'notification'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
