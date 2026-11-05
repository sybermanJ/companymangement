-- Phase 6: project lifecycle. Expense and fund requests (reimbursement, cash
-- advance, supplier payment, purchase, trip) with line items and receipts;
-- configurable approval routes shared by requests and contracts; contracts with
-- versioned files and payment milestones tied to phases; expense budgets per
-- project and phase.

ALTER TABLE project ADD COLUMN expense_budget_minor bigint
    CHECK (expense_budget_minor IS NULL OR expense_budget_minor >= 0);
ALTER TABLE project ADD COLUMN customer text NOT NULL DEFAULT '';
-- Phases carry their own expense budget; the phase holder is the phase manager.
ALTER TABLE work_item ADD COLUMN budget_minor bigint
    CHECK (budget_minor IS NULL OR budget_minor >= 0);
ALTER TABLE membership ADD COLUMN can_view_contracts boolean NOT NULL DEFAULT false;

ALTER TABLE cost_threshold DROP CONSTRAINT cost_threshold_metric_check;
ALTER TABLE cost_threshold ADD CONSTRAINT cost_threshold_metric_check
    CHECK (metric IN ('labour_cost', 'forecast', 'expense', 'total_cost'));

-- Approval routes. `steps` is a JSON array of
--   {"kind": "phase_manager" | "project_manager" | "department_head" | "finance" | "admin" | "user",
--    "user_id": uuid (kind = user), "label": text}
-- The route used is the active one for the subject kind with the highest
-- min_amount_minor not above the request amount (amounts in the request currency).
CREATE TABLE approval_route (
    id               uuid PRIMARY KEY,
    tenant_id        uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subject          text NOT NULL CHECK (subject IN ('reimbursement', 'advance', 'payment', 'purchase', 'trip', 'contract')),
    name             text NOT NULL CHECK (length(trim(name)) > 0),
    min_amount_minor bigint NOT NULL DEFAULT 0 CHECK (min_amount_minor >= 0),
    steps            jsonb NOT NULL,
    active           boolean NOT NULL DEFAULT true,
    created_by       uuid NOT NULL REFERENCES app_user (id),
    created_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX approval_route_subject ON approval_route (tenant_id, subject, min_amount_minor);

-- One running (or finished) approval of a request or contract. `steps` is the
-- route resolved to people at submission: [{"kind", "label", "users": [uuid]}].
CREATE TABLE approval (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subject      text NOT NULL,
    subject_id   uuid NOT NULL,
    route_id     uuid REFERENCES approval_route (id) ON DELETE SET NULL,
    steps        jsonb NOT NULL,
    current_step int NOT NULL DEFAULT 0,
    state        text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'approved', 'rejected', 'returned', 'cancelled')),
    round        int NOT NULL DEFAULT 1,
    created_at   timestamptz NOT NULL DEFAULT now(),
    decided_at   timestamptz
);
CREATE INDEX approval_subject ON approval (subject_id, created_at DESC);

-- Every action on a request or contract, in order: the audit trail people read.
CREATE TABLE approval_action (
    id          uuid PRIMARY KEY,
    tenant_id   uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subject_id  uuid NOT NULL,
    approval_id uuid REFERENCES approval (id) ON DELETE CASCADE,
    step        int,
    actor_id    uuid NOT NULL REFERENCES app_user (id),
    action      text NOT NULL,
    to_user     uuid REFERENCES app_user (id),
    note        text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX approval_action_subject ON approval_action (subject_id, created_at);

CREATE TABLE claim (
    id            uuid PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    key           text NOT NULL,
    kind          text NOT NULL CHECK (kind IN ('reimbursement', 'advance', 'payment', 'purchase', 'trip')),
    project_id    uuid REFERENCES project (id) ON DELETE SET NULL,
    phase_id      uuid REFERENCES work_item (id) ON DELETE SET NULL,
    parent_id     uuid REFERENCES claim (id) ON DELETE SET NULL,
    applicant_id  uuid NOT NULL REFERENCES app_user (id),
    title         text NOT NULL CHECK (length(trim(title)) > 0),
    justification text NOT NULL DEFAULT '',
    currency      char(3) NOT NULL,
    amount_minor  bigint NOT NULL DEFAULT 0 CHECK (amount_minor >= 0),
    payee         text NOT NULL DEFAULT '',
    -- Trip details (kind = trip).
    destination   text NOT NULL DEFAULT '',
    depart_on     date,
    return_on     date,
    travellers    uuid[] NOT NULL DEFAULT '{}',
    status        text NOT NULL DEFAULT 'draft'
                  CHECK (status IN ('draft', 'in_approval', 'returned', 'rejected', 'approved', 'paid', 'closed', 'cancelled')),
    approval_id   uuid REFERENCES approval (id) ON DELETE SET NULL,
    paid_at       timestamptz,
    paid_by       uuid REFERENCES app_user (id),
    payment_ref   text NOT NULL DEFAULT '',
    -- Amount converted to the project currency when it was approved (for cost).
    project_amount_minor bigint,
    submitted_at  timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, key),
    CHECK (return_on IS NULL OR depart_on IS NULL OR return_on >= depart_on)
);
CREATE INDEX claim_project ON claim (project_id, created_at DESC);
CREATE INDEX claim_applicant ON claim (applicant_id, created_at DESC);

