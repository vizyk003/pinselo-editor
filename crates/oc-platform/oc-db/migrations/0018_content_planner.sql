-- Content Planner (A16): connected social accounts, scheduled posts and their publish log.

CREATE TABLE social_accounts (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- Optional: shared with a team workspace so its members can schedule to it.
  workspace_id  uuid REFERENCES workspaces(id) ON DELETE SET NULL,
  provider      text NOT NULL,          -- facebook | instagram | linkedin | pinterest | x | tiktok | youtube | threads | bluesky | local
  external_id   text NOT NULL,
  display_name  text NOT NULL,
  handle        text,
  avatar_url    text,
  -- AES-256-GCM ciphertext of the provider tokens (key derived from OC_SECRET). Never sent to clients.
  tokens        text,
  token_expires_at timestamptz,
  meta          jsonb NOT NULL DEFAULT '{}',
  status        text NOT NULL DEFAULT 'ok' CHECK (status IN ('ok','reauth')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (user_id, provider, external_id)
);
CREATE INDEX social_accounts_ws ON social_accounts(workspace_id) WHERE workspace_id IS NOT NULL;

-- OAuth authorization-code + PKCE state (single use, short-lived).
CREATE TABLE social_oauth_states (
  state         text PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider      text NOT NULL,
  code_verifier text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE social_posts (
  id            uuid PRIMARY KEY,
  owner_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  workspace_id  uuid REFERENCES workspaces(id) ON DELETE CASCADE,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  pages         integer[] NOT NULL DEFAULT '{}',   -- 0-based page indexes; empty = all pages
  format        text NOT NULL CHECK (format IN ('png','jpg','mp4')),
  caption       text NOT NULL DEFAULT '',
  hashtags      text[] NOT NULL DEFAULT '{}',
  accounts      uuid[] NOT NULL DEFAULT '{}',
  options       jsonb NOT NULL DEFAULT '{}',       -- per-network extras (pinterest board, youtube title, …)
  scheduled_at  timestamptz,
  timezone      text NOT NULL DEFAULT 'UTC',
  status        text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','scheduled','publishing','published','failed')),
  media         jsonb NOT NULL DEFAULT '[]',       -- rendered files [{sha, mime, filename, width, height, bytes}]
  results       jsonb NOT NULL DEFAULT '{}',       -- per account id: {status, url, error, at}
  error         text,
  job_id        uuid,
  published_at  timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX social_posts_owner ON social_posts(owner_id, scheduled_at);
CREATE INDEX social_posts_ws ON social_posts(workspace_id, scheduled_at);
CREATE INDEX social_posts_due ON social_posts(scheduled_at) WHERE status = 'scheduled';

CREATE TABLE social_post_events (
  id            bigserial PRIMARY KEY,
  post_id       uuid NOT NULL REFERENCES social_posts(id) ON DELETE CASCADE,
  account_id    uuid,
  kind          text NOT NULL,   -- created | scheduled | rescheduled | rendering | rendered | published | failed | retry | cancelled | reminder
  message       text,
  data          jsonb,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX social_post_events_post ON social_post_events(post_id, id);

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check CHECK (kind IN ('mention','comment','share','reply','assign','publish'));
