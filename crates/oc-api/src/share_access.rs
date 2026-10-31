//! Port of services/api/src/shareAccess.ts — design access through teams and groups (g131, g149). `designRole`
//! (crate::auth::design_role) is the one place that resolves access; these helpers keep the team/group rules in
//! one spot so listings can reuse the SQL.

use oc_auth::Role;

/// `teamMemberDesignRole(teamRole, teamAccess)`: owners and admins control every team design; everyone else
/// gets the design's explicit team access level when set, otherwise their role's default (designers edit,
/// members comment).
pub fn team_member_design_role(team_role: &str, team_access: Option<&str>) -> Role {
    if team_role == "owner" || team_role == "admin" {
        return Role::Owner;
    }
    match team_access {
        Some("view") => Role::View,
        Some("comment") => Role::Comment,
        Some("edit") => Role::Edit,
        _ if team_role == "member" => Role::Comment,
        _ => Role::Edit,
    }
}

/// `SHARE_ROLES_SQL`: roles through direct shares and group shares ($1 = design id, $2 = user id). A group share
/// counts only while the person is in the group AND still a member of the group's team.
pub const SHARE_ROLES_SQL: &str = "
  SELECT role FROM design_shares WHERE design_id = $1 AND user_id = $2
  UNION ALL
  SELECT gs.role FROM design_group_shares gs
    JOIN team_group_members gm ON gm.group_id = gs.group_id AND gm.user_id = $2
    JOIN team_groups tg ON tg.id = gs.group_id
    JOIN memberships tm ON tm.workspace_id = tg.workspace_id AND tm.user_id = $2
  WHERE gs.design_id = $1";

/// `sharedWithSql(designCol, userParam)`: the design is shared with the user, directly or through a group.
pub fn shared_with_sql(design_col: &str, user_param: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM design_shares x WHERE x.design_id = {design_col} AND x.user_id = {user_param})
    OR EXISTS (SELECT 1 FROM design_group_shares xg
      JOIN team_group_members xgm ON xgm.group_id = xg.group_id AND xgm.user_id = {user_param}
      JOIN team_groups xtg ON xtg.id = xg.group_id
      JOIN memberships xtm ON xtm.workspace_id = xtg.workspace_id AND xtm.user_id = {user_param}
      WHERE xg.design_id = {design_col}))"
    )
}
