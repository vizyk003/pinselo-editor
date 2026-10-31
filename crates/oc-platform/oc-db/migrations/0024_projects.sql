-- Projects hub (plan 11 g17/g20): starred folders, and indexes for walking the folder tree and listing a folder.

CREATE TABLE folder_stars (
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  folder_id     uuid NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, folder_id)
);

CREATE INDEX folders_parent ON folders(parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX designs_folder ON designs(folder_id) WHERE folder_id IS NOT NULL;
