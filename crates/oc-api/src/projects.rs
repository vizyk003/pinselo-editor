//! Port of services/api/src/projects.ts — Projects hub (plan 11 g17/g18/g20): who may see and change a folder,
//! the folder tree (subfolders, moves without cycles) and the sort orders of the design lists.

use serde_json::json;
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::http::reply::{self, Reply};
use crate::state::AppState;

pub const DESIGN_SORTS: &[&str] = &["edited", "oldest", "az", "za"];

/// `designOrder(sort)`: ORDER BY for a design list; ties fall back to the id.
pub fn design_order(sort: &str) -> &'static str {
    match sort {
        "oldest" => "d.updated_at ASC, d.id",
        "az" => "lower(d.title) ASC, d.updated_at DESC, d.id",
        "za" => "lower(d.title) DESC, d.updated_at DESC, d.id",
        _ => "d.updated_at DESC, d.id",
    }
}

/// `IN_MY_TEAM`: `d` belongs to a team the user ($1) is in.
pub const IN_MY_TEAM: &str = "EXISTS (SELECT 1 FROM memberships mm JOIN workspaces ww ON ww.id = mm.workspace_id
  WHERE mm.workspace_id = d.workspace_id AND mm.user_id = $1 AND NOT ww.personal)";

/// `subtreeSql(p)`: ids of a folder and all its subfolders. UNION stops on a cycle.
pub fn subtree_sql(p: &str) -> String {
    format!("(WITH RECURSIVE sub(id) AS (SELECT {p}::uuid UNION SELECT f.id FROM folders f JOIN sub ON f.parent_id = sub.id) SELECT id FROM sub)")
}

#[derive(Debug, Clone)]
pub struct FolderAccess {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub owner_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub personal: bool,
    /// The user's role in the folder's team; `None` for a personal folder.
    pub role: Option<String>,
    pub manage: bool,
    pub add: bool,
    pub file: bool,
}

const ACCESS_SQL: &str = "SELECT f.id, f.workspace_id, f.owner_id, f.parent_id, w.personal, m.role
               FROM folders f JOIN workspaces w ON w.id = f.workspace_id
               LEFT JOIN memberships m ON m.workspace_id = f.workspace_id AND m.user_id = $1
               WHERE f.id = $2";

type AccessRow = (Uuid, Uuid, Uuid, Option<Uuid>, bool, Option<String>);

fn access_of(user_id: Uuid, f: AccessRow) -> Option<FolderAccess> {
    let (id, workspace_id, owner_id, parent_id, personal, role) = f;
    let mine = owner_id == user_id;
    let role = if personal { None } else { role };
    if if personal { !mine } else { role.is_none() } {
        return None;
    }
    let manage = if personal { mine } else { mine || matches!(role.as_deref(), Some("owner" | "admin")) };
    let add = if personal { mine } else { role.as_deref() != Some("member") };
    Some(FolderAccess { id, workspace_id, owner_id, parent_id, personal, role, manage, add, file: true })
}

/// `folderAccess(userId, folderId)`: what the user may do with a folder, or `None` when they can't see it.
pub async fn folder_access(s: &AppState, user_id: Uuid, folder_id: Uuid) -> Result<Option<FolderAccess>, sqlx::Error> {
    let f: Option<AccessRow> = sqlx::query_as(ACCESS_SQL).bind(user_id).bind(folder_id).fetch_optional(&s.pool).await?;
    Ok(f.and_then(|f| access_of(user_id, f)))
}

/// `inSubtree(folderId, targetId)`: `target` is the folder itself or one of its subfolders.
pub async fn in_subtree(c: &mut sqlx::PgConnection, folder_id: Uuid, target_id: Uuid) -> Result<bool, sqlx::Error> {
    let sql = format!("SELECT 1 FROM {} AS s WHERE s.id = $2::uuid LIMIT 1", subtree_sql("$1"));
    Ok(sqlx::query(&sql).bind(folder_id).bind(target_id).fetch_optional(c).await?.is_some())
}

/// `Failure`: `{ status, error, message? }`.
#[derive(Debug, Clone)]
pub struct Failure {
    pub status: u16,
    pub error: &'static str,
    pub message: Option<&'static str>,
}

impl Failure {
    pub const fn new(status: u16, error: &'static str) -> Failure {
        Failure { status, error, message: None }
    }
    pub const fn msg(status: u16, error: &'static str, message: &'static str) -> Failure {
        Failure { status, error, message: Some(message) }
    }
    /// `reply.code(status).send({ error })`.
    pub fn reply(&self) -> Reply {
        reply::json(self.status, json!({ "error": self.error }))
    }
    /// `reply.code(status).send({ error, message })` (an absent message is left out).
    pub fn reply_with_message(&self) -> Reply {
        match self.message {
            Some(m) => reply::json(self.status, json!({ "error": self.error, "message": m })),
            None => self.reply(),
        }
    }
}

/// `moveFolder(userId, folderId, parentId)`: under another folder of the same workspace, or to the top level.
pub async fn move_folder(s: &AppState, user_id: Uuid, folder_id: Uuid, parent_id: Option<Uuid>) -> Result<Option<Failure>, ApiError> {
    let Some(f) = folder_access(s, user_id, folder_id).await? else { return Ok(Some(Failure::new(404, "not_found"))) };
    if !f.manage {
        return Ok(Some(Failure::new(403, "forbidden")));
    }
    if parent_id == f.parent_id {
        return Ok(None);
    }
    if let Some(pid) = parent_id {
        let Some(p) = folder_access(s, user_id, pid).await? else { return Ok(Some(Failure::new(404, "parent_not_found"))) };
        if !p.add {
            return Ok(Some(Failure::msg(403, "forbidden", "You can’t add folders to that folder.")));
        }
        if p.workspace_id != f.workspace_id {
            return Ok(Some(Failure::msg(400, "cross_workspace", "Folders move within the same team (or within your own projects).")));
        }
    }
    let mut t = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(format!("oc-folders:{}", f.workspace_id)).execute(&mut *t).await?;
    if let Some(pid) = parent_id
        && in_subtree(&mut t, folder_id, pid).await?
    {
        t.commit().await?;
        return Ok(Some(Failure::msg(400, "cycle", "A folder can’t go inside itself or one of its subfolders.")));
    }
    sqlx::query("UPDATE folders SET parent_id = $2 WHERE id = $1").bind(folder_id).bind(parent_id).execute(&mut *t).await?;
    t.commit().await?;
    Ok(None)
}
