-- Optional Supabase Auth (plan 12): an OpenCanvas account linked to a Supabase Auth user (auth.users.id).
-- No foreign key to auth.users: plain Postgres has no such schema.
ALTER TABLE users ADD COLUMN supabase_id uuid UNIQUE;

-- Bridged Supabase access tokens are single-use (replay guard, plan 12 §B8); rows are useless after expires_at.
-- A table of its own rather than an auth_tokens kind, so auth_tokens_kind_check is never edited here.
CREATE TABLE supabase_token_uses (
  token_sha256  text PRIMARY KEY,
  user_id       uuid REFERENCES users(id) ON DELETE CASCADE,
  expires_at    timestamptz NOT NULL,
  used_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX supabase_token_uses_expiry ON supabase_token_uses(expires_at);
