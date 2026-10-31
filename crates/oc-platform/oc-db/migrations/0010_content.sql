-- Content (batch 4): where an asset came from + licence attribution, uploaded brand fonts, team templates.
ALTER TABLE assets ADD COLUMN source text NOT NULL DEFAULT 'upload';      -- upload | emoji | stock | gif
ALTER TABLE assets ADD COLUMN attribution jsonb;                           -- { provider, author, authorUrl, license, licenseUrl, sourceUrl }
ALTER TABLE assets ADD COLUMN animated boolean NOT NULL DEFAULT false;     -- animated GIF stored as a looping video proxy
CREATE INDEX assets_owner_source ON assets(owner_id, source, created_at DESC);

CREATE TABLE custom_fonts (
  id            text PRIMARY KEY,                    -- short id; file is served at /api/fonts/files/custom/{id}
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  uploaded_by   uuid REFERENCES users(id) ON DELETE SET NULL,
  family        text NOT NULL,
  weight        integer NOT NULL,
  italic        boolean NOT NULL,
  sha256        text NOT NULL,
  bytes         integer NOT NULL,
  format        text NOT NULL,                       -- ttf | otf
  license_ack   boolean NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (workspace_id, sha256)
);
CREATE INDEX custom_fonts_ws ON custom_fonts(workspace_id);

ALTER TABLE templates ADD COLUMN workspace_id uuid REFERENCES workspaces(id) ON DELETE CASCADE;   -- null = public catalogue
ALTER TABLE templates ADD COLUMN created_by uuid REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE templates ADD COLUMN description text;
ALTER TABLE templates ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE templates ADD COLUMN uses integer NOT NULL DEFAULT 0;
CREATE INDEX templates_ws ON templates(workspace_id) WHERE workspace_id IS NOT NULL;
