-- Wrong codes entered against an 'mfa' ticket; the API cancels the ticket at its limit.
ALTER TABLE auth_tokens ADD COLUMN failures integer NOT NULL DEFAULT 0;
