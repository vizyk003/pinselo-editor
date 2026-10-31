-- Forms (FM-001) and published websites (WS-001).
CREATE TABLE form_responses (
  id            uuid PRIMARY KEY,
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  element_id    text NOT NULL,
  data          jsonb NOT NULL,
  ip_hash       text,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX form_responses_design ON form_responses(design_id, element_id, created_at DESC);

CREATE TABLE sites (
  design_id     text PRIMARY KEY REFERENCES designs(id) ON DELETE CASCADE,
  slug          text NOT NULL UNIQUE,
  title         text NOT NULL,
  description   text,
  published     boolean NOT NULL DEFAULT false,
  password_hash text,
  noindex       boolean NOT NULL DEFAULT false,
  html_sha256   text,                       -- rendered site HTML in the object store
  version       integer NOT NULL DEFAULT 0,
  published_at  timestamptz,
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE site_domains (
  domain        text PRIMARY KEY,
  design_id     text NOT NULL REFERENCES sites(design_id) ON DELETE CASCADE,
  verified_at   timestamptz,
  token         text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE site_views (
  design_id     text NOT NULL REFERENCES sites(design_id) ON DELETE CASCADE,
  day           date NOT NULL,
  views         integer NOT NULL DEFAULT 0,
  PRIMARY KEY (design_id, day)
);
