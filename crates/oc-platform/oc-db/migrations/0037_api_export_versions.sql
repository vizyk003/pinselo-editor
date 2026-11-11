-- REST API exports that publish designs unattended (docs/rest-api.md): design content versions, API tokens limited
-- to one workspace, and JPEG exports with their public links.

-- A design's content version: 1 when created, +1 for every change written to the op log (editor, AI assistants,
-- brand template updates, load-time repairs). An export can be pinned to a version and is refused once it moved on.
ALTER TABLE designs ADD COLUMN content_version bigint NOT NULL DEFAULT 1;

CREATE FUNCTION oc_bump_content_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  UPDATE designs SET content_version = content_version + 1 WHERE id = NEW.design_id;
  RETURN NULL;
END
$$;

CREATE TRIGGER doc_updates_content_version AFTER INSERT ON doc_updates
  FOR EACH ROW EXECUTE FUNCTION oc_bump_content_version();

-- A personal access token can be limited to the designs of one workspace (designs elsewhere are "not found").
-- Limited tokens work on the REST API only, not on the MCP endpoint.
ALTER TABLE api_tokens ADD COLUMN workspace_id uuid REFERENCES workspaces(id) ON DELETE CASCADE;

-- JPEG exports of the REST API. `files`: [{ page, sha, width, height, bytes }] in the order asked for; the public
-- links are signed from the row (valid until `expires_at`). `idempotency_key` replays an export for 24 hours.
CREATE TABLE api_exports (
  id              text PRIMARY KEY,
  token_id        uuid NOT NULL REFERENCES api_tokens(id) ON DELETE CASCADE,
  user_id         uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  design_id       text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  design_version  bigint NOT NULL,
  idempotency_key text,
  request         jsonb NOT NULL,
  status          text NOT NULL DEFAULT 'rendering' CHECK (status IN ('rendering', 'done', 'failed')),
  files           jsonb,
  warnings        jsonb,
  error           jsonb,
  expires_at      timestamptz,
  created_at      timestamptz NOT NULL DEFAULT now(),
  finished_at     timestamptz
);
CREATE UNIQUE INDEX api_exports_idempotency ON api_exports(token_id, idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX api_exports_token ON api_exports(token_id, created_at);
