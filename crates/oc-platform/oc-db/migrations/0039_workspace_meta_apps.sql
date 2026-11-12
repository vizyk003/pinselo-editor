-- Content Planner: a team workspace can bring its own Meta app (the app of the business that owns its Pages
-- and Instagram accounts) instead of the server-wide one (OC_FACEBOOK_CLIENT_ID / _SECRET).
--
-- * workspace_social_apps: one Meta app per workspace. The app secret is AES-256-GCM ciphertext (key derived
--   from OC_SECRET, like social_accounts.tokens) and never leaves the server.
-- * social_accounts.social_app_id: the app an account was connected through. Publishing and token refresh
--   use that app; NULL means the server-wide app. Removing the app sets its accounts to 'reauth' (in the API)
--   and the reference to NULL.
-- * social_accounts.connected_via: 'oauth' (Facebook login) or 'system_user' (a pasted Meta system-user
--   token, for Pages a business owns; no Facebook login and no App Review needed).
-- * social_oauth_states.workspace_id: a Facebook login started for a workspace uses that workspace's app.

CREATE TABLE workspace_social_apps (
  id            uuid PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  provider      text NOT NULL CHECK (provider IN ('meta')),
  app_id        text NOT NULL CHECK (app_id ~ '^[0-9]{5,25}$'),
  app_secret    text NOT NULL,
  app_name      text,
  graph_version text CHECK (graph_version ~ '^v[0-9]{1,3}\.[0-9]$'),
  created_by    uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (workspace_id, provider)
);

ALTER TABLE social_accounts
  ADD COLUMN social_app_id uuid REFERENCES workspace_social_apps(id) ON DELETE SET NULL,
  ADD COLUMN connected_via text NOT NULL DEFAULT 'oauth' CHECK (connected_via IN ('oauth','system_user'));
CREATE INDEX social_accounts_app ON social_accounts(social_app_id) WHERE social_app_id IS NOT NULL;

ALTER TABLE social_oauth_states
  ADD COLUMN workspace_id uuid REFERENCES workspaces(id) ON DELETE CASCADE;
