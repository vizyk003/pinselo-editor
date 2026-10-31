-- Google Fonts on demand (FT-030): each face is fetched once, then pinned in the object store so the
-- browser, the renderer and every export use byte-identical files from then on.
CREATE TABLE google_font_files (
  file          text PRIMARY KEY,                -- 'gf/<slug>/<weight>[i].ttf'
  sha256        text NOT NULL,
  bytes         integer NOT NULL,
  source_url    text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
