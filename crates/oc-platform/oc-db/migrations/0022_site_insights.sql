-- Site insights (WS-010): first-party, cookie-free analytics for published websites and public view
-- links. Only daily aggregates are kept; unique visitors are counted with a hash of IP + user agent
-- salted per day (the salt can't be recomputed for past days), and those hashes are pruned after a day.
ALTER TABLE sites ADD COLUMN IF NOT EXISTS analytics boolean NOT NULL DEFAULT true;

CREATE TABLE insight_daily (
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  scope         text NOT NULL,              -- 'site' | 'view'
  day           date NOT NULL,
  kind          text NOT NULL,              -- view | visitor | referrer | country | device | section | click | form
  key           text NOT NULL DEFAULT '',
  n             integer NOT NULL DEFAULT 0,
  PRIMARY KEY (design_id, scope, day, kind, key)
);

CREATE TABLE insight_visitors (
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  scope         text NOT NULL,
  day           date NOT NULL,
  hash          text NOT NULL,
  PRIMARY KEY (design_id, scope, day, hash)
);
CREATE INDEX insight_visitors_day ON insight_visitors(day);
