-- Sharing (plan 11, batch 2): team groups as share targets (g149), an explicit team access level and a
-- custom public link slug (g131), and template links that hand out copies (g136).

-- Named member groups inside a team. Only current members of the team can be in its groups; access
-- through a group also requires the person to still be a member of the group's team.
CREATE TABLE team_groups (
  id            uuid PRIMARY KEY,
  workspace_id  uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  name          text NOT NULL,
  created_by    uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX team_groups_name ON team_groups(workspace_id, lower(name));

CREATE TABLE team_group_members (
  group_id      uuid NOT NULL REFERENCES team_groups(id) ON DELETE CASCADE,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (group_id, user_id)
);
CREATE INDEX team_group_members_user ON team_group_members(user_id);

CREATE TABLE design_group_shares (
  design_id     text NOT NULL REFERENCES designs(id) ON DELETE CASCADE,
  group_id      uuid NOT NULL REFERENCES team_groups(id) ON DELETE CASCADE,
  role          text NOT NULL CHECK (role IN ('view','comment','edit')),
  created_by    uuid REFERENCES users(id) ON DELETE SET NULL,
  created_at    timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (design_id, group_id)
);
CREATE INDEX design_group_shares_group ON design_group_shares(group_id);

-- Team designs: one level for every non-admin member ("<Team> can view/comment/edit"); NULL keeps the
-- role-based default (designers edit, members comment). Owners and admins always keep full control.
ALTER TABLE designs ADD COLUMN team_access text CHECK (team_access IN ('view','comment','edit'));

-- Custom slug for the public view link (/view/<slug>); lower-case, never shaped like a design id.
ALTER TABLE designs ADD COLUMN public_slug text;
CREATE UNIQUE INDEX designs_public_slug ON designs(public_slug) WHERE public_slug IS NOT NULL;

-- Template link (/t/<token>): whoever opens it gets their own copy, never access to the original.
ALTER TABLE designs ADD COLUMN template_token text;
ALTER TABLE designs ADD COLUMN template_uses integer NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX designs_template_token ON designs(template_token) WHERE template_token IS NOT NULL;
