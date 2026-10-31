-- Only slices produced by the email exporter are publicly served by hash.
CREATE TABLE email_images (sha256 text PRIMARY KEY, created_at timestamptz NOT NULL DEFAULT now());
