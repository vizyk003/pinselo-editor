-- Audit chain v2 hashes each event's own columns. Earlier rows become pre-chain history.
UPDATE audit_events SET prev_hash = NULL, hash = NULL;
