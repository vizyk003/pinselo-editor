-- Brand Hub (A8), part 2: brand templates with element locks, brand controls, approval workflow.

-- Team settings: { colors, fonts, requireTemplate, restrictUploads, requireApproval } (all booleans; admin only).
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS brand_controls jsonb NOT NULL DEFAULT '{}';

-- Brand templates: element locks applied (authoritatively, from this map) to designs created from them.
ALTER TABLE templates ADD COLUMN IF NOT EXISTS brand boolean NOT NULL DEFAULT false;
ALTER TABLE templates ADD COLUMN IF NOT EXISTS locks jsonb NOT NULL DEFAULT '{}';      -- { elementId: LockMask }
ALTER TABLE templates ADD COLUMN IF NOT EXISTS source_design_id text REFERENCES designs(id) ON DELETE SET NULL;
ALTER TABLE templates ADD COLUMN IF NOT EXISTS version integer NOT NULL DEFAULT 1;

-- Designs started from a team template keep the link (the team governs brand locks, controls and approvals).
ALTER TABLE designs ADD COLUMN IF NOT EXISTS brand_workspace_id uuid REFERENCES workspaces(id) ON DELETE SET NULL;
ALTER TABLE designs ADD COLUMN IF NOT EXISTS template_version integer;
CREATE INDEX IF NOT EXISTS designs_template ON designs(template_id) WHERE template_id IS NOT NULL;

CREATE TABLE design_approvals (
  id              uuid PRIMARY KEY,
  design_id       text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  workspace_id    uuid REFERENCES workspaces(id) ON DELETE SET NULL,
  requested_by    uuid REFERENCES users(id) ON DELETE SET NULL,
  reviewers       uuid[] NOT NULL DEFAULT '{}',
  message         text NOT NULL DEFAULT '',
  status          text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','changes_requested','cancelled')),
  decided_by      uuid REFERENCES users(id) ON DELETE SET NULL,
  decision_note   text,
  decided_at      timestamptz,
  -- Yjs snapshot (state vector + delete set) of the document when approved: any later edit invalidates the approval.
  approved_snapshot bytea,
  approved_doc_at timestamptz,
  created_at      timestamptz NOT NULL DEFAULT now(),
  updated_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX design_approvals_design ON design_approvals(design_id, created_at DESC);
CREATE INDEX design_approvals_reviewers ON design_approvals USING gin (reviewers) WHERE status = 'pending';

ALTER TABLE notifications DROP CONSTRAINT IF EXISTS notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check CHECK (kind IN ('mention','comment','share','reply','assign','publish','approval'));
