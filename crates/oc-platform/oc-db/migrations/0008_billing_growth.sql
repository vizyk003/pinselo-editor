-- Plans & billing (PL-020..026), consent (PL-008), analytics (PL-040), support (PL-050), status (PL-052).

CREATE TABLE subscriptions (
  workspace_id            uuid PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
  plan                    text NOT NULL CHECK (plan IN ('free','pro','teams')),
  status                  text NOT NULL CHECK (status IN ('trialing','active','past_due','canceled','incomplete')),
  seats                   integer NOT NULL DEFAULT 1,
  interval                text NOT NULL DEFAULT 'month' CHECK (interval IN ('month','year')),
  provider                text NOT NULL DEFAULT 'local',          -- 'stripe' | 'local'
  customer_id             text,
  subscription_id         text,
  current_period_end      timestamptz,
  cancel_at_period_end    boolean NOT NULL DEFAULT false,
  trial_end               timestamptz,
  updated_at              timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX subscriptions_customer ON subscriptions(customer_id);

-- Webhook idempotency: each provider event is applied once.
CREATE TABLE billing_events (
  id            text PRIMARY KEY,
  type          text NOT NULL,
  payload       jsonb NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE invoices (
  id            text PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  amount_cents  integer NOT NULL,
  currency      text NOT NULL,
  status        text NOT NULL,
  url           text,
  period_start  timestamptz,
  period_end    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX invoices_ws ON invoices(workspace_id, created_at DESC);

CREATE TABLE consents (
  id            bigserial PRIMARY KEY,
  user_id       uuid REFERENCES users(id) ON DELETE CASCADE,
  anon_id       text,
  kind          text NOT NULL,          -- 'analytics' | 'marketing' | 'terms'
  granted       boolean NOT NULL,
  version       text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX consents_user ON consents(user_id, kind, created_at DESC);

CREATE TABLE analytics_events (
  id            bigserial PRIMARY KEY,
  user_id       uuid REFERENCES users(id) ON DELETE SET NULL,
  anon_id       text,
  name          text NOT NULL,
  props         jsonb NOT NULL DEFAULT '{}',
  path          text,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX analytics_name_time ON analytics_events(name, created_at);
CREATE INDEX analytics_time ON analytics_events(created_at);

CREATE TABLE support_tickets (
  id            uuid PRIMARY KEY,
  user_id       uuid REFERENCES users(id) ON DELETE SET NULL,
  email         text NOT NULL,
  topic         text NOT NULL,
  subject       text NOT NULL,
  status        text NOT NULL DEFAULT 'open' CHECK (status IN ('open','answered','closed')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE support_messages (
  id            bigserial PRIMARY KEY,
  ticket_id     uuid NOT NULL REFERENCES support_tickets(id) ON DELETE CASCADE,
  from_staff    boolean NOT NULL,
  author_id     uuid REFERENCES users(id) ON DELETE SET NULL,
  body          text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE service_heartbeats (
  service       text PRIMARY KEY,
  at            timestamptz NOT NULL DEFAULT now(),
  info          jsonb
);
CREATE TABLE status_incidents (
  id            uuid PRIMARY KEY,
  title         text NOT NULL,
  impact        text NOT NULL CHECK (impact IN ('minor','major','critical','maintenance')),
  components    text[] NOT NULL DEFAULT '{}',
  status        text NOT NULL CHECK (status IN ('investigating','identified','monitoring','resolved','scheduled')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  resolved_at   timestamptz
);
CREATE TABLE status_updates (
  id            bigserial PRIMARY KEY,
  incident_id   uuid NOT NULL REFERENCES status_incidents(id) ON DELETE CASCADE,
  status        text NOT NULL,
  body          text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
