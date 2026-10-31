ALTER TABLE auth_tokens DROP CONSTRAINT auth_tokens_kind_check;
ALTER TABLE auth_tokens ADD CONSTRAINT auth_tokens_kind_check CHECK (kind IN ('magic','verify','reset','invite','mfa','billing'));
-- Premium templates (Pro/Teams).
ALTER TABLE templates ADD COLUMN premium boolean NOT NULL DEFAULT false;
