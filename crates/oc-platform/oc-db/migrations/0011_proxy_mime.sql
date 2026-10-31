-- Transparent animated GIFs get a VP9-with-alpha WebM proxy; others stay H.264 MP4 / AAC M4A.
ALTER TABLE assets ADD COLUMN proxy_mime text;
