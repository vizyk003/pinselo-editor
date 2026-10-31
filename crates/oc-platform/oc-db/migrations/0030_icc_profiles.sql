-- Print colour profiles: uploaded CMYK output (printer-class) ICC profiles, personal or shared with a team,
-- usable for professional print export (PDF/X OutputIntent) and the gamut check. Bundled profiles
-- (packages/schema/src/icc-profiles.json) are not stored here. The bytes live in the object store (sha).

CREATE TABLE icc_profiles (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,       -- uploader (owner of a personal profile)
  team_id       uuid REFERENCES workspaces(id) ON DELETE CASCADE,           -- null = personal; else shared with the team
  name          text NOT NULL,
  description   text NOT NULL DEFAULT '',                                   -- the profile's own 'desc' tag
  color_space   text NOT NULL DEFAULT 'CMYK' CHECK (color_space IN ('CMYK')),
  condition_id  text,                                                       -- OutputConditionIdentifier (optional)
  sha           text NOT NULL,                                              -- object store key (sha256 of the .icc)
  size          integer NOT NULL CHECK (size > 0),
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX icc_profiles_user ON icc_profiles(user_id, created_at) WHERE team_id IS NULL;
CREATE INDEX icc_profiles_team ON icc_profiles(team_id, created_at) WHERE team_id IS NOT NULL;
