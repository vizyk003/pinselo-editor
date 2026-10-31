-- OpenCanvas baseline schema (docs/plan/06-backlog-mvp.md E22-03). Every tenant table carries workspace_id + cell_id.

CREATE TABLE users (
  id            uuid PRIMARY KEY,
  email         text NOT NULL UNIQUE,
  name          text NOT NULL,
  color         text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
  id            text PRIMARY KEY,               -- sha256 of the opaque cookie token
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at    timestamptz NOT NULL DEFAULT now(),
  expires_at    timestamptz NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE workspaces (
  id            uuid PRIMARY KEY,
  name          text NOT NULL,
  cell_id       text NOT NULL DEFAULT 'us1',
  personal      boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role          text NOT NULL CHECK (role IN ('owner','admin','member')),
  PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE folders (
  id            uuid PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  cell_id       text NOT NULL DEFAULT 'us1',
  owner_id      uuid NOT NULL REFERENCES users(id),
  parent_id     uuid REFERENCES folders(id) ON DELETE CASCADE,
  name          text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE designs (
  id            text PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  cell_id       text NOT NULL DEFAULT 'us1',
  owner_id      uuid NOT NULL REFERENCES users(id),
  folder_id     uuid REFERENCES folders(id) ON DELETE SET NULL,
  title         text NOT NULL DEFAULT 'Untitled design',
  surface       text NOT NULL DEFAULT 'graphic',
  width         double precision NOT NULL,
  height        double precision NOT NULL,
  unit          text NOT NULL DEFAULT 'px',
  page_count    integer NOT NULL DEFAULT 1,
  link_access   text NOT NULL DEFAULT 'restricted' CHECK (link_access IN ('restricted','view','comment','edit')),
  template_id   text,
  thumbnail     text,
  trashed_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX designs_owner ON designs(owner_id, updated_at DESC);
CREATE INDEX designs_title_fts ON designs USING gin (to_tsvector('simple', title));

CREATE TABLE design_shares (
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role          text NOT NULL CHECK (role IN ('view','comment','edit')),
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (design_id, user_id)
);

CREATE TABLE stars (
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, design_id)
);

-- Live document state (compacted Yjs update). Written by the collab server.
CREATE TABLE doc_state (
  design_id     text PRIMARY KEY REFERENCES designs(id) ON DELETE CASCADE,
  state         bytea NOT NULL,
  epoch         integer NOT NULL DEFAULT 0,
  updated_at    timestamptz NOT NULL DEFAULT now()
);

-- Version history: automatic snapshots + named versions.
CREATE TABLE design_versions (
  id            uuid PRIMARY KEY,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  state         bytea NOT NULL,
  label         text,
  created_by    uuid REFERENCES users(id),
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX design_versions_design ON design_versions(design_id, created_at DESC);

CREATE TABLE assets (
  id            text PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  cell_id       text NOT NULL DEFAULT 'us1',
  owner_id      uuid NOT NULL REFERENCES users(id),
  sha256        text NOT NULL,
  mime          text NOT NULL,
  kind          text NOT NULL CHECK (kind IN ('image','svg')),
  bytes         bigint NOT NULL,
  width         integer NOT NULL,
  height        integer NOT NULL,
  name          text,
  trashed_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX assets_owner ON assets(owner_id, created_at DESC);

CREATE TABLE comments (
  id            uuid PRIMARY KEY,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  parent_id     uuid REFERENCES comments(id) ON DELETE CASCADE,
  page_id       text,
  element_id    text,
  author_id     uuid NOT NULL REFERENCES users(id),
  body          text NOT NULL,
  resolved      boolean NOT NULL DEFAULT false,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX comments_design ON comments(design_id, created_at);

CREATE TABLE templates (
  id            text PRIMARY KEY,
  title         text NOT NULL,
  category      text NOT NULL,
  preset        text NOT NULL,
  tags          text[] NOT NULL DEFAULT '{}',
  search_text   text NOT NULL DEFAULT '',          -- title + category + tags, written by the seeder
  width         double precision NOT NULL,
  height        double precision NOT NULL,
  doc           jsonb NOT NULL,
  thumbnail     text,
  published     boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX templates_fts ON templates USING gin (to_tsvector('simple', search_text));

CREATE TABLE audit_events (
  id            bigserial PRIMARY KEY,
  workspace_id  uuid,
  actor_id      uuid,
  action        text NOT NULL,
  target        text,
  meta          jsonb,
  created_at    timestamptz NOT NULL DEFAULT now()
);
