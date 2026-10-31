-- Accounts created before email verification existed (dev sign-in) count as verified.
UPDATE users SET email_verified_at = created_at WHERE email_verified_at IS NULL AND password_hash IS NULL;
