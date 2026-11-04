-- Phase 4: Git repositories (GitHub and self-hosted Gitea/GitLab/other) via
-- signed webhooks, commit author mapping, and commit ↔ work item links.

CREATE TABLE git_repository (
    id             uuid PRIMARY KEY,
    tenant_id      uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    provider       text NOT NULL CHECK (provider IN ('github', 'gitea', 'gitlab', 'generic')),
    name           text NOT NULL CHECK (length(trim(name)) > 0),
    url            text NOT NULL DEFAULT '',
    project_id     uuid REFERENCES project (id) ON DELETE SET NULL,
    webhook_secret text NOT NULL,
    created_by     uuid NOT NULL REFERENCES app_user (id),
    created_at     timestamptz NOT NULL DEFAULT now()
);

-- Webhooks arrive without a session, so the receiver needs the tenant of a
-- repository before it can open a tenant transaction. This global map holds
-- only ids (no secrets); the secret is checked inside the tenant transaction.
CREATE TABLE git_hook_route (
    repo_id   uuid PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE
);

CREATE TABLE git_identity (
    tenant_id uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    email     text NOT NULL,
    user_id   uuid NOT NULL REFERENCES app_user (id),
    PRIMARY KEY (tenant_id, email)
);

CREATE TABLE git_commit (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    repo_id      uuid REFERENCES git_repository (id) ON DELETE CASCADE,
    sha          text NOT NULL CHECK (sha ~ '^[0-9a-f]{7,64}$'),
    author_name  text NOT NULL DEFAULT '',
    author_email text NOT NULL DEFAULT '',
    user_id      uuid REFERENCES app_user (id),
    message      text NOT NULL DEFAULT '',
    url          text NOT NULL DEFAULT '',
    committed_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX git_commit_sha ON git_commit (tenant_id, COALESCE(repo_id, '00000000-0000-0000-0000-000000000000'::uuid), sha);

CREATE TABLE git_commit_link (
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    commit_id    uuid NOT NULL REFERENCES git_commit (id) ON DELETE CASCADE,
    work_item_id uuid NOT NULL REFERENCES work_item (id) ON DELETE CASCADE,
    linked_by    uuid REFERENCES app_user (id),
    PRIMARY KEY (commit_id, work_item_id)
);

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['git_repository', 'git_identity', 'git_commit', 'git_commit_link'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
