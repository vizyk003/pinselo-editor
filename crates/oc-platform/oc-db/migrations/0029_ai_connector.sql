-- AI connector (MCP): OAuth 2.1 clients and authorization codes for connecting AI assistants (Claude, ChatGPT,
-- Codex…), OAuth access tokens stored next to personal access tokens, and attribution of AI checkpoints.

-- Registered clients: dynamic client registration (RFC 7591) or a client ID metadata document (`id` is its https URL).
CREATE TABLE oauth_clients (
  id            text PRIMARY KEY,
  kind          text NOT NULL CHECK (kind IN ('dcr', 'cimd')),
  name          text NOT NULL,
  redirect_uris text[] NOT NULL,
  client_uri    text,
  logo_uri      text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  last_used_at  timestamptz
);

-- One-time authorization codes (PKCE S256), valid for a few minutes.
CREATE TABLE oauth_codes (
  code_sha256    text PRIMARY KEY,
  client_id      text NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
  user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  redirect_uri   text NOT NULL,
  code_challenge text NOT NULL,
  scopes         text[] NOT NULL,
  resource       text,
  expires_at     timestamptz NOT NULL,
  used_at        timestamptz,
  created_at     timestamptz NOT NULL DEFAULT now()
);

-- OAuth access tokens live in api_tokens (kind 'oauth') so every bearer path checks one table. Refresh tokens
-- rotate: each refresh revokes the row and issues a new one in the same family; reusing a rotated refresh
-- token revokes the whole family.
ALTER TABLE api_tokens
  ADD COLUMN kind               text NOT NULL DEFAULT 'pat' CHECK (kind IN ('pat', 'oauth')),
  ADD COLUMN client_id          text REFERENCES oauth_clients(id) ON DELETE CASCADE,
  ADD COLUMN refresh_sha256     text UNIQUE,
  ADD COLUMN refresh_expires_at timestamptz,
  ADD COLUMN family             uuid,
  ADD COLUMN resource           text;
CREATE INDEX api_tokens_family ON api_tokens(family) WHERE family IS NOT NULL;

-- Checkpoints taken before an AI assistant edits a design ("Undo AI changes"): the assistant's display name.
ALTER TABLE design_versions ADD COLUMN agent text;

-- Confidential clients (DCR with client_secret_post/basic): only the secret's hash is kept. Metadata documents
-- are refetched once they are a day old. A code remembers the token family it was exchanged for, so a replayed
-- code can revoke what it issued.
ALTER TABLE oauth_clients ADD COLUMN client_secret_sha256 text, ADD COLUMN fetched_at timestamptz;
ALTER TABLE oauth_codes ADD COLUMN family uuid;
