-- Comments as tasks (CO-030): assignee + due date, emoji reactions, edits.
ALTER TABLE comments ADD COLUMN assignee_id uuid REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE comments ADD COLUMN due_on date;
ALTER TABLE comments ADD COLUMN resolved_at timestamptz;
ALTER TABLE comments ADD COLUMN edited_at timestamptz;
CREATE INDEX comments_assignee ON comments(assignee_id) WHERE assignee_id IS NOT NULL AND NOT resolved;

CREATE TABLE comment_reactions (
  comment_id    uuid NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  emoji         text NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (comment_id, user_id, emoji)
);

ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check CHECK (kind IN ('mention','comment','share','reply','assign'));
