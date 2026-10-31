-- Brand kit v1 (CA-060..062), notifications (CO-070), AI usage metering (AI-001), public view links (CO-034).

CREATE TABLE brand_kits (
  workspace_id  uuid PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
  name          text NOT NULL DEFAULT 'Brand kit',
  colors        jsonb NOT NULL DEFAULT '[]',     -- [{ "name": "Primary", "hex": "#7c3aed" }]
  fonts         jsonb NOT NULL DEFAULT '{}',     -- { "heading": "Poppins", "subheading": "Inter", "body": "Inter" }
  logos         jsonb NOT NULL DEFAULT '[]',     -- [asset id]
  updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE notifications (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind          text NOT NULL CHECK (kind IN ('mention','comment','share','reply')),
  design_id     text REFERENCES designs(id) ON DELETE CASCADE,
  actor_id      uuid REFERENCES users(id) ON DELETE SET NULL,
  body          text NOT NULL,
  read_at       timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_user ON notifications(user_id, created_at DESC);

CREATE TABLE ai_usage (
  id            bigserial PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  feature       text NOT NULL,
  credits       integer NOT NULL,
  input_tokens  integer,
  output_tokens integer,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ai_usage_user_day ON ai_usage(user_id, created_at);

ALTER TABLE designs ADD COLUMN public_view boolean NOT NULL DEFAULT false;
