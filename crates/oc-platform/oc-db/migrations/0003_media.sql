-- Video/audio assets (MV-020/MV-030): proxies for playback, poster frames, waveform peaks.
ALTER TABLE assets DROP CONSTRAINT assets_kind_check;
ALTER TABLE assets ADD CONSTRAINT assets_kind_check CHECK (kind IN ('image','svg','video','audio'));
ALTER TABLE assets ADD COLUMN duration_ms integer;
ALTER TABLE assets ADD COLUMN has_audio boolean NOT NULL DEFAULT false;
ALTER TABLE assets ADD COLUMN proxy_sha256 text;    -- H.264/AAC MP4 (video) or AAC M4A (audio) used for playback + export
ALTER TABLE assets ADD COLUMN poster_sha256 text;   -- JPEG frame (video)
ALTER TABLE assets ADD COLUMN waveform jsonb;       -- normalized peaks for the timeline UI
