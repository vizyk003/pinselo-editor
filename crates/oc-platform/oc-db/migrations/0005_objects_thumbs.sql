-- Thumbnails live in the object store (S3-compatible in production) instead of the local disk.
ALTER TABLE designs ADD COLUMN thumb_sha256 text;
CREATE INDEX jobs_owner ON jobs(owner_id, created_at DESC);
