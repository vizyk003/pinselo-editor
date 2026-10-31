-- Launch platform (PL-001/002/004/008, CO-002/006/060/061, TS-001/003/006/022, PL-061 Postgres fallback, CO-070 email).

-- ---------------------------------------------------------------- identity
ALTER TABLE users ADD COLUMN password_hash text;
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
ALTER TABLE users ADD COLUMN totp_secret text;            -- set once 2FA is confirmed
ALTER TABLE users ADD COLUMN totp_pending text;           -- secret awaiting first code
ALTER TABLE users ADD COLUMN recovery_codes text[];       -- sha256 of unused codes
ALTER TABLE users ADD COLUMN is_staff boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN disabled_at timestamptz;
ALTER TABLE users ADD COLUMN disabled_reason text;
ALTER TABLE users ADD COLUMN birth_year integer;
ALTER TABLE users ADD COLUMN locale text NOT NULL DEFAULT 'en';
ALTER TABLE users ADD COLUMN deleted_at timestamptz;

ALTER TABLE sessions ADD COLUMN user_agent text;
ALTER TABLE sessions ADD COLUMN ip text;
ALTER TABLE sessions ADD COLUMN last_seen_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE sessions ADD COLUMN mfa_verified boolean NOT NULL DEFAULT true;

-- Single-use, hashed tokens: magic links, email verification, password reset, team invites.
CREATE TABLE auth_tokens (
  id            text PRIMARY KEY,                  -- sha256 of the token
  kind          text NOT NULL CHECK (kind IN ('magic','verify','reset','invite','mfa')),
  user_id       uuid REFERENCES users(id) ON DELETE CASCADE,
  email         text,
  data          jsonb,
  expires_at    timestamptz NOT NULL,
  used_at       timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX auth_tokens_user ON auth_tokens(user_id);

CREATE TABLE passkeys (
  id            text PRIMARY KEY,                  -- credential id (base64url)
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  public_key    bytea NOT NULL,
  counter       bigint NOT NULL DEFAULT 0,
  transports    text[],
  name          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  last_used_at  timestamptz
);
CREATE INDEX passkeys_user ON passkeys(user_id);

CREATE TABLE oauth_accounts (
  provider      text NOT NULL,
  subject       text NOT NULL,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email         text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (provider, subject)
);

-- ---------------------------------------------------------------- teams
ALTER TABLE workspaces ADD COLUMN owner_id uuid REFERENCES users(id);
ALTER TABLE memberships DROP CONSTRAINT memberships_role_check;
ALTER TABLE memberships ADD CONSTRAINT memberships_role_check CHECK (role IN ('owner','admin','designer','member'));
ALTER TABLE memberships ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();

-- ---------------------------------------------------------------- durable sync (ack-after-persist op log)
CREATE TABLE doc_updates (
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  seq           bigserial,
  epoch         integer NOT NULL DEFAULT 0,
  update        bytea NOT NULL,
  actor_id      uuid,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (design_id, seq)
);
ALTER TABLE doc_state ADD COLUMN seq bigint NOT NULL DEFAULT 0;   -- last update folded into the snapshot

-- ---------------------------------------------------------------- jobs (Postgres queue: SKIP LOCKED + LISTEN/NOTIFY)
CREATE TABLE jobs (
  id            uuid PRIMARY KEY,
  queue         text NOT NULL,
  payload       jsonb NOT NULL DEFAULT '{}',
  status        text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','done','failed','cancelled')),
  priority      integer NOT NULL DEFAULT 0,
  run_at        timestamptz NOT NULL DEFAULT now(),
  attempts      integer NOT NULL DEFAULT 0,
  max_attempts  integer NOT NULL DEFAULT 3,
  progress      real NOT NULL DEFAULT 0,
  label         text,
  result        jsonb,
  error         text,
  owner_id      uuid,
  locked_at     timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  finished_at   timestamptz
);
CREATE INDEX jobs_pick ON jobs(queue, status, run_at, priority DESC);

-- ---------------------------------------------------------------- email outbox
CREATE TABLE email_outbox (
  id            uuid PRIMARY KEY,
  to_email      text NOT NULL,
  subject       text NOT NULL,
  html          text NOT NULL,
  text          text NOT NULL,
  category      text NOT NULL,
  status        text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','sent','failed','suppressed')),
  error         text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  sent_at       timestamptz
);
CREATE INDEX email_outbox_to ON email_outbox(to_email, created_at DESC);

CREATE TABLE email_preferences (
  user_id       uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  mentions      boolean NOT NULL DEFAULT true,
  comments      boolean NOT NULL DEFAULT true,
  shares        boolean NOT NULL DEFAULT true,
  product       boolean NOT NULL DEFAULT false
);

-- ---------------------------------------------------------------- trust & safety
CREATE TABLE hash_blocklist (
  hash          text PRIMARY KEY,                  -- 64-bit perceptual hash, hex
  source        text NOT NULL,                     -- 'ncmec', 'stopncii', 'internal', …
  category      text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE assets ADD COLUMN phash text;
ALTER TABLE assets ADD COLUMN moderation text NOT NULL DEFAULT 'ok' CHECK (moderation IN ('ok','pending','quarantined','removed'));

CREATE TABLE reports (
  id            uuid PRIMARY KEY,
  target_type   text NOT NULL CHECK (target_type IN ('design','asset','comment','user','public_link')),
  target_id     text NOT NULL,
  reason        text NOT NULL,                     -- csam, ncii, violence, hate, spam, phishing, copyright, impersonation, other
  details       text,
  reporter_id   uuid REFERENCES users(id) ON DELETE SET NULL,
  reporter_email text,
  -- DSA notice fields / DMCA fields
  legal         jsonb,
  status        text NOT NULL DEFAULT 'open' CHECK (status IN ('open','actioned','dismissed')),
  priority      integer NOT NULL DEFAULT 0,
  decision      text,
  decided_by    uuid REFERENCES users(id),
  decided_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX reports_open ON reports(status, priority DESC, created_at);

-- Tamper-evident audit: each row hashes the previous one.
ALTER TABLE audit_events ADD COLUMN prev_hash text;
ALTER TABLE audit_events ADD COLUMN hash text;
ALTER TABLE audit_events ADD COLUMN ip text;