CREATE TABLE claim_line (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    claim_id     uuid NOT NULL REFERENCES claim (id) ON DELETE CASCADE,
    spent_on     date,
    category     text NOT NULL CHECK (category IN ('flight', 'train', 'hotel', 'meals', 'local_transport', 'allowance',
                                                   'equipment', 'materials', 'software', 'outsourcing', 'entertainment', 'other')),
    description  text NOT NULL DEFAULT '',
    amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
    sort_order   int NOT NULL DEFAULT 0
);
CREATE INDEX claim_line_claim ON claim_line (claim_id, sort_order);

CREATE TABLE contract (
    id            uuid PRIMARY KEY,
    tenant_id     uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    key           text NOT NULL,
    project_id    uuid REFERENCES project (id) ON DELETE SET NULL,
    title         text NOT NULL CHECK (length(trim(title)) > 0),
    counterparty  text NOT NULL DEFAULT '',
    kind          text NOT NULL CHECK (kind IN ('sales', 'purchase', 'outsourcing', 'nda', 'framework', 'other')),
    currency      char(3) NOT NULL,
    amount_minor  bigint NOT NULL DEFAULT 0 CHECK (amount_minor >= 0),
    owner_id      uuid NOT NULL REFERENCES app_user (id),
    status        text NOT NULL DEFAULT 'draft'
                  CHECK (status IN ('draft', 'in_approval', 'returned', 'rejected', 'approved', 'signed', 'completed', 'terminated')),
    signed_on     date,
    starts_on     date,
    ends_on       date,
    summary       text NOT NULL DEFAULT '',
    approval_id   uuid REFERENCES approval (id) ON DELETE SET NULL,
    created_by    uuid NOT NULL REFERENCES app_user (id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, key)
);
CREATE INDEX contract_project ON contract (project_id);

CREATE TABLE contract_milestone (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    contract_id  uuid NOT NULL REFERENCES contract (id) ON DELETE CASCADE,
    phase_id     uuid REFERENCES work_item (id) ON DELETE SET NULL,
    title        text NOT NULL CHECK (length(trim(title)) > 0),
    due_on       date,
    amount_minor bigint NOT NULL DEFAULT 0 CHECK (amount_minor >= 0),
    status       text NOT NULL DEFAULT 'planned' CHECK (status IN ('planned', 'invoiced', 'settled')),
    settled_on   date,
    sort_order   int NOT NULL DEFAULT 0
);
CREATE INDEX contract_milestone_contract ON contract_milestone (contract_id, sort_order);

-- Files for requests (receipts, quotes, tickets) and contracts (drafts and signed copies).
CREATE TABLE document (
    id           uuid PRIMARY KEY,
    tenant_id    uuid NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    owner_kind   text NOT NULL CHECK (owner_kind IN ('claim', 'contract')),
    owner_id     uuid NOT NULL,
    claim_line_id uuid REFERENCES claim_line (id) ON DELETE SET NULL,
    label        text NOT NULL DEFAULT '',
    version      int NOT NULL DEFAULT 1,
    file_name    text NOT NULL,
    mime         text NOT NULL,
    size_bytes   bigint NOT NULL CHECK (size_bytes >= 0),
    sha256       text NOT NULL,
    object_key   text NOT NULL,
    uploaded_by  uuid NOT NULL REFERENCES app_user (id),
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX document_owner ON document (owner_id, created_at);

-- Booking and other tasks created from a request (e.g. "buy the tickets for TRIP-3").
ALTER TABLE work_item ADD COLUMN claim_id uuid REFERENCES claim (id) ON DELETE SET NULL;

DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['approval_route', 'approval', 'approval_action', 'claim', 'claim_line',
                             'contract', 'contract_milestone', 'document'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I USING (tenant_id = app_tenant()) WITH CHECK (tenant_id = app_tenant())',
                       t || '_isolation', t);
    END LOOP;
END $$;
