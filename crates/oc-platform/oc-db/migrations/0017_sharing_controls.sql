-- Sharing controls (CO-040..043): public link expiry/password, team folders, access requests.
ALTER TABLE designs ADD COLUMN public_expires_at timestamptz;
ALTER TABLE designs ADD COLUMN public_password_hash text;

CREATE TABLE access_requests (
  id            uuid PRIMARY KEY,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role          text NOT NULL CHECK (role IN ('view','comment','edit')),
  message       text,
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','declined')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  decided_at    timestamptz
);
CREATE UNIQUE INDEX access_requests_pending ON access_requests(design_id, user_id) WHERE status = 'pending';
CREATE INDEX folders_workspace ON folders(workspace_id);
