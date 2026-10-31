-- Public API (API-001..010): personal access tokens and webhooks.
CREATE TABLE api_tokens (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name          text NOT NULL,
  token_sha256  text NOT NULL UNIQUE,
  prefix        text NOT NULL,                 -- first chars, shown in the UI
  scopes        text[] NOT NULL,
  last_used_at  timestamptz,
  expires_at    timestamptz,
  revoked_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX api_tokens_user ON api_tokens(user_id);

CREATE TABLE webhooks (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  url           text NOT NULL,
  events        text[] NOT NULL,
  secret        text NOT NULL,
  active        boolean NOT NULL DEFAULT true,
  failures      integer NOT NULL DEFAULT 0,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX webhooks_user ON webhooks(user_id);

CREATE TABLE webhook_deliveries (
  id            uuid PRIMARY KEY,
  webhook_id    uuid NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
  event         text NOT NULL,
  payload       jsonb NOT NULL,
  status        text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','delivered','failed')),
  attempts      integer NOT NULL DEFAULT 0,
  response_code integer,
  error         text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  delivered_at  timestamptz
);
CREATE INDEX webhook_deliveries_hook ON webhook_deliveries(webhook_id, created_at DESC);
