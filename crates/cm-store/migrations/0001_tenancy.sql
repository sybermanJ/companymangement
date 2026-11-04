-- Tenancy foundation. Every business table carries tenant_id and is protected
-- by row-level security. The application sets app.tenant_id (and app.user_id)
-- per transaction with set_config(..., true); when unset, policies match nothing.
-- FORCE ROW LEVEL SECURITY makes the policies apply to the table owner too, so
-- the app can run as the owning role. Never connect the app as a superuser:
-- superusers bypass RLS.

CREATE OR REPLACE FUNCTION app_tenant() RETURNS uuid
LANGUAGE sql STABLE AS $$ SELECT nullif(current_setting('app.tenant_id', true), '')::uuid $$;

CREATE OR REPLACE FUNCTION app_user() RETURNS uuid
LANGUAGE sql STABLE AS $$ SELECT nullif(current_setting('app.user_id', true), '')::uuid $$;

-- Global tables (no tenant): companies and identities.
CREATE TABLE tenant (
    id               uuid PRIMARY KEY,
    name             text NOT NULL CHECK (length(trim(name)) > 0),
    slug             text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,62}$'),
    plan             text NOT NULL DEFAULT 'trial',
    status           text NOT NULL DEFAULT 'active' CHECK (status IN ('trial', 'active', 'suspended')),
    default_currency char(3) NOT NULL DEFAULT 'CNY',
    timezone         text NOT NULL DEFAULT 'Asia/Shanghai',
    locale           text NOT NULL DEFAULT 'zh-CN',
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE app_user (
    id             uuid PRIMARY KEY,
    email          text NOT NULL,
    email_verified boolean NOT NULL DEFAULT false,
    password_hash  text NOT NULL,
    display_name   text NOT NULL,
    locale         text NOT NULL DEFAULT 'zh-CN',
    created_at     timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX app_user_email_key ON app_user (lower(email));

CREATE TABLE session (
    token_hash  bytea PRIMARY KEY,
    user_id     uuid NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    tenant_id   uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL
);

-- Tenant-scoped tables.
CREATE TABLE membership (
    tenant_id  uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    status     text NOT NULL DEFAULT 'active' CHECK (status IN ('invited', 'active', 'deactivated')),
    is_admin   boolean NOT NULL DEFAULT false,
    is_finance boolean NOT NULL DEFAULT false,
    timezone   text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE TABLE department (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    parent_id    uuid REFERENCES department (id),
    name         text NOT NULL CHECK (length(trim(name)) > 0),
    head_user_id uuid REFERENCES app_user (id),
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, parent_id, name)
);

CREATE TABLE department_deputy (
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    department_id uuid NOT NULL REFERENCES department (id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES app_user (id),
    PRIMARY KEY (department_id, user_id)
);

CREATE TABLE job_role (
    id            uuid PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    department_id uuid REFERENCES department (id),
    name          text NOT NULL CHECK (length(trim(name)) > 0),
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_role (
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES app_user (id),
    role_id       uuid NOT NULL REFERENCES job_role (id) ON DELETE CASCADE,
    department_id uuid NOT NULL REFERENCES department (id),
    valid_from    date NOT NULL DEFAULT current_date,
    valid_to      date,
    PRIMARY KEY (tenant_id, user_id, role_id, department_id, valid_from)
);

-- Membership rows are visible inside the current tenant, and a user can always
-- see their own memberships (needed to list which companies they belong to).
ALTER TABLE membership ENABLE ROW LEVEL SECURITY;
ALTER TABLE membership FORCE ROW LEVEL SECURITY;
CREATE POLICY membership_isolation ON membership
    USING (tenant_id = app_tenant() OR user_id = app_user())
    WITH CHECK (tenant_id = app_tenant());

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['department', 'department_deputy', 'job_role', 'user_role'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
