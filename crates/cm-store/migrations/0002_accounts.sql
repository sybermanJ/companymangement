-- Single-use tokens for email verification, password reset and invitations.
-- Only the SHA-256 of the token is stored. Server-only table (no tenant data).
CREATE TABLE auth_token (
    token_hash bytea PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    tenant_id  uuid REFERENCES tenant (id) ON DELETE CASCADE,
    purpose    text NOT NULL CHECK (purpose IN ('verify_email', 'reset_password', 'invite')),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at    timestamptz
);
CREATE INDEX auth_token_user ON auth_token (user_id, purpose);

-- Transactional email outbox: written in the same transaction as the change
-- that causes it, delivered by a background worker.
CREATE TABLE outbox_email (
    id         uuid PRIMARY KEY,
    to_addr    text NOT NULL,
    subject    text NOT NULL,
    body_text  text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at    timestamptz,
    attempts   int NOT NULL DEFAULT 0,
    last_error text
);
CREATE INDEX outbox_email_pending ON outbox_email (created_at) WHERE sent_at IS NULL;

ALTER TABLE membership ADD COLUMN invited_by uuid REFERENCES app_user (id);

-- A password hash of '!' means "no password set yet" (invited, not accepted).
