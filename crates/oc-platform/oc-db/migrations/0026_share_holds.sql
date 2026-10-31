-- Sharing follow-ups (plan 11, batch 2 review): custom link slugs are never recycled to another account,
-- and designs taken down by moderation stay on hold in the owner's trash.

-- Every custom public link slug ever claimed (g131). Embeds, oEmbed consumers and shared /view/<slug> links keep
-- pointing at a slug after it is changed, removed or its design is deleted, so only the account (or design) that
-- first claimed it may use it again. A deleted account's slugs (owner_id NULL) are retired for good.
CREATE TABLE public_slug_claims (
  slug          text PRIMARY KEY,
  owner_id      uuid REFERENCES users(id) ON DELETE SET NULL,
  design_id     text,
  claimed_at    timestamptz NOT NULL DEFAULT now()
);
INSERT INTO public_slug_claims(slug, owner_id, design_id)
  SELECT public_slug, owner_id, id FROM designs WHERE public_slug IS NOT NULL
  ON CONFLICT (slug) DO NOTHING;

-- Staff takedowns (safety console): the design sits in the owner's trash but the owner can't restore it or delete it
-- forever, like an upload with moderation = 'removed'. Cleared when staff restore it.
ALTER TABLE designs ADD COLUMN taken_down_at timestamptz;
UPDATE designs d SET taken_down_at = coalesce(r.decided_at, now())
  FROM reports r
  WHERE r.target_id = d.id AND r.target_type IN ('design','public_link') AND r.decision LIKE 'takedown:%' AND d.trashed_at IS NOT NULL;
