-- Phase 3: hourly rates with history, exchange rates, frozen cost per work
-- log, timesheet periods, cost thresholds, alerts and an audit log.

CREATE TABLE rate_card (
    id             uuid PRIMARY KEY,
    tenant_id      uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    user_id        uuid NOT NULL REFERENCES app_user (id),
    rate_minor     bigint NOT NULL CHECK (rate_minor >= 0),
    currency       char(3) NOT NULL,
    effective_from date NOT NULL,
    note           text NOT NULL DEFAULT '',
    set_by         uuid NOT NULL REFERENCES app_user (id),
    created_at     timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, user_id, effective_from)
);

CREATE TABLE fx_rate (
    id             uuid PRIMARY KEY,
    tenant_id      uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    from_ccy       char(3) NOT NULL,
    to_ccy         char(3) NOT NULL,
    rate           numeric(24, 12) NOT NULL CHECK (rate > 0),
    effective_date date NOT NULL,
    set_by         uuid NOT NULL REFERENCES app_user (id),
    created_at     timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, from_ccy, to_ccy, effective_date),
    CHECK (from_ccy <> to_ccy)
);

ALTER TABLE work_log
    ADD COLUMN rate_card_id uuid REFERENCES rate_card (id),
    ADD COLUMN fx_rate_id uuid REFERENCES fx_rate (id),
    ADD COLUMN project_currency char(3);

CREATE TABLE timesheet_period (
    id        uuid PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    starts_on date NOT NULL,
    ends_on   date NOT NULL,
    status    text NOT NULL DEFAULT 'closed' CHECK (status IN ('open', 'closed')),
    closed_by uuid REFERENCES app_user (id),
    closed_at timestamptz,
    CHECK (ends_on >= starts_on)
);

CREATE TABLE cost_threshold (
    id             uuid PRIMARY KEY,
    tenant_id      uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    project_id     uuid NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    metric         text NOT NULL DEFAULT 'labour_cost' CHECK (metric IN ('labour_cost', 'forecast')),
    mode           text NOT NULL CHECK (mode IN ('absolute', 'percent')),
    amount_minor   bigint CHECK (amount_minor IS NULL OR amount_minor >= 0),
    percent        int CHECK (percent IS NULL OR percent BETWEEN 1 AND 1000),
    level          text NOT NULL CHECK (level IN ('info', 'warn', 'critical')),
    notify_pm      boolean NOT NULL DEFAULT true,
    notify_finance boolean NOT NULL DEFAULT true,
    notify_admins  boolean NOT NULL DEFAULT false,
    created_by     uuid NOT NULL REFERENCES app_user (id),
    created_at     timestamptz NOT NULL DEFAULT now(),
    CHECK ((mode = 'absolute' AND amount_minor IS NOT NULL) OR (mode = 'percent' AND percent IS NOT NULL))
);

CREATE TABLE alert (
    id             uuid PRIMARY KEY,
    tenant_id      uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    threshold_id   uuid NOT NULL REFERENCES cost_threshold (id) ON DELETE CASCADE,
    project_id     uuid NOT NULL REFERENCES project (id) ON DELETE CASCADE,
    level          text NOT NULL,
    observed_minor bigint NOT NULL,
    limit_minor    bigint NOT NULL,
    state          text NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'acknowledged', 'resolved')),
    fired_at       timestamptz NOT NULL DEFAULT now(),
    ack_by         uuid REFERENCES app_user (id),
    ack_note       text NOT NULL DEFAULT '',
    resolved_at    timestamptz
);
-- Fire once per crossing: one unresolved alert per threshold.
CREATE UNIQUE INDEX alert_unresolved ON alert (threshold_id) WHERE state <> 'resolved';

CREATE TABLE audit_log (
    id         uuid PRIMARY KEY,
    tenant_id  uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    actor_id   uuid NOT NULL REFERENCES app_user (id),
    action     text NOT NULL,
    payload    jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['rate_card', 'fx_rate', 'timesheet_period', 'cost_threshold', 'alert', 'audit_log'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
