-- A short-lived, single-use address where a file can be sent into one user's uploads, for assistants that can't carry
-- it through a tool call. Only the token's hash is stored. `used_at` is set when a file is taken; `asset_ids` once
-- stored (a browser form can bring several files through one link).
CREATE TABLE upload_links (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_sha256 text NOT NULL UNIQUE,
  filename text,
  purpose text,
  created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  used_at timestamptz,
  asset_ids text[] NOT NULL DEFAULT '{}'
);
CREATE INDEX upload_links_user ON upload_links (user_id, created_at);
