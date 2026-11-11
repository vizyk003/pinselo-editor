-- Content Planner: approval workflow, editorial fields and posts published by an outside tool.
--
-- * Review statuses: draft → in_review → approved | changes_requested. The worker still only publishes
--   status = 'scheduled', so approved posts are never picked up by it.
-- * publisher = 'external': an outside tool (for example a team's own Instagram/Facebook publisher) claims
--   approved, due posts through /v1/planner and reports per-channel results. Pinselo's worker ignores these
--   posts entirely; `channels` names where the outside tool posts them.
-- * fields: free-form editorial fields (post reference, pillar, source, topic, fact IDs, reviewer note, format).
-- * approved_version: the design's content_version when the post was approved; the publisher exports exactly
--   that version (409 version_changed otherwise).

ALTER TABLE social_posts DROP CONSTRAINT social_posts_status_check;
ALTER TABLE social_posts ADD CONSTRAINT social_posts_status_check
  CHECK (status IN ('draft','in_review','changes_requested','approved','scheduled','publishing','published','failed'));

ALTER TABLE social_posts
  ADD COLUMN publisher        text NOT NULL DEFAULT 'pinselo' CHECK (publisher IN ('pinselo','external')),
  ADD COLUMN channels         text[] NOT NULL DEFAULT '{}',
  ADD COLUMN fields           jsonb NOT NULL DEFAULT '{}',
  ADD COLUMN submitted_by     uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN submitted_at     timestamptz,
  ADD COLUMN approved_by      uuid REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN approved_at      timestamptz,
  ADD COLUMN approved_version bigint,
  ADD COLUMN claim_token      text,
  ADD COLUMN claimed_at       timestamptz;

ALTER TABLE social_posts ADD CONSTRAINT social_posts_channels_check
  CHECK (channels <@ ARRAY['instagram','facebook']::text[]);

-- The outside publisher's "due" query.
CREATE INDEX social_posts_external_due ON social_posts(workspace_id, scheduled_at)
  WHERE publisher = 'external' AND status = 'approved';
-- One post reference (e.g. "P-001") per workspace.
CREATE UNIQUE INDEX social_posts_ref ON social_posts(workspace_id, (fields->>'ref'))
  WHERE fields ? 'ref' AND workspace_id IS NOT NULL;
